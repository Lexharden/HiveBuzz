//! Comandos de Tauri de Spotify: conexión, configuración y prueba de peticiones.

use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::app::AppState;
use crate::error::{AppError, Result};
use crate::spotify::api::Track;
use crate::spotify::service::SpotifyStatus;
use crate::spotify::{SpotifyConfig, CALLBACK_PATH};

fn redirect_uri(port: u16) -> String {
    format!("http://127.0.0.1:{port}{CALLBACK_PATH}")
}

#[tauri::command]
pub fn spotify_get_config(state: State<'_, AppState>) -> SpotifyConfig {
    state.spotify.config()
}

#[tauri::command]
pub async fn spotify_set_config(state: State<'_, AppState>, config: SpotifyConfig) -> Result<SpotifyConfig> {
    state.spotify.set_config(config).await
}

#[tauri::command]
pub fn spotify_status(state: State<'_, AppState>) -> SpotifyStatus {
    state.spotify.status(redirect_uri(state.server_port))
}

/// Abre el navegador del usuario en la página de autorización de Spotify.
#[tauri::command]
pub fn spotify_connect(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    if state.server_error.is_some() {
        return Err(AppError::Invalid("el servidor local no está activo; Spotify no podría devolver la autorización".into()));
    }
    let url = state.spotify_auth.begin(&redirect_uri(state.server_port))?;
    app.opener().open_url(url, None::<&str>).map_err(|e| AppError::Invalid(format!("no se pudo abrir el navegador: {e}")))
}

#[tauri::command]
pub fn spotify_disconnect(state: State<'_, AppState>) -> Result<()> {
    state.spotify_auth.disconnect()?;
    state.spotify.apply_playing(None);
    Ok(())
}

/// Pide una canción desde la UI (prueba): sin roles, coste ni cooldown.
#[tauri::command]
pub async fn spotify_queue_test(state: State<'_, AppState>, query: String) -> Result<Track> {
    state.spotify.manual_queue(&query).await
}
