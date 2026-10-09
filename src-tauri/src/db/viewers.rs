use sqlx::{sqlite::SqliteRow, Row, Sqlite, Transaction};

use super::Db;
use crate::error::{AppError, Result};
use crate::points::csv_io::{self, ImportRow};
use crate::points::logic::Award;
use crate::points::{HistoryEntry, SortKey, Viewer, PROVISIONAL_PREFIX};

/// Retención del historial de puntos.
pub const HISTORY_RETENTION_MS: i64 = 90 * 24 * 3600 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportMode {
    /// El CSV manda: los puntos de cada espectador pasan a ser los del archivo.
    Replace,
    /// Se suman a los puntos que ya tenía.
    Add,
}

#[derive(Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub created: u64,
    pub updated: u64,
    pub errors: Vec<String>,
}

const COLUMNS: &str = "user_id, unique_id, nickname, avatar, points, total_earned, total_spent, coins_gifted, \
                       comments, likes, shares, watch_minutes, first_seen_ms, last_seen_ms";

fn to_u64(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

fn to_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

fn viewer_from_row(r: &SqliteRow) -> Viewer {
    Viewer {
        user_id: r.get("user_id"),
        unique_id: r.get("unique_id"),
        nickname: r.get("nickname"),
        avatar: r.get("avatar"),
        points: to_u64(r.get("points")),
        total_earned: to_u64(r.get("total_earned")),
        total_spent: to_u64(r.get("total_spent")),
        coins_gifted: to_u64(r.get("coins_gifted")),
        comments: to_u64(r.get("comments")),
        likes: to_u64(r.get("likes")),
        shares: to_u64(r.get("shares")),
        watch_minutes: to_u64(r.get("watch_minutes")),
        first_seen_ms: r.get("first_seen_ms"),
        last_seen_ms: r.get("last_seen_ms"),
    }
}

/// Escapa `%`, `_` y `\` para usar la búsqueda del usuario en un `LIKE` literal.
fn like_pattern(search: &str) -> String {
    let escaped = search.trim().trim_start_matches('@').to_lowercase().replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
    format!("%{escaped}%")
}

impl Db {
    /// Aplica un lote de ganancias/gastos en una sola transacción.
    pub async fn apply_awards(&self, awards: &[Award]) -> Result<()> {
        if awards.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool.begin().await?;
        for a in awards {
            upsert_award(&mut tx, a).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn get_viewer(&self, user_id: &str) -> Result<Option<Viewer>> {
        let row = sqlx::query(sqlx::AssertSqlSafe(format!("SELECT {COLUMNS} FROM viewers WHERE user_id = ?1")))
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.as_ref().map(viewer_from_row))
    }

    /// Busca por `@usuario` (sin distinguir mayúsculas). Si hay varios, el de más puntos.
    pub async fn find_viewer_by_unique(&self, unique_id: &str) -> Result<Option<Viewer>> {
        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM viewers WHERE lower(unique_id) = lower(?1) ORDER BY points DESC LIMIT 1"
        )))
        .bind(unique_id.trim().trim_start_matches('@'))
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.as_ref().map(viewer_from_row))
    }

    pub async fn balance(&self, user_id: &str) -> Result<u64> {
        let row = sqlx::query("SELECT points FROM viewers WHERE user_id = ?1")
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map_or(0, |r| to_u64(r.get("points"))))
    }

    /// Gasta puntos de forma atómica: o hay saldo suficiente y se descuenta, o no pasa nada.
    /// Devuelve el saldo restante, o `None` si no alcanzaba.
    pub async fn spend_points(&self, user_id: &str, cost: u64, reason: &str, ts: i64) -> Result<Option<u64>> {
        let mut tx = self.pool.begin().await?;
        let r = sqlx::query("UPDATE viewers SET points = points - ?2, total_spent = total_spent + ?2 WHERE user_id = ?1 AND points >= ?2")
            .bind(user_id)
            .bind(to_i64(cost))
            .execute(&mut *tx)
            .await?;
        if r.rows_affected() == 0 {
            tx.rollback().await?;
            return Ok(None);
        }
        record_history(&mut tx, ts, user_id, -to_i64(cost), reason).await?;
        let left: i64 = sqlx::query_scalar("SELECT points FROM viewers WHERE user_id = ?1")
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Some(to_u64(left)))
    }

    /// Ajuste manual con signo (restar nunca baja de 0). Devuelve el saldo resultante.
    pub async fn adjust_points(&self, user_id: &str, delta: i64, reason: &str, ts: i64) -> Result<u64> {
        let mut tx = self.pool.begin().await?;
        let current: Option<i64> = sqlx::query_scalar("SELECT points FROM viewers WHERE user_id = ?1")
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
        let Some(current) = current else {
            return Err(AppError::Invalid("el espectador no existe".into()));
        };
        // Restar nunca deja el saldo por debajo de 0.
        let applied = if delta < 0 { -to_i64(delta.unsigned_abs().min(to_u64(current))) } else { delta };
        sqlx::query(
            "UPDATE viewers SET points = points + ?2, \
             total_earned = total_earned + MAX(?2, 0), total_spent = total_spent + MAX(-?2, 0) WHERE user_id = ?1",
        )
        .bind(user_id)
        .bind(applied)
        .execute(&mut *tx)
        .await?;
        if applied != 0 {
            record_history(&mut tx, ts, user_id, applied, reason).await?;
        }
        tx.commit().await?;
        Ok(to_u64(current.saturating_add(applied)))
    }

    /// Fija los puntos (la diferencia queda en el historial).
    pub async fn set_points(&self, user_id: &str, value: u64, ts: i64) -> Result<()> {
        let current = self.balance(user_id).await?;
        let delta = to_i64(value).saturating_sub(to_i64(current));
        self.adjust_points(user_id, delta, "manual", ts).await.map(|_| ())
    }

    pub async fn top_viewers_by_points(&self, limit: u32) -> Result<Vec<Viewer>> {
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM viewers WHERE points > 0 ORDER BY points DESC, unique_id ASC LIMIT ?1"
        )))
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(viewer_from_row).collect())
    }

    /// Lista paginada con búsqueda por usuario o apodo.
    pub async fn list_viewers(&self, search: &str, sort: SortKey, limit: u32, offset: u32) -> Result<Vec<Viewer>> {
        let order = match sort {
            SortKey::Points => "points DESC, unique_id ASC",
            SortKey::Name => "lower(nickname) ASC, unique_id ASC",
            SortKey::LastSeen => "last_seen_ms DESC",
            SortKey::CoinsGifted => "coins_gifted DESC, unique_id ASC",
        };
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM viewers \
             WHERE (lower(unique_id) LIKE ?1 ESCAPE '\\' OR lower(nickname) LIKE ?1 ESCAPE '\\') \
             ORDER BY {order} LIMIT ?2 OFFSET ?3"
        )))
        .bind(like_pattern(search))
        .bind(i64::from(limit))
        .bind(i64::from(offset))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(viewer_from_row).collect())
    }

    pub async fn count_viewers(&self, search: &str) -> Result<u64> {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM viewers WHERE lower(unique_id) LIKE ?1 ESCAPE '\\' OR lower(nickname) LIKE ?1 ESCAPE '\\'",
        )
        .bind(like_pattern(search))
        .fetch_one(&self.pool)
        .await?;
        Ok(to_u64(n))
    }

    pub async fn point_history(&self, user_id: &str, limit: u32) -> Result<Vec<HistoryEntry>> {
        let rows = sqlx::query("SELECT ts, delta, reason FROM point_history WHERE user_id = ?1 ORDER BY id DESC LIMIT ?2")
            .bind(user_id)
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(|r| HistoryEntry { ts: r.get("ts"), delta: r.get("delta"), reason: r.get("reason") }).collect())
    }

    pub async fn prune_point_history(&self, before_ms: i64) -> Result<u64> {
        let r = sqlx::query("DELETE FROM point_history WHERE ts < ?1").bind(before_ms).execute(&self.pool).await?;
        Ok(r.rows_affected())
    }

    pub async fn delete_viewer(&self, user_id: &str) -> Result<bool> {
        let r = sqlx::query("DELETE FROM viewers WHERE user_id = ?1").bind(user_id).execute(&self.pool).await?;
        sqlx::query("DELETE FROM point_history WHERE user_id = ?1").bind(user_id).execute(&self.pool).await?;
        Ok(r.rows_affected() > 0)
    }

    /// Borra TODOS los espectadores y su historial.
    pub async fn clear_viewers(&self) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM viewers").execute(&mut *tx).await?;
        sqlx::query("DELETE FROM point_history").execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn export_viewers_csv(&self) -> Result<String> {
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!("SELECT {COLUMNS} FROM viewers ORDER BY points DESC, unique_id ASC")))
            .fetch_all(&self.pool)
            .await?;
        csv_io::export(&rows.iter().map(viewer_from_row).collect::<Vec<_>>())
    }

    /// Importa un CSV. Las filas malas se saltan y se reportan; el resto se aplica en una transacción.
    pub async fn import_viewers_csv(&self, text: &str, mode: ImportMode, ts: i64) -> Result<ImportReport> {
        let parsed = csv_io::parse(text)?;
        let mut report = ImportReport { errors: parsed.errors, ..Default::default() };
        let mut tx = self.pool.begin().await?;
        for row in &parsed.rows {
            if import_row(&mut tx, row, mode, ts).await? {
                report.created += 1;
            } else {
                report.updated += 1;
            }
        }
        tx.commit().await?;
        Ok(report)
    }
}

async fn record_history(tx: &mut Transaction<'_, Sqlite>, ts: i64, user_id: &str, delta: i64, reason: &str) -> Result<()> {
    sqlx::query("INSERT INTO point_history (ts, user_id, delta, reason) VALUES (?1, ?2, ?3, ?4)")
        .bind(ts)
        .bind(user_id)
        .bind(delta)
        .bind(reason)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Si el espectador fue importado con id provisional, lo une a su id real de TikTok.
async fn merge_provisional(tx: &mut Transaction<'_, Sqlite>, user_id: &str, unique_id: &str) -> Result<()> {
    if unique_id.is_empty() {
        return Ok(());
    }
    let exists: Option<String> = sqlx::query_scalar("SELECT user_id FROM viewers WHERE user_id = ?1")
        .bind(user_id)
        .fetch_optional(&mut **tx)
        .await?;
    if exists.is_some() {
        return Ok(());
    }
    let provisional = format!("{PROVISIONAL_PREFIX}{}", unique_id.to_lowercase());
    let found: Option<String> = sqlx::query_scalar("SELECT user_id FROM viewers WHERE user_id = ?1")
        .bind(&provisional)
        .fetch_optional(&mut **tx)
        .await?;
    if found.is_some() {
        sqlx::query("UPDATE viewers SET user_id = ?1 WHERE user_id = ?2").bind(user_id).bind(&provisional).execute(&mut **tx).await?;
        sqlx::query("UPDATE point_history SET user_id = ?1 WHERE user_id = ?2").bind(user_id).bind(&provisional).execute(&mut **tx).await?;
    }
    Ok(())
}

async fn upsert_award(tx: &mut Transaction<'_, Sqlite>, a: &Award) -> Result<()> {
    merge_provisional(tx, &a.who.user_id, &a.who.unique_id).await?;
    let s = &a.stats;
    sqlx::query(
        "INSERT INTO viewers (user_id, unique_id, nickname, avatar, points, total_earned, total_spent, coins_gifted, \
                              comments, likes, shares, watch_minutes, first_seen_ms, last_seen_ms) \
         VALUES (?1, ?2, ?3, ?4, MAX(?5, 0), MAX(?5, 0), 0, ?6, ?7, ?8, ?9, ?10, ?11, ?11) \
         ON CONFLICT(user_id) DO UPDATE SET \
           unique_id = excluded.unique_id, nickname = excluded.nickname, avatar = excluded.avatar, \
           points = MAX(points + ?5, 0), \
           total_earned = total_earned + MAX(?5, 0), total_spent = total_spent + MAX(-?5, 0), \
           coins_gifted = coins_gifted + excluded.coins_gifted, comments = comments + excluded.comments, \
           likes = likes + excluded.likes, shares = shares + excluded.shares, \
           watch_minutes = watch_minutes + excluded.watch_minutes, last_seen_ms = MAX(last_seen_ms, excluded.last_seen_ms)",
    )
    .bind(&a.who.user_id)
    .bind(&a.who.unique_id)
    .bind(&a.who.nickname)
    .bind(&a.who.avatar)
    .bind(a.delta)
    .bind(to_i64(s.coins_gifted))
    .bind(to_i64(s.comments))
    .bind(to_i64(s.likes))
    .bind(to_i64(s.shares))
    .bind(to_i64(s.watch_minutes))
    .bind(a.ts)
    .execute(&mut **tx)
    .await?;
    if a.delta != 0 {
        record_history(tx, a.ts, &a.who.user_id, a.delta, &a.reason.label()).await?;
    }
    Ok(())
}

/// Devuelve `true` si creó el espectador, `false` si actualizó uno existente.
async fn import_row(tx: &mut Transaction<'_, Sqlite>, row: &ImportRow, mode: ImportMode, ts: i64) -> Result<bool> {
    let existing: Option<(String, i64)> = sqlx::query("SELECT user_id, points FROM viewers WHERE lower(unique_id) = ?1 ORDER BY points DESC LIMIT 1")
        .bind(&row.unique_id)
        .fetch_optional(&mut **tx)
        .await?
        .map(|r| (r.get("user_id"), r.get("points")));
    let nickname = row.nickname.clone().unwrap_or_else(|| row.unique_id.clone());
    match existing {
        Some((user_id, current)) => {
            let new_points = match mode {
                ImportMode::Replace => to_i64(row.points),
                ImportMode::Add => current.saturating_add(to_i64(row.points)),
            };
            let delta = new_points - current;
            sqlx::query(
                "UPDATE viewers SET points = ?2, nickname = COALESCE(?3, nickname), \
                 total_earned = total_earned + MAX(?4, 0), total_spent = total_spent + MAX(-?4, 0), \
                 coins_gifted = MAX(coins_gifted, COALESCE(?5, coins_gifted)), comments = MAX(comments, COALESCE(?6, comments)), \
                 likes = MAX(likes, COALESCE(?7, likes)), watch_minutes = MAX(watch_minutes, COALESCE(?8, watch_minutes)) \
                 WHERE user_id = ?1",
            )
            .bind(&user_id)
            .bind(new_points)
            .bind(row.nickname.as_deref())
            .bind(delta)
            .bind(row.coins_gifted.map(to_i64))
            .bind(row.comments.map(to_i64))
            .bind(row.likes.map(to_i64))
            .bind(row.watch_minutes.map(to_i64))
            .execute(&mut **tx)
            .await?;
            if delta != 0 {
                record_history(tx, ts, &user_id, delta, "import").await?;
            }
            Ok(false)
        }
        None => {
            let user_id = format!("{PROVISIONAL_PREFIX}{}", row.unique_id);
            sqlx::query(
                "INSERT INTO viewers (user_id, unique_id, nickname, avatar, points, total_earned, total_spent, coins_gifted, \
                                      comments, likes, shares, watch_minutes, first_seen_ms, last_seen_ms) \
                 VALUES (?1, ?2, ?3, '', ?4, ?5, ?6, ?7, ?8, ?9, 0, ?10, ?11, ?11)",
            )
            .bind(&user_id)
            .bind(&row.unique_id)
            .bind(&nickname)
            .bind(to_i64(row.points))
            .bind(to_i64(row.total_earned.unwrap_or(row.points)))
            .bind(to_i64(row.total_spent.unwrap_or(0)))
            .bind(to_i64(row.coins_gifted.unwrap_or(0)))
            .bind(to_i64(row.comments.unwrap_or(0)))
            .bind(to_i64(row.likes.unwrap_or(0)))
            .bind(to_i64(row.watch_minutes.unwrap_or(0)))
            .bind(ts)
            .execute(&mut **tx)
            .await?;
            if row.points > 0 {
                record_history(tx, ts, &user_id, to_i64(row.points), "import").await?;
            }
            Ok(true)
        }
    }
}

#[cfg(test)]
mod tests;
