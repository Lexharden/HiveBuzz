//! Comandos de Tauri de Twitch: inicio de sesión (opcional) y configuración avanzada.

use tauri::State;
use tauri_plugin_opener::OpenerExt;

use crate::app::AppState;
use crate::error::{AppError, Result};
use crate::twitch::auth::{DeviceInfo, LoginState};
use crate::twitch::service::{TwitchConfig, TwitchStatus};

#[tauri::command]
pub async fn twitch_status(state: State<'_, AppState>) -> Result<TwitchStatus> {
    Ok(state.twitch.status().await)
}

#[tauri::command]
pub fn twitch_get_config(state: State<'_, AppState>) -> TwitchConfig {
    state.twitch.config()
}

#[tauri::command]
pub async fn twitch_set_config(state: State<'_, AppState>, config: TwitchConfig) -> Result<TwitchConfig> {
    state.twitch.set_config(config).await
}

/// Empieza el inicio de sesión: devuelve el código que el streamer debe escribir en twitch.tv/activate.
#[tauri::command]
pub async fn twitch_login_start(state: State<'_, AppState>) -> Result<DeviceInfo> {
    state.twitch.auth().begin_login().await.map_err(AppError::from)
}

#[tauri::command]
pub fn twitch_login_state(state: State<'_, AppState>) -> LoginState {
    state.twitch.auth().login_state()
}

#[tauri::command]
pub fn twitch_login_cancel(state: State<'_, AppState>) {
    state.twitch.auth().cancel_login();
}

#[tauri::command]
pub fn twitch_logout(state: State<'_, AppState>) -> Result<()> {
    state.twitch.auth().logout()
}

/// Abre la página de activación de Twitch en el navegador (solo esa dirección, nunca una que llegue de fuera).
#[tauri::command]
pub fn twitch_open_activation(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<()> {
    let url = match state.twitch.auth().login_state() {
        LoginState::Pending { info } if info.verification_uri.starts_with("https://www.twitch.tv/") || info.verification_uri.starts_with("https://twitch.tv/") => info.verification_uri,
        _ => "https://www.twitch.tv/activate".to_string(),
    };
    app.opener().open_url(url, None::<&str>).map_err(|e| AppError::Invalid(format!("no se pudo abrir el navegador: {e}")))
}
