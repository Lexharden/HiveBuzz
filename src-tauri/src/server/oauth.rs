//! Vuelta del navegador tras autorizar en Spotify: `GET /spotify/callback?code=…&state=…`.
//!
//! Es la única ruta del servidor que no pide el token de overlays (Spotify no puede enviarlo); se
//! protege con el `state` aleatorio de un solo uso de la autorización en curso. La página que se
//! devuelve es estática: nada de lo que llega en la URL se refleja en ella.

use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::{Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use serde::Deserialize;

use super::ServerState;
use crate::error::Result;
use crate::spotify::auth::SpotifyAuth;
use crate::spotify::CALLBACK_PATH;

#[async_trait]
pub trait OAuthCallback: Send + Sync {
    async fn spotify(&self, code: &str, state: &str) -> Result<()>;
}

pub type SharedOAuth = Arc<dyn OAuthCallback>;

#[async_trait]
impl OAuthCallback for SpotifyAuth {
    async fn spotify(&self, code: &str, state: &str) -> Result<()> {
        self.complete(code, state).await
    }
}

#[derive(Deserialize)]
struct CallbackParams {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

const PAGE: &str = "<!doctype html><html lang=\"es\"><head><meta charset=\"utf-8\"><title>HiveBuzz</title>\
<style>body{font-family:system-ui,sans-serif;background:#001a38;color:#fafafa;display:grid;place-items:center;height:100vh;margin:0}\
main{text-align:center;max-width:28rem;padding:1rem}h1{color:#ffc113;font-size:1.4rem}</style></head>\
<body><main><h1>HiveBuzz</h1><p>MESSAGE</p></main></body></html>";

fn page(status: StatusCode, message: &str) -> Response {
    let mut res = (status, axum::response::Html(PAGE.replace("MESSAGE", message))).into_response();
    let h = res.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'none'; style-src 'unsafe-inline'"));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    res
}

async fn callback(State(state): State<ServerState>, Query(p): Query<CallbackParams>) -> Response {
    let Some(oauth) = state.oauth.as_ref() else {
        return page(StatusCode::NOT_FOUND, "Esta función no está disponible.");
    };
    if p.error.is_some() {
        return page(StatusCode::BAD_REQUEST, "No se autorizó el acceso a Spotify. Puedes cerrar esta pestaña e intentarlo de nuevo desde HiveBuzz.");
    }
    let (Some(code), Some(st)) = (p.code.filter(|c| !c.is_empty() && c.len() < 2048), p.state.filter(|s| !s.is_empty() && s.len() < 256)) else {
        return page(StatusCode::BAD_REQUEST, "Faltan datos en la respuesta de Spotify.");
    };
    match oauth.spotify(&code, &st).await {
        Ok(()) => page(StatusCode::OK, "¡Spotify conectado! Ya puedes cerrar esta pestaña y volver a HiveBuzz."),
        Err(e) => {
            tracing::warn!(error = %e, "falló la autorización de Spotify");
            page(StatusCode::BAD_REQUEST, "No se pudo completar la conexión con Spotify. Vuelve a HiveBuzz e inténtalo de nuevo.")
        }
    }
}

pub fn routes() -> Router<ServerState> {
    Router::new().route(CALLBACK_PATH, get(callback))
}
