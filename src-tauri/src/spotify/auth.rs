//! OAuth de Spotify con PKCE: URL de autorización, intercambio del código y renovación del token.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};
use std::time::Duration;

use serde::Deserialize;

use super::{pkce, ApiError, KEY_SPOTIFY_REFRESH, SCOPES};
use crate::actions::clock::Clock;
use crate::error::{AppError, Result};
use crate::executors::percent_encode;
use crate::secrets::SecretStore;

pub const AUTHORIZE_URL: &str = "https://accounts.spotify.com/authorize";
pub const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";
/// Cuánto vale una autorización iniciada y no terminada.
const PENDING_TTL_MS: i64 = 10 * 60 * 1000;
/// Se renueva el token un poco antes de que caduque.
const REFRESH_MARGIN_MS: i64 = 60 * 1000;

struct Pending {
    verifier: String,
    state: String,
    redirect_uri: String,
    client_id: String,
    created_ms: i64,
}

struct Access {
    token: String,
    expires_ms: i64,
}

#[derive(Default)]
struct Inner {
    pending: Option<Pending>,
    access: Option<Access>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

pub struct SpotifyAuth {
    http: reqwest::Client,
    secrets: Arc<dyn SecretStore>,
    clock: Arc<dyn Clock>,
    authorize_url: String,
    token_url: String,
    client_id: RwLock<String>,
    inner: Mutex<Inner>,
    /// Evita dos renovaciones simultáneas (Spotify rota el refresh token).
    refresh_lock: tokio::sync::Mutex<()>,
}

fn form(pairs: &[(&str, &str)]) -> String {
    pairs.iter().map(|(k, v)| format!("{k}={}", percent_encode(v))).collect::<Vec<_>>().join("&")
}

impl SpotifyAuth {
    pub fn new(secrets: Arc<dyn SecretStore>, clock: Arc<dyn Clock>) -> Result<Arc<Self>> {
        Self::with_endpoints(secrets, clock, AUTHORIZE_URL, TOKEN_URL)
    }

    pub fn with_endpoints(secrets: Arc<dyn SecretStore>, clock: Arc<dyn Clock>, authorize_url: &str, token_url: &str) -> Result<Arc<Self>> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| AppError::Invalid(format!("cliente HTTP: {e}")))?;
        Ok(Arc::new(Self {
            http,
            secrets,
            clock,
            authorize_url: authorize_url.into(),
            token_url: token_url.into(),
            client_id: RwLock::new(String::new()),
            inner: Mutex::new(Inner::default()),
            refresh_lock: tokio::sync::Mutex::new(()),
        }))
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn set_client_id(&self, id: &str) {
        let mut cur = self.client_id.write().unwrap_or_else(PoisonError::into_inner);
        if *cur != id {
            *cur = id.to_string();
            // Otro Client ID = otra app de Spotify: los tokens anteriores ya no sirven.
            drop(cur);
            self.lock().access = None;
        }
    }

    fn client_id(&self) -> String {
        self.client_id.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// ¿Hay una sesión guardada (refresh token en el llavero)?
    pub fn is_connected(&self) -> bool {
        self.secrets.get(KEY_SPOTIFY_REFRESH).ok().flatten().is_some_and(|t| !t.is_empty())
    }

    pub fn disconnect(&self) -> Result<()> {
        self.lock().access = None;
        self.lock().pending = None;
        self.secrets.delete(KEY_SPOTIFY_REFRESH)
    }

    /// Empieza la autorización: devuelve la URL a abrir en el navegador del usuario.
    pub fn begin(&self, redirect_uri: &str) -> Result<String> {
        let client_id = self.client_id();
        if client_id.is_empty() {
            return Err(AppError::Invalid("falta el Client ID de Spotify".into()));
        }
        let (verifier, state) = (pkce::random_string(96), pkce::random_string(32));
        let url = format!(
            "{}?{}",
            self.authorize_url,
            form(&[
                ("client_id", &client_id),
                ("response_type", "code"),
                ("redirect_uri", redirect_uri),
                ("code_challenge_method", "S256"),
                ("code_challenge", &pkce::challenge(&verifier)),
                ("state", &state),
                ("scope", SCOPES),
            ])
        );
        self.lock().pending = Some(Pending { verifier, state, redirect_uri: redirect_uri.into(), client_id, created_ms: self.clock.now_ms() });
        Ok(url)
    }

    /// Termina la autorización con lo que Spotify devolvió al navegador. El `state` debe coincidir
    /// con el de la autorización en curso y solo vale una vez.
    pub async fn complete(&self, code: &str, state: &str) -> Result<()> {
        let pending = {
            let mut inner = self.lock();
            let fresh = inner.pending.as_ref().is_some_and(|p| self.clock.now_ms() - p.created_ms <= PENDING_TTL_MS);
            let matches = inner.pending.as_ref().is_some_and(|p| crate::server::constant_time_eq(p.state.as_bytes(), state.as_bytes()));
            if !fresh || !matches {
                // Un intento ajeno o vencido no debe poder borrar el que sí está en curso, salvo que ya caducó.
                if !fresh {
                    inner.pending = None;
                }
                return Err(AppError::Invalid("la autorización de Spotify no coincide o caducó; inicia de nuevo".into()));
            }
            inner.pending.take()
        };
        let Some(p) = pending else { return Err(AppError::Invalid("no hay una autorización en curso".into())) };
        let body = form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", &p.redirect_uri),
            ("client_id", &p.client_id),
            ("code_verifier", &p.verifier),
        ]);
        let tokens = self.token_request(body).await.map_err(AppError::from)?;
        let refresh = tokens.refresh_token.clone().ok_or_else(|| AppError::Invalid("Spotify no devolvió refresh token".into()))?;
        self.secrets.set(KEY_SPOTIFY_REFRESH, &refresh)?;
        self.store_access(&tokens);
        Ok(())
    }

    fn store_access(&self, t: &TokenResponse) {
        let expires = self.clock.now_ms() + t.expires_in.unwrap_or(3600).clamp(60, 24 * 3600) * 1000;
        self.lock().access = Some(Access { token: t.access_token.clone(), expires_ms: expires });
    }

    async fn token_request(&self, body: String) -> std::result::Result<TokenResponse, ApiError> {
        let resp = self
            .http
            .post(&self.token_url)
            .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .map_err(|e| ApiError::Other(format!("no se pudo contactar con Spotify: {}", e.without_url())))?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| ApiError::Other(e.without_url().to_string()))?;
        if !status.is_success() {
            // El cuerpo de error de Spotify (`error_description`) es seguro de mostrar; los tokens nunca van ahí.
            let why = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| v.get("error_description").or_else(|| v.get("error")).and_then(|x| x.as_str().map(str::to_string)))
                .unwrap_or_else(|| status.to_string());
            return Err(ApiError::Other(why));
        }
        serde_json::from_str(&text).map_err(|_| ApiError::Other("respuesta de token ilegible".into()))
    }

    /// Un access token válido; lo renueva con el refresh token si hace falta.
    pub async fn access_token(&self) -> std::result::Result<String, ApiError> {
        if let Some(t) = self.cached() {
            return Ok(t);
        }
        let _guard = self.refresh_lock.lock().await;
        // Otro llamador pudo renovarlo mientras esperábamos.
        if let Some(t) = self.cached() {
            return Ok(t);
        }
        let refresh = self
            .secrets
            .get(KEY_SPOTIFY_REFRESH)
            .map_err(|e| ApiError::Other(e.to_string()))?
            .filter(|t| !t.is_empty())
            .ok_or(ApiError::NotConnected)?;
        let client_id = self.client_id();
        if client_id.is_empty() {
            return Err(ApiError::NotConnected);
        }
        let body = form(&[("grant_type", "refresh_token"), ("refresh_token", &refresh), ("client_id", &client_id)]);
        match self.token_request(body).await {
            Ok(t) => {
                if let Some(new_refresh) = t.refresh_token.as_deref().filter(|r| !r.is_empty() && *r != refresh) {
                    self.secrets.set(KEY_SPOTIFY_REFRESH, new_refresh).map_err(|e| ApiError::Other(e.to_string()))?;
                }
                self.store_access(&t);
                Ok(t.access_token)
            }
            Err(ApiError::Other(why)) if why.contains("invalid_grant") || why.contains("revoked") => {
                // El usuario revocó el acceso: se olvida la sesión para que la UI pida reconectar.
                let _ = self.secrets.delete(KEY_SPOTIFY_REFRESH);
                Err(ApiError::NotConnected)
            }
            Err(e) => Err(e),
        }
    }

    fn cached(&self) -> Option<String> {
        let now = self.clock.now_ms();
        self.lock().access.as_ref().filter(|a| a.expires_ms - REFRESH_MARGIN_MS > now).map(|a| a.token.clone())
    }

    /// Descarta el access token en caché (Spotify respondió 401).
    pub fn invalidate_access(&self) {
        self.lock().access = None;
    }
}
