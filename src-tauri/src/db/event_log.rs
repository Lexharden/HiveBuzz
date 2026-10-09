//! Log de eventos en SQLite con rotación.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sqlx::Row;
use tokio::sync::broadcast::error::RecvError;

use super::Db;
use crate::bus::EventBus;
use crate::error::Result;
use crate::events::LiveEvent;

/// Cuánto se conserva el log de eventos.
pub const RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// Máximo de eventos por transacción al volcar el bus.
const BATCH: usize = 200;

impl Db {
    pub async fn insert_events(&self, events: &[Arc<LiveEvent>]) -> Result<()> {
        if events.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool.begin().await?;
        for ev in events {
            let payload = serde_json::to_string(ev.as_ref())?;
            let kind = serde_json::to_value(ev.kind)?
                .as_str()
                .unwrap_or("unknown")
                .to_string();
            sqlx::query("INSERT INTO event_log (event_id, type, ts, payload) VALUES (?1, ?2, ?3, ?4)")
                .bind(&ev.id)
                .bind(kind)
                .bind(ev.ts)
                .bind(payload)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Los últimos `limit` eventos, en orden cronológico (el más reciente al final).
    pub async fn recent_events(&self, limit: u32) -> Result<Vec<LiveEvent>> {
        let rows = sqlx::query("SELECT payload FROM event_log ORDER BY id DESC LIMIT ?1")
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows.iter().rev() {
            // Una fila corrupta no debe impedir ver el resto.
            match serde_json::from_str::<LiveEvent>(&row.get::<String, _>("payload")) {
                Ok(ev) => out.push(ev),
                Err(e) => tracing::warn!(error = %e, "fila ilegible en event_log"),
            }
        }
        Ok(out)
    }

    /// Borra los eventos con `ts` anterior a `ts_ms`; devuelve cuántos.
    pub async fn purge_before(&self, ts_ms: i64) -> Result<u64> {
        let r = sqlx::query("DELETE FROM event_log WHERE ts < ?1")
            .bind(ts_ms)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected())
    }

    pub async fn purge_expired(&self) -> Result<u64> {
        let cutoff = now_ms().saturating_sub(i64::try_from(RETENTION.as_millis()).unwrap_or(i64::MAX));
        self.purge_before(cutoff).await
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// Vuelca el bus al log en lotes. Termina cuando el bus se cierra.
pub fn spawn_log_writer(db: Db, bus: &EventBus) -> tokio::task::JoinHandle<()> {
    let mut rx = bus.subscribe();
    tokio::spawn(async move {
        let mut batch: Vec<Arc<LiveEvent>> = Vec::with_capacity(BATCH);
        loop {
            match rx.recv().await {
                Ok(ev) => batch.push(ev),
                Err(RecvError::Lagged(n)) => {
                    tracing::warn!(missed = n, "el log de eventos se quedó atrás");
                    continue;
                }
                Err(RecvError::Closed) => break,
            }
            // Agrupa lo que ya esté disponible sin esperar.
            while batch.len() < BATCH {
                match rx.try_recv() {
                    Ok(ev) => batch.push(ev),
                    Err(_) => break,
                }
            }
            if let Err(e) = db.insert_events(&batch).await {
                tracing::error!(error = %e, "no se pudo escribir en event_log");
            }
            batch.clear();
        }
    })
}

/// Rotación: borra lo vencido al arrancar y luego cada hora.
pub fn spawn_rotation(db: Db) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(3600));
        loop {
            tick.tick().await; // el primer tick es inmediato
            match db.purge_expired().await {
                Ok(0) => {}
                Ok(n) => tracing::info!(deleted = n, "rotación del log de eventos"),
                Err(e) => tracing::error!(error = %e, "falló la rotación del log"),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::testing::sample_event;

    fn at(id: &str, ts: i64) -> Arc<LiveEvent> {
        let mut e = sample_event(id);
        e.ts = ts;
        Arc::new(e)
    }

    #[tokio::test]
    async fn insert_and_read_back_in_chronological_order() {
        let db = Db::open_memory().await.expect("db");
        db.insert_events(&[at("a", 1), at("b", 2), at("c", 3)])
            .await
            .expect("insert");
        let ids: Vec<_> = db.recent_events(2).await.expect("recent").into_iter().map(|e| e.id).collect();
        assert_eq!(ids, ["b", "c"]);
    }

    #[tokio::test]
    async fn purge_deletes_only_older_events() {
        let db = Db::open_memory().await.expect("db");
        db.insert_events(&[at("old", 100), at("edge", 200), at("new", 300)])
            .await
            .expect("insert");
        assert_eq!(db.purge_before(200).await.expect("purge"), 1);
        let ids: Vec<_> = db.recent_events(10).await.expect("recent").into_iter().map(|e| e.id).collect();
        assert_eq!(ids, ["edge", "new"]);
    }

    #[tokio::test]
    async fn purge_expired_keeps_recent_and_drops_week_old() {
        let db = Db::open_memory().await.expect("db");
        let now = now_ms();
        let eight_days = 8 * 24 * 3600 * 1000;
        db.insert_events(&[at("viejo", now - eight_days), at("nuevo", now)])
            .await
            .expect("insert");
        assert_eq!(db.purge_expired().await.expect("purge"), 1);
        let ids: Vec<_> = db.recent_events(10).await.expect("recent").into_iter().map(|e| e.id).collect();
        assert_eq!(ids, ["nuevo"]);
    }

    #[tokio::test]
    async fn corrupt_rows_are_skipped() {
        let db = Db::open_memory().await.expect("db");
        db.insert_events(&[at("ok", 1)]).await.expect("insert");
        sqlx::query("INSERT INTO event_log (event_id, type, ts, payload) VALUES ('x','chat',2,'{no json')")
            .execute(&db.pool)
            .await
            .expect("raw insert");
        let ids: Vec<_> = db.recent_events(10).await.expect("recent").into_iter().map(|e| e.id).collect();
        assert_eq!(ids, ["ok"]);
    }

    #[tokio::test]
    async fn log_writer_persists_bus_events() {
        let db = Db::open_memory().await.expect("db");
        let bus = EventBus::new(16);
        let _task = spawn_log_writer(db.clone(), &bus);
        bus.publish(sample_event("p1"));
        bus.publish(sample_event("p2"));
        // Espera a que el escritor los vuelque.
        for _ in 0..50 {
            if db.recent_events(10).await.expect("recent").len() == 2 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("el escritor no persistió los eventos");
    }
}
