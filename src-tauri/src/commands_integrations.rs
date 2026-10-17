//! Comandos de Tauri de las integraciones (Fase 5): OBS.

use tauri::State;

use crate::app::AppState;
use crate::error::Result;
use crate::actions::{ActionContext, ActionExecutor};
use crate::executors::obs::{ObsConfig, ObsInfo};
use crate::executors::webhook::{WebhookExecutor, WebhookReport};
use crate::rules::template::event_vars;
use crate::simulator::{build_event, SimKind};

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

/// Lanza el webhook de verdad (con los datos de un regalo de ejemplo) y cuenta qué respondió el
/// servidor, para probar la URL, las cabeceras y el cuerpo sin esperar a un evento en vivo.
#[tauri::command]
pub async fn test_webhook(action: serde_json::Map<String, serde_json::Value>) -> Result<WebhookReport> {
    let executor = WebhookExecutor::new()?;
    executor.validate(&action)?;
    let ctx = ActionContext { rule_id: "prueba".into(), vars: event_vars(&build_event(SimKind::BigGift)) };
    executor.send(&ctx, &action, true).await
}
