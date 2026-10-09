//! Servicio de conexión: orquesta una `LiveSource`, mantiene el estado visible en la UI y
//! vuelca los eventos al bus.

use std::sync::Arc;

use tokio::sync::{mpsc, watch};

use crate::bus::EventBus;
use crate::error::{AppError, Result};
use crate::events::Platform;
use crate::source::protocol::{ConnectionState, SessionPayload};
use crate::source::{ConnectTarget, LiveSource, SourceMessage, StatusUpdate};

pub struct ConnectionService {
    platform: Platform,
    source: Arc<dyn LiveSource>,
    status: watch::Sender<StatusUpdate>,
    viewers: watch::Sender<u64>,
}

impl ConnectionService {
    /// Conexión de TikTok.
    pub fn new(source: Arc<dyn LiveSource>) -> Arc<Self> {
        Self::with_platform(source, Platform::Tiktok)
    }

    pub fn with_platform(source: Arc<dyn LiveSource>, platform: Platform) -> Arc<Self> {
        let (status, _) = watch::channel(StatusUpdate::new(ConnectionState::Disconnected));
        let (viewers, _) = watch::channel(0);
        Arc::new(Self { platform, source, status, viewers })
    }

    pub fn platform(&self) -> Platform {
        self.platform
    }

    pub fn status(&self) -> StatusUpdate {
        self.status.borrow().clone()
    }

    pub fn subscribe_status(&self) -> watch::Receiver<StatusUpdate> {
        self.status.subscribe()
    }

    /// Espectadores conectados ahora mismo (0 si no hay LIVE).
    pub fn subscribe_viewers(&self) -> watch::Receiver<u64> {
        self.viewers.subscribe()
    }

    pub async fn connect(&self, raw_username: &str, euler_api_key: Option<String>, session: Option<SessionPayload>) -> Result<()> {
        let unique_id = match self.platform {
            Platform::Tiktok => normalize_username(raw_username)?,
            Platform::Twitch => crate::twitch::normalize_channel(raw_username)?,
        };
        self.source
            .connect(ConnectTarget {
                unique_id,
                euler_api_key,
                session,
            })
            .await
    }

    pub async fn disconnect(&self) -> Result<()> {
        self.source.disconnect().await
    }

    /// Escribe en el chat del LIVE (necesita sesión de TikTok).
    pub async fn send_chat(&self, text: String) -> Result<()> {
        self.source.send_chat(text).await
    }

    /// Consume los mensajes de la fuente: eventos → bus, estados → `watch`.
    pub fn spawn_pump(
        self: &Arc<Self>,
        mut rx: mpsc::Receiver<SourceMessage>,
        bus: EventBus,
    ) -> tokio::task::JoinHandle<()> {
        let this = Arc::clone(self);
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                match msg {
                    SourceMessage::Event(mut ev) => {
                        // La plataforma la pone la conexión, no la fuente: así ningún conector puede confundirse.
                        ev.platform = this.platform;
                        bus.publish(*ev);
                    }
                    SourceMessage::Status(s) => {
                        // Sin conexión no hay espectadores: el contador no debe quedarse congelado.
                        if matches!(s.state, ConnectionState::Disconnected | ConnectionState::WaitingLive) {
                            this.viewers.send_replace(0);
                        }
                        this.status.send_replace(s);
                    }
                    SourceMessage::Viewers(n) => {
                        this.viewers.send_replace(n);
                    }
                    // Los logs del sidecar ya se registraron con `tracing` al recibirlos.
                    SourceMessage::Log { .. } => {}
                }
            }
        })
    }
}

/// Acepta `@usuario`, `usuario` o una URL de perfil/LIVE y devuelve el `uniqueId` limpio.
/// Los usuarios de TikTok son letras, números, `_` y `.` (hasta 24 caracteres).
pub fn normalize_username(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    let candidate = match trimmed.find("tiktok.com/@") {
        Some(i) => trimmed[i + "tiktok.com/@".len()..]
            .split(['/', '?', '#'])
            .next()
            .unwrap_or(""),
        None => trimmed.trim_start_matches('@'),
    };
    let valid = !candidate.is_empty()
        && candidate.len() <= 24
        && candidate
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.');
    if valid {
        Ok(candidate.to_string())
    } else {
        Err(AppError::Invalid(
            "usuario de TikTok no válido (solo letras, números, '_' y '.', máximo 24)".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::testing::sample_event;
    use async_trait::async_trait;
    use std::sync::Mutex;

    #[test]
    fn normalizes_usernames() {
        assert_eq!(normalize_username("@ana_99").expect("ok"), "ana_99");
        assert_eq!(normalize_username("  ana.b ").expect("ok"), "ana.b");
        assert_eq!(
            normalize_username("https://www.tiktok.com/@ana/live?x=1").expect("ok"),
            "ana"
        );
        for bad in ["", "@", "a b", "ana/../x", "a".repeat(25).as_str(), "ñandú", "<script>"] {
            assert!(normalize_username(bad).is_err(), "{bad:?} debía fallar");
        }
    }

    #[derive(Default)]
    struct FakeSource {
        calls: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl LiveSource for FakeSource {
        async fn connect(&self, t: ConnectTarget) -> Result<()> {
            self.calls
                .lock()
                .expect("lock")
                .push(format!("connect {} key={}", t.unique_id, t.euler_api_key.is_some()));
            Ok(())
        }
        async fn disconnect(&self) -> Result<()> {
            self.calls.lock().expect("lock").push("disconnect".into());
            Ok(())
        }
    }

    #[tokio::test]
    async fn connect_validates_then_delegates() {
        let src = Arc::new(FakeSource::default());
        let svc = ConnectionService::new(src.clone());
        assert!(svc.connect("no valido!", None, None).await.is_err());
        svc.connect("@ana", Some("k".into()), None).await.expect("connect");
        svc.disconnect().await.expect("disconnect");
        assert_eq!(
            *src.calls.lock().expect("lock"),
            ["connect ana key=true", "disconnect"]
        );
    }

    #[tokio::test]
    async fn pump_tracks_viewers_and_zeroes_them_when_the_live_is_gone() {
        let svc = ConnectionService::new(Arc::new(FakeSource::default()));
        let mut viewers = svc.subscribe_viewers();
        let (tx, rx) = mpsc::channel(8);
        let _pump = svc.spawn_pump(rx, EventBus::new(8));
        tx.send(SourceMessage::Viewers(250)).await.expect("send");
        viewers.changed().await.expect("changed");
        assert_eq!(*viewers.borrow_and_update(), 250);
        tx.send(SourceMessage::Status(StatusUpdate::new(ConnectionState::Reconnecting))).await.expect("send");
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        assert_eq!(*viewers.borrow(), 250, "reconectando no borra el último dato conocido");
        tx.send(SourceMessage::Status(StatusUpdate::new(ConnectionState::WaitingLive))).await.expect("send");
        viewers.changed().await.expect("changed");
        assert_eq!(*viewers.borrow_and_update(), 0);
    }

    #[tokio::test]
    async fn pump_routes_events_to_bus_and_status_to_watch() {
        let svc = ConnectionService::new(Arc::new(FakeSource::default()));
        let bus = EventBus::new(8);
        let mut events = bus.subscribe();
        let mut status = svc.subscribe_status();
        let (tx, rx) = mpsc::channel(8);
        let _pump = svc.spawn_pump(rx, bus);

        assert_eq!(svc.status().state, ConnectionState::Disconnected);
        tx.send(SourceMessage::Status(StatusUpdate::new(ConnectionState::Connected)))
            .await
            .expect("send");
        status.changed().await.expect("changed");
        assert_eq!(status.borrow().state, ConnectionState::Connected);

        tx.send(SourceMessage::Event(Box::new(sample_event("e1"))))
            .await
            .expect("send");
        assert_eq!(events.recv().await.expect("event").id, "e1");
    }
}
