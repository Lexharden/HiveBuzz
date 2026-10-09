//! Servicio de metas: aplica los eventos del bus a cada meta, avisa cuando se alcanzan
//! (dispara reglas), guarda el progreso y lo publica a los overlays.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::broadcast::{self, error::RecvError};

use super::{add, adjust, contribution, Goal};
use crate::bus::EventBus;
use crate::db::Db;
use crate::error::{AppError, Result};
use crate::events::LiveEvent;
use crate::overlay::OverlayHub;
use crate::rules::engine::{SystemEvent, SystemSink};
use crate::session::SessionStarted;

/// Canal retenido que leen los overlays de metas.
pub const CHANNEL: &str = "goals";

#[derive(Debug, Clone, Copy)]
pub struct GoalTiming {
    /// Cada cuánto se publica el progreso a los overlays (si cambió).
    pub publish_every: Duration,
    /// Cada cuánto se guarda el progreso en la base de datos (si cambió).
    pub save_every: Duration,
}

impl Default for GoalTiming {
    fn default() -> Self {
        Self { publish_every: Duration::from_millis(250), save_every: Duration::from_secs(3) }
    }
}

pub struct GoalService {
    goals: Mutex<Vec<Goal>>,
    publish_dirty: AtomicBool,
    save_dirty: AtomicBool,
    hub: OverlayHub,
    db: Db,
    /// Se engancha después de crear el servicio: el motor de reglas necesita el registro de
    /// ejecutores, y uno de ellos («modificar meta») necesita este servicio.
    sink: OnceLock<Arc<dyn SystemSink>>,
}

impl GoalService {
    pub fn new(db: Db, hub: OverlayHub) -> Arc<Self> {
        Arc::new(Self {
            goals: Mutex::new(Vec::new()),
            publish_dirty: AtomicBool::new(false),
            save_dirty: AtomicBool::new(false),
            hub,
            db,
            sink: OnceLock::new(),
        })
    }

    /// Conecta quien recibe «meta alcanzada» (el motor de reglas).
    pub fn attach_sink(&self, sink: Arc<dyn SystemSink>) {
        if self.sink.set(sink).is_err() {
            tracing::warn!("el sumidero de metas ya estaba conectado");
        }
    }

    async fn fire_reached(&self, id: &str, times: u32) {
        let Some(sink) = self.sink.get() else { return };
        for _ in 0..times {
            sink.fire(SystemEvent::GoalReached(id.to_string())).await;
        }
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Goal>> {
        self.goals.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Carga las metas guardadas y las publica.
    pub async fn load(&self) -> Result<usize> {
        let goals = self.db.list_goals().await?;
        let n = goals.len();
        *self.lock() = goals;
        self.publish();
        Ok(n)
    }

    pub fn list(&self) -> Vec<Goal> {
        self.lock().clone()
    }

    /// Crea o edita una meta. Al editar se conserva el progreso real (la UI pudo cargar la meta
    /// hace rato): para cambiarlo están `set_current`, `adjust` y `reset`.
    pub async fn upsert(&self, mut goal: Goal) -> Result<()> {
        goal.validate()?;
        {
            let mut goals = self.lock();
            match goals.iter_mut().find(|g| g.id == goal.id) {
                Some(existing) => {
                    goal.current = existing.current;
                    goal.reached_count = existing.reached_count;
                    *existing = goal.clone();
                }
                None => {
                    goal.current = 0;
                    goal.reached_count = 0;
                    goals.push(goal.clone());
                }
            }
        }
        self.db.save_goal(&goal).await?;
        self.publish();
        Ok(())
    }

    pub async fn delete(&self, id: &str) -> Result<bool> {
        self.lock().retain(|g| g.id != id);
        let existed = self.db.delete_goal(id).await?;
        self.publish();
        Ok(existed)
    }

    /// Suma o resta (acción «modificar meta»). Sumar puede alcanzar la meta y disparar reglas.
    pub async fn adjust(&self, id: &str, delta: i64) -> Result<()> {
        let reached = self.with_goal(id, |g| adjust(g, delta))?;
        self.after_change(id, reached).await;
        Ok(())
    }

    /// Fija el progreso (no dispara la meta).
    pub async fn set_current(&self, id: &str, value: u64) -> Result<()> {
        self.with_goal(id, |g| {
            g.current = value;
            0
        })?;
        self.after_change(id, 0).await;
        Ok(())
    }

    pub async fn reset(&self, id: &str) -> Result<()> {
        self.set_current(id, 0).await
    }

    fn with_goal(&self, id: &str, f: impl FnOnce(&mut Goal) -> u32) -> Result<u32> {
        let mut goals = self.lock();
        let goal = goals
            .iter_mut()
            .find(|g| g.id == id)
            .ok_or_else(|| AppError::Invalid("la meta no existe".into()))?;
        Ok(f(goal))
    }

    async fn after_change(&self, id: &str, reached: u32) {
        self.mark_dirty();
        self.fire_reached(id, reached).await;
    }

    /// Aplica un evento en vivo a todas las metas. Devuelve `(id, veces alcanzada)`.
    pub fn apply_event(&self, ev: &LiveEvent) -> Vec<(String, u32)> {
        let mut reached = Vec::new();
        let mut changed = false;
        for g in self.lock().iter_mut() {
            let amount = contribution(&g.kind, ev);
            if amount == 0 {
                continue;
            }
            changed = true;
            let n = add(g, amount);
            if n > 0 {
                reached.push((g.id.clone(), n));
            }
        }
        if changed {
            self.mark_dirty();
        }
        reached
    }

    /// Una sesión nueva reinicia las metas que así lo piden.
    pub fn on_session_started(&self) {
        let mut any = false;
        for g in self.lock().iter_mut().filter(|g| g.reset_on_session) {
            g.current = 0;
            any = true;
        }
        if any {
            self.mark_dirty();
        }
    }

    fn mark_dirty(&self) {
        self.publish_dirty.store(true, Ordering::Relaxed);
        self.save_dirty.store(true, Ordering::Relaxed);
    }

    /// Estado que reciben los overlays de metas.
    pub fn snapshot(&self) -> Value {
        let goals: Vec<Value> = self
            .lock()
            .iter()
            .map(|g| {
                json!({
                    "id": g.id,
                    "name": g.name,
                    "kind": serde_json::to_value(&g.kind).ok().and_then(|k| k.get("type").cloned()).unwrap_or(Value::Null),
                    "current": g.current,
                    "target": g.target,
                    "percent": g.percent(),
                    "reachedCount": g.reached_count,
                })
            })
            .collect();
        json!({ "goals": goals })
    }

    fn publish(&self) {
        self.hub.publish_retained(CHANNEL, self.snapshot());
    }

    /// Guarda el progreso ahora mismo (también al cerrar la app).
    pub async fn flush(&self) {
        if self.save_dirty.swap(false, Ordering::Relaxed) {
            let goals = self.list();
            if let Err(e) = self.db.save_goals(&goals).await {
                tracing::error!(error = %e, "no se pudo guardar el progreso de las metas");
                self.save_dirty.store(true, Ordering::Relaxed);
            }
        }
    }

    /// Arranca las tareas de fondo: eventos del bus, nueva sesión y publicación/guardado periódicos.
    pub fn spawn(
        self: &Arc<Self>,
        bus: &EventBus,
        mut sessions: broadcast::Receiver<SessionStarted>,
        timing: GoalTiming,
    ) {
        let this = Arc::clone(self);
        let mut events = bus.subscribe();
        tokio::spawn(async move {
            let mut sessions_open = true;
            loop {
                tokio::select! {
                    ev = events.recv() => match ev {
                        Ok(ev) => {
                            for (id, n) in this.apply_event(&ev) {
                                this.fire_reached(&id, n).await;
                            }
                        }
                        Err(RecvError::Lagged(n)) => tracing::warn!(missed = n, "las metas se quedaron atrás en el bus"),
                        Err(RecvError::Closed) => break,
                    },
                    // Si el canal de sesiones se cierra, se deja de escucharlo (si no, giraría sin parar).
                    s = sessions.recv(), if sessions_open => match s {
                        Ok(_) => this.on_session_started(),
                        Err(RecvError::Lagged(_)) => {}
                        Err(RecvError::Closed) => sessions_open = false,
                    },
                }
            }
        });

        let this = Arc::clone(self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(timing.publish_every);
            let ticks_per_save = (timing.save_every.as_millis() / timing.publish_every.as_millis().max(1)).max(1);
            let mut n: u128 = 0;
            loop {
                tick.tick().await;
                if this.publish_dirty.swap(false, Ordering::Relaxed) {
                    this.publish();
                }
                n += 1;
                if n.is_multiple_of(ticks_per_save) {
                    this.flush().await;
                }
            }
        });
    }
}

#[cfg(test)]
mod tests;
