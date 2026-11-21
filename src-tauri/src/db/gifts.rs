use sqlx::{Row, SqliteConnection};

use super::Db;
use crate::error::Result;
use crate::gifts::LearnedGift;

fn to_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

impl Db {
    /// Todo lo aprendido y dado de alta a mano.
    pub async fn list_learned_gifts(&self) -> Result<Vec<LearnedGift>> {
        let rows = sqlx::query("SELECT key, gift_id, name_en, name_es, also, diamonds, seen, last_seen, manual FROM gift_catalog")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .iter()
            .map(|r| LearnedGift {
                key: r.get("key"),
                gift_id: r.get("gift_id"),
                name_en: r.get("name_en"),
                name_es: r.get("name_es"),
                // Un JSON dañado en una fila no debe tirar el catálogo entero: se pierden solo sus alias.
                also: serde_json::from_str(r.get::<&str, _>("also")).unwrap_or_default(),
                diamonds: r.get::<Option<i64>, _>("diamonds").and_then(|d| u64::try_from(d).ok()),
                seen: u64::try_from(r.get::<i64, _>("seen")).unwrap_or(0),
                last_seen: r.get("last_seen"),
                manual: r.get::<i64, _>("manual") != 0,
            })
            .collect())
    }

    /// Guarda (inserta o reemplaza) varias filas en una sola transacción.
    pub async fn upsert_learned_gifts(&self, gifts: &[LearnedGift]) -> Result<()> {
        if gifts.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        for g in gifts {
            upsert(&mut tx, g).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Borra una fila. Devuelve si existía.
    pub async fn delete_learned_gift(&self, key: &str) -> Result<bool> {
        let r = sqlx::query("DELETE FROM gift_catalog WHERE key = ?1").bind(key).execute(&self.pool).await?;
        Ok(r.rows_affected() > 0)
    }

    /// Borra todo lo aprendido y dado de alta a mano.
    pub async fn clear_learned_gifts(&self) -> Result<()> {
        sqlx::query("DELETE FROM gift_catalog").execute(&self.pool).await?;
        Ok(())
    }
}

/// Inserta o reemplaza una fila dentro de una transacción ya abierta.
pub(super) async fn upsert(conn: &mut SqliteConnection, g: &LearnedGift) -> Result<()> {
    sqlx::query(
        "INSERT INTO gift_catalog (key, gift_id, name_en, name_es, also, diamonds, seen, last_seen, manual)          VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)          ON CONFLICT(key) DO UPDATE SET gift_id = excluded.gift_id, name_en = excluded.name_en,            name_es = excluded.name_es, also = excluded.also, diamonds = excluded.diamonds, seen = excluded.seen,            last_seen = excluded.last_seen, manual = excluded.manual",
    )
    .bind(&g.key)
    .bind(g.gift_id)
    .bind(&g.name_en)
    .bind(&g.name_es)
    .bind(serde_json::to_string(&g.also)?)
    .bind(g.diamonds.map(to_i64))
    .bind(to_i64(g.seen))
    .bind(g.last_seen)
    .bind(i64::from(g.manual))
    .execute(conn)
    .await?;
    Ok(())
}
