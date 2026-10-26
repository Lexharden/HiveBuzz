//! Peticiones de canciones por chat (`!sr`), control del reproductor, cola de próximas canciones y
//! estado de «Sonando ahora».

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::broadcast::error::RecvError;

use super::api::{PlayerCmd, Playing, SpotifyApi, Track};
use super::auth::SpotifyAuth;
use super::{
    default_client_id, effective_client_id, ApiError, ControlConfig, ControlOp, MinRole, SpotifyConfig, CHANNEL, KEY_SPOTIFY_CONFIG, QUEUE_CHANNEL,
};
use crate::actions::clock::Clock;
use crate::bus::EventBus;
use crate::db::Db;
use crate::error::{AppError, Result};
use crate::events::{EventType, LiveEvent, User};
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
/// La cola de Spotify se vuelve a leer al cambiar de canción y, si no, cada tanto (alguien puede añadir desde Spotify).
const QUEUE_REFRESH_MS: i64 = 15_000;
/// Canciones que se recuerdan como «sonó hace poco» para no repetirlas.
const RECENT_MAX: usize = 30;
const MAX_VOTERS: usize = 2_000;

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
    /// Próximas canciones (última lectura de la cola de Spotify) y cuándo/por qué canción se leyó.
    queue: Vec<QueueItem>,
    queue_at_ms: i64,
    queue_for: Option<String>,
    /// Canciones que sonaron hace poco (la más reciente al final).
    recent: VecDeque<String>,
    /// Quién votó saltar la canción actual.
    votes: HashSet<String>,
    votes_for: Option<String>,
}

/// Una canción de la cola de Spotify y quién la pidió (si fue por HiveBuzz).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueItem {
    pub track: Track,
    pub requested_by: Option<String>,
}

/// Lo que muestra el panel del reproductor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub playing: Option<Playing>,
    pub queue: Vec<QueueItem>,
}

fn queue_payload(items: &[QueueItem]) -> Value {
    let items: Vec<Value> = items
        .iter()
        .map(|i| json!({ "title": i.track.name, "artist": i.track.artist_line(), "image": i.track.image, "requestedBy": i.requested_by }))
        .collect();
    json!({ "kind": "queue", "items": items })
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
        hub: OverlayHub,
        clock: Arc<dyn Clock>,
    ) -> Arc<Self> {
        Arc::new(Self { cfg: RwLock::new(SpotifyConfig::default()), state: Mutex::new(State::default()), db, auth, api, wallet, hub, clock })
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

    /// Reproductor y cola tal como los vio el último sondeo.
    pub fn overview(&self) -> Overview {
        let st = self.lock();
        Overview { playing: st.playing.clone(), queue: st.queue.clone() }
    }

    /// Ejecuta una orden al reproductor (botones, reglas o chat).
    pub async fn control(&self, op: ControlOp) -> Result<()> {
        let cmd = match op {
            ControlOp::Next => PlayerCmd::Next,
            ControlOp::Previous => PlayerCmd::Previous,
            ControlOp::Pause => PlayerCmd::Pause,
            ControlOp::Resume => PlayerCmd::Resume,
            ControlOp::VolumeSet(v) => PlayerCmd::Volume(v.min(100)),
            ControlOp::VolumeChange(delta) => {
                let current = self.api.volume().await.map_err(AppError::from)?.ok_or(ApiError::NoDevice).map_err(AppError::from)?;
                let next = (i32::from(current) + delta).clamp(0, 100);
                PlayerCmd::Volume(u8::try_from(next).unwrap_or(100))
            }
        };
        self.api.control(cmd).await.map_err(AppError::from)
    }

    /// Pide una canción desde la UI (prueba): sin roles, coste ni cooldown.
    pub async fn manual_queue(&self, query: &str) -> Result<Track> {
        let q: String = query.trim().chars().take(MAX_QUERY_CHARS).collect();
        let track = self.api.find_track(&q).await.map_err(AppError::from)?.ok_or_else(|| AppError::Invalid("no se encontró esa canción".into()))?;
        self.api.queue(&track).await.map_err(AppError::from)?;
        Ok(track)
    }

    // ---------------------------------------------------------------- chat

    /// Atiende un mensaje de chat. Si era un comando de canciones devuelve el resultado de la petición
    /// («añadida…», «espera 30 s…», etc.), que se deja en el registro: no hay forma de escribir en el chat.
    pub async fn on_event(&self, ev: &LiveEvent) -> Option<String> {
        if ev.kind != EventType::Chat || ev.is_simulated() {
            return None;
        }
        let chat = ev.chat.as_ref()?;
        let cfg = self.config();
        if !cfg.song.enabled && !cfg.control.enabled {
            return None;
        }
        let (name, rest) = split_command(&chat.text)?;
        if cfg.song.enabled && cfg.song.commands.contains(&name) {
            return Some(self.request(&ev.user, rest).await);
        }
        if cfg.control.enabled {
            return self.control_command(&cfg.control, &name, rest, &ev.user).await;
        }
        None
    }

    /// Comandos de control del reproductor. `None` si `name` no es uno de ellos.
    async fn control_command(&self, c: &ControlConfig, name: &str, rest: &str, user: &User) -> Option<String> {
        let me = who(user);
        let has = |list: &[String]| list.iter().any(|n| n == name);
        let allowed = user.is_moderator || role_ok(c.min_role, user);
        let denied = || format!("{me} no tienes permiso para eso.");
        let done = |r: Result<()>, ok: &str| match r {
            Ok(()) => format!("{me} {ok}"),
            Err(e) => format!("{me} {e}"),
        };

        if has(&c.skip_commands) {
            if allowed {
                return Some(done(self.control(ControlOp::Next).await, "saltó la canción."));
            }
            if c.skip_votes == 0 {
                return Some(denied());
            }
            let uid = if user.id.is_empty() { user.unique_id.clone() } else { user.id.clone() };
            let votes = {
                let mut st = self.lock();
                if st.votes.len() < MAX_VOTERS {
                    st.votes.insert(uid);
                }
                u32::try_from(st.votes.len()).unwrap_or(u32::MAX)
            };
            if votes < c.skip_votes {
                return Some(format!("{me} voto para saltar: {votes}/{}.", c.skip_votes));
            }
            self.lock().votes.clear();
            return Some(done(self.control(ControlOp::Next).await, "votación completa: se salta la canción."));
        }
        if has(&c.pause_commands) {
            return Some(if allowed { done(self.control(ControlOp::Pause).await, "pausó la música.") } else { denied() });
        }
        if has(&c.resume_commands) {
            return Some(if allowed { done(self.control(ControlOp::Resume).await, "reanudó la música.") } else { denied() });
        }
        if has(&c.volume_commands) {
            if !allowed {
                return Some(denied());
            }
            let step = i32::from(c.volume_step);
            let arg = rest.trim();
            let op = match arg {
                "+" => Some(ControlOp::VolumeChange(step)),
                "-" => Some(ControlOp::VolumeChange(-step)),
                _ => arg.parse::<i32>().ok().map(|n| {
                    if arg.starts_with('+') || arg.starts_with('-') {
                        ControlOp::VolumeChange(n.clamp(-100, 100))
                    } else {
                        ControlOp::VolumeSet(u8::try_from(n.clamp(0, 100)).unwrap_or(100))
                    }
                }),
            };
            let Some(op) = op else {
                let cmd = c.volume_commands.first().map_or("volumen", String::as_str);
                return Some(format!("{me} uso: !{cmd} <0-100> o !{cmd} + / -"));
            };
            return Some(done(self.control(op).await, "cambió el volumen."));
        }
        if has(&c.queue_commands) {
            self.hub.publish(QUEUE_CHANNEL, json!({ "kind": "show", "seconds": c.queue_show_s }));
            let next: Vec<String> = self.lock().queue.iter().take(3).map(|i| format!("«{}»", i.track.name)).collect();
            return Some(if next.is_empty() { format!("{me} la cola está vacía.") } else { format!("{me} próximas: {}", next.join(", ")) });
        }
        None
    }

    /// Procesa una petición y devuelve su resultado en una frase.
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
            Ok(None) => return format!("{me} no encontré «{}».", query.chars().take(60).collect::<String>()),
            Err(e) => return format!("{me} {}", Self::friendly(&e)),
        };
        let hay = format!("{} {}", track.name, track.artist_line()).to_lowercase();
        if s.blocked_terms.iter().any(|t| hay.contains(t.as_str())) {
            return format!("{me} esa canción está bloqueada.");
        }
        if track.duration_ms > s.max_duration_s.saturating_mul(1000) {
            return format!("{me} esa canción dura demasiado (máximo {} min).", s.max_duration_s / 60);
        }
        if s.no_repeat && self.already_there(&track.uri) {
            // El intento rechazado no debe dejar al espectador esperando el cooldown.
            self.lock().last_by_user.remove(&uid);
            return format!("{me} esa canción ya está en la cola o sonó hace poco.");
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

    /// ¿Ya está pedida, en la cola de Spotify, sonando o sonó hace poco?
    fn already_there(&self, uri: &str) -> bool {
        let st = self.lock();
        st.pending.iter().any(|r| r.uri == uri)
            || st.queue.iter().any(|i| i.track.uri == uri)
            || st.recent.iter().any(|r| r == uri)
            || st.playing.as_ref().is_some_and(|p| p.track.uri == uri)
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
            if let Some(prev) = prev_uri.as_ref().filter(|p| Some(*p) != cur_uri.as_ref()) {
                if st.recent.len() >= RECENT_MAX {
                    st.recent.pop_front();
                }
                st.recent.push_back(prev.clone());
            }
            // Los votos para saltar valen solo para la canción en la que se emitieron.
            if st.votes_for != cur_uri {
                st.votes.clear();
                st.votes_for = cur_uri.clone();
            }
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

    /// Lee la cola de Spotify al cambiar de canción y cada `QUEUE_REFRESH_MS`; sin canción la vacía.
    async fn refresh_queue(&self, current: Option<String>) {
        if current.is_none() {
            self.apply_queue(Vec::new());
            return;
        }
        let now = self.clock.now_ms();
        {
            let st = self.lock();
            if st.queue_for == current && now - st.queue_at_ms < QUEUE_REFRESH_MS {
                return;
            }
        }
        match self.api.queue_list().await {
            Ok(tracks) => {
                {
                    let mut st = self.lock();
                    st.queue_for = current;
                    st.queue_at_ms = now;
                }
                self.apply_queue(tracks);
            }
            Err(e) => tracing::debug!(error = %e, "no se pudo leer la cola de Spotify"),
        }
    }

    /// Guarda la cola (con quién pidió cada canción) y la publica si cambió.
    pub fn apply_queue(&self, tracks: Vec<Track>) {
        let payload = {
            let mut st = self.lock();
            let items: Vec<QueueItem> = tracks
                .into_iter()
                .map(|track| {
                    let requested_by = st.pending.iter().find(|r| r.uri == track.uri).map(|r| r.unique_id.clone()).filter(|u| !u.is_empty());
                    QueueItem { track, requested_by }
                })
                .collect();
            if st.queue == items {
                return;
            }
            st.queue = items;
            queue_payload(&st.queue)
        };
        self.hub.publish_retained(QUEUE_CHANNEL, payload);
    }

    /// Vuelve a publicar la cola real (tras una prueba del overlay con datos de ejemplo).
    pub fn republish_queue(&self) {
        let payload = queue_payload(&self.lock().queue);
        self.hub.publish_retained(QUEUE_CHANNEL, payload);
    }

    /// Un sondeo. Devuelve cuánto esperar hasta el siguiente.
    pub async fn poll_once(&self) -> Duration {
        let configured = !effective_client_id(&self.config().client_id, default_client_id()).is_empty() && self.auth.is_connected();
        if !configured {
            self.apply_playing(None);
            self.apply_queue(Vec::new());
            return POLL_DISCONNECTED;
        }
        match self.api.now_playing().await {
            Ok(p) => {
                let playing = p.as_ref().is_some_and(|p| p.is_playing);
                let uri = p.as_ref().map(|p| p.track.uri.clone());
                self.apply_playing(p);
                self.refresh_queue(uri).await;
                if playing { POLL_PLAYING } else { POLL_IDLE }
            }
            Err(ApiError::RateLimited(s)) => Duration::from_secs(s),
            Err(ApiError::NotConnected) => {
                self.apply_playing(None);
                self.apply_queue(Vec::new());
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
                        if let Some(outcome) = svc.on_event(&ev).await {
                            tracing::debug!(%outcome, "petición de canción");
                        }
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
