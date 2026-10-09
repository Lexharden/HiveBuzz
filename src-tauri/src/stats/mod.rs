//! Estadísticas por transmisión: monedas, top de donadores, pico de espectadores y regalos por tipo.
//! Se acumulan en memoria para el LIVE en curso y se guardan en SQLite cada pocos segundos, al
//! empezar una sesión nueva y al cerrar la app.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, watch};

use crate::actions::clock::Clock;
use crate::bus::EventBus;
use crate::db::{Db, StreamRow};
use crate::error::{AppError, Result};
use crate::events::{EventType, LiveEvent};
use crate::session::SessionStarted;

/// Donadores que se conservan por transmisión.
const MAX_DONORS: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GiftStat {
    pub name: String,
    pub count: u64,
    pub coins: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DonorStat {
    pub user_id: String,
    pub unique_id: String,
    pub nickname: String,
    pub coins: u64,
    pub gifts: u64,
}

/// Resumen completo de una transmisión.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct StreamStats {
    pub id: i64,
    pub ended_ms: i64,
    pub coins: u64,
    pub peak_viewers: u64,
    pub gifts_total: u64,
    pub chats: u64,
    pub likes: u64,
    pub follows: u64,
    pub shares: u64,
    pub subscribers: u64,
    pub gifts: Vec<GiftStat>,
    pub donors: Vec<DonorStat>,
}

/// Para listar: lo mismo sin los vectores.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamSummary {
    pub id: i64,
    pub ended_ms: i64,
    pub coins: u64,
    pub peak_viewers: u64,
    pub gifts_total: u64,
    pub chats: u64,
    pub likes: u64,
    pub follows: u64,
    pub shares: u64,
    pub subscribers: u64,
}

impl StreamStats {
    fn summary(&self) -> StreamSummary {
        StreamSummary {
            id: self.id,
            ended_ms: self.ended_ms,
            coins: self.coins,
            peak_viewers: self.peak_viewers,
            gifts_total: self.gifts_total,
            chats: self.chats,
            likes: self.likes,
            follows: self.follows,
            shares: self.shares,
            subscribers: self.subscribers,
        }
    }
}

/// Acumulador de la transmisión en curso (lógica pura).
#[derive(Debug, Default)]
struct Acc {
    stats: StreamStats,
    gifts: HashMap<String, (u64, u64)>,
    donors: HashMap<String, DonorStat>,
    dirty: bool,
}

impl Acc {
    fn started(id: i64) -> Self {
        Self { stats: StreamStats { id, ended_ms: id, ..StreamStats::default() }, ..Self::default() }
    }

    fn touch(&mut self, now_ms: i64) {
        self.stats.ended_ms = now_ms;
        self.dirty = true;
    }

    fn on_event(&mut self, ev: &LiveEvent, now_ms: i64) {
        let s = &mut self.stats;
        match ev.kind {
            EventType::Gift => {
                let Some(g) = ev.gift.as_ref() else { return };
                s.coins += g.coins;
                s.gifts_total += u64::from(g.count);
                let e = self.gifts.entry(g.name.clone()).or_default();
                e.0 += u64::from(g.count);
                e.1 += g.coins;
                if g.coins > 0 && !ev.user.id.is_empty() {
                    let d = self.donors.entry(ev.user.id.clone()).or_insert_with(|| DonorStat {
                        user_id: ev.user.id.clone(),
                        unique_id: ev.user.unique_id.clone(),
                        nickname: String::new(),
                        coins: 0,
                        gifts: 0,
                    });
                    d.coins += g.coins;
                    d.gifts += u64::from(g.count);
                    d.nickname = if ev.user.nickname.is_empty() { ev.user.unique_id.clone() } else { ev.user.nickname.clone() };
                }
            }
            EventType::Chat => s.chats += 1,
            EventType::Like => s.likes += ev.like.as_ref().map_or(0, |l| l.count),
            EventType::Follow => s.follows += 1,
            EventType::Share => s.shares += 1,
            EventType::Subscribe => s.subscribers += 1,
            _ => return,
        }
        self.touch(now_ms);
    }

    fn on_viewers(&mut self, n: u64, now_ms: i64) {
        if n > self.stats.peak_viewers {
            self.stats.peak_viewers = n;
            self.touch(now_ms);
        }
    }

    /// Foto lista para guardar o mostrar.
    fn snapshot(&self) -> StreamStats {
        let mut out = self.stats.clone();
        let mut gifts: Vec<GiftStat> =
            self.gifts.iter().map(|(name, (count, coins))| GiftStat { name: name.clone(), count: *count, coins: *coins }).collect();
        gifts.sort_by(|a, b| b.coins.cmp(&a.coins).then(b.count.cmp(&a.count)).then_with(|| a.name.cmp(&b.name)));
        let mut donors: Vec<DonorStat> = self.donors.values().cloned().collect();
        donors.sort_by(|a, b| b.coins.cmp(&a.coins).then_with(|| a.unique_id.cmp(&b.unique_id)));
        donors.truncate(MAX_DONORS);
        out.gifts = gifts;
        out.donors = donors;
        out
    }

    fn is_empty(&self) -> bool {
        let s = &self.stats;
        s.coins == 0 && s.chats == 0 && s.likes == 0 && s.follows == 0 && s.shares == 0 && s.subscribers == 0 && s.peak_viewers == 0
    }
}

pub struct StatsService {
    acc: Mutex<Acc>,
    db: Db,
    clock: Arc<dyn Clock>,
}

fn to_row(s: &StreamStats) -> Result<StreamRow> {
    Ok(StreamRow {
        id: s.id,
        ended_ms: s.ended_ms,
        coins: i64::try_from(s.coins).unwrap_or(i64::MAX),
        peak_viewers: i64::try_from(s.peak_viewers).unwrap_or(i64::MAX),
        json: serde_json::to_string(s)?,
    })
}

fn from_row(r: &StreamRow) -> Option<StreamStats> {
    serde_json::from_str::<StreamStats>(&r.json).ok().map(|mut s| {
        // Las columnas son la fuente de verdad del identificador y las fechas.
        s.id = r.id;
        s.ended_ms = r.ended_ms;
        s
    })
}

impl StatsService {
    pub fn new(db: Db, clock: Arc<dyn Clock>) -> Arc<Self> {
        let start = clock.now_ms();
        Arc::new(Self { acc: Mutex::new(Acc::started(start)), db, clock })
    }

    fn lock(&self) -> MutexGuard<'_, Acc> {
        self.acc.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn on_event(&self, ev: &LiveEvent) {
        if ev.is_simulated() {
            return;
        }
        let now = self.clock.now_ms();
        self.lock().on_event(ev, now);
    }

    pub fn on_viewers(&self, n: u64) {
        let now = self.clock.now_ms();
        self.lock().on_viewers(n, now);
    }

    /// La transmisión en curso (aunque aún esté vacía).
    pub fn current(&self) -> StreamStats {
        self.lock().snapshot()
    }

    /// Guarda la transmisión en curso si cambió y no está vacía.
    pub async fn flush(&self) -> Result<()> {
        let snap = {
            let mut acc = self.lock();
            if !acc.dirty || acc.is_empty() {
                return Ok(());
            }
            acc.dirty = false;
            acc.snapshot()
        };
        if let Err(e) = self.db.upsert_stream_stats(&to_row(&snap)?).await {
            self.lock().dirty = true;
            return Err(e);
        }
        Ok(())
    }

    /// Cierra la transmisión en curso y empieza otra.
    pub async fn new_session(&self) {
        if let Err(e) = self.flush().await {
            tracing::error!(error = %e, "no se pudieron guardar las estadísticas");
        }
        let start = self.clock.now_ms();
        *self.lock() = Acc::started(start);
    }

    pub async fn list(&self, limit: u32) -> Result<Vec<StreamSummary>> {
        // La del LIVE en curso se guarda antes para que aparezca al día.
        self.flush().await?;
        Ok(self.db.list_stream_stats(limit.clamp(1, 400)).await?.iter().filter_map(from_row).map(|s| s.summary()).collect())
    }

    pub async fn get(&self, id: i64) -> Result<StreamStats> {
        if self.lock().stats.id == id {
            return Ok(self.current());
        }
        let row = self.db.get_stream_stats(id).await?.ok_or_else(|| AppError::Invalid("esa transmisión no existe".into()))?;
        from_row(&row).ok_or_else(|| AppError::Invalid("las estadísticas guardadas están dañadas".into()))
    }

    pub async fn delete(&self, id: i64) -> Result<bool> {
        if self.lock().stats.id == id {
            // La en curso se reinicia en vez de borrarse.
            let start = self.clock.now_ms();
            *self.lock() = Acc::started(start);
        }
        self.db.delete_stream_stats(id).await
    }

    pub fn spawn(
        self: &Arc<Self>,
        bus: &EventBus,
        mut viewers: watch::Receiver<u64>,
        mut sessions: broadcast::Receiver<SessionStarted>,
        flush_every: Duration,
    ) {
        let this = Arc::clone(self);
        let mut events = bus.subscribe();
        tokio::spawn(async move {
            let (mut sessions_open, mut viewers_open) = (true, true);
            let mut tick = tokio::time::interval(flush_every);
            loop {
                tokio::select! {
                    ev = events.recv() => match ev {
                        Ok(ev) => this.on_event(&ev),
                        Err(RecvError::Lagged(n)) => tracing::warn!(missed = n, "las estadísticas se quedaron atrás en el bus"),
                        Err(RecvError::Closed) => break,
                    },
                    s = sessions.recv(), if sessions_open => match s {
                        Ok(_) => this.new_session().await,
                        Err(RecvError::Lagged(_)) => {}
                        Err(RecvError::Closed) => sessions_open = false,
                    },
                    changed = viewers.changed(), if viewers_open => match changed {
                        Ok(()) => { let n = *viewers.borrow_and_update(); this.on_viewers(n); }
                        Err(_) => viewers_open = false,
                    },
                    _ = tick.tick() => {
                        if let Err(e) = this.flush().await {
                            tracing::error!(error = %e, "no se pudieron guardar las estadísticas");
                        }
                    }
                }
            }
        });
    }
}

#[cfg(test)]
mod tests;
