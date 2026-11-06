//! Ranking de likes («tap taps»): de la sesión (memoria) y del día (SQLite).
//! Alimenta el overlay «Top likes». Es hermano de `leaderboard` (donadores) y no comparte estado con él.
//!
//! Los likes llegan del sidecar ya agrupados por usuario en ventanas de ~1 s: cada evento `like` trae en
//! `count` los likes de ESE usuario en la ventana (el `total` es el de toda la sala y aquí no se usa).

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

/// Canal retenido que lee el overlay de likes.
pub const CHANNEL: &str = "likeboard";
/// Cuántos puestos se publican por ámbito (el overlay muestra los que configure el usuario).
pub const PUBLISHED_SIZE: u32 = 15;
/// Tope de usuarios distintos en la memoria de la sesión. Un directo enorme no debe crecer sin límite: al
/// pasarse, se descartan los de menos likes (que de todos modos no llegarían al ranking publicado).
const MAX_SESSION_USERS: usize = 5_000;
const TRIMMED_SESSION_USERS: usize = 4_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LikeEntry {
    pub user_id: String,
    pub unique_id: String,
    pub nickname: String,
    pub avatar: String,
    pub likes: u64,
}

/// Likes de un usuario listos para sumarse al acumulado diario.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LikeTap {
    pub day: String,
    pub user_id: String,
    pub unique_id: String,
    pub nickname: String,
    pub avatar: String,
    pub likes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Session,
    Day,
}

/// Convierte un evento en likes, si lo es (con cantidad y un usuario identificable).
pub fn tap_from(ev: &LiveEvent, day: &str) -> Option<LikeTap> {
    if ev.kind != EventType::Like {
        return None;
    }
    let like = ev.like.as_ref().filter(|l| l.count > 0)?;
    if ev.user.id.trim().is_empty() {
        return None;
    }
    Some(LikeTap {
        day: day.to_string(),
        user_id: ev.user.id.clone(),
        unique_id: ev.user.unique_id.clone(),
        nickname: if ev.user.nickname.is_empty() { ev.user.unique_id.clone() } else { ev.user.nickname.clone() },
        avatar: ev.user.avatar.clone(),
        likes: like.count,
    })
}

/// Ordena de más a menos likes; los empates, por `@usuario` (orden estable y predecible).
fn ranked(mut v: Vec<LikeEntry>, limit: usize) -> Vec<LikeEntry> {
    v.sort_by(|a, b| b.likes.cmp(&a.likes).then_with(|| a.unique_id.cmp(&b.unique_id)));
    v.truncate(limit);
    v
}

#[derive(Debug, Clone, Copy)]
pub struct LikeBoardTiming {
    /// Cada cuánto se vuelca a la base y se republica el ranking (si hubo cambios).
    pub flush_every: Duration,
}

impl Default for LikeBoardTiming {
    fn default() -> Self {
        Self { flush_every: Duration::from_millis(500) }
    }
}

type DayFn = Arc<dyn Fn() -> String + Send + Sync>;

pub struct LikeBoardService {
    session: Mutex<HashMap<String, LikeEntry>>,
    /// Pendientes de guardar, ya fusionados por (día, usuario): así no crecen con cada evento si la base falla.
    pending: Mutex<HashMap<(String, String), LikeTap>>,
    dirty: AtomicBool,
    hub: OverlayHub,
    db: Db,
    today: DayFn,
}

impl LikeBoardService {
    /// `today` devuelve el día actual como `YYYY-MM-DD` (inyectable para los tests).
    pub fn new(db: Db, hub: OverlayHub, today: DayFn) -> Arc<Self> {
        Arc::new(Self {
            session: Mutex::new(HashMap::new()),
            pending: Mutex::new(HashMap::new()),
            dirty: AtomicBool::new(false),
            hub,
            db,
            today,
        })
    }

    fn session(&self) -> MutexGuard<'_, HashMap<String, LikeEntry>> {
        self.session.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn pending(&self) -> MutexGuard<'_, HashMap<(String, String), LikeTap>> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Registra el evento si son likes.
    pub fn record(&self, ev: &LiveEvent) -> bool {
        let Some(t) = tap_from(ev, &(self.today)()) else {
            return false;
        };
        {
            let mut s = self.session();
            let e = s.entry(t.user_id.clone()).or_insert_with(|| LikeEntry {
                user_id: t.user_id.clone(),
                unique_id: t.unique_id.clone(),
                nickname: t.nickname.clone(),
                avatar: t.avatar.clone(),
                likes: 0,
            });
            e.likes = e.likes.saturating_add(t.likes);
            e.unique_id.clone_from(&t.unique_id);
            e.nickname.clone_from(&t.nickname);
            e.avatar.clone_from(&t.avatar);
            if s.len() > MAX_SESSION_USERS {
                trim_session(&mut s);
            }
        }
        // Lo simulado se ve en la sesión (para probar el overlay) pero no entra al día.
        if !ev.is_simulated() {
            let mut p = self.pending();
            match p.get_mut(&(t.day.clone(), t.user_id.clone())) {
                Some(acc) => {
                    acc.likes = acc.likes.saturating_add(t.likes);
                    acc.unique_id.clone_from(&t.unique_id);
                    acc.nickname.clone_from(&t.nickname);
                    acc.avatar.clone_from(&t.avatar);
                }
                None => {
                    p.insert((t.day.clone(), t.user_id.clone()), t);
                }
            }
        }
        self.dirty.store(true, Ordering::Relaxed);
        true
    }

    /// Vacía el ranking de la sesión (el del día se conserva).
    pub fn reset_session(&self) {
        self.session().clear();
        self.dirty.store(true, Ordering::Relaxed);
    }

    /// Borra los likes guardados (día).
    pub async fn clear_history(&self) -> Result<()> {
        self.pending().clear();
        self.db.clear_likers().await?;
        self.dirty.store(true, Ordering::Relaxed);
        Ok(())
    }

    /// Vuelca a la base los likes pendientes.
    pub async fn flush(&self) -> Result<()> {
        let batch: Vec<LikeTap> = std::mem::take(&mut *self.pending()).into_values().collect();
        if batch.is_empty() {
            return Ok(());
        }
        if let Err(e) = self.db.add_likes(&batch).await {
            // Se devuelven a la cola, fusionándolos con lo que llegó mientras tanto, para reintentar sin perder nada.
            let mut p = self.pending();
            for t in batch {
                let key = (t.day.clone(), t.user_id.clone());
                match p.get_mut(&key) {
                    Some(acc) => acc.likes = acc.likes.saturating_add(t.likes),
                    None => {
                        p.insert(key, t);
                    }
                }
            }
            return Err(e);
        }
        Ok(())
    }

    pub async fn top(&self, scope: Scope, limit: u32) -> Result<Vec<LikeEntry>> {
        match scope {
            Scope::Session => Ok(ranked(self.session().values().cloned().collect(), limit as usize)),
            Scope::Day => {
                self.flush().await?;
                self.db.top_likers_day(&(self.today)(), limit).await
            }
        }
    }

    /// Estado que recibe el overlay: los dos ámbitos a la vez (él elige cuál mostrar).
    pub async fn snapshot(&self) -> Result<Value> {
        Ok(json!({
            "session": self.top(Scope::Session, PUBLISHED_SIZE).await?,
            "day": self.top(Scope::Day, PUBLISHED_SIZE).await?,
        }))
    }

    pub async fn publish(&self) {
        match self.snapshot().await {
            Ok(v) => self.hub.publish_retained(CHANNEL, v),
            Err(e) => tracing::warn!(error = %e, "no se pudo calcular el ranking de likes"),
        }
    }

    /// Arranca las tareas de fondo: eventos del bus, sesión nueva y volcado/publicación periódicos.
    pub fn spawn(self: &Arc<Self>, bus: &EventBus, mut sessions: broadcast::Receiver<SessionStarted>, timing: LikeBoardTiming) {
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
                        Err(RecvError::Lagged(n)) => tracing::warn!(missed = n, "el ranking de likes se quedó atrás en el bus"),
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
                        tracing::error!(error = %e, "no se pudieron guardar los likes");
                        this.dirty.store(true, Ordering::Relaxed);
                        continue;
                    }
                    this.publish().await;
                }
            }
        });
    }
}

/// Deja solo los usuarios con más likes (los demás no llegarían al ranking).
fn trim_session(s: &mut HashMap<String, LikeEntry>) {
    let kept = ranked(s.drain().map(|(_, e)| e).collect(), TRIMMED_SESSION_USERS);
    s.extend(kept.into_iter().map(|e| (e.user_id.clone(), e)));
}

#[cfg(test)]
mod tests;
