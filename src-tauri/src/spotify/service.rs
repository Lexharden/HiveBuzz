//! Peticiones de canciones por chat (`!sr`) y estado de «Sonando ahora».

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::broadcast::error::RecvError;

use super::api::{Playing, SpotifyApi, Track};
use super::auth::SpotifyAuth;
use super::{default_client_id, effective_client_id, ApiError, MinRole, SpotifyConfig, CHANNEL, KEY_SPOTIFY_CONFIG};
use crate::actions::clock::Clock;
use crate::bot::service::BotService;
use crate::bus::EventBus;
use crate::db::Db;
use crate::error::{AppError, Result};
use crate::events::{EventType, LiveEvent, Platform, User};
use crate::overlay::OverlayHub;
use crate::rules::engine::PointsGate;

/// Una petición espera en la cola de Spotify como mucho esto antes de olvidarla.
const PENDING_MAX_AGE_MS: i64 = 3 * 60 * 60 * 1000;
const MAX_PENDING: usize = 200;
const MAX_QUERY_CHARS: usize = 200;
const POLL_PLAYING: Duration = Duration::from_secs(4);
const POLL_IDLE: Duration = Duration::from_secs(10);
const POLL_ERROR: Duration = Duration::from_secs(20);
const POLL_DISCONNECTED: Duration = Duration::from_secs(5);

/// Salida al chat (el bot); un trait para probar sin TikTok.
pub trait ChatOut: Send + Sync {
    fn say(&self, text: &str, source: &str) -> bool;
}

impl ChatOut for BotService {
    fn say(&self, text: &str, source: &str) -> bool {
        BotService::say(self, text, source)
    }
}

#[derive(Debug, Clone)]
struct Request {
    user_id: String,
    unique_id: String,
    uri: String,
    queued_ms: i64,
}

#[derive(Default)]
struct State {
    pending: Vec<Request>,
    last_by_user: HashMap<String, i64>,
    playing: Option<Playing>,
    /// Última vez que se publicó el estado, para no repetir lo mismo en cada sondeo.
    published: Option<(String, bool)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpotifyStatus {
    pub connected: bool,
    pub client_id_set: bool,
    /// Se usa la app de Spotify integrada en HiveBuzz (el caso normal: no hay nada que configurar).
    pub uses_builtin_app: bool,
    pub redirect_uri: String,
    pub pending_requests: usize,
}

pub struct SongService {
    cfg: RwLock<SpotifyConfig>,
    state: Mutex<State>,
    db: Db,
    auth: Arc<SpotifyAuth>,
    api: Arc<dyn SpotifyApi>,
    wallet: Arc<dyn PointsGate>,
    chat: Arc<dyn ChatOut>,
    hub: OverlayHub,
    clock: Arc<dyn Clock>,
}

fn role_ok(min: MinRole, u: &User) -> bool {
    match min {
        MinRole::Everyone => true,
        MinRole::Follower => u.is_follower || u.is_subscriber || u.is_moderator,
        MinRole::Subscriber => u.is_subscriber || u.is_moderator,
        MinRole::Moderator => u.is_moderator,
    }
}

fn who(u: &User) -> String {
    if u.unique_id.is_empty() { u.nickname.clone() } else { format!("@{}", u.unique_id) }
}

/// Texto tras el comando: `!sr hola mundo` → `("sr", "hola mundo")`.
fn split_command(text: &str) -> Option<(String, &str)> {
    let t = text.trim().strip_prefix('!')?;
    let (name, rest) = t.split_once(char::is_whitespace).unwrap_or((t, ""));
    (!name.is_empty()).then(|| (name.to_lowercase(), rest.trim()))
}

impl SongService {
    pub fn new(
        db: Db,
        auth: Arc<SpotifyAuth>,
        api: Arc<dyn SpotifyApi>,
        wallet: Arc<dyn PointsGate>,
        chat: Arc<dyn ChatOut>,
        hub: OverlayHub,
        clock: Arc<dyn Clock>,
    ) -> Arc<Self> {
        Arc::new(Self { cfg: RwLock::new(SpotifyConfig::default()), state: Mutex::new(State::default()), db, auth, api, wallet, chat, hub, clock })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub async fn load_config(&self) -> Result<()> {
        let cfg = match self.db.get_setting(KEY_SPOTIFY_CONFIG).await? {
            Some(json) => serde_json::from_str::<SpotifyConfig>(&json).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "configuración de Spotify ilegible; se usa la de fábrica");
                SpotifyConfig::default()
            }),
            None => SpotifyConfig::default(),
        }
        .sanitized();
        self.auth.set_client_id(effective_client_id(&cfg.client_id, default_client_id()));
        *self.cfg.write().unwrap_or_else(PoisonError::into_inner) = cfg;
        Ok(())
    }

    pub fn config(&self) -> SpotifyConfig {
        self.cfg.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    pub async fn set_config(&self, cfg: SpotifyConfig) -> Result<SpotifyConfig> {
        let cfg = cfg.sanitized();
        cfg.validate()?;
        self.db.set_setting(KEY_SPOTIFY_CONFIG, &serde_json::to_string(&cfg)?).await?;
        self.auth.set_client_id(effective_client_id(&cfg.client_id, default_client_id()));
        *self.cfg.write().unwrap_or_else(PoisonError::into_inner) = cfg.clone();
        Ok(cfg)
    }

    pub fn status(&self, redirect_uri: String) -> SpotifyStatus {
        SpotifyStatus {
            connected: self.auth.is_connected(),
            client_id_set: !effective_client_id(&self.config().client_id, default_client_id()).is_empty(),
            uses_builtin_app: self.config().client_id.trim().is_empty() && !default_client_id().is_empty(),
            redirect_uri,
            pending_requests: self.lock().pending.len(),
        }
    }

    /// Lo que suena ahora (último sondeo).
    pub fn playing(&self) -> Option<Playing> {
        self.lock().playing.clone()
    }

    /// Pide una canción desde la UI (prueba): sin roles, coste ni cooldown.
    pub async fn manual_queue(&self, query: &str) -> Result<Track> {
        let q: String = query.trim().chars().take(MAX_QUERY_CHARS).collect();
        let track = self.api.find_track(&q).await.map_err(AppError::from)?.ok_or_else(|| AppError::Invalid("no se encontró esa canción".into()))?;
        self.api.queue(&track).await.map_err(AppError::from)?;
        Ok(track)
    }

    // ---------------------------------------------------------------- chat

    /// Atiende un mensaje de chat. Devuelve `true` si era un comando de canciones.
    pub async fn on_event(&self, ev: &LiveEvent) -> bool {
        if ev.kind != EventType::Chat || ev.is_simulated() {
            return false;
        }
        let Some(chat) = &ev.chat else { return false };
        let cfg = self.config();
        if !cfg.song.enabled {
            return false;
        }
        let Some((name, rest)) = split_command(&chat.text) else { return false };
        if cfg.song.commands.contains(&name) {
            let reply = self.request(&ev.user, rest).await;
            // La respuesta solo puede salir por el chat de TikTok.
            if cfg.song.reply && ev.platform == Platform::Tiktok {
                self.chat.say(&reply, "canciones");
            }
            true
        } else if cfg.song.now_playing_commands.contains(&name) {
            if cfg.song.reply && ev.platform == Platform::Tiktok {
                let text = match self.playing().filter(|p| p.is_playing) {
                    Some(p) => format!("🎵 Suena: «{}» — {}", p.track.name, p.track.artist_line()),
                    None => "🎵 Ahora mismo no suena nada.".to_string(),
                };
                self.chat.say(&text, "canciones");
            }
            true
        } else {
            false
        }
    }

    /// Procesa una petición y devuelve el mensaje de respuesta para el chat.
    pub async fn request(&self, user: &User, query: &str) -> String {
        let cfg = self.config();
        let s = &cfg.song;
        let me = who(user);
        let uid = if user.id.is_empty() { user.unique_id.clone() } else { user.id.clone() };
        let now = self.clock.now_ms();

        if !role_ok(s.min_role, user) {
            return format!("{me} no tienes permiso para pedir canciones.");
        }
        if s.blocked_users.iter().any(|b| b.eq_ignore_ascii_case(&user.unique_id)) {
            return format!("{me} no puedes pedir canciones.");
        }
        let query: String = query.chars().filter(|c| !c.is_control()).take(MAX_QUERY_CHARS).collect();
        let query = query.trim();
        if query.is_empty() {
            let cmd = s.commands.first().map_or("sr", String::as_str);
            return format!("{me} uso: !{cmd} <canción o enlace de Spotify>");
        }
        {
            let mut st = self.lock();
            st.pending.retain(|r| now - r.queued_ms < PENDING_MAX_AGE_MS);
            if let Some(&last) = st.last_by_user.get(&uid) {
                let wait = i64::try_from(s.user_cooldown_s).unwrap_or(i64::MAX).saturating_mul(1000) - (now - last);
                if wait > 0 {
                    return format!("{me} espera {} s para pedir otra.", (wait + 999) / 1000);
                }
            }
            let mine = st.pending.iter().filter(|r| r.user_id == uid).count();
            if mine >= s.per_user_limit as usize {
                return format!("{me} ya tienes {mine} canciones en la cola.");
            }
            st.last_by_user.insert(uid.clone(), now);
            if st.last_by_user.len() > 5_000 {
                st.last_by_user.retain(|_, t| now - *t < 3_600_000);
            }
        }

        let track = match self.api.find_track(query).await {
            Ok(Some(t)) => t,
            Ok(None) => return format!("{me} no encontré «{}».", crate::bot::engine::fit(query)),
            Err(e) => return format!("{me} {}", Self::friendly(&e)),
        };
        let hay = format!("{} {}", track.name, track.artist_line()).to_lowercase();
        if s.blocked_terms.iter().any(|t| hay.contains(t.as_str())) {
            return format!("{me} esa canción está bloqueada.");
        }
        if track.duration_ms > s.max_duration_s.saturating_mul(1000) {
            return format!("{me} esa canción dura demasiado (máximo {} min).", s.max_duration_s / 60);
        }

        let charged = if s.cost_points > 0 && !user.id.is_empty() {
            match self.wallet.spend(&user.id, s.cost_points, "canción").await {
                Ok(Some(_)) => true,
                Ok(None) => return format!("{me} necesitas {} puntos para pedir una canción.", s.cost_points),
                Err(e) => {
                    tracing::error!(error = %e, "no se pudo cobrar la canción");
                    return format!("{me} no se pudo cobrar la canción; inténtalo luego.");
                }
            }
        } else {
            false
        };

        if let Err(e) = self.api.queue(&track).await {
            if charged {
                if let Err(re) = self.wallet.refund(&user.id, s.cost_points, "canción no encolada").await {
                    tracing::error!(error = %re, "no se pudieron devolver los puntos de la canción");
                }
            }
            // El intento fallido no debe dejar al espectador esperando el cooldown.
            self.lock().last_by_user.remove(&uid);
            return format!("{me} {}", Self::friendly(&e));
        }

        {
            let mut st = self.lock();
            if st.pending.len() >= MAX_PENDING {
                st.pending.remove(0);
            }
            st.pending.push(Request { user_id: uid, unique_id: user.unique_id.clone(), uri: track.uri.clone(), queued_ms: now });
        }
        format!("{me} 🎵 añadida «{}» — {}", track.name, track.artist_line())
    }

    fn friendly(e: &ApiError) -> String {
        match e {
            ApiError::NoDevice => "no hay Spotify activo ahora mismo.".into(),
            ApiError::PremiumRequired => "la cuenta del streamer necesita Spotify Premium para la cola.".into(),
            ApiError::NotConnected => "las canciones no están disponibles por ahora.".into(),
            ApiError::RateLimited(_) => "Spotify está saturado, inténtalo en un momento.".into(),
            ApiError::Forbidden | ApiError::Other(_) => "no se pudo añadir la canción.".into(),
        }
    }

    // ---------------------------------------------------------------- sonando ahora

    /// Aplica lo que devolvió Spotify: actualiza las peticiones pendientes y publica para el overlay.
    pub fn apply_playing(&self, playing: Option<Playing>) {
        let now = self.clock.now_ms();
        let payload = {
            let mut st = self.lock();
            let prev_uri = st.playing.as_ref().map(|p| p.track.uri.clone());
            let cur_uri = playing.as_ref().map(|p| p.track.uri.clone());
            // Cuando la canción cambia, la que acaba de terminar ya no cuenta como pendiente.
            if let Some(prev) = prev_uri.filter(|p| Some(p) != cur_uri.as_ref()) {
                if let Some(i) = st.pending.iter().position(|r| r.uri == prev) {
                    st.pending.remove(i);
                }
            }
            let requested_by = playing.as_ref().and_then(|p| st.pending.iter().find(|r| r.uri == p.track.uri)).map(|r| r.unique_id.clone());
            let key = (cur_uri.unwrap_or_default(), playing.as_ref().is_some_and(|p| p.is_playing));
            let changed = st.published.as_ref() != Some(&key);
            st.playing = playing.clone();
            // Mientras suena la misma canción se republica igualmente (el progreso se corrige solo), pero
            // sin canción no hace falta repetir «nada».
            if !changed && playing.is_none() {
                return;
            }
            st.published = Some(key);
            playing.map_or_else(
                || json!({ "kind": "none" }),
                |p| {
                    json!({
                        "kind": "nowplaying",
                        "playing": p.is_playing,
                        "title": p.track.name,
                        "artist": p.track.artist_line(),
                        "image": p.track.image,
                        "progressMs": p.progress_ms,
                        "durationMs": p.track.duration_ms,
                        "requestedBy": requested_by,
                        "fetchedAtMs": now,
                    })
                },
            )
        };
        self.hub.publish_retained(CHANNEL, payload);
    }

    /// Vuelve a publicar el estado real (tras una prueba del overlay con datos de ejemplo).
    pub fn republish(&self) {
        let current = {
            let mut st = self.lock();
            st.published = None;
            st.playing.clone()
        };
        self.apply_playing(current);
        // Sin canción, `apply_playing` no publica si nada cambió: se fuerza el «nada».
        if self.playing().is_none() {
            self.hub.publish_retained(CHANNEL, json!({ "kind": "none" }));
        }
    }

    /// Un sondeo. Devuelve cuánto esperar hasta el siguiente.
    pub async fn poll_once(&self) -> Duration {
        let configured = !effective_client_id(&self.config().client_id, default_client_id()).is_empty() && self.auth.is_connected();
        if !configured {
            self.apply_playing(None);
            return POLL_DISCONNECTED;
        }
        match self.api.now_playing().await {
            Ok(p) => {
                let playing = p.as_ref().is_some_and(|p| p.is_playing);
                self.apply_playing(p);
                if playing { POLL_PLAYING } else { POLL_IDLE }
            }
            Err(ApiError::RateLimited(s)) => Duration::from_secs(s),
            Err(ApiError::NotConnected) => {
                self.apply_playing(None);
                POLL_DISCONNECTED
            }
            // No se arregla solo (cuenta no registrada en la app): que el overlay no se quede con la última canción.
            Err(e @ ApiError::Forbidden) => {
                tracing::warn!(error = %e, "Spotify rechaza las consultas");
                self.apply_playing(None);
                POLL_ERROR
            }
            Err(e) => {
                tracing::debug!(error = %e, "no se pudo consultar Spotify");
                POLL_ERROR
            }
        }
    }

    pub fn spawn(self: &Arc<Self>, bus: &EventBus) {
        let (svc, mut events) = (Arc::clone(self), bus.subscribe());
        tokio::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(ev) => {
                        svc.on_event(&ev).await;
                    }
                    Err(RecvError::Lagged(n)) => tracing::warn!(missed = n, "las canciones se quedaron atrás en el bus"),
                    Err(RecvError::Closed) => break,
                }
            }
        });
        let svc = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                let wait = svc.poll_once().await;
                tokio::time::sleep(wait).await;
            }
        });
    }

    /// Valor publicado hoy (para pruebas y para el overlay recién abierto).
    pub fn snapshot(&self) -> Value {
        match self.playing() {
            Some(p) => json!({ "kind": "nowplaying", "playing": p.is_playing, "title": p.track.name, "artist": p.track.artist_line() }),
            None => json!({ "kind": "none" }),
        }
    }
}
