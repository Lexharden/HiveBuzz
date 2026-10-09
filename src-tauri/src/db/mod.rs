//! Persistencia local en SQLite (`sqlx`) con migraciones.

mod backup;
mod donors;
mod event_log;
mod goals;
mod media;
mod overlay_config;
mod profiles;
mod rules;
mod settings;
mod sounds;
mod stats;
mod timers;
mod viewers;

pub use profiles::ProfileRow;
pub use stats::StreamRow;
pub use viewers::{ImportMode, ImportReport, HISTORY_RETENTION_MS};

use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;

use crate::error::Result;

pub use event_log::{spawn_log_writer, spawn_rotation, RETENTION};
pub use settings::{DEFAULT_SERVER_PORT, KEY_LAST_USERNAME, KEY_SERVER_PORT};

#[derive(Clone)]
pub struct Db {
    pool: SqlitePool,
}

impl Db {
    /// Abre (y crea si falta) la base en `path` y aplica las migraciones.
    pub async fn open(path: &Path) -> Result<Self> {
        let opts = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(opts)
            .await?;
        Self::migrated(pool).await
    }

    /// Base en memoria para tests (una sola conexión: cada conexión tendría su propia base).
    pub async fn open_memory() -> Result<Self> {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")?;
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await?;
        Self::migrated(pool).await
    }

    async fn migrated(pool: SqlitePool) -> Result<Self> {
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool })
    }

    /// SQL crudo para preparar (o romper a propósito) la base en los tests.
    #[cfg(test)]
    pub async fn exec_for_tests(&self, sql: &str) {
        // Solo se usa con SQL literal de los propios tests.
        sqlx::query(sqlx::AssertSqlSafe(sql.to_string())).execute(&self.pool).await.expect("SQL de prueba");
    }
}
