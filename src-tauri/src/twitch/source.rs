//! `TwitchSource`: la fuente de eventos de Twitch (`LiveSource`).
//!
//! - **Siempre**: chat, bits y suscripciones por IRC anónimo (sin cuenta).
//! - **Con sesión iniciada en el propio canal**: además seguidores, estado del directo y espectadores.
//!
//! Resiliencia: si algo se cae, todo se reinicia con backoff exponencial y jitter; los mensajes que Twitch
//! reenvía al reconectar se descartan por id; si pasa mucho tiempo sin ver nada (ni siquiera un PING), se reconecta.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;

use super::auth::{AuthError, TwitchAuth};
use super::eventsub::{self, EsEnd, Notice, EVENTSUB_URL};
use super::helix::Helix;
use super::irc;
use super::Seen;
use crate::actions::clock::Clock;
use crate::error::Result;
use crate::source::protocol::ConnectionState;
use crate::source::sidecar::backoff_delay;
use crate::source::{ConnectTarget, LiveSource, SourceMessage, StatusUpdate};

pub const IRC_URL: &str = "wss://irc-ws.chat.twitch.tv:443";

/// Direcciones (cambiables en las pruebas).
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub irc_url: String,
    pub eventsub_url: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self { irc_url: IRC_URL.into(), eventsub_url: EVENTSUB_URL.into() }
    }
}

/// Tiempos (cambiables en las pruebas).
#[derive(Debug, Clone, Copy)]
pub struct Tuning {
    pub backoff_base: Duration,
    pub backoff_max: Duration,
    /// Una sesión que dure al menos esto se considera sana y reinicia el backoff.
    pub healthy_after: Duration,
    /// Sin recibir NADA (ni PING) durante este tiempo, se reconecta. Twitch envía PING cada ~5 min.
    pub idle_timeout: Duration,
    pub viewers_every: Duration,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            backoff_base: Duration::from_secs(1),
            backoff_max: Duration::from_secs(60),
            healthy_after: Duration::from_secs(30),
            idle_timeout: Duration::from_secs(6 * 60),
            viewers_every: Duration::from_secs(60),
        }
    }
}

/// Código de `detail` del estado conectado, para que la UI explique qué se está recibiendo.
pub const DETAIL_CHAT: &str = "chat";
pub const DETAIL_FULL: &str = "full";
pub const DETAIL_NOT_OWNER: &str = "notOwner";

#[derive(Clone)]
struct Ctx {
    tx: mpsc::Sender<SourceMessage>,
    auth: Arc<TwitchAuth>,
    helix: Arc<Helix>,
    endpoints: Endpoints,
    clock: Arc<dyn Clock>,
    tuning: Tuning,
}

pub struct TwitchSource {
    ctx: Ctx,
    task: Mutex<Option<JoinHandle<()>>>,
}

/// Aborta la tarea al salir de ámbito (así una sesión que termina no deja tareas huérfanas).
struct AbortOnDrop(JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl TwitchSource {
    pub fn new(tx: mpsc::Sender<SourceMessage>, auth: Arc<TwitchAuth>, helix: Arc<Helix>, clock: Arc<dyn Clock>) -> Self {
        Self::with_options(tx, auth, helix, clock, Endpoints::default(), Tuning::default())
    }

    pub fn with_options(
        tx: mpsc::Sender<SourceMessage>,
        auth: Arc<TwitchAuth>,
        helix: Arc<Helix>,
        clock: Arc<dyn Clock>,
        endpoints: Endpoints,
        tuning: Tuning,
    ) -> Self {
        Self { ctx: Ctx { tx, auth, helix, endpoints, clock, tuning }, task: Mutex::new(None) }
    }

    fn replace_task(&self, next: Option<JoinHandle<()>>) {
        let mut slot = self.task.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(old) = slot.take() {
            old.abort();
        }
        *slot = next;
    }
}

#[async_trait]
impl LiveSource for TwitchSource {
    async fn connect(&self, target: ConnectTarget) -> Result<()> {
        let ctx = self.ctx.clone();
        let channel = target.unique_id;
        let handle = tokio::spawn(async move { supervise(ctx, channel).await });
        self.replace_task(Some(handle));
        Ok(())
    }

    async fn disconnect(&self) -> Result<()> {
        self.replace_task(None);
        let _ = self.ctx.tx.send(SourceMessage::Status(StatusUpdate::new(ConnectionState::Disconnected))).await;
        let _ = self.ctx.tx.send(SourceMessage::Viewers(0)).await;
        Ok(())
    }
}

async fn status(ctx: &Ctx, state: ConnectionState, detail: Option<&str>) {
    let mut s = StatusUpdate::new(state);
    s.detail = detail.map(str::to_string);
    let _ = ctx.tx.send(SourceMessage::Status(s)).await;
}

/// Bucle exterior: sesión → fallo → backoff → sesión…
async fn supervise(ctx: Ctx, channel: String) {
    let mut seen = Seen::new(10_000);
    let mut attempt: u32 = 0;
    status(&ctx, ConnectionState::WaitingLive, Some("connecting")).await;
    loop {
        let started = Instant::now();
        let why = session(&ctx, &channel, &mut seen).await;
        if started.elapsed() >= ctx.tuning.healthy_after {
            attempt = 0;
        }
        attempt += 1;
        let delay = backoff_delay(attempt - 1, ctx.tuning.backoff_base, ctx.tuning.backoff_max, rand::random::<f64>());
        tracing::warn!(canal = %channel, motivo = %why, intento = attempt, "Twitch: reconectando");
        let mut s = StatusUpdate::new(ConnectionState::Reconnecting);
        s.detail = Some(why);
        s.attempt = Some(attempt);
        s.retry_in_ms = Some(u64::try_from(delay.as_millis()).unwrap_or(u64::MAX));
        let _ = ctx.tx.send(SourceMessage::Status(s)).await;
        tokio::time::sleep(delay).await;
    }
}

/// Una sesión de IRC (y, si procede, de EventSub). Devuelve el motivo por el que terminó.
async fn session(ctx: &Ctx, channel: &str, seen: &mut Seen) -> String {
    let (mut ws, _) = match tokio_tungstenite::connect_async(&ctx.endpoints.irc_url).await {
        Ok(c) => c,
        Err(e) => return format!("no se pudo conectar con el chat de Twitch: {e}"),
    };
    let nick = format!("justinfan{}", rand::random_range(10_000..100_000));
    for line in ["CAP REQ :twitch.tv/tags twitch.tv/commands".to_string(), "PASS SCHMOOPIIE".to_string(), format!("NICK {nick}"), format!("JOIN #{channel}")] {
        if ws.send(Message::text(line)).await.is_err() {
            return "se cortó al entrar al chat".into();
        }
    }
    // Con sesión iniciada en el propio canal se añaden seguidores y estado del directo.
    let (premium_tx, mut premium_rx) = mpsc::channel::<()>(1);
    let premium = AbortOnDrop({
        let (ctx, channel) = (ctx.clone(), channel.to_string());
        tokio::spawn(async move {
            premium_task(&ctx, &channel).await;
            let _ = premium_tx.send(()).await;
        })
    });
    let _ = &premium;
    let mut announced = false;
    let mut premium_open = true;

    loop {
        let msg = tokio::select! {
            m = timeout(ctx.tuning.idle_timeout, ws.next()) => match m {
                Err(_) => return "el chat de Twitch dejó de responder".into(),
                Ok(None) => return "Twitch cerró el chat".into(),
                Ok(Some(Err(e))) => return format!("chat de Twitch: {e}"),
                Ok(Some(Ok(m))) => m,
            },
            // Que termine la tarea de EventSub no tumba el chat.
            _ = premium_rx.recv(), if premium_open => {
                premium_open = false;
                continue;
            }
        };
        let text = match msg {
            Message::Text(t) => t.to_string(),
            Message::Close(_) => return "Twitch cerró el chat".into(),
            Message::Ping(p) => {
                let _ = ws.send(Message::Pong(p)).await;
                continue;
            }
            _ => continue,
        };
        for line in text.split("\r\n").filter(|l| !l.is_empty()) {
            let Some(m) = irc::parse(line) else { continue };
            match m.command.as_str() {
                "PING" => {
                    let reply = format!("PONG :{}", m.trailing.as_deref().unwrap_or("tmi.twitch.tv"));
                    if ws.send(Message::text(reply)).await.is_err() {
                        return "se cortó el chat de Twitch".into();
                    }
                }
                "RECONNECT" => return "Twitch pidió reconectar".into(),
                "NOTICE" if m.params.first().map(String::as_str) == Some("*") => {
                    return format!("Twitch rechazó la conexión: {}", m.trailing.as_deref().unwrap_or("sin detalle"));
                }
                // Confirmación de que se entró al canal (también llega con canales sin directo).
                "ROOMSTATE" | "366" if !announced => {
                    announced = true;
                    // Si hay sesión del dueño, el estado lo decide la tarea premium (directo o no).
                    if !ctx.auth.is_logged_in() {
                        status(ctx, ConnectionState::Connected, Some(DETAIL_CHAT)).await;
                    }
                }
                _ => {
                    for ev in irc::to_events(&m, ctx.clock.now_ms()) {
                        if seen.insert(&ev.id) && ctx.tx.send(SourceMessage::Event(Box::new(ev))).await.is_err() {
                            return "la aplicación se está cerrando".into();
                        }
                    }
                }
            }
        }
    }
}

/// Seguidores, estado del directo y espectadores. Termina sola si no hay sesión o no es el dueño del canal.
async fn premium_task(ctx: &Ctx, channel: &str) {
    if !ctx.auth.is_logged_in() {
        return;
    }
    let account = match ctx.auth.validate().await {
        Ok(a) => a,
        Err(AuthError::NotLoggedIn) => {
            status(ctx, ConnectionState::Connected, Some(DETAIL_CHAT)).await;
            return;
        }
        Err(e) => {
            tracing::warn!(error = %e, "Twitch: no se pudo validar la sesión; solo chat");
            status(ctx, ConnectionState::Connected, Some(DETAIL_CHAT)).await;
            return;
        }
    };
    if account.login != channel {
        status(ctx, ConnectionState::Connected, Some(DETAIL_NOT_OWNER)).await;
        return;
    }
    let uid = account.user_id;
    let mut notices = {
        let (ntx, nrx) = mpsc::channel::<Notice>(64);
        let (ctx2, uid2) = (ctx.clone(), uid.clone());
        let task = tokio::spawn(async move {
            let mut attempt: u32 = 0;
            loop {
                let started = Instant::now();
                let clock = Arc::clone(&ctx2.clock);
                let end = eventsub::run(&ctx2.endpoints.eventsub_url, &ctx2.helix, &uid2, ntx.clone(), move || clock.now_ms()).await;
                match end {
                    EsEnd::Revoked => {
                        tracing::warn!("Twitch revocó los permisos de EventSub");
                        return;
                    }
                    EsEnd::Dropped(why) => {
                        if started.elapsed() >= ctx2.tuning.healthy_after {
                            attempt = 0;
                        }
                        tracing::debug!(motivo = %why, "EventSub se cayó; se reintenta");
                        let delay = backoff_delay(attempt, ctx2.tuning.backoff_base, ctx2.tuning.backoff_max, rand::random::<f64>());
                        attempt += 1;
                        tokio::time::sleep(delay).await;
                    }
                }
            }
        });
        (nrx, AbortOnDrop(task))
    };

    refresh_stream(ctx, &uid).await;
    let mut tick = tokio::time::interval(ctx.tuning.viewers_every);
    tick.tick().await;
    loop {
        tokio::select! {
            _ = tick.tick() => refresh_stream(ctx, &uid).await,
            n = notices.0.recv() => match n {
                Some(Notice::Follow(ev)) => {
                    if ctx.tx.send(SourceMessage::Event(ev)).await.is_err() {
                        return;
                    }
                }
                Some(Notice::Online | Notice::Offline) => refresh_stream(ctx, &uid).await,
                None => return,
            },
        }
    }
}

/// Consulta si el canal está en directo y cuántos espectadores tiene; actualiza estado y contador.
async fn refresh_stream(ctx: &Ctx, uid: &str) {
    match ctx.helix.stream_viewers(uid).await {
        Ok(Some(n)) => {
            status(ctx, ConnectionState::Connected, Some(DETAIL_FULL)).await;
            let _ = ctx.tx.send(SourceMessage::Viewers(n)).await;
        }
        Ok(None) => {
            let _ = ctx.tx.send(SourceMessage::Viewers(0)).await;
            status(ctx, ConnectionState::WaitingLive, Some(DETAIL_FULL)).await;
        }
        Err(e) => tracing::debug!(error = %e, "Twitch: no se pudo consultar el directo"),
    }
}

#[cfg(test)]
mod tests;
