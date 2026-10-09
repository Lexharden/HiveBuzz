use sqlx::Row;

use super::Db;
use crate::error::Result;

impl Db {
    pub async fn get_overlay_config(&self, id: &str) -> Result<Option<String>> {
        let row = sqlx::query("SELECT json FROM overlay_config WHERE id = ?1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.get::<String, _>("json")))
    }

    pub async fn set_overlay_config(&self, id: &str, json: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO overlay_config (id, json) VALUES (?1, ?2) \
             ON CONFLICT(id) DO UPDATE SET json = excluded.json",
        )
        .bind(id)
        .bind(json)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_overlay_config(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM overlay_config WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

impl Db {
    /// Todas las configuraciones guardadas de overlays: `(id, json)`.
    pub async fn list_overlay_configs(&self) -> Result<Vec<(String, String)>> {
        let rows = sqlx::query("SELECT id, json FROM overlay_config ORDER BY id").fetch_all(&self.pool).await?;
        Ok(rows.iter().map(|r| (r.get::<String, _>("id"), r.get::<String, _>("json"))).collect())
    }
}
