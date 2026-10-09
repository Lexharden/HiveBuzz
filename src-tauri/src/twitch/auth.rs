//! Inicio de sesión de Twitch con el flujo de **código de dispositivo**: la app muestra un código, el
//! streamer lo escribe en `twitch.tv/activate` y la app recibe el token. No hay redirección, ni servidor
//! local de por medio, ni secreto de cliente.
//!
//! Solo se pide `moderator:read:followers` (ver seguidores del propio canal). El refresh token vive en el
//! llavero del sistema; el access token, solo en memoria.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::KEY_TWITCH_REFRESH;
use crate::actions::clock::Clock;
use crate::executors::percent_encode;
use crate::secrets::SecretStore;

pub const ID_BASE: &str = "https://id.twitch.tv";
pub const SCOPES: &str = "moderator:read:followers";
const REFRESH_MARGIN_MS: i64 = 60 * 1000;
const DEFAULT_INTERVAL_S: u64 = 5;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum AuthError {
    #[error("no has iniciado sesión en Twitch")]
    NotLoggedIn,
    #[error("falta el Client ID de Twitch")]
    NoClientId,
    #[error("Twitch: {0}")]
    Other(String),
}

impl From<AuthError> for crate::error::AppError {
    fn from(e: AuthError) -> Self {
        crate::error::AppError::Invalid(e.to_string())
    }
}

/// Lo que se le muestra al streamer para que active la app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in_s: u64,
}

/// Estado del inicio de sesión, tal como lo ve la UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum LoginState {
    Idle,
    Pending { info: DeviceInfo },
    Done,
    Failed { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub login: String,
    pub user_id: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

#[derive(Deserialize)]
struct DeviceResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    interval: Option<u64>,
}

struct Access {
    token: String,
    expires_ms: i64,
}

struct Inner {
    access: Option<Access>,
    login: LoginState,
    /// Generación del intento de inicio de sesión: un intento nuevo (o cancelado) invalida al anterior.
    attempt: u64,
}

pub struct TwitchAuth {
    http: reqwest::Client,
    secrets: Arc<dyn SecretStore>,
    clock: Arc<dyn Clock>,
    id_base: String,
    client_id: RwLock<String>,
    inner: Mutex<Inner>,
    refresh_lock: tokio::sync::Mutex<()>,
    interval_override: Option<Duration>,
}

fn form(pairs: &[(&str, &str)]) -> String {
    pairs.iter().map(|(k, v)| format!("{k}={}", percent_encode(v))).collect::<Vec<_>>().join("&")
}

/// Texto de error de Twitch (`message`) si lo hay; nunca incluye tokens.
fn error_text(body: &str, status: reqwest::StatusCode) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("message").and_then(|m| m.as_str().map(str::to_string)))
        .unwrap_or_else(|| status.to_string())
}

impl TwitchAuth {
    pub fn new(secrets: Arc<dyn SecretStore>, clock: Arc<dyn Clock>) -> Result<Arc<Self>, AuthError> {
        Self::with_endpoint(secrets, clock, ID_BASE, None)
    }

    pub fn with_endpoint(secrets: Arc<dyn SecretStore>, clock: Arc<dyn Clock>, id_base: &str, interval_override: Option<Duration>) -> Result<Arc<Self>, AuthError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| AuthError::Other(e.to_string()))?;
        Ok(Arc::new(Self {
            http,
            secrets,
            clock,
            id_base: id_base.trim_end_matches('/').to_string(),
            client_id: RwLock::new(String::new()),
            inner: Mutex::new(Inner { access: None, login: LoginState::Idle, attempt: 0 }),
            refresh_lock: tokio::sync::Mutex::new(()),
            interval_override,
        }))
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn set_client_id(&self, id: &str) {
        let mut cur = self.client_id.write().unwrap_or_else(PoisonError::into_inner);
        if *cur != id {
            *cur = id.to_string();
            drop(cur);
            // Otro Client ID = otra app de Twitch: los tokens anteriores ya no sirven.
            self.lock().access = None;
        }
    }

    pub fn client_id(&self) -> String {
        self.client_id.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    pub fn is_logged_in(&self) -> bool {
        self.secrets.get(KEY_TWITCH_REFRESH).ok().flatten().is_some_and(|t| !t.is_empty())
    }

    pub fn login_state(&self) -> LoginState {
        self.lock().login.clone()
    }

    pub fn logout(&self) -> crate::error::Result<()> {
        let mut inner = self.lock();
        inner.access = None;
        inner.login = LoginState::Idle;
        inner.attempt += 1;
        drop(inner);
        self.secrets.delete(KEY_TWITCH_REFRESH)
    }

    pub fn cancel_login(&self) {
        let mut inner = self.lock();
        inner.attempt += 1;
        if matches!(inner.login, LoginState::Pending { .. }) {
            inner.login = LoginState::Idle;
        }
    }

    /// Empieza el inicio de sesión: devuelve el código a mostrar y sigue esperando en segundo plano.
    pub async fn begin_login(self: &Arc<Self>) -> Result<DeviceInfo, AuthError> {
        let client_id = self.client_id();
        if client_id.is_empty() {
            return Err(AuthError::NoClientId);
        }
        let body = form(&[("client_id", &client_id), ("scopes", SCOPES)]);
        let resp = self
            .http
            .post(format!("{}/oauth2/device", self.id_base))
            .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .map_err(|e| AuthError::Other(format!("no se pudo contactar con Twitch: {}", e.without_url())))?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| AuthError::Other(e.without_url().to_string()))?;
        if !status.is_success() {
            return Err(AuthError::Other(error_text(&text, status)));
        }
        let d: DeviceResponse = serde_json::from_str(&text).map_err(|_| AuthError::Other("respuesta de Twitch ilegible".into()))?;
        let info = DeviceInfo { user_code: d.user_code, verification_uri: d.verification_uri, expires_in_s: d.expires_in.unwrap_or(1800).clamp(30, 3600) };
        let attempt = {
            let mut inner = self.lock();
            inner.attempt += 1;
            inner.login = LoginState::Pending { info: info.clone() };
            inner.attempt
        };
        let this = Arc::clone(self);
        let interval = self.interval_override.unwrap_or_else(|| Duration::from_secs(d.interval.unwrap_or(DEFAULT_INTERVAL_S).clamp(1, 30)));
        let deadline = self.clock.now_ms() + i64::try_from(info.expires_in_s).unwrap_or(1800) * 1000;
        tokio::spawn(async move { this.poll_device(d.device_code, attempt, interval, deadline).await });
        Ok(info)
    }

    fn still_current(&self, attempt: u64) -> bool {
        self.lock().attempt == attempt
    }

    fn finish(&self, attempt: u64, state: LoginState) {
        let mut inner = self.lock();
        if inner.attempt == attempt {
            inner.login = state;
        }
    }

    async fn poll_device(self: Arc<Self>, device_code: String, attempt: u64, mut interval: Duration, deadline_ms: i64) {
        let client_id = self.client_id();
        loop {
            tokio::time::sleep(interval).await;
            if !self.still_current(attempt) {
                return;
            }
            if self.clock.now_ms() > deadline_ms {
                self.finish(attempt, LoginState::Failed { reason: "el código caducó; inténtalo de nuevo".into() });
                return;
            }
            let body = form(&[
                ("client_id", &client_id),
                ("scopes", SCOPES),
                ("device_code", &device_code),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ]);
            let resp = match self
                .http
                .post(format!("{}/oauth2/token", self.id_base))
                .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(body)
                .send()
                .await
            {
                Ok(r) => r,
                // Un fallo de red momentáneo no cancela el inicio de sesión.
                Err(_) => continue,
            };
            let status = resp.status();
            let Ok(text) = resp.text().await else { continue };
            if status.is_success() {
                match serde_json::from_str::<TokenResponse>(&text) {
                    Ok(t) if t.refresh_token.as_deref().is_some_and(|r| !r.is_empty()) => {
                        if let Err(e) = self.secrets.set(KEY_TWITCH_REFRESH, t.refresh_token.as_deref().unwrap_or("")) {
                            self.finish(attempt, LoginState::Failed { reason: format!("no se pudo guardar la sesión: {e}") });
                            return;
                        }
                        if self.still_current(attempt) {
                            self.store_access(&t);
                        }
                        self.finish(attempt, LoginState::Done);
                    }
                    _ => self.finish(attempt, LoginState::Failed { reason: "Twitch no devolvió una sesión válida".into() }),
                }
                return;
            }
            let why = error_text(&text, status);
            match why.as_str() {
                "authorization_pending" => {}
                "slow_down" => interval += Duration::from_secs(5),
                other => {
                    let reason = if other.contains("denied") { "se rechazó el acceso".to_string() } else { format!("Twitch: {other}") };
                    self.finish(attempt, LoginState::Failed { reason });
                    return;
                }
            }
        }
    }

    fn store_access(&self, t: &TokenResponse) {
        let expires = self.clock.now_ms() + t.expires_in.unwrap_or(3600).clamp(60, 24 * 3600) * 1000;
        self.lock().access = Some(Access { token: t.access_token.clone(), expires_ms: expires });
    }

    fn cached(&self) -> Option<String> {
        let now = self.clock.now_ms();
        self.lock().access.as_ref().filter(|a| a.expires_ms - REFRESH_MARGIN_MS > now).map(|a| a.token.clone())
    }

    /// Un access token válido; lo renueva con el refresh token si hace falta.
    pub async fn access_token(&self) -> Result<String, AuthError> {
        if let Some(t) = self.cached() {
            return Ok(t);
        }
        let _guard = self.refresh_lock.lock().await;
        if let Some(t) = self.cached() {
            return Ok(t);
        }
        let refresh = self
            .secrets
            .get(KEY_TWITCH_REFRESH)
            .map_err(|e| AuthError::Other(e.to_string()))?
            .filter(|t| !t.is_empty())
            .ok_or(AuthError::NotLoggedIn)?;
        let client_id = self.client_id();
        if client_id.is_empty() {
            return Err(AuthError::NoClientId);
        }
        let body = form(&[("grant_type", "refresh_token"), ("refresh_token", &refresh), ("client_id", &client_id)]);
        let resp = self
            .http
            .post(format!("{}/oauth2/token", self.id_base))
            .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .map_err(|e| AuthError::Other(format!("no se pudo contactar con Twitch: {}", e.without_url())))?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| AuthError::Other(e.without_url().to_string()))?;
        if !status.is_success() {
            let why = error_text(&text, status);
            // Sesión revocada o caducada: se olvida para que la UI pida entrar de nuevo.
            if status == reqwest::StatusCode::BAD_REQUEST || status == reqwest::StatusCode::UNAUTHORIZED {
                let _ = self.secrets.delete(KEY_TWITCH_REFRESH);
                self.lock().access = None;
                return Err(AuthError::NotLoggedIn);
            }
            return Err(AuthError::Other(why));
        }
        let t: TokenResponse = serde_json::from_str(&text).map_err(|_| AuthError::Other("respuesta de token ilegible".into()))?;
        if let Some(new_refresh) = t.refresh_token.as_deref().filter(|r| !r.is_empty() && *r != refresh) {
            self.secrets.set(KEY_TWITCH_REFRESH, new_refresh).map_err(|e| AuthError::Other(e.to_string()))?;
        }
        self.store_access(&t);
        Ok(t.access_token)
    }

    /// Descarta el access token en caché (Twitch respondió 401).
    pub fn invalidate_access(&self) {
        self.lock().access = None;
    }

    /// Comprueba el token (Twitch lo exige cada hora) y dice a quién pertenece.
    pub async fn validate(&self) -> Result<Account, AuthError> {
        let token = self.access_token().await?;
        let resp = self
            .http
            .get(format!("{}/oauth2/validate", self.id_base))
            .header(reqwest::header::AUTHORIZATION, format!("OAuth {token}"))
            .send()
            .await
            .map_err(|e| AuthError::Other(format!("no se pudo contactar con Twitch: {}", e.without_url())))?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| AuthError::Other(e.without_url().to_string()))?;
        if status == reqwest::StatusCode::UNAUTHORIZED {
            self.invalidate_access();
            return Err(AuthError::NotLoggedIn);
        }
        if !status.is_success() {
            return Err(AuthError::Other(error_text(&text, status)));
        }
        let v: serde_json::Value = serde_json::from_str(&text).map_err(|_| AuthError::Other("respuesta ilegible".into()))?;
        let (Some(login), Some(user_id)) = (v.get("login").and_then(|x| x.as_str()), v.get("user_id").and_then(|x| x.as_str())) else {
            return Err(AuthError::Other("Twitch no dijo a quién pertenece la sesión".into()));
        };
        Ok(Account { login: login.to_ascii_lowercase(), user_id: user_id.to_string() })
    }
}

#[cfg(test)]
mod tests;
