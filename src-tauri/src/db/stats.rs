use sqlx::Row;

use super::Db;
use crate::error::Result;

/// Fila cruda de `stream_stats`.
pub struct StreamRow {
    pub id: i64,
    pub ended_ms: i64,
    pub coins: i64,
    pub peak_viewers: i64,
    pub json: String,
}

/// Cuántas transmisiones se conservan.
pub const KEEP_STREAMS: i64 = 400;

impl Db {
    pub async fn upsert_stream_stats(&self, row: &StreamRow) -> Result<()> {
        sqlx::query(
            "INSERT INTO stream_stats (id, ended_ms, coins, peak_viewers, json) VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(id) DO UPDATE SET ended_ms = excluded.ended_ms, coins = excluded.coins, \
               peak_viewers = excluded.peak_viewers, json = excluded.json",
        )
        .bind(row.id)
        .bind(row.ended_ms)
        .bind(row.coins)
        .bind(row.peak_viewers)
        .bind(&row.json)
        .execute(&self.pool)
        .await?;
        // Rotación: solo las más recientes.
        sqlx::query("DELETE FROM stream_stats WHERE id NOT IN (SELECT id FROM stream_stats ORDER BY id DESC LIMIT ?1)")
            .bind(KEEP_STREAMS)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_stream_stats(&self, limit: u32) -> Result<Vec<StreamRow>> {
        let rows = sqlx::query("SELECT id, ended_ms, coins, peak_viewers, json FROM stream_stats ORDER BY id DESC LIMIT ?1")
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(row_of).collect())
    }

    pub async fn get_stream_stats(&self, id: i64) -> Result<Option<StreamRow>> {
        let row = sqlx::query("SELECT id, ended_ms, coins, peak_viewers, json FROM stream_stats WHERE id = ?1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.as_ref().map(row_of))
    }

    pub async fn delete_stream_stats(&self, id: i64) -> Result<bool> {
        Ok(sqlx::query("DELETE FROM stream_stats WHERE id = ?1").bind(id).execute(&self.pool).await?.rows_affected() > 0)
    }
}

fn row_of(r: &sqlx::sqlite::SqliteRow) -> StreamRow {
    StreamRow {
        id: r.get("id"),
        ended_ms: r.get("ended_ms"),
        coins: r.get("coins"),
        peak_viewers: r.get("peak_viewers"),
        json: r.get("json"),
    }
}
