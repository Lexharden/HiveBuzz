use sqlx::{sqlite::SqliteRow, Row};

use super::Db;
use crate::error::Result;
use crate::sounds::Sound;

fn from_row(r: &SqliteRow) -> Sound {
    Sound {
        id: r.get("id"),
        name: r.get("name"),
        file: r.get("file"),
        volume: u8::try_from(r.get::<i64, _>("volume").clamp(0, 100)).unwrap_or(100),
        created_ms: r.get("created_ms"),
    }
}

impl Db {
    pub async fn list_sounds(&self) -> Result<Vec<Sound>> {
        let rows = sqlx::query("SELECT id, name, file, volume, created_ms FROM sounds ORDER BY name COLLATE NOCASE, created_ms")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(from_row).collect())
    }

    pub async fn get_sound(&self, id: &str) -> Result<Option<Sound>> {
        let row = sqlx::query("SELECT id, name, file, volume, created_ms FROM sounds WHERE id = ?1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.as_ref().map(from_row))
    }

    pub async fn insert_sound(&self, s: &Sound) -> Result<()> {
        sqlx::query("INSERT INTO sounds (id, name, file, volume, created_ms) VALUES (?1, ?2, ?3, ?4, ?5)")
            .bind(&s.id)
            .bind(&s.name)
            .bind(&s.file)
            .bind(i64::from(s.volume))
            .bind(s.created_ms)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn update_sound(&self, id: &str, name: &str, volume: u8) -> Result<bool> {
        let r = sqlx::query("UPDATE sounds SET name = ?2, volume = ?3 WHERE id = ?1")
            .bind(id)
            .bind(name)
            .bind(i64::from(volume))
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn delete_sound(&self, id: &str) -> Result<bool> {
        let r = sqlx::query("DELETE FROM sounds WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }
}
