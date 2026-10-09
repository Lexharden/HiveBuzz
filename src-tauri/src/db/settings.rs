use sqlx::Row;

use super::Db;
use crate::error::{AppError, Result};

pub const KEY_SERVER_PORT: &str = "server_port";
pub const KEY_LAST_USERNAME: &str = "last_username";
pub const DEFAULT_SERVER_PORT: u16 = 17890;

impl Db {
    pub async fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let row = sqlx::query("SELECT value FROM settings WHERE key = ?1")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.get::<String, _>("value")))
    }

    pub async fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_setting(&self, key: &str) -> Result<()> {
        sqlx::query("DELETE FROM settings WHERE key = ?1")
            .bind(key)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Puerto del servidor local; si el valor guardado es inválido, se usa el de fábrica.
    pub async fn server_port(&self) -> Result<u16> {
        Ok(self
            .get_setting(KEY_SERVER_PORT)
            .await?
            .and_then(|v| v.parse::<u16>().ok())
            .filter(|p| *p >= 1024)
            .unwrap_or(DEFAULT_SERVER_PORT))
    }

    pub async fn set_server_port(&self, port: u16) -> Result<()> {
        if port < 1024 {
            return Err(AppError::Invalid(
                "el puerto debe estar entre 1024 y 65535".into(),
            ));
        }
        self.set_setting(KEY_SERVER_PORT, &port.to_string()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn set_get_overwrite_and_delete() {
        let db = Db::open_memory().await.expect("db");
        assert_eq!(db.get_setting("a").await.expect("get"), None);
        db.set_setting("a", "1").await.expect("set");
        db.set_setting("a", "2").await.expect("overwrite");
        assert_eq!(db.get_setting("a").await.expect("get").as_deref(), Some("2"));
        db.delete_setting("a").await.expect("delete");
        assert_eq!(db.get_setting("a").await.expect("get"), None);
    }

    #[tokio::test]
    async fn server_port_defaults_and_validates() {
        let db = Db::open_memory().await.expect("db");
        assert_eq!(db.server_port().await.expect("port"), DEFAULT_SERVER_PORT);
        db.set_server_port(20000).await.expect("set");
        assert_eq!(db.server_port().await.expect("port"), 20000);
        assert!(db.set_server_port(80).await.is_err());
        db.set_setting(KEY_SERVER_PORT, "basura").await.expect("set");
        assert_eq!(db.server_port().await.expect("port"), DEFAULT_SERVER_PORT);
    }
}
