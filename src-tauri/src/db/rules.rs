use async_trait::async_trait;
use sqlx::Row;

use super::Db;
use crate::actions::store::{Job, JobStore};
use crate::error::Result;
use crate::rules::model::Rule;

impl Db {
    /// Reglas en su orden de presentación. Una fila ilegible se omite con un aviso.
    pub async fn list_rules(&self) -> Result<Vec<Rule>> {
        let rows = sqlx::query("SELECT id, json FROM rules ORDER BY position, rowid")
            .fetch_all(&self.pool)
            .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let id: String = row.get("id");
            match serde_json::from_str::<Rule>(&row.get::<String, _>("json")) {
                Ok(r) => out.push(r),
                Err(e) => tracing::warn!(rule = %id, error = %e, "regla ilegible en la base de datos"),
            }
        }
        Ok(out)
    }

    /// Inserta o actualiza una regla (las nuevas van al final).
    pub async fn save_rule(&self, rule: &Rule, now_ms: i64) -> Result<()> {
        let json = serde_json::to_string(rule)?;
        sqlx::query(
            "INSERT INTO rules (id, name, position, json, updated_ms) \
             VALUES (?1, ?2, COALESCE((SELECT MAX(position) + 1 FROM rules), 0), ?3, ?4) \
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, json = excluded.json, \
             updated_ms = excluded.updated_ms",
        )
        .bind(&rule.id)
        .bind(&rule.name)
        .bind(json)
        .bind(now_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Sustituye todas las reglas por las dadas, en ese orden, en una sola transacción.
    pub async fn replace_rules(&self, rules: &[Rule], now_ms: i64) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM rules").execute(&mut *tx).await?;
        for (i, rule) in rules.iter().enumerate() {
            sqlx::query("INSERT INTO rules (id, name, position, json, updated_ms) VALUES (?1, ?2, ?3, ?4, ?5)")
                .bind(&rule.id)
                .bind(&rule.name)
                .bind(i64::try_from(i).unwrap_or(i64::MAX))
                .bind(serde_json::to_string(rule)?)
                .bind(now_ms)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn delete_rule(&self, id: &str) -> Result<bool> {
        let r = sqlx::query("DELETE FROM rules WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }
}

#[async_trait]
impl JobStore for Db {
    async fn save(&self, job: &Job) -> Result<()> {
        let payload = serde_json::to_string(job)?;
        sqlx::query(
            "INSERT INTO action_jobs (id, expires_ms, next_step, payload) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(id) DO UPDATE SET expires_ms = excluded.expires_ms, \
             next_step = excluded.next_step, payload = excluded.payload",
        )
        .bind(&job.id)
        .bind(job.expires_ms)
        .bind(i64::try_from(job.next_step).unwrap_or(0))
        .bind(payload)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn set_progress(&self, id: &str, next_step: usize) -> Result<()> {
        sqlx::query("UPDATE action_jobs SET next_step = ?2 WHERE id = ?1")
            .bind(id)
            .bind(i64::try_from(next_step).unwrap_or(0))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn delete(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM action_jobs WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn load_all(&self) -> Result<Vec<Job>> {
        let rows = sqlx::query("SELECT id, next_step, payload FROM action_jobs")
            .fetch_all(&self.pool)
            .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let id: String = row.get("id");
            match serde_json::from_str::<Job>(&row.get::<String, _>("payload")) {
                Ok(mut job) => {
                    // La columna `next_step` es la fuente de verdad del progreso.
                    job.next_step = usize::try_from(row.get::<i64, _>("next_step")).unwrap_or(0);
                    out.push(job);
                }
                Err(e) => {
                    tracing::warn!(job = %id, error = %e, "job ilegible; se descarta");
                    let _ = JobStore::delete(self, &id).await;
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::model::{ActionPlan, ActionSpec, Conditions, PlanMode, Step, Trigger};
    use serde_json::json;

    fn rule(id: &str, name: &str) -> Rule {
        Rule {
            id: id.into(),
            name: name.into(),
            enabled: true,
            trigger: Trigger::Follow,
            conditions: Conditions::default(),
            plan: ActionPlan {
                mode: PlanMode::Sequence,
                steps: vec![Step { delay_ms: 0, action: ActionSpec::new("tts", json!({"text": "hi"})) }],
            },
            priority: None,
            ttl_ms: 1000,
            cost_points: None,
        }
    }

    #[tokio::test]
    async fn rules_keep_insertion_order_and_update_in_place() {
        let db = Db::open_memory().await.expect("db");
        for (id, n) in [("a", "Uno"), ("b", "Dos"), ("c", "Tres")] {
            db.save_rule(&rule(id, n), 1).await.expect("save");
        }
        db.save_rule(&rule("a", "Uno editada"), 2).await.expect("update");
        let names: Vec<_> = db.list_rules().await.expect("list").into_iter().map(|r| r.name).collect();
        assert_eq!(names, ["Uno editada", "Dos", "Tres"]);
    }

    #[tokio::test]
    async fn delete_reports_whether_something_was_removed() {
        let db = Db::open_memory().await.expect("db");
        db.save_rule(&rule("a", "A"), 1).await.expect("save");
        assert!(db.delete_rule("a").await.expect("del"));
        assert!(!db.delete_rule("a").await.expect("del"));
    }

    #[tokio::test]
    async fn unreadable_rules_are_skipped() {
        let db = Db::open_memory().await.expect("db");
        db.save_rule(&rule("ok", "Ok"), 1).await.expect("save");
        sqlx::query("INSERT INTO rules (id, name, position, json, updated_ms) VALUES ('x','x',99,'{roto',0)")
            .execute(&db.pool)
            .await
            .expect("raw");
        assert_eq!(db.list_rules().await.expect("list").len(), 1);
    }

    #[tokio::test]
    async fn jobs_roundtrip_with_progress() {
        let db = Db::open_memory().await.expect("db");
        let job = Job {
            id: "j1".into(),
            rule_id: "r".into(),
            mode: PlanMode::Sequence,
            steps: rule("r", "r").plan.steps,
            vars: [("nickname".to_string(), "Ana".to_string())].into(),
            priority: 3,
            created_ms: 1,
            expires_ms: 99,
            next_step: 0,
            refund: None,
        };
        JobStore::save(&db, &job).await.expect("save");
        JobStore::set_progress(&db, "j1", 1).await.expect("progress");
        let loaded = JobStore::load_all(&db).await.expect("load");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].next_step, 1);
        assert_eq!(loaded[0].vars["nickname"], "Ana");
        JobStore::delete(&db, "j1").await.expect("delete");
        assert!(JobStore::load_all(&db).await.expect("load").is_empty());
    }

    #[tokio::test]
    async fn corrupt_jobs_are_purged_on_load() {
        let db = Db::open_memory().await.expect("db");
        sqlx::query("INSERT INTO action_jobs (id, expires_ms, next_step, payload) VALUES ('bad', 1, 0, 'no json')")
            .execute(&db.pool)
            .await
            .expect("raw");
        assert!(JobStore::load_all(&db).await.expect("load").is_empty());
        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM action_jobs")
            .fetch_one(&db.pool)
            .await
            .expect("count");
        assert_eq!(left, 0);
    }
}
