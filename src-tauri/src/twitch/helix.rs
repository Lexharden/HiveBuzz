//! Cliente mínimo de la API Helix de Twitch (solo lo que hace falta).

use std::sync::Arc;
use std::time::Duration;

use reqwest::StatusCode;
use serde_json::{json, Value};

use super::auth::{AuthError, TwitchAuth};

pub const HELIX_BASE: &str = "https://api.twitch.tv/helix";

pub struct Helix {
    http: reqwest::Client,
    auth: Arc<TwitchAuth>,
    base: String,
}

impl Helix {
    pub fn new(auth: Arc<TwitchAuth>) -> Result<Self, AuthError> {
        Self::with_base(auth, HELIX_BASE)
    }

    pub fn with_base(auth: Arc<TwitchAuth>, base: &str) -> Result<Self, AuthError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(12))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| AuthError::Other(e.to_string()))?;
        Ok(Self { http, auth, base: base.trim_end_matches('/').to_string() })
    }

    /// Petición con el token del streamer; si Twitch responde 401 renueva una vez y reintenta.
    async fn send(&self, method: reqwest::Method, path: &str, body: Option<&Value>) -> Result<(StatusCode, Value), AuthError> {
        for attempt in 0..2 {
            let token = self.auth.access_token().await?;
            let mut req = self
                .http
                .request(method.clone(), format!("{}{path}", self.base))
                .header(reqwest::header::AUTHORIZATION, format!("Bearer {token}"))
                .header("Client-Id", self.auth.client_id());
            if let Some(b) = body {
                req = req.header(reqwest::header::CONTENT_TYPE, "application/json").body(b.to_string());
            }
            let resp = req.send().await.map_err(|e| AuthError::Other(format!("no se pudo contactar con Twitch: {}", e.without_url())))?;
            let status = resp.status();
            let text = resp.text().await.map_err(|e| AuthError::Other(e.without_url().to_string()))?;
            if status == StatusCode::UNAUTHORIZED && attempt == 0 {
                self.auth.invalidate_access();
                continue;
            }
            return Ok((status, serde_json::from_str(&text).unwrap_or(Value::Null)));
        }
        Err(AuthError::NotLoggedIn)
    }

    fn fail(status: StatusCode, body: &Value) -> AuthError {
        let msg = body.get("message").and_then(Value::as_str).unwrap_or("");
        AuthError::Other(if msg.is_empty() { status.to_string() } else { format!("{status}: {msg}") })
    }

    /// Id numérico de un login (`None` si no existe).
    pub async fn user_id(&self, login: &str) -> Result<Option<String>, AuthError> {
        let (status, body) = self.send(reqwest::Method::GET, &format!("/users?login={}", crate::executors::percent_encode(login)), None).await?;
        if !status.is_success() {
            return Err(Self::fail(status, &body));
        }
        Ok(body.pointer("/data/0/id").and_then(Value::as_str).map(str::to_string))
    }

    /// Espectadores del directo, o `None` si el canal no está en directo.
    pub async fn stream_viewers(&self, user_id: &str) -> Result<Option<u64>, AuthError> {
        let (status, body) = self.send(reqwest::Method::GET, &format!("/streams?user_id={}", crate::executors::percent_encode(user_id)), None).await?;
        if !status.is_success() {
            return Err(Self::fail(status, &body));
        }
        Ok(body.pointer("/data/0").map(|s| s.get("viewer_count").and_then(Value::as_u64).unwrap_or(0)))
    }

    /// Suscribe esta sesión de EventSub a un tipo de evento.
    pub async fn subscribe(&self, session_id: &str, kind: &str, version: &str, condition: Value) -> Result<(), AuthError> {
        let body = json!({
            "type": kind, "version": version, "condition": condition,
            "transport": { "method": "websocket", "session_id": session_id }
        });
        let (status, resp) = self.send(reqwest::Method::POST, "/eventsub/subscriptions", Some(&body)).await?;
        if status.is_success() {
            Ok(())
        } else {
            Err(Self::fail(status, &resp))
        }
    }
}
