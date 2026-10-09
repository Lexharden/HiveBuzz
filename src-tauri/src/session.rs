//! Sesión de transmisión: qué es «este LIVE». Una sesión nueva empieza cuando la conexión vuelve a
//! estar activa después de que el LIVE anterior terminó (`liveEnd`), o a petición del usuario.
//! Los rankings «de la sesión» y las metas con reinicio automático se apoyan en esto.

use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::{broadcast, watch};

use crate::bus::EventBus;
use crate::events::EventType;
use crate::source::protocol::ConnectionState;
use crate::source::StatusUpdate;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionStarted {
    pub id: u64,
}

/// Lógica pura de transición de sesiones.
#[derive(Debug, Default)]
pub struct SessionTracker {
    ended: bool,
    id: u64,
}

impl SessionTracker {
    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn on_live_end(&mut self) {
        self.ended = true;
    }

    /// Devuelve `true` si este estado de conexión abre una sesión nueva.
    pub fn on_state(&mut self, state: ConnectionState) -> bool {
        if state == ConnectionState::Connected && self.ended {
            self.ended = false;
            self.id += 1;
            return true;
        }
        false
    }

    pub fn start_new(&mut self) -> u64 {
        self.ended = false;
        self.id += 1;
        self.id
    }
}

#[derive(Clone)]
pub struct SessionService {
    tracker: Arc<Mutex<SessionTracker>>,
    tx: broadcast::Sender<SessionStarted>,
}

impl Default for SessionService {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionService {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(16);
        Self {
            tracker: Arc::default(),
            tx,
        }
    }

    pub fn current_id(&self) -> u64 {
        self.lock().id()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SessionStarted> {
        self.tx.subscribe()
    }

    /// El usuario pide empezar de cero (botón «nueva sesión»).
    pub fn start_new(&self) -> u64 {
        let id = self.lock().start_new();
        let _ = self.tx.send(SessionStarted { id });
        id
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, SessionTracker> {
        self.tracker.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Vigila el bus (fin del LIVE) y el estado de conexión (reconexión).
    pub fn spawn(&self, bus: &EventBus, mut status: watch::Receiver<StatusUpdate>) {
        let this = self.clone();
        let mut events = bus.subscribe();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    ev = events.recv() => match ev {
                        Ok(ev) if ev.kind == EventType::LiveEnd => this.lock().on_live_end(),
                        Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                        Err(broadcast::error::RecvError::Closed) => break,
                    },
                    changed = status.changed() => {
                        if changed.is_err() {
                            break;
                        }
                        let state = status.borrow_and_update().state;
                        let started = {
                            let mut t = this.lock();
                            t.on_state(state).then(|| t.id())
                        };
                        if let Some(id) = started {
                            let _ = this.tx.send(SessionStarted { id });
                        }
                    }
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::testing::sample_event;

    #[test]
    fn first_connection_does_not_start_a_new_session() {
        let mut t = SessionTracker::default();
        assert!(!t.on_state(ConnectionState::Connected));
        assert_eq!(t.id(), 0);
    }

    #[test]
    fn reconnecting_after_a_live_end_starts_a_new_session_once() {
        let mut t = SessionTracker::default();
        t.on_live_end();
        assert!(!t.on_state(ConnectionState::WaitingLive), "esperar no cuenta");
        assert!(t.on_state(ConnectionState::Connected));
        assert!(!t.on_state(ConnectionState::Connected), "no se abre dos veces");
        assert_eq!(t.id(), 1);
    }

    #[test]
    fn a_dropped_connection_without_live_end_keeps_the_session() {
        let mut t = SessionTracker::default();
        for s in [ConnectionState::Reconnecting, ConnectionState::Connected, ConnectionState::Reconnecting, ConnectionState::Connected] {
            assert!(!t.on_state(s));
        }
        assert_eq!(t.id(), 0);
    }

    #[test]
    fn manual_start_always_works_and_clears_the_pending_end() {
        let mut t = SessionTracker::default();
        t.on_live_end();
        assert_eq!(t.start_new(), 1);
        assert!(!t.on_state(ConnectionState::Connected), "el fin pendiente ya se consumió");
        assert_eq!(t.start_new(), 2);
    }

    #[tokio::test]
    async fn service_announces_sessions_driven_by_the_bus_and_status() {
        let bus = EventBus::new(8);
        let (tx, rx) = watch::channel(StatusUpdate::new(ConnectionState::Disconnected));
        let svc = SessionService::new();
        svc.spawn(&bus, rx);
        let mut sub = svc.subscribe();

        let mut end = sample_event("end");
        end.kind = EventType::LiveEnd;
        bus.publish(end);
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        tx.send_replace(StatusUpdate::new(ConnectionState::Connected));

        let got = tokio::time::timeout(std::time::Duration::from_secs(2), sub.recv()).await.expect("a tiempo").expect("recv");
        assert_eq!(got, SessionStarted { id: 1 });
        assert_eq!(svc.current_id(), 1);
    }

    #[tokio::test]
    async fn manual_new_session_is_broadcast() {
        let svc = SessionService::new();
        let mut sub = svc.subscribe();
        assert_eq!(svc.start_new(), 1);
        assert_eq!(sub.recv().await.expect("recv"), SessionStarted { id: 1 });
    }
}
