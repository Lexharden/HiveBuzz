use sqlx::Row;

use super::Db;
use crate::error::Result;
use crate::likeboard::{LikeEntry, LikeTap};

impl Db {
    /// Suma likes al acumulado diario (un solo viaje a la base por lote).
    pub async fn add_likes(&self, taps: &[LikeTap]) -> Result<()> {
        if taps.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        for t in taps {
            sqlx::query(
                "INSERT INTO like_daily (day, user_id, unique_id, nickname, avatar, likes) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
                 ON CONFLICT(day, user_id) DO UPDATE SET \
                   likes = likes + excluded.likes, \
                   unique_id = excluded.unique_id, nickname = excluded.nickname, avatar = excluded.avatar",
            )
            .bind(&t.day)
            .bind(&t.user_id)
            .bind(&t.unique_id)
            .bind(&t.nickname)
            .bind(&t.avatar)
            .bind(i64::try_from(t.likes).unwrap_or(i64::MAX))
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Quienes más likes dieron en un día (`YYYY-MM-DD`).
    pub async fn top_likers_day(&self, day: &str, limit: u32) -> Result<Vec<LikeEntry>> {
        let rows = sqlx::query(
            "SELECT user_id, unique_id, nickname, avatar, likes FROM like_daily \
             WHERE day = ?1 AND likes > 0 ORDER BY likes DESC, unique_id ASC LIMIT ?2",
        )
        .bind(day)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .map(|r| LikeEntry {
                user_id: r.get("user_id"),
                unique_id: r.get("unique_id"),
                nickname: r.get("nickname"),
                avatar: r.get("avatar"),
                likes: u64::try_from(r.get::<i64, _>("likes")).unwrap_or(0),
            })
            .collect())
    }

    /// Borra todos los likes guardados.
    pub async fn clear_likers(&self) -> Result<()> {
        sqlx::query("DELETE FROM like_daily").execute(&self.pool).await?;
        Ok(())
    }
}
