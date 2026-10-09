//! Contadores en vivo (likes y espectadores) para el overlay de contadores. También guarda el
//! pico de espectadores de la sesión, que usarán las estadísticas.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::broadcast::{self, error::RecvError};
use tokio::sync::watch;

use crate::bus::EventBus;
use crate::events::{EventType, LiveEvent};
use crate::overlay::OverlayHub;
use crate::session::SessionStarted;

/// Canal retenido que lee el overlay de contadores.
pub const CHANNEL: &str = "counters";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counters {
    /// Likes de la sala (el total que informa TikTok; si no lo informa, la suma de lo visto).
    pub likes: u64,
    pub viewers: u64,
    pub peak_viewers: u64,
}

impl Counters {
    pub fn on_event(&mut self, ev: &LiveEvent) -> bool {
        if ev.kind != EventType::Like {
            return false;
        }
        let Some(like) = &ev.like else { return false };
        let before = self.likes;
        // El total de la sala es monótono: nunca debe retroceder por un mensaje atrasado.
        self.likes = if like.total > 0 { self.likes.max(like.total) } else { self.likes.saturating_add(like.count) };
        self.likes != before
    }

    pub fn on_viewers(&mut self, n: u64) -> bool {
        let before = *self;
        self.viewers = n;
        self.peak_viewers = self.peak_viewers.max(n);
        *self != before
    }

    /// Sesión nueva: se reinician los likes y el pico (los espectadores actuales se conservan).
    pub fn on_new_session(&mut self) {
        self.likes = 0;
        self.peak_viewers = self.viewers;
    }
}

pub struct CounterService {
    state: Mutex<Counters>,
    dirty: AtomicBool,
    hub: OverlayHub,
}

impl CounterService {
    pub fn new(hub: OverlayHub) -> Arc<Self> {
        Arc::new(Self { state: Mutex::new(Counters::default()), dirty: AtomicBool::new(false), hub })
    }

    fn lock(&self) -> MutexGuard<'_, Counters> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn get(&self) -> Counters {
        *self.lock()
    }

    pub fn snapshot(&self) -> Value {
        json!(self.get())
    }

    fn publish(&self) {
        self.hub.publish_retained(CHANNEL, self.snapshot());
    }

    pub fn spawn(
        self: &Arc<Self>,
        bus: &EventBus,
        mut viewers: watch::Receiver<u64>,
        mut sessions: broadcast::Receiver<SessionStarted>,
        publish_every: Duration,
    ) {
        self.publish();
        let this = Arc::clone(self);
        let mut events = bus.subscribe();
        tokio::spawn(async move {
            let (mut sessions_open, mut viewers_open) = (true, true);
            loop {
                tokio::select! {
                    ev = events.recv() => match ev {
                        Ok(ev) => {
                            if this.lock().on_event(&ev) {
                                this.dirty.store(true, Ordering::Relaxed);
                            }
                        }
                        Err(RecvError::Lagged(_)) => {}
                        Err(RecvError::Closed) => break,
                    },
                    changed = viewers.changed(), if viewers_open => {
                        if changed.is_err() {
                            viewers_open = false;
                        } else {
                            let n = *viewers.borrow_and_update();
                            if this.lock().on_viewers(n) {
                                this.dirty.store(true, Ordering::Relaxed);
                            }
                        }
                    },
                    s = sessions.recv(), if sessions_open => match s {
                        Ok(_) => {
                            this.lock().on_new_session();
                            this.dirty.store(true, Ordering::Relaxed);
                        }
                        Err(RecvError::Lagged(_)) => {}
                        Err(RecvError::Closed) => sessions_open = false,
                    },
                }
            }
        });

        let this = Arc::clone(self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(publish_every);
            loop {
                tick.tick().await;
                if this.dirty.swap(false, Ordering::Relaxed) {
                    this.publish();
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::testing::sample_event;
    use crate::events::Like;
    use crate::session::SessionService;

    fn like(count: u64, total: u64) -> LiveEvent {
        let mut e = sample_event("l");
        e.kind = EventType::Like;
        e.chat = None;
        e.like = Some(Like { count, total });
        e
    }

    #[test]
    fn likes_follow_the_room_total_and_never_go_backwards() {
        let mut c = Counters::default();
        assert!(c.on_event(&like(5, 100)));
        assert_eq!(c.likes, 100);
        assert!(!c.on_event(&like(3, 90)), "un total atrasado no retrocede");
        assert!(c.on_event(&like(3, 130)));
        assert_eq!(c.likes, 130);
    }

    #[test]
    fn without_a_room_total_likes_are_summed() {
        let mut c = Counters::default();
        c.on_event(&like(5, 0));
        c.on_event(&like(7, 0));
        assert_eq!(c.likes, 12);
    }

    #[test]
    fn other_events_do_not_touch_likes() {
        let mut c = Counters::default();
        assert!(!c.on_event(&sample_event("chat")));
        assert_eq!(c.likes, 0);
    }

    #[test]
    fn viewers_track_the_peak() {
        let mut c = Counters::default();
        assert!(c.on_viewers(10));
        assert!(c.on_viewers(50));
        assert!(c.on_viewers(20));
        assert!(!c.on_viewers(20), "sin cambios");
        assert_eq!((c.viewers, c.peak_viewers), (20, 50));
    }

    #[test]
    fn a_new_session_resets_likes_and_peak_but_keeps_current_viewers() {
        let mut c = Counters::default();
        c.on_event(&like(1, 500));
        c.on_viewers(80);
        c.on_viewers(30);
        c.on_new_session();
        assert_eq!((c.likes, c.viewers, c.peak_viewers), (0, 30, 30));
    }

    #[test]
    fn snapshot_uses_camel_case_for_the_overlay() {
        let svc = CounterService::new(OverlayHub::new(4));
        svc.lock().on_viewers(9);
        let v = svc.snapshot();
        assert_eq!(v["viewers"], 9);
        assert_eq!(v["peakViewers"], 9);
        assert_eq!(v["likes"], 0);
    }

    #[tokio::test]
    async fn service_publishes_likes_viewers_and_resets_on_new_session() {
        let hub = OverlayHub::new(16);
        let svc = CounterService::new(hub.clone());
        let bus = EventBus::new(16);
        let (vtx, vrx) = watch::channel(0u64);
        let sessions = SessionService::new();
        svc.spawn(&bus, vrx, sessions.subscribe(), Duration::from_millis(15));
        assert!(hub.retained_for("counters").is_some(), "publica el estado inicial");

        bus.publish(like(4, 300));
        vtx.send_replace(42);
        for _ in 0..100 {
            if hub.retained_for("counters").is_some_and(|m| m.data["likes"] == 300 && m.data["viewers"] == 42) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
        let d = hub.retained_for("counters").expect("ret").data.clone();
        assert_eq!((d["likes"].clone(), d["viewers"].clone(), d["peakViewers"].clone()), (json!(300), json!(42), json!(42)));

        sessions.start_new();
        for _ in 0..100 {
            if hub.retained_for("counters").is_some_and(|m| m.data["likes"] == 0) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
        panic!("la sesión nueva no reinició los likes");
    }
}
