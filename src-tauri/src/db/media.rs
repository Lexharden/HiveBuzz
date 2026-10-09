use sqlx::{sqlite::SqliteRow, Row};

use super::Db;
use crate::error::Result;
use crate::media::{Media, MediaKind};

fn from_row(r: &SqliteRow) -> Media {
    Media {
        id: r.get("id"),
        name: r.get("name"),
        file: r.get("file"),
        kind: if r.get::<String, _>("kind") == "video" {
            MediaKind::Video
        } else {
            MediaKind::Image
        },
        created_ms: r.get("created_ms"),
    }
}

impl Db {
    pub async fn list_media(&self) -> Result<Vec<Media>> {
        let rows = sqlx::query("SELECT id, name, file, kind, created_ms FROM media ORDER BY name COLLATE NOCASE, created_ms")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(from_row).collect())
    }

    pub async fn get_media(&self, id: &str) -> Result<Option<Media>> {
        let row = sqlx::query("SELECT id, name, file, kind, created_ms FROM media WHERE id = ?1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.as_ref().map(from_row))
    }

    pub async fn insert_media(&self, m: &Media) -> Result<()> {
        sqlx::query("INSERT INTO media (id, name, file, kind, created_ms) VALUES (?1, ?2, ?3, ?4, ?5)")
            .bind(&m.id)
            .bind(&m.name)
            .bind(&m.file)
            .bind(match m.kind {
                MediaKind::Image => "image",
                MediaKind::Video => "video",
            })
            .bind(m.created_ms)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn rename_media(&self, id: &str, name: &str) -> Result<bool> {
        let r = sqlx::query("UPDATE media SET name = ?2 WHERE id = ?1")
            .bind(id)
            .bind(name)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn delete_media(&self, id: &str) -> Result<bool> {
        let r = sqlx::query("DELETE FROM media WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }
}
