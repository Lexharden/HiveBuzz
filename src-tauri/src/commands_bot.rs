//! Comandos de la UI para puntos, chatbot y sesión de TikTok.

use std::path::PathBuf;

use serde::Serialize;
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};

use crate::app::AppState;
use crate::bot::model::BotConfig;
use crate::bot::outbox::LogEntry;
use crate::db::{ImportMode, ImportReport};
use crate::error::{AppError, Result};
use crate::interact::poll::PollView;
use crate::interact::wheel::WheelConfig;
use crate::points::config::PointsConfig;
use crate::points::{HistoryEntry, SortKey, Viewer};
use crate::source::protocol::SessionPayload;

// ---- Puntos -------------------------------------------------------------------------------------

#[tauri::command]
pub fn get_points_config(state: State<'_, AppState>) -> PointsConfig {
    state.points.config()
}

#[tauri::command]
pub async fn set_points_config(state: State<'_, AppState>, config: PointsConfig) -> Result<PointsConfig> {
    state.points.set_config(config).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewerPage {
    pub items: Vec<Viewer>,
    pub total: u64,
}

fn sort_key(raw: &str) -> Result<SortKey> {
    Ok(match raw {
        "points" => SortKey::Points,
        "name" => SortKey::Name,
        "lastSeen" => SortKey::LastSeen,
        "coinsGifted" => SortKey::CoinsGifted,
        other => return Err(AppError::Invalid(format!("orden desconocido: «{other}»"))),
    })
}

#[tauri::command]
pub async fn list_viewers(state: State<'_, AppState>, search: String, sort: String, limit: u32, offset: u32) -> Result<ViewerPage> {
    let (items, total) = state.points.list(&search, sort_key(&sort)?, limit.clamp(1, 500), offset).await?;
    Ok(ViewerPage { items, total })
}

#[tauri::command]
pub async fn viewer_history(state: State<'_, AppState>, user_id: String) -> Result<Vec<HistoryEntry>> {
    state.points.history(&user_id, 100).await
}

#[tauri::command]
pub async fn adjust_viewer_points(state: State<'_, AppState>, user_id: String, delta: i64) -> Result<u64> {
    state.points.adjust(&user_id, delta).await
}

#[tauri::command]
pub async fn set_viewer_points(state: State<'_, AppState>, user_id: String, value: u64) -> Result<()> {
    state.points.set_points(&user_id, value).await
}

#[tauri::command]
pub async fn delete_viewer(state: State<'_, AppState>, user_id: String) -> Result<bool> {
    state.points.delete_viewer(&user_id).await
}

#[tauri::command]
pub async fn clear_viewers(state: State<'_, AppState>) -> Result<()> {
    state.points.clear_all().await
}

/// Escribe el CSV de espectadores en `path` (elegida por el usuario en el diálogo de guardado).
#[tauri::command]
pub async fn export_viewers_csv(state: State<'_, AppState>, path: String) -> Result<()> {
    let csv = state.points.export_csv().await?;
    tokio::fs::write(PathBuf::from(path), csv).await?;
    Ok(())
}

/// `mode`: `replace` (los saldos del archivo sustituyen a los actuales) o `add` (se suman).
#[tauri::command]
pub async fn import_viewers_csv(state: State<'_, AppState>, path: String, mode: String) -> Result<ImportReport> {
    let mode = match mode.as_str() {
        "replace" => ImportMode::Replace,
        "add" => ImportMode::Add,
        other => return Err(AppError::Invalid(format!("modo de importación desconocido: «{other}»"))),
    };
    const MAX_CSV_BYTES: u64 = 20 * 1024 * 1024;
    let path = PathBuf::from(path);
    if tokio::fs::metadata(&path).await?.len() > MAX_CSV_BYTES {
        return Err(AppError::Invalid("el archivo CSV es demasiado grande (máximo 20 MB)".into()));
    }
    let text = tokio::fs::read_to_string(&path).await?;
    state.points.import_csv(&text, mode).await
}

// ---- Chatbot ------------------------------------------------------------------------------------

#[tauri::command]
pub fn get_bot_config(state: State<'_, AppState>) -> BotConfig {
    state.bot.config()
}

#[tauri::command]
pub async fn set_bot_config(state: State<'_, AppState>, config: BotConfig) -> Result<BotConfig> {
    state.bot.set_config(config).await
}

#[tauri::command]
pub fn get_bot_log(state: State<'_, AppState>) -> Vec<LogEntry> {
    state.bot.log()
}

#[tauri::command]
pub fn clear_bot_log(state: State<'_, AppState>) {
    state.bot.clear_log();
}

/// Mensaje de prueba: sirve para comprobar que la sesión de TikTok permite escribir.
#[tauri::command]
pub fn bot_say(state: State<'_, AppState>, text: String) -> Result<()> {
    if state.bot.say(&text, "prueba manual") {
        Ok(())
    } else {
        Err(AppError::Invalid("el mensaje está vacío o la cola del bot está llena".into()))
    }
}

// ---- Ruleta y encuestas -----------------------------------------------------------------------------

#[tauri::command]
pub fn get_wheel_config(state: State<'_, AppState>) -> WheelConfig {
    state.wheel.config()
}

#[tauri::command]
pub async fn set_wheel_config(state: State<'_, AppState>, config: WheelConfig) -> Result<WheelConfig> {
    state.wheel.set_config(config).await
}

/// Gira la ruleta como prueba (el premio también se ejecuta). No espera al final del giro.
#[tauri::command]
pub fn spin_wheel_test(state: State<'_, AppState>) -> Result<()> {
    if state.wheel.config().segments.iter().all(|s| s.weight == 0) {
        return Err(AppError::Invalid("la ruleta no tiene premios con probabilidad".into()));
    }
    let wheel = std::sync::Arc::clone(&state.wheel);
    tauri::async_runtime::spawn(async move {
        let vars = [("user".to_string(), "prueba".to_string()), ("nickname".to_string(), "Prueba".to_string())].into();
        if let Err(e) = wheel.spin(vars).await {
            tracing::warn!(error = %e, "falló el giro de prueba");
        }
    });
    Ok(())
}

#[tauri::command]
pub fn get_poll(state: State<'_, AppState>) -> Option<PollView> {
    state.polls.current()
}

#[tauri::command]
pub fn start_poll(state: State<'_, AppState>, question: String, options: Vec<String>, duration_sec: u64) -> Result<PollView> {
    state.polls.start(&question, &options, duration_sec)
}

#[tauri::command]
pub fn stop_poll(state: State<'_, AppState>) -> Option<PollView> {
    state.polls.stop()
}

#[tauri::command]
pub fn clear_poll(state: State<'_, AppState>) {
    state.polls.clear();
}

// ---- Sesión de TikTok (opcional; solo para que el bot escriba) ------------------------------------

const LOGIN_WINDOW: &str = "tiktok-login";
const LOGIN_URL: &str = "https://www.tiktok.com/login";
const COOKIES_URL: &str = "https://www.tiktok.com/";

fn parse_url(raw: &str) -> Result<tauri::Url> {
    raw.parse().map_err(|_| AppError::Invalid("URL no válida".into()))
}

/// ¿Hay una sesión de TikTok guardada?
#[tauri::command]
pub fn has_tiktok_session(state: State<'_, AppState>) -> Result<bool> {
    Ok(crate::secrets::load_tiktok_session(state.secrets.as_ref())?.is_some())
}

/// Abre la página de inicio de sesión de TikTok en una ventana aparte. La contraseña se escribe en la
/// web de TikTok; HiveBuzz solo lee después las cookies de sesión y las guarda en el llavero del sistema.
#[tauri::command]
pub async fn tiktok_login_start(app: AppHandle) -> Result<()> {
    if let Some(w) = app.get_webview_window(LOGIN_WINDOW) {
        let _ = w.set_focus();
        return Ok(());
    }
    WebviewWindowBuilder::new(&app, LOGIN_WINDOW, WebviewUrl::External(parse_url(LOGIN_URL)?))
        .title("Iniciar sesión en TikTok")
        .inner_size(520.0, 760.0)
        .build()
        .map_err(|e| AppError::Invalid(format!("no se pudo abrir la ventana de TikTok: {e}")))?;
    Ok(())
}

/// Lee las cookies de la ventana de inicio de sesión y, si hay sesión, la guarda y cierra la ventana.
/// Devuelve `false` si aún no ha terminado de iniciar sesión.
#[tauri::command]
pub async fn tiktok_login_finish(app: AppHandle, state: State<'_, AppState>) -> Result<bool> {
    let Some(w) = app.get_webview_window(LOGIN_WINDOW) else {
        return Err(AppError::Invalid("la ventana de inicio de sesión no está abierta".into()));
    };
    let cookies = w
        .cookies_for_url(parse_url(COOKIES_URL)?)
        .map_err(|e| AppError::Invalid(format!("no se pudieron leer las cookies: {e}")))?;
    let find = |name: &str| cookies.iter().find(|c| c.name() == name).map(|c| c.value().to_string());
    let (Some(session_id), Some(tt_target_idc)) = (find("sessionid"), find("tt-target-idc")) else {
        return Ok(false);
    };
    crate::secrets::save_tiktok_session(state.secrets.as_ref(), &SessionPayload { session_id, tt_target_idc })?;
    let _ = w.close();
    Ok(true)
}

/// Cierra la sesión: borra las cookies guardadas y los datos de la ventana de TikTok.
#[tauri::command]
pub async fn tiktok_logout(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    crate::secrets::clear_tiktok_session(state.secrets.as_ref())?;
    if let Some(w) = app.get_webview_window(LOGIN_WINDOW) {
        let _ = w.clear_all_browsing_data();
        let _ = w.close();
    }
    Ok(())
}
