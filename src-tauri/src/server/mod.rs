//! Servidor local (HTTP + WebSocket) en 127.0.0.1. Sirve los overlays para OBS /
//! TikTok LIVE Studio y emite los `LiveEvent` normalizados a quien se suscriba.
//!
//! Seguridad: solo escucha en loopback; toda petición exige el token (`?token=`); se
//! rechazan `Host` no locales (DNS rebinding) y `Origin` ajenos (páginas web de terceros).

pub mod api;
mod auth;
pub mod oauth;
mod pages;

pub use auth::constant_time_eq;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::{broadcast, oneshot};
use tower_http::services::ServeDir;

use crate::bus::EventBus;
use crate::error::{AppError, Result};
use crate::events::LiveEvent;
use crate::overlay::{OverlayHub, OverlayMessage, RecentEvents};

/// Política de contenido de las páginas: scripts/estilos en línea (los nuestros), imágenes y
/// video por HTTPS o del propio servidor, y conexión WS al propio servidor.
const CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; \
                   img-src 'self' https: data:; media-src 'self'; \
                   connect-src 'self' ws://127.0.0.1:* ws://localhost:*";

/// Todo lo que el servidor necesita para arrancar.
#[derive(Clone)]
pub struct ServerDeps {
    pub bus: EventBus,
    pub hub: OverlayHub,
    /// Últimos eventos, para el historial que reciben los overlays al conectarse.
    pub recent: RecentEvents,
    /// Carpeta de la biblioteca de medios, servida en `/media`.
    pub media_dir: PathBuf,
    pub token: String,
    /// API local para disparar reglas. Sin ella, los endpoints `/api/*` responden 503.
    pub api: Option<api::SharedApi>,
    /// Recibe la vuelta de Spotify tras autorizar (`/spotify/callback`).
    pub oauth: Option<oauth::SharedOAuth>,
}

#[derive(Clone)]
pub struct ServerState {
    pub bus: EventBus,
    pub hub: OverlayHub,
    pub recent: RecentEvents,
    pub token: Arc<str>,
    pub api: Option<api::SharedApi>,
    pub oauth: Option<oauth::SharedOAuth>,
}

/// Mensajes que el servidor envía por el WebSocket.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum Outgoing<'a> {
    Hello { version: u8 },
    /// Eventos recientes (de más viejo a más nuevo), para no abrir el overlay vacío.
    History { events: Vec<&'a LiveEvent> },
    Event { event: &'a LiveEvent },
    Overlay { channel: &'a str, data: &'a Value },
    Lagged { missed: u64 },
}

pub fn router(deps: &ServerDeps) -> Router {
    let state = ServerState {
        bus: deps.bus.clone(),
        hub: deps.hub.clone(),
        recent: deps.recent.clone(),
        token: Arc::from(deps.token.as_str()),
        api: deps.api.clone(),
        oauth: deps.oauth.clone(),
    };
    Router::new()
        .route("/overlay/{name}", get(overlay_page))
        .route("/ws", get(ws_upgrade))
        .merge(api::routes())
        .merge(oauth::routes())
        .nest_service("/media", ServeDir::new(&deps.media_dir))
        .layer(middleware::from_fn_with_state(state.clone(), auth::guard))
        .with_state(state)
}

async fn overlay_page(Path(name): Path<String>) -> Response {
    let Some(html) = pages::render(&name) else {
        return (StatusCode::NOT_FOUND, "overlay desconocido").into_response();
    };
    let mut res = Html(html).into_response();
    let h = res.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(CSP));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    res
}

async fn ws_upgrade(ws: WebSocketUpgrade, State(state): State<ServerState>) -> Response {
    // Primero se suscribe y después se toma la foto del estado: así no se pierde ningún cambio
    // entre ambos pasos (un duplicado es inofensivo; un hueco no).
    let events = state.bus.subscribe();
    let overlays = state.hub.subscribe();
    let retained = state.hub.retained();
    let history = state.recent.snapshot();
    ws.on_upgrade(move |socket| client_loop(socket, events, overlays, retained, history))
}

async fn client_loop(
    mut socket: WebSocket,
    mut events: broadcast::Receiver<Arc<LiveEvent>>,
    mut overlays: broadcast::Receiver<Arc<OverlayMessage>>,
    retained: Vec<Arc<OverlayMessage>>,
    history: Vec<Arc<LiveEvent>>,
) {
    if send(&mut socket, &Outgoing::Hello { version: 1 }).await.is_err() {
        return;
    }
    // Primero la configuración y el estado de metas/timers/ranking; después el historial.
    for m in &retained {
        if send(&mut socket, &Outgoing::Overlay { channel: &m.channel, data: &m.data }).await.is_err() {
            return;
        }
    }
    if !history.is_empty() {
        let refs: Vec<&LiveEvent> = history.iter().map(AsRef::as_ref).collect();
        if send(&mut socket, &Outgoing::History { events: refs }).await.is_err() {
            return;
        }
    }
    loop {
        tokio::select! {
            ev = events.recv() => {
                let sent = match ev {
                    Ok(ev) => send(&mut socket, &Outgoing::Event { event: &ev }).await,
                    // Un cliente lento no frena al resto: se le avisa de lo que se perdió.
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        send(&mut socket, &Outgoing::Lagged { missed: n }).await
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                if sent.is_err() {
                    break;
                }
            }
            msg = overlays.recv() => {
                let sent = match msg {
                    Ok(m) => send(&mut socket, &Outgoing::Overlay { channel: &m.channel, data: &m.data }).await,
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        send(&mut socket, &Outgoing::Lagged { missed: n }).await
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                if sent.is_err() {
                    break;
                }
            }
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                // Los pings se contestan solos; el resto de mensajes del cliente se ignora.
                Some(Ok(_)) => {}
            },
        }
    }
}

async fn send(socket: &mut WebSocket, msg: &Outgoing<'_>) -> std::result::Result<(), ()> {
    let json = serde_json::to_string(msg).map_err(|e| {
        tracing::error!(error = %e, "no se pudo serializar el mensaje WS");
    })?;
    socket.send(Message::text(json)).await.map_err(|_| ())
}

/// Servidor en marcha. Se apaga al soltarlo.
pub struct ServerHandle {
    pub addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

/// Arranca el servidor en `127.0.0.1:port` (0 = puerto libre al azar, útil en tests).
pub async fn start(deps: &ServerDeps, port: u16) -> Result<ServerHandle> {
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port)))
        .await
        .map_err(|e| AppError::Server(format!("no se pudo abrir 127.0.0.1:{port}: {e}")))?;
    let addr = listener.local_addr()?;
    let app = router(deps);
    let (tx, rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        let serve = axum::serve(listener, app).with_graceful_shutdown(async {
            let _ = rx.await;
        });
        if let Err(e) = serve.await {
            tracing::error!(error = %e, "el servidor local terminó con error");
        }
    });
    tracing::info!(%addr, "servidor local en marcha");
    Ok(ServerHandle {
        addr,
        shutdown: Some(tx),
    })
}

#[cfg(test)]
mod tests;
