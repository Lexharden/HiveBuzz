//! Servicio de TTS: une motores, filtros, política de chat y reproducción.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError, RwLock};
use std::time::{Duration, Instant};

use serde_json::json;
use tokio::sync::broadcast::error::RecvError;

use super::engine::{split_voice_id, SynthRequest, TtsEngine};
use super::filters::{clean, Cleaned};
use super::piper::PiperPaths;
use super::policy::{decide_chat, resolve_voice, ChatDecision, ChatMemory, MicGuard, MicGuardMode, TtsConfig, VoiceInfo};
use crate::actions::clock::Clock;
use crate::actions::queue::{ActionQueue, NewJob, Outcome};
use crate::audio::{AudioBackend, TAG_TTS};
use crate::bus::EventBus;
use crate::db::Db;
use crate::error::{AppError, Result};
use crate::mic::{MicSettings, SpeechDetector};
use crate::rules::model::{ActionSpec, PlanMode, Step};
use crate::rules::template::Vars;

/// Clave de la configuración en la tabla `settings`.
pub const KEY_TTS_CONFIG: &str = "tts_config";
/// Las voces instaladas casi no cambian: se evita relistar (SAPI lanza PowerShell).
const VOICES_TTL: Duration = Duration::from_secs(60);
/// Cada cuántos eventos se purga la memoria del lector de chat.
const PRUNE_EVERY: u64 = 3_000;
const MEMORY_KEEP_MS: i64 = 6 * 60 * 60 * 1000;
/// Lo máximo que una lectura espera a que el streamer calle (antes de empezar o en pausa). Pasado
/// esto se descarta: el chat caduca, y la cola corta las acciones a los 120 s.
const MIC_MAX_WAIT: Duration = Duration::from_secs(30);
/// Cuánto se retrocede al reanudar para repetir la palabra cortada.
const REPEAT_WORD_REWIND: Duration = Duration::from_millis(1_200);

/// Lo que se puede ajustar por cada lectura.
#[derive(Debug, Clone, Copy, Default)]
pub struct SpeakOptions {
    /// Velocidad 0.5–2.0 (si falta, la de la configuración).
    pub rate: Option<f64>,
    /// Volumen 0–100 de la acción; multiplica el de la configuración.
    pub volume: Option<f64>,
}

pub struct TtsService {
    engines: HashMap<String, Arc<dyn TtsEngine>>,
    config: RwLock<TtsConfig>,
    memory: Mutex<ChatMemory>,
    voices_cache: Mutex<Option<(Instant, Vec<VoiceInfo>)>>,
    audio: Arc<dyn AudioBackend>,
    /// La cola se engancha después de crearla (ella necesita el ejecutor `tts`, que necesita este servicio).
    queue: OnceLock<Arc<ActionQueue>>,
    clock: Arc<dyn Clock>,
    db: Db,
    cache_dir: PathBuf,
    piper_paths: Arc<RwLock<PiperPaths>>,
    default_piper: PiperPaths,
    handled: Mutex<u64>,
    mic: Option<Arc<dyn SpeechDetector>>,
}

pub struct TtsDeps {
    pub engines: Vec<Arc<dyn TtsEngine>>,
    pub audio: Arc<dyn AudioBackend>,
    pub clock: Arc<dyn Clock>,
    pub db: Db,
    /// Carpeta temporal del audio generado.
    pub cache_dir: PathBuf,
    /// Rutas compartidas con el motor Piper (el servicio las actualiza al cambiar la configuración).
    pub piper_paths: Arc<RwLock<PiperPaths>>,
    /// Rutas de Piper cuando la configuración no indica otras (las que instala la app).
    pub default_piper: PiperPaths,
    /// Micrófono para no hablar encima del streamer (`None` = sin esa función).
    pub mic: Option<Arc<dyn SpeechDetector>>,
}

impl TtsService {
    pub fn new(deps: TtsDeps) -> Result<Arc<Self>> {
        std::fs::create_dir_all(&deps.cache_dir)?;
        // Restos de una sesión anterior (la app se cerró a media lectura).
        if let Ok(entries) = std::fs::read_dir(&deps.cache_dir) {
            for e in entries.flatten() {
                let _ = std::fs::remove_file(e.path());
            }
        }
        let engines = deps.engines.into_iter().map(|e| (e.id().to_string(), e)).collect();
        Ok(Arc::new(Self {
            engines,
            config: RwLock::new(TtsConfig::default()),
            memory: Mutex::new(ChatMemory::default()),
            voices_cache: Mutex::new(None),
            audio: deps.audio,
            queue: OnceLock::new(),
            clock: deps.clock,
            db: deps.db,
            cache_dir: deps.cache_dir,
            piper_paths: deps.piper_paths,
            default_piper: deps.default_piper,
            handled: Mutex::new(0),
            mic: deps.mic,
        }))
    }

    /// Rutas de Piper en uso (las configuradas o las de la app).
    pub fn piper_paths(&self) -> PiperPaths {
        self.piper_paths.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Conecta la cola de acciones por la que se encola la lectura del chat.
    pub fn attach_queue(&self, queue: Arc<ActionQueue>) {
        if self.queue.set(queue).is_err() {
            tracing::warn!("la cola del TTS ya estaba conectada");
        }
    }

    // ---- Configuración ----

    /// Carga la configuración guardada (si es ilegible se usa la de fábrica y se avisa).
    pub async fn load_config(&self) -> Result<()> {
        let cfg = match self.db.get_setting(KEY_TTS_CONFIG).await? {
            Some(json) => serde_json::from_str::<TtsConfig>(&json).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "configuración de TTS ilegible; se usa la de fábrica");
                TtsConfig::default()
            }),
            None => TtsConfig::default(),
        };
        self.apply(cfg.sanitized());
        Ok(())
    }

    pub fn config(&self) -> TtsConfig {
        self.config.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Valida, guarda y aplica una configuración nueva. Devuelve la versión saneada.
    pub async fn set_config(&self, cfg: TtsConfig) -> Result<TtsConfig> {
        let cfg = cfg.sanitized();
        self.db.set_setting(KEY_TTS_CONFIG, &serde_json::to_string(&cfg)?).await?;
        self.apply(cfg.clone());
        Ok(cfg)
    }

    fn apply(&self, cfg: TtsConfig) {
        let paths = PiperPaths {
            exe: cfg.piper_path.as_deref().filter(|p| !p.trim().is_empty()).map_or_else(|| self.default_piper.exe.clone(), PathBuf::from),
            voices_dir: cfg
                .piper_voices_dir
                .as_deref()
                .filter(|p| !p.trim().is_empty())
                .map_or_else(|| self.default_piper.voices_dir.clone(), PathBuf::from),
        };
        *self.piper_paths.write().unwrap_or_else(PoisonError::into_inner) = paths;
        if let Some(mic) = &self.mic {
            let g = &cfg.mic_guard;
            mic.configure(g.enabled.then(|| MicSettings { device: g.device.clone(), threshold_db: g.threshold_db, hold_ms: g.hold_ms }));
        }
        *self.config.write().unwrap_or_else(PoisonError::into_inner) = cfg;
        self.invalidate_voices();
    }

    /// Fuerza a relistar las voces (tras instalar Piper o una voz nueva).
    pub fn invalidate_voices(&self) {
        *self.voices_cache.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }

    // ---- Voces y lectura ----

    pub async fn voices(&self) -> Vec<VoiceInfo> {
        if let Some((at, v)) = &*self.voices_cache.lock().unwrap_or_else(PoisonError::into_inner) {
            if at.elapsed() < VOICES_TTL {
                return v.clone();
            }
        }
        // Orden estable: primero los locales (Piper, SAPI) y al final los de red (Edge).
        let mut all = Vec::new();
        for id in ["piper", "sapi", "edge"] {
            if let Some(engine) = self.engines.get(id) {
                all.extend(engine.voices().await);
            }
        }
        *self.voices_cache.lock().unwrap_or_else(PoisonError::into_inner) = Some((Instant::now(), all.clone()));
        all
    }

    /// Limpia, sintetiza y reproduce `text`. Un texto filtrado no es un error: simplemente no se lee.
    pub async fn speak(&self, text: &str, vars: &Vars, voice: Option<&str>, opts: SpeakOptions) -> Result<()> {
        let cfg = self.config();
        let spoken = match clean(text, &cfg.filters) {
            Cleaned::Say(s) => s,
            Cleaned::Skip(reason) => {
                tracing::debug!(reason, "TTS: texto descartado");
                return Ok(());
            }
        };

        let voices = self.voices().await;
        let voice_id = resolve_voice(&cfg, vars, &voices, voice).ok_or_else(|| {
            AppError::Invalid("no hay voces de TTS disponibles: instala Piper o usa las de Windows (SAPI)".into())
        })?;
        let (engine_id, voice_name) = split_voice_id(&voice_id).ok_or_else(|| AppError::Invalid(format!("voz no válida: {voice_id}")))?;
        let engine = self
            .engines
            .get(engine_id)
            .ok_or_else(|| AppError::Invalid(format!("motor de TTS desconocido: {engine_id}")))?;

        let path = engine
            .synthesize(&SynthRequest {
                text: spoken,
                voice: voice_name.to_string(),
                rate: opts.rate.unwrap_or(cfg.rate).clamp(0.5, 2.0),
                out_dir: self.cache_dir.clone(),
            })
            .await?;

        #[allow(clippy::cast_possible_truncation)]
        let volume = (f64::from(cfg.volume) / 100.0 * opts.volume.unwrap_or(100.0).clamp(0.0, 100.0) / 100.0) as f32;
        let played = self.play_guarded(path.clone(), volume, &cfg.mic_guard).await;
        // El audio temporal se borra siempre, haya salido bien o no.
        let _ = tokio::fs::remove_file(&path).await;
        played
    }

    /// Reproduce sin hablar encima del streamer: espera a que calle para empezar y, si empieza a
    /// hablar a mitad, pausa (y repite la palabra o el mensaje) o salta, según la configuración.
    async fn play_guarded(&self, path: PathBuf, volume: f32, guard: &MicGuard) -> Result<()> {
        let Some(mic) = self.mic.as_ref().filter(|_| guard.enabled) else {
            return self.audio.play(path, volume, TAG_TTS).await;
        };
        let mut rx = mic.subscribe();
        if *rx.borrow_and_update() {
            tracing::debug!("TTS: el streamer está hablando; la lectura espera");
            match tokio::time::timeout(MIC_MAX_WAIT, rx.wait_for(|talking| !*talking)).await {
                Ok(_) => {}
                Err(_) => {
                    tracing::debug!("TTS: el streamer no dejó de hablar; se descarta la lectura");
                    return Ok(());
                }
            }
        }

        let play = self.audio.play(path, volume, TAG_TTS);
        tokio::pin!(play);
        let mut paused_until: Option<tokio::time::Instant> = None;
        loop {
            tokio::select! {
                done = &mut play => return done,
                changed = rx.changed() => {
                    if changed.is_err() {
                        // Sin detector: se termina de leer con normalidad.
                        if paused_until.take().is_some() {
                            self.audio.resume_tag(TAG_TTS, Duration::ZERO);
                        }
                        return play.await;
                    }
                    let talking = *rx.borrow_and_update();
                    match (talking, guard.mode, paused_until.is_some()) {
                        (true, MicGuardMode::Skip, _) => {
                            tracing::debug!("TTS: el streamer habla; se salta el mensaje");
                            self.audio.stop_tag(TAG_TTS);
                        }
                        (true, _, false) => {
                            tracing::debug!("TTS: el streamer habla; lectura en pausa");
                            self.audio.pause_tag(TAG_TTS);
                            paused_until = Some(tokio::time::Instant::now() + MIC_MAX_WAIT);
                        }
                        (false, mode, true) => {
                            let rewind = if mode == MicGuardMode::RepeatMessage { Duration::MAX } else { REPEAT_WORD_REWIND };
                            self.audio.resume_tag(TAG_TTS, rewind);
                            paused_until = None;
                        }
                        _ => {}
                    }
                }
                () = tokio::time::sleep_until(paused_until.unwrap_or_else(tokio::time::Instant::now)), if paused_until.is_some() => {
                    tracing::debug!("TTS: pausa demasiado larga; se descarta el mensaje");
                    paused_until = None;
                    self.audio.stop_tag(TAG_TTS);
                }
            }
        }
    }

    /// Botón «saltar»: corta la voz que esté sonando (la cola sigue con la siguiente).
    pub fn skip(&self) {
        self.audio.stop_tag(TAG_TTS);
    }

    // ---- Lectura automática del chat ----

    pub fn spawn(self: &Arc<Self>, bus: &EventBus) -> tokio::task::JoinHandle<()> {
        let this = Arc::clone(self);
        let mut rx = bus.subscribe();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(ev) => this.on_event(&ev).await,
                    Err(RecvError::Lagged(n)) => tracing::warn!(missed = n, "el lector de chat se quedó atrás"),
                    Err(RecvError::Closed) => break,
                }
            }
        })
    }

    async fn on_event(&self, ev: &crate::events::LiveEvent) {
        let cfg = self.config();
        let now = self.clock.now_ms();
        let decision = {
            let mut mem = self.memory.lock().unwrap_or_else(PoisonError::into_inner);
            mem.observe(ev);
            let mut n = self.handled.lock().unwrap_or_else(PoisonError::into_inner);
            *n += 1;
            if (*n).is_multiple_of(PRUNE_EVERY) {
                mem.prune(now, MEMORY_KEEP_MS);
            }
            decide_chat(&cfg, ev, &mut mem, now)
        };
        let ChatDecision::Speak { vars, .. } = decision else {
            if let ChatDecision::Skip(reason) = decision {
                if ev.kind == crate::events::EventType::Chat {
                    tracing::debug!(user = %ev.user.unique_id, reason, "TTS: chat no leído");
                }
            }
            return;
        };
        let Some(queue) = self.queue.get() else {
            tracing::warn!("TTS: la cola de acciones no está conectada; el chat no se lee");
            return;
        };
        let job_user = ev.user.unique_id.clone();
        let job = NewJob {
            rule_id: "tts:chat".into(),
            mode: PlanMode::Sequence,
            steps: vec![Step { delay_ms: 0, action: ActionSpec::new("tts", json!({ "text": cfg.template })) }],
            vars,
            priority: 0,
            ttl_ms: cfg.max_wait_ms,
            refund: None,
        };
        match queue.enqueue(job).await {
            Ok(Outcome::Dropped) => tracing::debug!("TTS: cola llena, se descartó un mensaje de chat"),
            Ok(_) => tracing::debug!(user = %job_user, "TTS: chat encolado"),
            Err(e) => tracing::warn!(error = %e, "TTS: no se pudo encolar un mensaje"),
        }
    }
}

#[cfg(test)]
mod tests;
