use sqlx::Row;

use super::Db;
use crate::error::Result;
use crate::goals::Goal;

impl Db {
    /// Metas en su orden de presentación. Una fila ilegible se omite con un aviso.
    pub async fn list_goals(&self) -> Result<Vec<Goal>> {
        let rows = sqlx::query("SELECT id, json FROM goals ORDER BY position, rowid")
            .fetch_all(&self.pool)
            .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let id: String = row.get("id");
            match serde_json::from_str::<Goal>(&row.get::<String, _>("json")) {
                Ok(g) => out.push(g),
                Err(e) => tracing::warn!(goal = %id, error = %e, "meta ilegible en la base de datos"),
            }
        }
        Ok(out)
    }

    /// Inserta o actualiza una meta (las nuevas van al final).
    pub async fn save_goal(&self, goal: &Goal) -> Result<()> {
        sqlx::query(
            "INSERT INTO goals (id, position, json) \
             VALUES (?1, COALESCE((SELECT MAX(position) + 1 FROM goals), 0), ?2) \
             ON CONFLICT(id) DO UPDATE SET json = excluded.json",
        )
        .bind(&goal.id)
        .bind(serde_json::to_string(goal)?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Guarda varias metas en una transacción (el progreso cambia con cada like).
    pub async fn save_goals(&self, goals: &[Goal]) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        for g in goals {
            sqlx::query(
                "INSERT INTO goals (id, position, json) \
                 VALUES (?1, COALESCE((SELECT MAX(position) + 1 FROM goals), 0), ?2) \
                 ON CONFLICT(id) DO UPDATE SET json = excluded.json",
            )
            .bind(&g.id)
            .bind(serde_json::to_string(g)?)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn delete_goal(&self, id: &str) -> Result<bool> {
        let r = sqlx::query("DELETE FROM goals WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }
}
