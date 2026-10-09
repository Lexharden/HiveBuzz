//! Comandos de Tauri de las integraciones (Fase 5): OBS.

use tauri::State;

use crate::app::AppState;
use crate::error::Result;
use crate::executors::obs::{ObsConfig, ObsInfo};

#[tauri::command]
pub fn get_obs_config(state: State<'_, AppState>) -> ObsConfig {
    state.obs.config()
}

#[tauri::command]
pub async fn set_obs_config(state: State<'_, AppState>, config: ObsConfig) -> Result<ObsConfig> {
    state.obs.set_config(config).await
}

#[tauri::command]
pub fn has_obs_password(state: State<'_, AppState>) -> bool {
    state.obs.has_password()
}

/// Guarda la contraseña de OBS en el llavero (vacía = borrarla). Nunca se devuelve a la UI.
#[tauri::command]
pub fn set_obs_password(state: State<'_, AppState>, password: String) -> Result<()> {
    state.obs.set_password(&password)
}

#[tauri::command]
pub async fn test_obs(state: State<'_, AppState>) -> Result<ObsInfo> {
    state.obs.test().await
}
