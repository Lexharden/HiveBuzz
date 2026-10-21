//! Comandos de la UI para puntos, ruleta y encuestas.

use std::path::PathBuf;

use serde::Serialize;
use tauri::State;

use crate::app::AppState;
use crate::db::{ImportMode, ImportReport};
use crate::error::{AppError, Result};
use crate::interact::poll::PollView;
use crate::interact::wheel::WheelConfig;
use crate::points::config::PointsConfig;
use crate::points::{HistoryEntry, SortKey, Viewer};

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
