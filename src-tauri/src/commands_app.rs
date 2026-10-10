//! Comandos de Tauri de la app en sí: preferencias, inicio con el sistema y logs.

use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt;

use crate::app::AppState;
use crate::error::{AppError, Result};
use crate::logs::{self, LogEntry};
use crate::prefs::AppPrefs;

#[tauri::command]
pub fn get_app_prefs(state: State<'_, AppState>) -> AppPrefs {
    state.prefs.get()
}

#[tauri::command]
pub async fn set_app_prefs(state: State<'_, AppState>, prefs: AppPrefs) -> Result<AppPrefs> {
    state.prefs.set(prefs).await
}

#[tauri::command]
pub fn get_autostart(app: AppHandle) -> Result<bool> {
    app.autolaunch().is_enabled().map_err(|e| AppError::Invalid(format!("inicio con el sistema: {e}")))
}

#[tauri::command]
pub fn set_autostart(app: AppHandle, enabled: bool) -> Result<bool> {
    let manager = app.autolaunch();
    let res = if enabled { manager.enable() } else { manager.disable() };
    res.map_err(|e| AppError::Invalid(format!("inicio con el sistema: {e}")))?;
    manager.is_enabled().map_err(|e| AppError::Invalid(format!("inicio con el sistema: {e}")))
}

#[tauri::command]
pub fn get_logs(min_level: String, limit: usize) -> Vec<LogEntry> {
    logs::global().snapshot(&min_level, limit.clamp(1, 2_000))
}

#[tauri::command]
pub fn clear_logs() {
    logs::global().clear();
}

#[tauri::command]
pub async fn export_logs(path: String) -> Result<()> {
    tokio::fs::write(path, logs::global().to_text()).await?;
    Ok(())
}

// ---- Estadísticas por transmisión ----

use crate::stats::{StreamStats, StreamSummary};

#[tauri::command]
pub async fn list_streams(state: State<'_, AppState>, limit: Option<u32>) -> Result<Vec<StreamSummary>> {
    state.stats.list(limit.unwrap_or(100)).await
}

#[tauri::command]
pub async fn get_stream(state: State<'_, AppState>, id: i64) -> Result<StreamStats> {
    state.stats.get(id).await
}

#[tauri::command]
pub async fn delete_stream(state: State<'_, AppState>, id: i64) -> Result<bool> {
    state.stats.delete(id).await
}

// ---- Perfiles, exportar e importar ----

use crate::backup::{self, Summary};
use crate::profiles::ProfileInfo;

#[tauri::command]
pub async fn list_profiles(state: State<'_, AppState>) -> Result<Vec<ProfileInfo>> {
    state.profiles.list().await
}

#[tauri::command]
pub fn active_profile(state: State<'_, AppState>) -> Option<String> {
    state.profiles.active()
}

/// Guarda las reglas y overlays actuales como perfil (nuevo, o sobrescribiendo `id`).
#[tauri::command]
pub async fn save_profile(state: State<'_, AppState>, id: Option<String>, name: String) -> Result<ProfileInfo> {
    state.profiles.save_current(id, &name).await
}

#[tauri::command]
pub async fn apply_profile(state: State<'_, AppState>, id: String) -> Result<()> {
    state.profiles.apply(&id).await
}

#[tauri::command]
pub async fn rename_profile(state: State<'_, AppState>, id: String, name: String) -> Result<()> {
    state.profiles.rename(&id, &name).await
}

#[tauri::command]
pub async fn delete_profile(state: State<'_, AppState>, id: String) -> Result<bool> {
    state.profiles.delete(&id).await
}

#[tauri::command]
pub async fn export_config(state: State<'_, AppState>, path: String) -> Result<Summary> {
    // Lo que está en memoria (progreso de metas y timers) se vuelca antes para exportarlo al día.
    state.goals.flush().await;
    state.timers.flush().await;
    backup::export(&state.db, &state.data_dir, std::path::Path::new(&path), env!("CARGO_PKG_VERSION"), state.clock.now_ms()).await
}

/// Valida el archivo y lo deja pendiente: se aplica al reiniciar.
#[tauri::command]
pub async fn import_config(state: State<'_, AppState>, path: String) -> Result<Summary> {
    let (src, data_dir) = (std::path::PathBuf::from(path), state.data_dir.clone());
    tokio::task::spawn_blocking(move || backup::stage_import(&src, &data_dir))
        .await
        .map_err(|e| AppError::Invalid(format!("la importación falló: {e}")))?
}

#[tauri::command]
pub fn has_pending_import(state: State<'_, AppState>) -> bool {
    backup::has_pending(&state.data_dir)
}

#[tauri::command]
pub fn cancel_pending_import(state: State<'_, AppState>) -> Result<()> {
    backup::cancel_pending(&state.data_dir)
}

/// Aviso del último arranque sobre una importación (aplicada o fallida), una sola vez.
#[tauri::command]
pub fn take_import_notice(state: State<'_, AppState>) -> Option<String> {
    state.import_notice.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()
}

#[tauri::command]
pub fn restart_app(app: AppHandle) {
    app.restart();
}

// ---- Actualizaciones ----

use crate::updater::{self, ReleaseInfo, UpdateInfo};

/// Busca la última versión publicada.
#[tauri::command]
pub async fn check_update(app: AppHandle, state: State<'_, AppState>) -> Result<UpdateInfo> {
    updater::check(&app, None, &state.pending_update).await
}

/// Versiones publicadas, de la más nueva a la más antigua.
#[tauri::command]
pub async fn list_releases(app: AppHandle) -> Result<Vec<ReleaseInfo>> {
    updater::list_releases(&app.package_info().version.to_string()).await
}

/// Prepara una versión concreta (también anterior) para instalarla con `install_update`.
#[tauri::command]
pub async fn prepare_release(app: AppHandle, state: State<'_, AppState>, tag: String) -> Result<UpdateInfo> {
    updater::check(&app, Some(&tag), &state.pending_update).await
}

#[tauri::command]
pub async fn install_update(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    // En Windows el instalador cierra el proceso sin pasar por `RunEvent::Exit`: se guarda antes.
    updater::install(&app, &state.pending_update, state.persist()).await
}
