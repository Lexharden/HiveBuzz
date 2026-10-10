//! EventSub por WebSocket (solo con sesión iniciada): seguidores y estado del directo.

use std::time::Duration;

use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;

use super::helix::Helix;
use super::{Seen, USER_ID_PREFIX};
use crate::events::{EventType, LiveEvent, Platform, User};

pub const EVENTSUB_URL: &str = "wss://eventsub.wss.twitch.tv/ws";
/// Margen sobre el `keepalive_timeout_seconds` que anuncia Twitch.
const KEEPALIVE_SLACK: Duration = Duration::from_secs(10);
const MAX_RECONNECTS_FOLLOWED: usize = 5;

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Lo que EventSub le cuenta al resto de la fuente.
#[derive(Debug, Clone, PartialEq)]
pub enum Notice {
    Follow(Box<LiveEvent>),
    Online,
    Offline,
}

/// Por qué terminó la sesión de EventSub.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EsEnd {
    /// Se cortó la conexión o faltó el keepalive: conviene reconectar.
    Dropped(String),
    /// Twitch revocó los permisos: hay que volver a iniciar sesión.
    Revoked,
}

/// Convierte una notificación de EventSub. `None` para tipos que no usamos.
pub fn map_notification(kind: &str, event: &Value, message_id: &str, now_ms: i64) -> Option<Notice> {
    match kind {
        "stream.online" => Some(Notice::Online),
        "stream.offline" => Some(Notice::Offline),
        "channel.follow" => {
            let uid = event.get("user_id")?.as_str().filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))?;
            let login = event.get("user_login")?.as_str().filter(|s| !s.is_empty())?;
            let name = event.get("user_name").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or(login);
            Some(Notice::Follow(Box::new(LiveEvent {
                id: format!("es-{message_id}"),
                platform: Platform::Twitch,
                kind: EventType::Follow,
                user: User {
                    id: format!("{USER_ID_PREFIX}{uid}"),
                    unique_id: login.to_ascii_lowercase(),
                    nickname: name.to_string(),
                    avatar: String::new(),
                    is_moderator: false,
                    is_subscriber: false,
                    is_follower: true,
                    team_level: None,
                    gifter_level: None,
                },
                gift: None,
                chat: None,
                like: None,
                ts: now_ms,
            })))
        }
        _ => None,
    }
}

/// Suscripciones que se piden al recibir `session_welcome`. El seguimiento exige ser el dueño del canal;
/// si Twitch lo rechaza se sigue con el estado del directo.
async fn subscribe_all(helix: &Helix, session_id: &str, broadcaster_id: &str) {
    let by_broadcaster = json!({ "broadcaster_user_id": broadcaster_id });
    for kind in ["stream.online", "stream.offline"] {
        if let Err(e) = helix.subscribe(session_id, kind, "1", by_broadcaster.clone()).await {
            tracing::warn!(error = %e, kind, "no se pudo suscribir a EventSub");
        }
    }
    let follow = json!({ "broadcaster_user_id": broadcaster_id, "moderator_user_id": broadcaster_id });
    if let Err(e) = helix.subscribe(session_id, "channel.follow", "2", follow).await {
        tracing::warn!(error = %e, "no se pudieron suscribir los seguidores (¿la sesión es del dueño del canal?)");
    }
}

/// Mensaje de la conexión que se está relevando: solo se atienden notificaciones y revocaciones.
async fn handle_old_message(text: &str, seen: &mut Seen, out: &mpsc::Sender<Notice>, now_ms: &impl Fn() -> i64) -> Option<EsEnd> {
    let v = serde_json::from_str::<Value>(text).ok()?;
    let kind = v.pointer("/metadata/message_type").and_then(Value::as_str).unwrap_or("");
    let mid = v.pointer("/metadata/message_id").and_then(Value::as_str).unwrap_or("");
    if !mid.is_empty() && !seen.insert(mid) {
        return None;
    }
    match kind {
        "notification" => {
            let sub = v.pointer("/payload/subscription/type").and_then(Value::as_str).unwrap_or("");
            let n = v.pointer("/payload/event").and_then(|e| map_notification(sub, e, mid, now_ms()))?;
            out.send(n).await.is_err().then(|| EsEnd::Dropped("la fuente se cerró".into()))
        }
        "revocation" => Some(EsEnd::Revoked),
        _ => None,
    }
}

/// Mantiene una sesión de EventSub hasta que se corta. `now_ms` da la hora para los eventos.
pub async fn run(url: &str, helix: &Helix, broadcaster_id: &str, out: mpsc::Sender<Notice>, now_ms: impl Fn() -> i64) -> EsEnd {
    let mut seen = Seen::new(2_000);
    let mut target = url.to_string();
    let mut reconnects = 0usize;
    let mut subscribed = false;
    // Tras un `session_reconnect`, la conexión vieja sigue viva (y se sigue leyendo) hasta que la
    // nueva da la bienvenida, como pide Twitch: así no se pierde nada durante el relevo.
    let mut handover: Option<Ws> = None;
    let mut keepalive = Duration::from_secs(40);
    'conn: loop {
        let (mut ws, _) = match tokio_tungstenite::connect_async(&target).await {
            Ok(c) => c,
            Err(e) => return EsEnd::Dropped(format!("no se pudo conectar con EventSub: {e}")),
        };
        let mut old = handover.take();
        loop {
            let (from_old, next) = match old.as_mut() {
                Some(o) => tokio::select! {
                    m = timeout(keepalive + KEEPALIVE_SLACK, ws.next()) => (false, m),
                    m = o.next() => (true, Ok(m)),
                },
                None => (false, timeout(keepalive + KEEPALIVE_SLACK, ws.next()).await),
            };
            if from_old {
                // La vieja se apaga sola; solo interesan sus notificaciones pendientes.
                match next {
                    Ok(Some(Ok(Message::Text(t)))) => {
                        if let Some(end) = handle_old_message(t.as_ref(), &mut seen, &out, &now_ms).await {
                            return end;
                        }
                    }
                    Ok(Some(Ok(_))) => {}
                    _ => old = None,
                }
                continue;
            }
            let msg = match next {
                Err(_) => return EsEnd::Dropped("EventSub dejó de enviar keepalive".into()),
                Ok(None) => return EsEnd::Dropped("EventSub cerró la conexión".into()),
                Ok(Some(Err(e))) => return EsEnd::Dropped(format!("EventSub: {e}")),
                Ok(Some(Ok(m))) => m,
            };
            let text = match msg {
                Message::Text(t) => t.to_string(),
                Message::Close(_) => return EsEnd::Dropped("EventSub cerró la conexión".into()),
                _ => continue,
            };
            let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
            let kind = v.pointer("/metadata/message_type").and_then(Value::as_str).unwrap_or("");
            let mid = v.pointer("/metadata/message_id").and_then(Value::as_str).unwrap_or("");
            if !mid.is_empty() && !seen.insert(mid) {
                continue;
            }
            match kind {
                "session_welcome" => {
                    // La nueva conexión ya recibe: la vieja se cierra al soltarla.
                    old = None;
                    if let Some(s) = v.pointer("/payload/session/keepalive_timeout_seconds").and_then(Value::as_u64) {
                        keepalive = Duration::from_secs(s.clamp(5, 600));
                    }
                    // En una reconexión pedida por Twitch las suscripciones se conservan.
                    if !subscribed {
                        if let Some(id) = v.pointer("/payload/session/id").and_then(Value::as_str) {
                            subscribe_all(helix, id, broadcaster_id).await;
                            subscribed = true;
                        }
                    }
                }
                "session_reconnect" => {
                    reconnects += 1;
                    let Some(next) = v.pointer("/payload/session/reconnect_url").and_then(Value::as_str) else {
                        return EsEnd::Dropped("EventSub pidió reconectar sin dirección".into());
                    };
                    if reconnects > MAX_RECONNECTS_FOLLOWED {
                        return EsEnd::Dropped("EventSub pidió reconectar demasiadas veces".into());
                    }
                    target = next.to_string();
                    handover = Some(ws);
                    continue 'conn;
                }
                "notification" => {
                    let sub = v.pointer("/payload/subscription/type").and_then(Value::as_str).unwrap_or("");
                    if let Some(n) = v.pointer("/payload/event").and_then(|e| map_notification(sub, e, mid, now_ms())) {
                        if out.send(n).await.is_err() {
                            return EsEnd::Dropped("la fuente se cerró".into());
                        }
                    }
                }
                "revocation" => return EsEnd::Revoked,
                _ => {}
            }
        }
    }
}
