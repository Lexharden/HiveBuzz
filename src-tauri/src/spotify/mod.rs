//! Spotify: autenticación OAuth con PKCE (sin servidor propio ni secreto de cliente), peticiones de
//! canciones por chat (`!sr`) y el overlay «Sonando ahora».
//!
//! El usuario crea su propia app en el panel de desarrolladores de Spotify, copia su Client ID en
//! Ajustes y registra como redirect URI la dirección que HiveBuzz le muestra. El refresh token vive
//! en el llavero del sistema; el access token solo en memoria.

pub mod api;
pub mod auth;
pub mod pkce;
pub mod service;

use serde::{Deserialize, Serialize};

use crate::error::AppError;

pub const KEY_SPOTIFY_CONFIG: &str = "spotify_config";
pub const KEY_SPOTIFY_REFRESH: &str = "spotify_refresh_token";
/// Canal retenido que lee el overlay «Sonando ahora».
pub const CHANNEL: &str = "nowplaying";
/// Ruta (en el servidor local) a la que Spotify devuelve al usuario tras autorizar.
pub const CALLBACK_PATH: &str = "/spotify/callback";
pub const SCOPES: &str = "user-read-playback-state user-modify-playback-state user-read-currently-playing";

/// Errores de la API de Spotify que el resto de la app trata de forma distinta.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ApiError {
    #[error("Spotify no está conectado")]
    NotConnected,
    #[error("Spotify limitó las peticiones; reintenta en {0} s")]
    RateLimited(u64),
    #[error("no hay un dispositivo de Spotify activo (abre Spotify y reproduce algo)")]
    NoDevice,
    #[error("añadir a la cola requiere una cuenta Spotify Premium")]
    PremiumRequired,
    #[error("Spotify: {0}")]
    Other(String),
}

impl From<ApiError> for AppError {
    fn from(e: ApiError) -> Self {
        AppError::Invalid(e.to_string())
    }
}

/// Quién puede pedir canciones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MinRole {
    #[default]
    Everyone,
    Follower,
    Subscriber,
    Moderator,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SongConfig {
    pub enabled: bool,
    /// Comandos para pedir (sin `!`).
    pub commands: Vec<String>,
    /// Comandos que responden qué suena ahora.
    pub now_playing_commands: Vec<String>,
    pub min_role: MinRole,
    /// Puntos que cuesta pedir una canción (0 = gratis).
    pub cost_points: u64,
    /// Máximo de canciones de un mismo espectador esperando en la cola.
    pub per_user_limit: u32,
    pub user_cooldown_s: u64,
    pub max_duration_s: u64,
    /// Texto que, si aparece en el título o en el artista, bloquea la canción.
    pub blocked_terms: Vec<String>,
    /// `@usuario` que no pueden pedir.
    pub blocked_users: Vec<String>,
    /// Responder en el chat (necesita la sesión de TikTok).
    pub reply: bool,
}

impl Default for SongConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            commands: vec!["sr".into(), "pedir".into()],
            now_playing_commands: vec!["song".into(), "cancion".into()],
            min_role: MinRole::Everyone,
            cost_points: 0,
            per_user_limit: 2,
            user_cooldown_s: 30,
            max_duration_s: 600,
            blocked_terms: Vec::new(),
            blocked_users: Vec::new(),
            reply: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct SpotifyConfig {
    /// Client ID de la app de Spotify del usuario (público: PKCE no usa secreto).
    pub client_id: String,
    pub song: SongConfig,
}

/// Client ID de la app de Spotify de HiveBuzz, fijado al compilar con la variable de entorno
/// `HIVEBUZZ_SPOTIFY_CLIENT_ID`. Con PKCE el Client ID es público (no hay secreto), así que
/// puede ir dentro del programa y el usuario final solo pulsa «Conectar».
pub fn default_client_id() -> &'static str {
    option_env!("HIVEBUZZ_SPOTIFY_CLIENT_ID").unwrap_or("")
}

/// El Client ID que se usa: el propio del usuario (opción avanzada) o, si no hay, el integrado.
pub fn effective_client_id<'a>(configured: &'a str, builtin: &'a str) -> &'a str {
    if configured.trim().is_empty() {
        builtin
    } else {
        configured.trim()
    }
}

const MAX_LIST: usize = 200;

fn clean_list(v: &[String], strip_prefix: &[char], lower: bool, max_chars: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in v {
        let mut t: String = s.chars().filter(|c| !c.is_control()).collect::<String>().trim().trim_start_matches(strip_prefix).trim().chars().take(max_chars).collect();
        if lower {
            t = t.to_lowercase();
        }
        if !t.is_empty() && !out.contains(&t) && out.len() < MAX_LIST {
            out.push(t);
        }
    }
    out
}

impl SpotifyConfig {
    pub fn sanitized(mut self) -> Self {
        self.client_id = self.client_id.trim().to_string();
        let s = &mut self.song;
        s.commands = clean_list(&s.commands, &['!'], true, 24);
        s.now_playing_commands = clean_list(&s.now_playing_commands, &['!'], true, 24);
        s.blocked_terms = clean_list(&s.blocked_terms, &[], true, 80);
        s.blocked_users = clean_list(&s.blocked_users, &['@'], true, 40);
        s.per_user_limit = s.per_user_limit.clamp(1, 50);
        s.user_cooldown_s = s.user_cooldown_s.min(3_600);
        s.max_duration_s = s.max_duration_s.clamp(30, 3_600 * 3);
        s.cost_points = s.cost_points.min(1_000_000_000);
        self
    }

    pub fn validate(&self) -> crate::error::Result<()> {
        if !self.client_id.is_empty() && (self.client_id.len() > 64 || !self.client_id.chars().all(|c| c.is_ascii_alphanumeric())) {
            return Err(AppError::Invalid("el Client ID de Spotify solo lleva letras y números".into()));
        }
        if self.song.enabled && self.song.commands.is_empty() {
            return Err(AppError::Invalid("indica al menos un comando para pedir canciones".into()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
