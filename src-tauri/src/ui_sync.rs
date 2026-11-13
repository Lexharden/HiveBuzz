//! Avisos a la interfaz de que algo cambió (`goals`, `timers`, `poll`…), para que cada pantalla vuelva a pedir
//! sus datos solo cuando hace falta en lugar de consultarlos cada segundo.
//!
//! Dos fuentes:
//! - el [`OverlayHub`]: metas, timers, encuesta, ruleta, jarrón y Spotify ya publican ahí cada cambio para los
//!   overlays, así que basta con traducir el canal a un tema;
//! - sondas baratas en memoria (la cola de acciones, la conexión de Spotify) que se comparan cada medio segundo
//!   sin pasar por IPC: solo se avisa si cambiaron.
//!
//! Los avisos se agrupan cada [`FLUSH_EVERY`]. Mientras la ventana está oculta en la bandeja no se envía nada: los
//! temas pendientes se acumulan y llegan juntos al volver a mostrarla.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tokio::sync::broadcast::error::RecvError;

use crate::overlay::OverlayHub;

/// Evento de Tauri con los temas que cambiaron (`UiChanged`).
pub const EVT_UI_CHANGED: &str = "ui-changed";
/// Cada cuánto se envían los temas acumulados.
pub const FLUSH_EVERY: Duration = Duration::from_millis(250);
/// Cada cuánto se comparan las sondas.
pub const PROBE_EVERY: Duration = Duration::from_millis(500);

/// La ventana principal está visible (no oculta en la bandeja). La actualizan quienes la ocultan o la muestran.
static UI_VISIBLE: AtomicBool = AtomicBool::new(true);

pub fn set_ui_visible(visible: bool) {
    UI_VISIBLE.store(visible, Ordering::Relaxed);
}

pub fn ui_visible() -> bool {
    UI_VISIBLE.load(Ordering::Relaxed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Topic {
    Goals,
    Timers,
    Poll,
    Wheel,
    Jar,
    Spotify,
    Queue,
}

/// Todos los temas que pueden llegar por el hub (si el receptor se queda atrás, se avisan todos).
const HUB_TOPICS: [Topic; 6] = [Topic::Goals, Topic::Timers, Topic::Poll, Topic::Wheel, Topic::Jar, Topic::Spotify];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UiChanged {
    pub topics: Vec<Topic>,
}

/// Tema de la interfaz que corresponde a un canal del hub (los canales de configuración y alertas no interesan).
pub fn topic_for_channel(channel: &str) -> Option<Topic> {
    Some(match channel {
        crate::goals::service::CHANNEL => Topic::Goals,
        crate::timers::service::CHANNEL => Topic::Timers,
        crate::interact::poll::CHANNEL => Topic::Poll,
        crate::interact::wheel::CHANNEL => Topic::Wheel,
        crate::jar::CHANNEL => Topic::Jar,
        crate::spotify::CHANNEL | crate::spotify::QUEUE_CHANNEL => Topic::Spotify,
        _ => return None,
    })
}

/// Destino de los avisos (la ventana principal; en los tests, un vector).
pub trait UiSink: Send + Sync + 'static {
    fn changed(&self, change: UiChanged);
}

/// Sonda: una huella barata del estado; si cambia entre dos lecturas, se avisa el tema.
pub struct Probe {
    pub topic: Topic,
    pub read: Box<dyn Fn() -> u64 + Send + Sync>,
}

/// Huella de cualquier valor con `Hash` (solo se compara dentro del mismo proceso).
pub fn fingerprint<T: std::hash::Hash>(value: &T) -> u64 {
    use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};
    BuildHasherDefault::<DefaultHasher>::default().hash_one(value)
}

/// Visibilidad de la ventana (en producción, [`ui_visible`]).
pub type Visible = Arc<dyn Fn() -> bool + Send + Sync>;

pub fn spawn(sink: Arc<dyn UiSink>, hub: &OverlayHub, probes: Vec<Probe>, visible: Visible) {
    let mut rx = hub.subscribe();
    tokio::spawn(async move {
        let mut pending = BTreeSet::new();
        let mut last: Vec<u64> = probes.iter().map(|p| (p.read)()).collect();
        let mut flush = tokio::time::interval(FLUSH_EVERY);
        let mut probe = tokio::time::interval(PROBE_EVERY);
        loop {
            tokio::select! {
                msg = rx.recv() => match msg {
                    Ok(m) => {
                        if let Some(t) = topic_for_channel(&m.channel) {
                            pending.insert(t);
                        }
                    }
                    Err(RecvError::Lagged(_)) => pending.extend(HUB_TOPICS),
                    Err(RecvError::Closed) => break,
                },
                _ = probe.tick() => {
                    for (p, seen) in probes.iter().zip(last.iter_mut()) {
                        let now = (p.read)();
                        if now != *seen {
                            *seen = now;
                            pending.insert(p.topic);
                        }
                    }
                }
                _ = flush.tick() => {
                    if !pending.is_empty() && visible() {
                        sink.changed(UiChanged { topics: std::mem::take(&mut pending).into_iter().collect() });
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicU64;
    use std::sync::Mutex;

    use serde_json::json;

    use super::*;

    #[derive(Default)]
    struct Recorder(Mutex<Vec<UiChanged>>);

    impl UiSink for Recorder {
        fn changed(&self, change: UiChanged) {
            self.0.lock().expect("lock").push(change);
        }
    }

    impl Recorder {
        fn take(&self) -> Vec<UiChanged> {
            std::mem::take(&mut *self.0.lock().expect("lock"))
        }
    }

    async fn settle() {
        // Deja correr la tarea y vence el siguiente envío agrupado.
        for _ in 0..3 {
            tokio::task::yield_now().await;
        }
        tokio::time::sleep(FLUSH_EVERY + Duration::from_millis(10)).await;
    }

    #[test]
    fn channels_map_to_topics() {
        assert_eq!(topic_for_channel("goals"), Some(Topic::Goals));
        assert_eq!(topic_for_channel("timer"), Some(Topic::Timers));
        assert_eq!(topic_for_channel("nowplaying"), Some(Topic::Spotify));
        assert_eq!(topic_for_channel("songqueue"), Some(Topic::Spotify));
        assert_eq!(topic_for_channel("config:chat"), None);
        assert_eq!(topic_for_channel("alerts"), None);
        assert_eq!(serde_json::to_value(UiChanged { topics: vec![Topic::Goals, Topic::Queue] }).expect("json"), json!({ "topics": ["goals", "queue"] }));
    }

    #[tokio::test(start_paused = true)]
    async fn hub_changes_are_grouped_and_probes_fire_only_on_change() {
        let hub = OverlayHub::new(16);
        let sink = Arc::new(Recorder::default());
        let queue = Arc::new(AtomicU64::new(0));
        let q = Arc::clone(&queue);
        spawn(sink.clone(), &hub, vec![Probe { topic: Topic::Queue, read: Box::new(move || q.load(Ordering::Relaxed)) }], Arc::new(|| true));
        settle().await;
        assert!(sink.take().is_empty(), "sin cambios no se avisa nada");

        hub.publish_retained("goals", json!({}));
        hub.publish_retained("goals", json!({}));
        hub.publish_retained("timer", json!({}));
        hub.publish("alerts", json!({}));
        settle().await;
        assert_eq!(sink.take(), vec![UiChanged { topics: vec![Topic::Goals, Topic::Timers] }], "agrupados y sin repetir");

        queue.store(3, Ordering::Relaxed);
        tokio::time::sleep(PROBE_EVERY).await;
        settle().await;
        assert_eq!(sink.take(), vec![UiChanged { topics: vec![Topic::Queue] }]);
        tokio::time::sleep(PROBE_EVERY * 2).await;
        assert!(sink.take().is_empty(), "la sonda solo avisa cuando cambia");
    }

    #[tokio::test(start_paused = true)]
    async fn nothing_is_sent_while_hidden_and_it_all_arrives_on_show() {
        let hub = OverlayHub::new(16);
        let sink = Arc::new(Recorder::default());
        let shown = Arc::new(AtomicBool::new(true));
        let s = Arc::clone(&shown);
        spawn(sink.clone(), &hub, Vec::new(), Arc::new(move || s.load(Ordering::Relaxed)));
        settle().await;
        shown.store(false, Ordering::Relaxed);
        hub.publish_retained("jar", json!({}));
        hub.publish_retained("poll", json!({}));
        settle().await;
        let hidden = sink.take();
        shown.store(true, Ordering::Relaxed);
        settle().await;
        assert!(hidden.is_empty(), "oculta no se envía");
        assert_eq!(sink.take(), vec![UiChanged { topics: vec![Topic::Poll, Topic::Jar] }]);
    }
}
