//! Ranking de donadores: de la sesión (memoria), del día y de todos los tiempos (SQLite).
//! Alimenta el overlay de «Top donadores».

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::broadcast::{self, error::RecvError};

use crate::bus::EventBus;
use crate::db::Db;
use crate::error::Result;
use crate::events::{EventType, LiveEvent};
use crate::overlay::OverlayHub;
use crate::session::SessionStarted;

/// Canal retenido que lee el overlay de ranking.
pub const CHANNEL: &str = "leaderboard";
/// Cuántos puestos se publican por ámbito (el overlay muestra los que configure el usuario).
pub const PUBLISHED_SIZE: u32 = 15;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DonorEntry {
    pub user_id: String,
    pub unique_id: String,
    pub nickname: String,
    pub avatar: String,
    pub coins: u64,
    pub gifts: u64,
}

/// Una donación lista para sumarse al acumulado diario.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Donation {
    pub day: String,
    pub user_id: String,
    pub unique_id: String,
    pub nickname: String,
    pub avatar: String,
    pub coins: u64,
    pub gifts: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Session,
    Day,
    All,
}

/// Convierte un evento en donación, si lo es (regalo con monedas y un usuario identificable).
pub fn donation_from(ev: &LiveEvent, day: &str) -> Option<Donation> {
    if ev.kind != EventType::Gift {
        return None;
    }
    let g = ev.gift.as_ref().filter(|g| g.coins > 0)?;
    if ev.user.id.trim().is_empty() {
        return None;
    }
    Some(Donation {
        day: day.to_string(),
        user_id: ev.user.id.clone(),
        unique_id: ev.user.unique_id.clone(),
        nickname: if ev.user.nickname.is_empty() { ev.user.unique_id.clone() } else { ev.user.nickname.clone() },
        avatar: ev.user.avatar.clone(),
        coins: g.coins,
        gifts: u64::from(g.count),
    })
}

/// Ordena de mayor a menor monedas; los empates, por `@usuario` (orden estable y predecible).
fn ranked(mut v: Vec<DonorEntry>, limit: usize) -> Vec<DonorEntry> {
    v.sort_by(|a, b| b.coins.cmp(&a.coins).then_with(|| a.unique_id.cmp(&b.unique_id)));
    v.truncate(limit);
    v
}

#[derive(Debug, Clone, Copy)]
pub struct LeaderboardTiming {
    /// Cada cuánto se vuelca a la base y se republica el ranking (si hubo cambios).
    pub flush_every: Duration,
}

impl Default for LeaderboardTiming {
    fn default() -> Self {
        Self { flush_every: Duration::from_millis(500) }
    }
}

type DayFn = Arc<dyn Fn() -> String + Send + Sync>;

pub struct LeaderboardService {
    session: Mutex<HashMap<String, DonorEntry>>,
    pending: Mutex<Vec<Donation>>,
    dirty: AtomicBool,
    hub: OverlayHub,
    db: Db,
    today: DayFn,
}

impl LeaderboardService {
    /// `today` devuelve el día actual como `YYYY-MM-DD` (inyectable para los tests).
    pub fn new(db: Db, hub: OverlayHub, today: DayFn) -> Arc<Self> {
        Arc::new(Self {
            session: Mutex::new(HashMap::new()),
            pending: Mutex::new(Vec::new()),
            dirty: AtomicBool::new(false),
            hub,
            db,
            today,
        })
    }

    /// Día local actual, `YYYY-MM-DD`.
    pub fn local_today() -> String {
        chrono::Local::now().format("%Y-%m-%d").to_string()
    }

    fn session(&self) -> MutexGuard<'_, HashMap<String, DonorEntry>> {
        self.session.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Registra el evento si es una donación.
    pub fn record(&self, ev: &LiveEvent) -> bool {
        let Some(d) = donation_from(ev, &(self.today)()) else {
            return false;
        };
        {
            let mut s = self.session();
            let e = s.entry(d.user_id.clone()).or_insert_with(|| DonorEntry {
                user_id: d.user_id.clone(),
                unique_id: d.unique_id.clone(),
                nickname: d.nickname.clone(),
                avatar: d.avatar.clone(),
                coins: 0,
                gifts: 0,
            });
            e.coins = e.coins.saturating_add(d.coins);
            e.gifts = e.gifts.saturating_add(d.gifts);
            e.unique_id.clone_from(&d.unique_id);
            e.nickname.clone_from(&d.nickname);
            e.avatar.clone_from(&d.avatar);
        }
        // Lo simulado se ve en la sesión (para probar el overlay) pero no entra al día ni al histórico.
        if !ev.is_simulated() {
            self.pending.lock().unwrap_or_else(PoisonError::into_inner).push(d);
        }
        self.dirty.store(true, Ordering::Relaxed);
        true
    }

    /// Vacía el ranking de la sesión (el del día y el histórico se conservan).
    pub fn reset_session(&self) {
        self.session().clear();
        self.dirty.store(true, Ordering::Relaxed);
    }

    /// Borra el día y el histórico.
    pub async fn clear_history(&self) -> Result<()> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner).clear();
        self.db.clear_donors().await?;
        self.dirty.store(true, Ordering::Relaxed);
        Ok(())
    }

    /// Vuelca a la base las donaciones pendientes.
    pub async fn flush(&self) -> Result<()> {
        let batch: Vec<Donation> = std::mem::take(&mut *self.pending.lock().unwrap_or_else(PoisonError::into_inner));
        if batch.is_empty() {
            return Ok(());
        }
        if let Err(e) = self.db.add_donations(&batch).await {
            // Se reencola al principio para no perder nada y reintentar en el próximo ciclo.
            let mut p = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
            let mut restored = batch;
            restored.append(&mut p);
            *p = restored;
            return Err(e);
        }
        Ok(())
    }

    pub async fn top(&self, scope: Scope, limit: u32) -> Result<Vec<DonorEntry>> {
        match scope {
            Scope::Session => Ok(ranked(self.session().values().cloned().collect(), limit as usize)),
            Scope::Day => {
                self.flush().await?;
                self.db.top_donors_day(&(self.today)(), limit).await
            }
            Scope::All => {
                self.flush().await?;
                self.db.top_donors_all(limit).await
            }
        }
    }

    /// Estado que recibe el overlay: los tres ámbitos a la vez (él elige cuál mostrar).
    pub async fn snapshot(&self) -> Result<Value> {
        Ok(json!({
            "session": self.top(Scope::Session, PUBLISHED_SIZE).await?,
            "day": self.top(Scope::Day, PUBLISHED_SIZE).await?,
            "all": self.top(Scope::All, PUBLISHED_SIZE).await?,
        }))
    }

    pub async fn publish(&self) {
        match self.snapshot().await {
            Ok(v) => self.hub.publish_retained(CHANNEL, v),
            Err(e) => tracing::warn!(error = %e, "no se pudo calcular el ranking de donadores"),
        }
    }

    /// Arranca las tareas de fondo: eventos del bus, sesión nueva y volcado/publicación periódicos.
    pub fn spawn(self: &Arc<Self>, bus: &EventBus, mut sessions: broadcast::Receiver<SessionStarted>, timing: LeaderboardTiming) {
        let this = Arc::clone(self);
        let mut events = bus.subscribe();
        tokio::spawn(async move {
            let mut sessions_open = true;
            loop {
                tokio::select! {
                    ev = events.recv() => match ev {
                        Ok(ev) => {
                            this.record(&ev);
                        }
                        Err(RecvError::Lagged(n)) => tracing::warn!(missed = n, "el ranking se quedó atrás en el bus"),
                        Err(RecvError::Closed) => break,
                    },
                    s = sessions.recv(), if sessions_open => match s {
                        Ok(_) => this.reset_session(),
                        Err(RecvError::Lagged(_)) => {}
                        Err(RecvError::Closed) => sessions_open = false,
                    },
                }
            }
        });

        let this = Arc::clone(self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(timing.flush_every);
            loop {
                tick.tick().await;
                if this.dirty.swap(false, Ordering::Relaxed) {
                    if let Err(e) = this.flush().await {
                        tracing::error!(error = %e, "no se pudo guardar las donaciones");
                        this.dirty.store(true, Ordering::Relaxed);
                        continue;
                    }
                    this.publish().await;
                }
            }
        });
    }
}

#[cfg(test)]
mod tests;
