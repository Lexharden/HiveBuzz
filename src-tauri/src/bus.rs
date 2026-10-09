//! Bus de eventos: punto único por el que circulan los `LiveEvent` normalizados.
//! Lo consumen la UI, el WebSocket de overlays, el log en SQLite y (en fases
//! siguientes) el motor de reglas, los puntos, las metas y las estadísticas.

use std::sync::Arc;

use tokio::sync::broadcast;

use crate::events::LiveEvent;

#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<Arc<LiveEvent>>,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Publica un evento. Sin suscriptores no es un error.
    pub fn publish(&self, event: LiveEvent) {
        let _ = self.tx.send(Arc::new(event));
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Arc<LiveEvent>> {
        self.tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::testing::sample_event;

    #[tokio::test]
    async fn delivers_to_every_subscriber() {
        let bus = EventBus::new(8);
        let mut a = bus.subscribe();
        let mut b = bus.subscribe();
        bus.publish(sample_event("1"));
        assert_eq!(a.recv().await.expect("a").id, "1");
        assert_eq!(b.recv().await.expect("b").id, "1");
    }

    #[test]
    fn publishing_without_subscribers_is_fine() {
        EventBus::new(8).publish(sample_event("x"));
    }
}
