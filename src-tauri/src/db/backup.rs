use super::Db;
use crate::backup::Bundle;
use crate::error::Result;
use crate::media::MediaKind;

impl Db {
    /// Sustituye la configuración (reglas, metas, timers, overlays, bibliotecas, perfiles y los
    /// ajustes de la lista blanca) por la del paquete, todo en una transacción: o entra entero o no entra.
    /// No toca viewers, puntos, donaciones ni el log de eventos (son datos de la comunidad).
    pub async fn replace_config(&self, b: &Bundle, now_ms: i64) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        for stmt in [
            "DELETE FROM rules",
            "DELETE FROM goals",
            "DELETE FROM timers",
            "DELETE FROM overlay_config",
            "DELETE FROM sounds",
            "DELETE FROM media",
            "DELETE FROM profiles",
        ] {
            sqlx::query(stmt).execute(&mut *tx).await?;
        }
        for (i, r) in b.rules.iter().enumerate() {
            sqlx::query("INSERT INTO rules (id, name, position, json, updated_ms) VALUES (?1, ?2, ?3, ?4, ?5)")
                .bind(&r.id)
                .bind(&r.name)
                .bind(i64::try_from(i).unwrap_or(i64::MAX))
                .bind(serde_json::to_string(r)?)
                .bind(now_ms)
                .execute(&mut *tx)
                .await?;
        }
        for (i, g) in b.goals.iter().enumerate() {
            sqlx::query("INSERT INTO goals (id, position, json) VALUES (?1, ?2, ?3)")
                .bind(&g.id)
                .bind(i64::try_from(i).unwrap_or(i64::MAX))
                .bind(serde_json::to_string(g)?)
                .execute(&mut *tx)
                .await?;
        }
        for (i, t) in b.timers.iter().enumerate() {
            sqlx::query("INSERT INTO timers (id, position, json) VALUES (?1, ?2, ?3)")
                .bind(&t.config.id)
                .bind(i64::try_from(i).unwrap_or(i64::MAX))
                .bind(serde_json::to_string(t)?)
                .execute(&mut *tx)
                .await?;
        }
        for (id, cfg) in &b.overlays {
            sqlx::query("INSERT INTO overlay_config (id, json) VALUES (?1, ?2)").bind(id).bind(serde_json::to_string(cfg)?).execute(&mut *tx).await?;
        }
        for s in &b.sounds {
            sqlx::query("INSERT INTO sounds (id, name, file, volume, created_ms) VALUES (?1, ?2, ?3, ?4, ?5)")
                .bind(&s.id)
                .bind(&s.name)
                .bind(&s.file)
                .bind(i64::from(s.volume))
                .bind(s.created_ms)
                .execute(&mut *tx)
                .await?;
        }
        for m in &b.media {
            sqlx::query("INSERT INTO media (id, name, file, kind, created_ms) VALUES (?1, ?2, ?3, ?4, ?5)")
                .bind(&m.id)
                .bind(&m.name)
                .bind(&m.file)
                .bind(match m.kind {
                    MediaKind::Image => "image",
                    MediaKind::Video => "video",
                })
                .bind(m.created_ms)
                .execute(&mut *tx)
                .await?;
        }
        for p in &b.profiles {
            sqlx::query("INSERT INTO profiles (id, name, updated_ms, json) VALUES (?1, ?2, ?3, ?4)")
                .bind(&p.id)
                .bind(&p.name)
                .bind(p.updated_ms)
                .bind(serde_json::to_string(&p.data)?)
                .execute(&mut *tx)
                .await?;
        }
        for (key, value) in &b.settings {
            sqlx::query("INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
                .bind(key)
                .bind(value)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }
}
