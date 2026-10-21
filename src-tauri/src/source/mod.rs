//! Fuentes de eventos LIVE. El resto de la app solo conoce el trait `LiveSource`,
//! de modo que cambiar de librería o de proveedor de firma no toca el núcleo.

pub mod protocol;
pub mod sidecar;
pub mod tauri_spawner;

use async_trait::async_trait;
use serde::Serialize;

use crate::error::Result;
use crate::events::LiveEvent;
use protocol::{ConnectionState, LogLevel};

/// A qué LIVE conectarse.
#[derive(Clone, PartialEq, Eq)]
pub struct ConnectTarget {
    /// @usuario del streamer (sin la arroba).
    pub unique_id: String,
    /// API key propia de Euler Stream (opcional; sale del llavero del sistema).
    pub euler_api_key: Option<String>,
}

impl ConnectTarget {
    pub fn new(unique_id: impl Into<String>) -> Self {
        Self { unique_id: unique_id.into(), euler_api_key: None }
    }
}

// `Debug` manual: contiene un secreto (la API key) que no debe acabar en un log.
impl std::fmt::Debug for ConnectTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectTarget")
            .field("unique_id", &self.unique_id)
            .field("euler_api_key", &self.euler_api_key.as_ref().map(|_| "***"))
            .finish()
    }
}

/// Estado de conexión tal como lo ve la UI.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusUpdate {
    pub state: ConnectionState,
    pub detail: Option<String>,
    pub attempt: Option<u32>,
    pub retry_in_ms: Option<u64>,
}

impl StatusUpdate {
    pub fn new(state: ConnectionState) -> Self {
        Self {
            state,
            detail: None,
            attempt: None,
            retry_in_ms: None,
        }
    }
}

/// Lo que una fuente entrega al resto de la app.
#[derive(Debug, Clone, PartialEq)]
pub enum SourceMessage {
    Event(Box<LiveEvent>),
    Status(StatusUpdate),
    Log { level: LogLevel, message: String },
    /// Espectadores conectados ahora mismo.
    Viewers(u64),
}

#[async_trait]
pub trait LiveSource: Send + Sync {
    /// Empieza (o reinicia) la conexión. Es idempotente y no espera a estar conectado:
    /// el progreso llega como `SourceMessage::Status`.
    async fn connect(&self, target: ConnectTarget) -> Result<()>;
    async fn disconnect(&self) -> Result<()>;
}
