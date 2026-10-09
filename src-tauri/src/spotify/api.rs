//! Cliente mínimo de la API web de Spotify: buscar canciones, encolar y ver qué suena.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::{ACCEPT, AUTHORIZATION, RETRY_AFTER};
use reqwest::StatusCode;
use serde::Serialize;
use serde_json::Value;

use super::auth::SpotifyAuth;
use super::ApiError;
use crate::executors::percent_encode;

pub const API_BASE: &str = "https://api.spotify.com/v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: String,
    pub uri: String,
    pub name: String,
    pub artists: Vec<String>,
    pub duration_ms: u64,
    /// Carátula (https, CDN de Spotify).
    pub image: Option<String>,
}

impl Track {
    pub fn artist_line(&self) -> String {
        self.artists.join(", ")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Playing {
    pub track: Track,
    pub progress_ms: u64,
    pub is_playing: bool,
}

/// Lo que el servicio de peticiones necesita de Spotify (así se prueba con uno falso).
#[async_trait]
pub trait SpotifyApi: Send + Sync {
    /// Busca una canción por texto, o la resuelve si el texto es un enlace/URI de Spotify.
    async fn find_track(&self, query: &str) -> Result<Option<Track>, ApiError>;
    async fn queue(&self, track: &Track) -> Result<(), ApiError>;
    async fn now_playing(&self) -> Result<Option<Playing>, ApiError>;
}

/// Extrae el id de una canción de `https://open.spotify.com/track/ID?...` o `spotify:track:ID`.
pub fn track_id_from_ref(text: &str) -> Option<String> {
    let t = text.trim();
    let id = if let Some(rest) = t.strip_prefix("spotify:track:") {
        rest
    } else {
        let after = t.strip_prefix("https://open.spotify.com/").or_else(|| t.strip_prefix("http://open.spotify.com/"))?;
        // Los enlaces pueden llevar un prefijo de idioma: /intl-es/track/ID
        let after = after.strip_prefix("intl-").and_then(|r| r.split_once('/').map(|(_, rest)| rest)).unwrap_or(after);
        after.strip_prefix("track/")?
    };
    let id = id.split(['?', '#', '/']).next().unwrap_or("");
    (id.len() == 22 && id.chars().all(|c| c.is_ascii_alphanumeric())).then(|| id.to_string())
}

pub(crate) fn parse_track(v: &Value) -> Option<Track> {
    let id = v.get("id")?.as_str()?.to_string();
    let artists: Vec<String> = v.get("artists")?.as_array()?.iter().filter_map(|a| a.get("name").and_then(Value::as_str)).map(str::to_string).collect();
    // Carátula: la imagen más pequeña que no sea diminuta (suficiente para un overlay).
    let image = v
        .get("album")
        .and_then(|a| a.get("images"))
        .and_then(Value::as_array)
        .and_then(|imgs| imgs.iter().filter_map(|i| Some((i.get("width")?.as_u64()?, i.get("url")?.as_str()?))).filter(|(w, _)| *w >= 200).min_by_key(|(w, _)| *w).or_else(|| imgs.iter().filter_map(|i| Some((i.get("width")?.as_u64()?, i.get("url")?.as_str()?))).max_by_key(|(w, _)| *w)))
        .map(|(_, u)| u.to_string())
        .filter(|u| u.starts_with("https://"));
    Some(Track {
        uri: v.get("uri").and_then(Value::as_str).map_or_else(|| format!("spotify:track:{id}"), str::to_string),
        id,
        name: v.get("name")?.as_str()?.to_string(),
        artists,
        duration_ms: v.get("duration_ms").and_then(Value::as_u64).unwrap_or(0),
        image,
    })
}

pub struct HttpSpotify {
    http: reqwest::Client,
    auth: Arc<SpotifyAuth>,
    base: String,
}

impl HttpSpotify {
    pub fn new(auth: Arc<SpotifyAuth>) -> Result<Self, ApiError> {
        Self::with_base(auth, API_BASE)
    }

    pub fn with_base(auth: Arc<SpotifyAuth>, base: &str) -> Result<Self, ApiError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(12))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| ApiError::Other(e.to_string()))?;
        Ok(Self { http, auth, base: base.trim_end_matches('/').to_string() })
    }

    /// Hace la petición con el token; si Spotify dice 401 renueva una vez y reintenta.
    async fn send(&self, method: reqwest::Method, path_and_query: &str) -> Result<(StatusCode, String, Option<u64>), ApiError> {
        for attempt in 0..2 {
            let token = self.auth.access_token().await?;
            let mut req = self
                .http
                .request(method.clone(), format!("{}{path_and_query}", self.base))
                .header(AUTHORIZATION, format!("Bearer {token}"))
                .header(ACCEPT, "application/json");
            if method == reqwest::Method::POST {
                req = req.header(reqwest::header::CONTENT_LENGTH, "0");
            }
            let resp = req.send().await.map_err(|e| ApiError::Other(format!("no se pudo contactar con Spotify: {}", e.without_url())))?;
            let status = resp.status();
            let retry = resp.headers().get(RETRY_AFTER).and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<u64>().ok());
            let text = resp.text().await.map_err(|e| ApiError::Other(e.without_url().to_string()))?;
            if status == StatusCode::UNAUTHORIZED && attempt == 0 {
                self.auth.invalidate_access();
                continue;
            }
            return Ok((status, text, retry));
        }
        Err(ApiError::NotConnected)
    }

    fn error_for(status: StatusCode, body: &str, retry: Option<u64>) -> ApiError {
        let msg = serde_json::from_str::<Value>(body)
            .ok()
            .and_then(|v| v.pointer("/error/message").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_default();
        let reason = serde_json::from_str::<Value>(body).ok().and_then(|v| v.pointer("/error/reason").and_then(Value::as_str).map(str::to_string)).unwrap_or_default();
        match status {
            StatusCode::TOO_MANY_REQUESTS => ApiError::RateLimited(retry.unwrap_or(5).clamp(1, 3600)),
            StatusCode::NOT_FOUND if reason == "NO_ACTIVE_DEVICE" || msg.to_lowercase().contains("no active device") => ApiError::NoDevice,
            StatusCode::FORBIDDEN if reason == "PREMIUM_REQUIRED" || msg.to_lowercase().contains("premium") => ApiError::PremiumRequired,
            StatusCode::UNAUTHORIZED => ApiError::NotConnected,
            s => ApiError::Other(if msg.is_empty() { s.to_string() } else { format!("{s}: {msg}") }),
        }
    }
}

#[async_trait]
impl SpotifyApi for HttpSpotify {
    async fn find_track(&self, query: &str) -> Result<Option<Track>, ApiError> {
        if let Some(id) = track_id_from_ref(query) {
            let (status, body, retry) = self.send(reqwest::Method::GET, &format!("/tracks/{id}")).await?;
            return match status {
                StatusCode::OK => Ok(serde_json::from_str::<Value>(&body).ok().and_then(|v| parse_track(&v))),
                StatusCode::NOT_FOUND | StatusCode::BAD_REQUEST => Ok(None),
                _ => Err(Self::error_for(status, &body, retry)),
            };
        }
        let q = query.trim();
        if q.is_empty() {
            return Ok(None);
        }
        let (status, body, retry) = self.send(reqwest::Method::GET, &format!("/search?q={}&type=track&limit=1", percent_encode(q))).await?;
        if status != StatusCode::OK {
            return Err(Self::error_for(status, &body, retry));
        }
        let v: Value = serde_json::from_str(&body).map_err(|_| ApiError::Other("respuesta de búsqueda ilegible".into()))?;
        Ok(v.pointer("/tracks/items/0").and_then(parse_track))
    }

    async fn queue(&self, track: &Track) -> Result<(), ApiError> {
        let (status, body, retry) = self.send(reqwest::Method::POST, &format!("/me/player/queue?uri={}", percent_encode(&track.uri))).await?;
        if status.is_success() {
            Ok(())
        } else {
            Err(Self::error_for(status, &body, retry))
        }
    }

    async fn now_playing(&self) -> Result<Option<Playing>, ApiError> {
        let (status, body, retry) = self.send(reqwest::Method::GET, "/me/player/currently-playing").await?;
        match status {
            StatusCode::NO_CONTENT => Ok(None),
            StatusCode::OK => {
                let v: Value = serde_json::from_str(&body).map_err(|_| ApiError::Other("respuesta ilegible".into()))?;
                // Anuncios o podcasts: no hay `item` de tipo canción.
                let Some(track) = v.get("item").filter(|i| !i.is_null()).and_then(parse_track) else { return Ok(None) };
                Ok(Some(Playing {
                    track,
                    progress_ms: v.get("progress_ms").and_then(Value::as_u64).unwrap_or(0),
                    is_playing: v.get("is_playing").and_then(Value::as_bool).unwrap_or(false),
                }))
            }
            _ => Err(Self::error_for(status, &body, retry)),
        }
    }
}
