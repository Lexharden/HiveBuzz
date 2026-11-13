//! Arranque en segundo plano.
//!
//! Inicializar HiveBuzz (base de datos y migraciones, servicios, certificado HTTPS, servidores, sidecar…) lleva
//! su tiempo. Antes se hacía dentro de `setup`, con la ventana en blanco hasta terminar y, si algo fallaba, la app
//! se cerraba sin decir nada (en Windows no hay consola). Ahora `setup` solo deja preparado este estado y la
//! inicialización corre aparte: la ventana muestra un splash, y la interfaz se monta al llegar `ready` (antes no
//! existe el estado de la app y sus comandos fallarían). Si falla, el splash enseña el motivo.

use std::sync::{Mutex, PoisonError};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime, State};

/// Evento de Tauri con el estado del arranque (`BootStatus`).
pub const EVT_BOOT: &str = "boot-status";

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum BootStatus {
    #[default]
    Starting,
    Ready,
    Failed { error: String },
}

#[derive(Default)]
pub struct Boot(Mutex<BootStatus>);

impl Boot {
    pub fn get(&self) -> BootStatus {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Cambia el estado y avisa a la ventana. Se guarda antes de avisar: quien pregunte tras el aviso ya lo ve.
    pub fn set<R: Runtime>(&self, app: &AppHandle<R>, status: BootStatus) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = status.clone();
        if let Err(e) = app.emit(EVT_BOOT, status) {
            tracing::warn!(error = %e, "no se pudo avisar del estado del arranque");
        }
    }
}

/// Estado del arranque (la interfaz lo pregunta al cargar, por si el aviso llegó antes de escucharlo).
#[tauri::command(async)]
pub fn app_status(boot: State<'_, Boot>) -> BootStatus {
    boot.get()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn status_is_serialized_for_the_ui() {
        assert_eq!(serde_json::to_value(BootStatus::Starting).expect("json"), json!({ "state": "starting" }));
        assert_eq!(serde_json::to_value(BootStatus::Ready).expect("json"), json!({ "state": "ready" }));
        assert_eq!(
            serde_json::to_value(BootStatus::Failed { error: "puerto ocupado".into() }).expect("json"),
            json!({ "state": "failed", "error": "puerto ocupado" })
        );
        assert_eq!(Boot::default().get(), BootStatus::Starting);
    }
}
