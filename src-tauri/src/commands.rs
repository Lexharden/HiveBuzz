//! Comandos que invoca la UI.

use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Map, Value};
use tauri::{AppHandle, Emitter, State};

use crate::actions::queue::{Outcome, QueueStats};
use crate::app::{AppState, EVT_TTS_INSTALL};
use crate::audio::TAG_SOUND;
use crate::counters::Counters;
use crate::db::KEY_LAST_USERNAME;

/// Último canal de Twitch al que se conectó (para rellenar el campo la próxima vez).
const KEY_LAST_TWITCH_CHANNEL: &str = "twitch_last_channel";
use crate::error::{AppError, Result};
use crate::events::{LiveEvent, Platform};
use crate::goals::Goal;
use crate::leaderboard::{DonorEntry, Scope};
use crate::media::Media;
use crate::overlay_config::schema::{registry, OverlayDef};
use crate::rules::model::Rule;
use crate::secrets::KEY_EULER_API;
use crate::simulator::SimKind;
use crate::sounds::Sound;
use crate::connections::PlatformStatus;
use crate::timers::service::{Control, TimerView};
use crate::timers::TimerConfig;
use crate::tts::policy::{TtsConfig, VoiceInfo};
use crate::tts::provision::{self, Progress, VOICE_CATALOG};
use crate::tts::service::SpeakOptions;

// ---- Conexión y ajustes generales ---------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayInfo {
    pub id: &'static str,
    /// Clave de i18n del nombre.
    pub name: &'static str,
    pub url: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub server_port: u16,
    pub server_error: Option<String>,
    pub overlays: Vec<OverlayInfo>,
    /// `http://127.0.0.1:<puerto>/media`: base para mostrar miniaturas de la biblioteca de medios.
    pub media_base: String,
    pub overlay_token: String,
    pub last_username: Option<String>,
    pub last_twitch_channel: Option<String>,
    pub has_euler_api_key: bool,
}

/// Conecta una plataforma. `name` es el @usuario de TikTok o el canal de Twitch.
#[tauri::command]
pub async fn connect(state: State<'_, AppState>, platform: Platform, username: String) -> Result<()> {
    match platform {
        Platform::Tiktok => {
            let key = state.secrets.get(KEY_EULER_API)?;
            let session = crate::secrets::load_tiktok_session(state.secrets.as_ref())?;
            state.conn.connect(&username, key, session).await?;
            // Solo se guarda si la validación del nombre pasó (connect devuelve error si no).
            let clean = crate::connection::normalize_username(&username)?;
            state.db.set_setting(KEY_LAST_USERNAME, &clean).await
        }
        Platform::Twitch => {
            state.connections.service(Platform::Twitch).connect(&username, None, None).await?;
            let clean = crate::twitch::normalize_channel(&username)?;
            state.db.set_setting(KEY_LAST_TWITCH_CHANNEL, &clean).await
        }
    }
}

#[tauri::command]
pub async fn disconnect(state: State<'_, AppState>, platform: Platform) -> Result<()> {
    state.connections.service(platform).disconnect().await
}

#[tauri::command]
pub fn get_statuses(state: State<'_, AppState>) -> Vec<PlatformStatus> {
    state.connections.statuses()
}

#[tauri::command]
pub async fn get_app_info(state: State<'_, AppState>) -> Result<AppInfo> {
    let base = format!("http://127.0.0.1:{}", state.server_port);
    let token = &state.overlay_token;
    // Los overlays hablan el idioma de la app (el español es el de fábrica).
    let lang = match state.prefs.get().language.as_str() {
        "es" => String::new(),
        other => format!("&lang={other}"),
    };
    Ok(AppInfo {
        server_port: state.server_port,
        server_error: state.server_error.clone(),
        overlays: registry()
            .into_iter()
            .map(|d| OverlayInfo { id: d.id, name: d.name, url: format!("{base}/overlay/{}?token={token}{lang}", d.id) })
            .collect(),
        media_base: format!("{base}/media"),
        overlay_token: token.clone(),
        last_username: state.db.get_setting(KEY_LAST_USERNAME).await?,
        last_twitch_channel: state.db.get_setting(KEY_LAST_TWITCH_CHANNEL).await?,
        has_euler_api_key: state.secrets.get(KEY_EULER_API)?.is_some(),
    })
}

/// Guarda (o borra, si llega vacía) la API key propia de Euler Stream en el llavero.
/// Nunca se devuelve a la UI.
#[tauri::command]
pub fn set_euler_api_key(state: State<'_, AppState>, key: Option<String>) -> Result<()> {
    match key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty()) {
        Some(k) => state.secrets.set(KEY_EULER_API, &k),
        None => state.secrets.delete(KEY_EULER_API),
    }
}

/// El nuevo puerto se aplica al reiniciar la app.
#[tauri::command]
pub async fn set_server_port(state: State<'_, AppState>, port: u16) -> Result<()> {
    state.db.set_server_port(port).await
}

#[tauri::command]
pub fn simulate_event(state: State<'_, AppState>, kind: SimKind, platform: Option<Platform>) -> LiveEvent {
    state.sim.emit_for(kind, platform.unwrap_or_default())
}

#[tauri::command]
pub async fn simulate_burst(state: State<'_, AppState>, count: u32, platform: Option<Platform>) -> Result<()> {
    if !(1..=200).contains(&count) {
        return Err(AppError::Invalid("count debe estar entre 1 y 200".into()));
    }
    // La tarea se ejecuta sola; no hace falta esperarla.
    drop(state.sim.burst_for(count, Duration::from_millis(250), platform.unwrap_or_default()));
    Ok(())
}

#[tauri::command]
pub async fn recent_events(state: State<'_, AppState>, limit: u32) -> Result<Vec<LiveEvent>> {
    state.db.recent_events(limit.min(500)).await
}

// ---- Reglas y cola -------------------------------------------------------------------------------

#[tauri::command]
pub fn list_rules(state: State<'_, AppState>) -> Vec<Rule> {
    state.rules.list()
}

#[tauri::command]
pub async fn save_rule(state: State<'_, AppState>, rule: Rule) -> Result<()> {
    state.rules.upsert(rule).await
}

#[tauri::command]
pub async fn delete_rule(state: State<'_, AppState>, id: String) -> Result<bool> {
    state.rules.delete(&id).await
}

#[tauri::command]
pub async fn set_rule_enabled(state: State<'_, AppState>, id: String, enabled: bool) -> Result<()> {
    state.rules.set_enabled(&id, enabled).await
}

/// Botón «probar»: ejecuta la regla con un evento de ejemplo. Devuelve `queued`, `dropped` o `empty`.
#[tauri::command]
pub async fn test_rule(state: State<'_, AppState>, id: String) -> Result<&'static str> {
    Ok(match state.rules.test_rule(&id).await? {
        Outcome::Queued => "queued",
        Outcome::Dropped => "dropped",
        Outcome::Empty => "empty",
    })
}

/// Tipos de acción disponibles (los ejecutores registrados).
#[tauri::command]
pub fn list_action_types(state: State<'_, AppState>) -> Vec<String> {
    state.registry.kinds().into_iter().map(str::to_string).collect()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueInfo {
    pub pending: usize,
    pub running: usize,
    pub jobs: usize,
}

impl From<QueueStats> for QueueInfo {
    fn from(s: QueueStats) -> Self {
        Self { pending: s.pending, running: s.running, jobs: s.jobs }
    }
}

#[tauri::command]
pub fn queue_stats(state: State<'_, AppState>) -> QueueInfo {
    state.queue.stats().into()
}

#[tauri::command]
pub async fn clear_queue(state: State<'_, AppState>) -> Result<()> {
    state.queue.clear().await;
    state.audio.stop_all();
    Ok(())
}

// ---- Sonidos y medios --------------------------------------------------------------------------------

#[tauri::command]
pub async fn list_sounds(state: State<'_, AppState>) -> Result<Vec<Sound>> {
    state.sounds.list().await
}

#[tauri::command]
pub async fn import_sound(state: State<'_, AppState>, path: String, name: Option<String>) -> Result<Sound> {
    state.sounds.import(&PathBuf::from(path), name).await
}

#[tauri::command]
pub async fn update_sound(state: State<'_, AppState>, id: String, name: Option<String>, volume: Option<u8>) -> Result<Sound> {
    state.sounds.update(&id, name, volume).await
}

#[tauri::command]
pub async fn delete_sound(state: State<'_, AppState>, id: String) -> Result<()> {
    state.sounds.delete(&id).await
}

/// Vista previa de un sonido de la biblioteca (con su volumen propio).
#[tauri::command]
pub async fn preview_sound(state: State<'_, AppState>, id: String) -> Result<()> {
    let sound = state
        .sounds
        .get(&id)
        .await?
        .ok_or_else(|| AppError::Invalid("el sonido no existe".into()))?;
    state.audio.play(state.sounds.path_of(&sound), f32::from(sound.volume) / 100.0, TAG_SOUND).await
}

#[tauri::command]
pub fn stop_audio(state: State<'_, AppState>) {
    state.audio.stop_all();
}

#[tauri::command]
pub async fn list_media(state: State<'_, AppState>) -> Result<Vec<Media>> {
    state.media.list().await
}

#[tauri::command]
pub async fn import_media(state: State<'_, AppState>, path: String, name: Option<String>) -> Result<Media> {
    state.media.import(&PathBuf::from(path), name).await
}

#[tauri::command]
pub async fn rename_media(state: State<'_, AppState>, id: String, name: String) -> Result<()> {
    state.media.rename(&id, &name).await
}

#[tauri::command]
pub async fn delete_media(state: State<'_, AppState>, id: String) -> Result<()> {
    state.media.delete(&id).await
}

// ---- TTS ------------------------------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogEntry {
    pub id: &'static str,
    pub label: &'static str,
    pub approx_mb: u32,
    pub installed: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsStatus {
    pub piper_installed: bool,
    /// La instalación automática de Piper solo existe en Windows.
    pub can_install_piper: bool,
    pub catalog: Vec<CatalogEntry>,
}

#[tauri::command]
pub fn get_tts_config(state: State<'_, AppState>) -> TtsConfig {
    state.tts.config()
}

#[tauri::command]
pub async fn set_tts_config(state: State<'_, AppState>, config: TtsConfig) -> Result<TtsConfig> {
    state.tts.set_config(config).await
}

#[tauri::command]
pub async fn list_tts_voices(state: State<'_, AppState>) -> Result<Vec<VoiceInfo>> {
    Ok(state.tts.voices().await)
}

#[tauri::command]
pub fn get_tts_status(state: State<'_, AppState>) -> TtsStatus {
    let paths = state.tts.piper_paths();
    TtsStatus {
        piper_installed: paths.exe.is_file(),
        can_install_piper: cfg!(windows),
        catalog: VOICE_CATALOG
            .iter()
            .map(|v| CatalogEntry {
                id: v.id,
                label: v.label,
                approx_mb: v.approx_mb,
                installed: paths.voices_dir.join(format!("{}.onnx", v.id)).is_file(),
            })
            .collect(),
    }
}

/// Lee un texto con la configuración actual (sin pasar por la cola).
#[tauri::command]
pub async fn tts_preview(state: State<'_, AppState>, text: String, voice: Option<String>) -> Result<()> {
    let vars = [("nickname".to_string(), "Prueba".to_string())].into();
    state.tts.speak(&text, &vars, voice.as_deref().filter(|v| !v.is_empty()), SpeakOptions::default()).await
}

/// Botón «saltar»: corta la voz actual; la cola sigue con la siguiente.
#[tauri::command]
pub fn tts_skip(state: State<'_, AppState>) {
    state.tts.skip();
}

#[tauri::command]
pub async fn install_piper(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    let on = |p: Progress| {
        let _ = app.emit(EVT_TTS_INSTALL, p);
    };
    provision::install_piper(&state.tts_dir, &on).await?;
    state.tts.invalidate_voices();
    Ok(())
}

#[tauri::command]
pub async fn install_piper_voice(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<()> {
    let on = |p: Progress| {
        let _ = app.emit(EVT_TTS_INSTALL, p);
    };
    provision::install_voice(&state.tts.piper_paths().voices_dir, &id, &on).await?;
    state.tts.invalidate_voices();
    Ok(())
}

// ---- Configuración y pruebas de overlays -----------------------------------------------------------------

/// Esquema de todos los overlays (campos, rangos, valores por defecto) para generar el editor.
#[tauri::command]
pub fn list_overlays() -> Vec<OverlayDef> {
    registry()
}

#[tauri::command]
pub async fn get_overlay_config(state: State<'_, AppState>, id: String) -> Result<Map<String, Value>> {
    state.overlay_cfg.get(&id).await
}

/// Aplica un parche (solo las opciones que cambian). Si algo es inválido no se aplica nada.
#[tauri::command]
pub async fn set_overlay_config(state: State<'_, AppState>, id: String, patch: Map<String, Value>) -> Result<Map<String, Value>> {
    state.overlay_cfg.set(&id, &patch).await
}

#[tauri::command]
pub async fn reset_overlay_config(state: State<'_, AppState>, id: String) -> Result<Map<String, Value>> {
    state.overlay_cfg.reset(&id).await
}

/// Manda contenido de ejemplo a un overlay para ver cómo queda. Los overlays de estado (metas, timer,
/// ranking, contadores) muestran datos reales y no tienen prueba.
#[tauri::command]
pub fn test_overlay(state: State<'_, AppState>, id: String) -> Result<()> {
    match id.as_str() {
        "alerts" => state.hub.publish(
            "alerts",
            json!({
                "id": uuid::Uuid::new_v4().to_string(),
                "title": "Luna",
                "text": "envió 5× Rose",
                "durationMs": 4000,
            }),
        ),
        "chat" => {
            for _ in 0..3 {
                state.sim.emit(SimKind::Chat);
            }
        }
        "feed" => {
            for kind in [SimKind::Follow, SimKind::Chat, SimKind::Gift, SimKind::Share] {
                state.sim.emit(kind);
            }
        }
        "gifts" => {
            state.sim.emit(SimKind::Gift);
            state.sim.emit(SimKind::BigGift);
        }
        "nowplaying" => {
            // Un ejemplo que no se retiene; a los pocos segundos se vuelve a publicar el estado real.
            state.hub.publish(
                "nowplaying",
                json!({ "kind": "nowplaying", "playing": true, "title": "Canción de ejemplo", "artist": "Artista", "image": null, "progressMs": 60000, "durationMs": 180_000, "requestedBy": "luna" }),
            );
            let spotify = std::sync::Arc::clone(&state.spotify);
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(8)).await;
                spotify.republish();
            });
        }
        _ => return Err(AppError::Invalid("este overlay muestra datos reales y no tiene prueba".into())),
    }
    Ok(())
}

// ---- Metas ----------------------------------------------------------------------------------------------------

#[tauri::command]
pub fn list_goals(state: State<'_, AppState>) -> Vec<Goal> {
    state.goals.list()
}

#[tauri::command]
pub async fn save_goal(state: State<'_, AppState>, goal: Goal) -> Result<()> {
    state.goals.upsert(goal).await
}

#[tauri::command]
pub async fn delete_goal(state: State<'_, AppState>, id: String) -> Result<bool> {
    state.goals.delete(&id).await
}

#[tauri::command]
pub async fn adjust_goal(state: State<'_, AppState>, id: String, delta: i64) -> Result<()> {
    state.goals.adjust(&id, delta).await
}

#[tauri::command]
pub async fn reset_goal(state: State<'_, AppState>, id: String) -> Result<()> {
    state.goals.reset(&id).await
}

// ---- Timers ----------------------------------------------------------------------------------------------------

#[tauri::command]
pub fn list_timers(state: State<'_, AppState>) -> Vec<TimerView> {
    state.timers.list()
}

#[tauri::command]
pub async fn save_timer(state: State<'_, AppState>, config: TimerConfig) -> Result<()> {
    state.timers.upsert(config).await
}

#[tauri::command]
pub async fn delete_timer(state: State<'_, AppState>, id: String) -> Result<bool> {
    state.timers.delete(&id).await
}

/// `op`: `start`, `pause`, `resume`, `reset` o `add` (con `seconds`, puede ser negativo).
#[tauri::command]
pub async fn control_timer(state: State<'_, AppState>, id: String, op: String, seconds: Option<i64>) -> Result<()> {
    let control = match op.as_str() {
        "start" => Control::Start,
        "pause" => Control::Pause,
        "resume" => Control::Resume,
        "reset" => Control::Reset,
        "add" => Control::AddSeconds(seconds.ok_or_else(|| AppError::Invalid("falta «seconds»".into()))?),
        other => return Err(AppError::Invalid(format!("operación de timer desconocida: «{other}»"))),
    };
    state.timers.control(&id, control).await
}

// ---- Ranking, contadores y sesión ----------------------------------------------------------------------

/// `scope`: `session`, `day` o `all`.
#[tauri::command]
pub async fn get_leaderboard(state: State<'_, AppState>, scope: String, limit: u32) -> Result<Vec<DonorEntry>> {
    let scope = match scope.as_str() {
        "session" => Scope::Session,
        "day" => Scope::Day,
        "all" => Scope::All,
        other => return Err(AppError::Invalid(format!("ámbito desconocido: «{other}»"))),
    };
    state.leaderboard.top(scope, limit.min(100)).await
}

#[tauri::command]
pub async fn clear_donor_history(state: State<'_, AppState>) -> Result<()> {
    state.leaderboard.clear_history().await?;
    state.leaderboard.publish().await;
    Ok(())
}

#[tauri::command]
pub fn get_counters(state: State<'_, AppState>) -> Counters {
    state.counters.get()
}

/// Empieza una sesión nueva (vacía el ranking de la sesión y reinicia las metas que así lo piden).
#[tauri::command]
pub fn new_session(state: State<'_, AppState>) -> u64 {
    state.session.start_new()
}