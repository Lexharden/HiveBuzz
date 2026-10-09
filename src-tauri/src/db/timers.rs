use sqlx::Row;

use super::Db;
use crate::error::Result;
use crate::timers::service::StoredTimer;

impl Db {
    /// Timers en su orden de presentación. Una fila ilegible se omite con un aviso.
    pub async fn list_timers(&self) -> Result<Vec<StoredTimer>> {
        let rows = sqlx::query("SELECT id, json FROM timers ORDER BY position, rowid")
            .fetch_all(&self.pool)
            .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let id: String = row.get("id");
            match serde_json::from_str::<StoredTimer>(&row.get::<String, _>("json")) {
                Ok(t) => out.push(t),
                Err(e) => tracing::warn!(timer = %id, error = %e, "timer ilegible en la base de datos"),
            }
        }
        Ok(out)
    }

    /// Guarda varios timers en una transacción (las nuevas filas van al final).
    pub async fn save_timers(&self, timers: &[StoredTimer]) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        for t in timers {
            sqlx::query(
                "INSERT INTO timers (id, position, json) \
                 VALUES (?1, COALESCE((SELECT MAX(position) + 1 FROM timers), 0), ?2) \
                 ON CONFLICT(id) DO UPDATE SET json = excluded.json",
            )
            .bind(&t.config.id)
            .bind(serde_json::to_string(t)?)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn delete_timer(&self, id: &str) -> Result<bool> {
        let r = sqlx::query("DELETE FROM timers WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }
}
