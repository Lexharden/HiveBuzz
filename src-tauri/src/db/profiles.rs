use sqlx::Row;

use super::Db;
use crate::error::Result;

/// Fila cruda de `profiles`.
#[derive(Debug, Clone)]
pub struct ProfileRow {
    pub id: String,
    pub name: String,
    pub updated_ms: i64,
    pub json: String,
}

fn row_of(r: &sqlx::sqlite::SqliteRow) -> ProfileRow {
    ProfileRow { id: r.get("id"), name: r.get("name"), updated_ms: r.get("updated_ms"), json: r.get("json") }
}

impl Db {
    pub async fn list_profiles(&self) -> Result<Vec<ProfileRow>> {
        let rows = sqlx::query("SELECT id, name, updated_ms, json FROM profiles ORDER BY name COLLATE NOCASE, id")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(row_of).collect())
    }

    pub async fn get_profile(&self, id: &str) -> Result<Option<ProfileRow>> {
        let row = sqlx::query("SELECT id, name, updated_ms, json FROM profiles WHERE id = ?1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.as_ref().map(row_of))
    }

    pub async fn save_profile(&self, p: &ProfileRow) -> Result<()> {
        sqlx::query(
            "INSERT INTO profiles (id, name, updated_ms, json) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, updated_ms = excluded.updated_ms, json = excluded.json",
        )
        .bind(&p.id)
        .bind(&p.name)
        .bind(p.updated_ms)
        .bind(&p.json)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_profile(&self, id: &str) -> Result<bool> {
        Ok(sqlx::query("DELETE FROM profiles WHERE id = ?1").bind(id).execute(&self.pool).await?.rows_affected() > 0)
    }
}
