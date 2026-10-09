use sqlx::Row;

use super::Db;
use crate::error::Result;
use crate::leaderboard::{Donation, DonorEntry};

impl Db {
    /// Suma donaciones al acumulado diario (un solo viaje a la base por lote).
    pub async fn add_donations(&self, donations: &[Donation]) -> Result<()> {
        if donations.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool.begin().await?;
        for d in donations {
            sqlx::query(
                "INSERT INTO donor_daily (day, user_id, unique_id, nickname, avatar, coins, gifts) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
                 ON CONFLICT(day, user_id) DO UPDATE SET \
                   coins = coins + excluded.coins, gifts = gifts + excluded.gifts, \
                   unique_id = excluded.unique_id, nickname = excluded.nickname, avatar = excluded.avatar",
            )
            .bind(&d.day)
            .bind(&d.user_id)
            .bind(&d.unique_id)
            .bind(&d.nickname)
            .bind(&d.avatar)
            .bind(i64::try_from(d.coins).unwrap_or(i64::MAX))
            .bind(i64::try_from(d.gifts).unwrap_or(i64::MAX))
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Mejores donadores de un día (`YYYY-MM-DD`).
    pub async fn top_donors_day(&self, day: &str, limit: u32) -> Result<Vec<DonorEntry>> {
        let rows = sqlx::query(
            "SELECT user_id, unique_id, nickname, avatar, coins, gifts FROM donor_daily \
             WHERE day = ?1 AND coins > 0 ORDER BY coins DESC, unique_id ASC LIMIT ?2",
        )
        .bind(day)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(entry_from_row).collect())
    }

    /// Mejores donadores de todos los tiempos. El nombre y el avatar son los más recientes.
    pub async fn top_donors_all(&self, limit: u32) -> Result<Vec<DonorEntry>> {
        let rows = sqlx::query(
            "SELECT d.user_id AS user_id, \
                    (SELECT unique_id FROM donor_daily x WHERE x.user_id = d.user_id ORDER BY day DESC LIMIT 1) AS unique_id, \
                    (SELECT nickname  FROM donor_daily x WHERE x.user_id = d.user_id ORDER BY day DESC LIMIT 1) AS nickname, \
                    (SELECT avatar    FROM donor_daily x WHERE x.user_id = d.user_id ORDER BY day DESC LIMIT 1) AS avatar, \
                    SUM(d.coins) AS coins, SUM(d.gifts) AS gifts \
             FROM donor_daily d GROUP BY d.user_id HAVING SUM(d.coins) > 0 \
             ORDER BY coins DESC, unique_id ASC LIMIT ?1",
        )
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(entry_from_row).collect())
    }

    /// Borra todo el histórico de donaciones.
    pub async fn clear_donors(&self) -> Result<()> {
        sqlx::query("DELETE FROM donor_daily").execute(&self.pool).await?;
        Ok(())
    }
}

fn entry_from_row(r: &sqlx::sqlite::SqliteRow) -> DonorEntry {
    DonorEntry {
        user_id: r.get("user_id"),
        unique_id: r.get("unique_id"),
        nickname: r.get("nickname"),
        avatar: r.get("avatar"),
        coins: u64::try_from(r.get::<i64, _>("coins")).unwrap_or(0),
        gifts: u64::try_from(r.get::<i64, _>("gifts")).unwrap_or(0),
    }
}
