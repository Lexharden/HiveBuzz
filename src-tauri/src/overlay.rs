//! Canal hacia los overlays. Cualquier parte de la app (alertas, metas, timers…) publica
//! mensajes por canal y el servidor local los reenvía a los overlays conectados por WebSocket.
//!
//! Algunos canales son *retenidos*: guardan su último mensaje y se reenvía a cada overlay que
//! se conecta, de modo que un overlay recién abierto (o refrescado en OBS) muestra de inmediato
//! el estado actual de una meta, un timer o un ranking, sin esperar al próximo cambio.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::Value;
use tokio::sync::broadcast;

use crate::bus::EventBus;
use crate::events::{EventType, LiveEvent};

#[derive(Debug, Clone, PartialEq)]
pub struct OverlayMessage {
    /// Nombre lógico del destinatario: `alerts`, `goals`, `timer`, `config:chat`…
    pub channel: String,
    pub data: Value,
}

#[derive(Clone)]
pub struct OverlayHub {
    tx: broadcast::Sender<Arc<OverlayMessage>>,
    retained: Arc<Mutex<HashMap<String, Arc<OverlayMessage>>>>,
}

impl OverlayHub {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self {
            tx,
            retained: Arc::default(),
        }
    }

    /// Publica un mensaje efímero. Sin overlays conectados no es un error.
    pub fn publish(&self, channel: &str, data: Value) {
        let _ = self.tx.send(Arc::new(OverlayMessage {
            channel: channel.to_string(),
            data,
        }));
    }

    /// Publica y además recuerda el mensaje para quien se conecte después.
    pub fn publish_retained(&self, channel: &str, data: Value) {
        let msg = Arc::new(OverlayMessage {
            channel: channel.to_string(),
            data,
        });
        self.retained
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(channel.to_string(), Arc::clone(&msg));
        let _ = self.tx.send(msg);
    }

    /// Último mensaje de cada canal retenido (en orden estable por nombre).
    pub fn retained(&self) -> Vec<Arc<OverlayMessage>> {
        let mut all: Vec<_> = self
            .retained
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .cloned()
            .collect();
        all.sort_by(|a, b| a.channel.cmp(&b.channel));
        all
    }

    /// Último mensaje retenido de un canal.
    pub fn retained_for(&self, channel: &str) -> Option<Arc<OverlayMessage>> {
        self.retained
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(channel)
            .cloned()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Arc<OverlayMessage>> {
        self.tx.subscribe()
    }
}

/// Memoria de los últimos eventos relevantes, para que un overlay recién abierto muestre el
/// chat y los regalos recientes en vez de empezar vacío. Los likes y las entradas se omiten:
/// son tan frecuentes que desplazarían al resto.
#[derive(Clone)]
pub struct RecentEvents {
    inner: Arc<Mutex<VecDeque<Arc<LiveEvent>>>>,
    capacity: usize,
}

impl RecentEvents {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::default(),
            capacity,
        }
    }

    fn keeps(kind: EventType) -> bool {
        !matches!(kind, EventType::Like | EventType::Join)
    }

    pub fn push(&self, ev: Arc<LiveEvent>) {
        if !Self::keeps(ev.kind) {
            return;
        }
        let mut q = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        q.push_back(ev);
        while q.len() > self.capacity {
            q.pop_front();
        }
    }

    pub fn snapshot(&self) -> Vec<Arc<LiveEvent>> {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .cloned()
            .collect()
    }

    /// Alimenta la memoria desde el bus mientras haya bus.
    pub fn spawn(&self, bus: &EventBus) -> tokio::task::JoinHandle<()> {
        let this = self.clone();
        let mut rx = bus.subscribe();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(ev) => this.push(ev),
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::testing::sample_event;
    use serde_json::json;

    #[tokio::test]
    async fn subscribers_receive_channel_and_data() {
        let hub = OverlayHub::new(8);
        let mut rx = hub.subscribe();
        hub.publish("alerts", json!({"text": "hola"}));
        let m = rx.recv().await.expect("msg");
        assert_eq!(m.channel, "alerts");
        assert_eq!(m.data["text"], "hola");
    }

    #[test]
    fn publishing_without_listeners_is_fine() {
        OverlayHub::new(4).publish("x", json!(null));
        OverlayHub::new(4).publish_retained("x", json!(null));
    }

    #[test]
    fn plain_messages_are_not_retained() {
        let hub = OverlayHub::new(4);
        hub.publish("alerts", json!(1));
        assert!(hub.retained().is_empty());
    }

    #[test]
    fn retained_keeps_only_the_latest_per_channel() {
        let hub = OverlayHub::new(4);
        hub.publish_retained("goals", json!({"v": 1}));
        hub.publish_retained("timer", json!({"v": 7}));
        hub.publish_retained("goals", json!({"v": 2}));
        let all = hub.retained();
        assert_eq!(all.iter().map(|m| m.channel.as_str()).collect::<Vec<_>>(), ["goals", "timer"]);
        assert_eq!(all[0].data["v"], 2);
        assert_eq!(hub.retained_for("timer").expect("timer").data["v"], 7);
        assert!(hub.retained_for("nada").is_none());
    }

    #[tokio::test]
    async fn retained_messages_are_also_broadcast_live() {
        let hub = OverlayHub::new(4);
        let mut rx = hub.subscribe();
        hub.publish_retained("goals", json!({"v": 1}));
        assert_eq!(rx.recv().await.expect("msg").channel, "goals");
    }

    #[test]
    fn recent_events_keep_a_bounded_window_and_skip_noisy_types() {
        let recent = RecentEvents::new(3);
        for i in 0..5 {
            recent.push(Arc::new(sample_event(&format!("c{i}"))));
        }
        let mut like = sample_event("like");
        like.kind = EventType::Like;
        recent.push(Arc::new(like));
        let mut join = sample_event("join");
        join.kind = EventType::Join;
        recent.push(Arc::new(join));
        let ids: Vec<_> = recent.snapshot().iter().map(|e| e.id.clone()).collect();
        assert_eq!(ids, ["c2", "c3", "c4"]);
    }

    #[tokio::test]
    async fn recent_events_follow_the_bus() {
        let bus = EventBus::new(8);
        let recent = RecentEvents::new(10);
        let _task = recent.spawn(&bus);
        bus.publish(sample_event("x1"));
        for _ in 0..50 {
            if !recent.snapshot().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(recent.snapshot()[0].id, "x1");
    }
}
