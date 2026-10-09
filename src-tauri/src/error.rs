use serde::{Serialize, Serializer};

/// Error común de la app. Se serializa como texto para poder devolverlo a la UI
/// desde los comandos de Tauri.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("sidecar: {0}")]
    Sidecar(String),
    #[error("base de datos: {0}")]
    Db(#[from] sqlx::Error),
    #[error("migraciones: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("llavero del sistema: {0}")]
    Keyring(#[from] keyring::Error),
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("E/S: {0}")]
    Io(#[from] std::io::Error),
    #[error("servidor local: {0}")]
    Server(String),
    #[error("{0}")]
    Invalid(String),
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

pub type Result<T> = std::result::Result<T, AppError>;
