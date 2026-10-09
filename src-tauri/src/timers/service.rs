//! Servicio de timers: aplica las extensiones del bus, detecta cuándo terminan (dispara reglas),
//! guarda el estado y lo publica a los overlays.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::broadcast::error::RecvError;

use super::{extension_seconds, Status, TimerConfig, TimerState};
use crate::actions::clock::Clock;
use crate::bus::EventBus;
use crate::db::Db;
use crate::error::{AppError, Result};
use crate::events::LiveEvent;
use crate::overlay::OverlayHub;
use crate::rules::engine::{SystemEvent, SystemSink};

/// Canal retenido que leen los overlays de timer.
pub const CHANNEL: &str = "timer";

/// Lo que se persiste de cada timer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredTimer {
    pub config: TimerConfig,
    pub state: TimerState,
}

/// Vista para la UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimerView {
    pub config: TimerConfig,
    pub status: Status,
    pub remaining_ms: i64,
}

#[derive(Debug, Clone, Copy)]
pub enum Control {
    Start,
    Pause,
    Resume,
    Reset,
    /// Suma (o resta, si es negativo) segundos.
    AddSeconds(i64),
}

#[derive(Debug, Clone, Copy)]
pub struct TimerTiming {
    /// Cada cuánto se comprueba si algún timer terminó (y se publican los cambios acumulados).
    pub tick_every: Duration,
    /// Cada cuánto se guarda el estado si cambió por extensiones.
    pub save_every: Duration,
}

impl Default for TimerTiming {
    fn default() -> Self {
        Self { tick_every: Duration::from_millis(250), save_every: Duration::from_secs(2) }
    }
}

pub struct TimerService {
    timers: Mutex<Vec<StoredTimer>>,
    publish_dirty: AtomicBool,
    save_dirty: AtomicBool,
    hub: OverlayHub,
    db: Db,
    clock: Arc<dyn Clock>,
    sink: OnceLock<Arc<dyn SystemSink>>,
}

impl TimerService {
    pub fn new(db: Db, hub: OverlayHub, clock: Arc<dyn Clock>) -> Arc<Self> {
        Arc::new(Self {
            timers: Mutex::new(Vec::new()),
            publish_dirty: AtomicBool::new(false),
            save_dirty: AtomicBool::new(false),
            hub,
            db,
            clock,
            sink: OnceLock::new(),
        })
    }

    /// Conecta quien recibe «timer terminado» (el motor de reglas).
    pub fn attach_sink(&self, sink: Arc<dyn SystemSink>) {
        if self.sink.set(sink).is_err() {
            tracing::warn!("el sumidero de timers ya estaba conectado");
        }
    }

    fn lock(&self) -> MutexGuard<'_, Vec<StoredTimer>> {
        self.timers.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Carga los timers guardados. Uno que terminó mientras la app estaba cerrada queda `Ended`
    /// sin disparar reglas (ya no tiene sentido «celebrarlo» horas después).
    pub async fn load(&self) -> Result<usize> {
        let mut timers = self.db.list_timers().await?;
        let now = self.clock.now_ms();
        for t in &mut timers {
            t.state.tick(now);
        }
        let n = timers.len();
        *self.lock() = timers;
        self.publish();
        Ok(n)
    }

    pub fn list(&self) -> Vec<TimerView> {
        let now = self.clock.now_ms();
        self.lock()
            .iter()
            .map(|t| TimerView { config: t.config.clone(), status: t.state.status, remaining_ms: t.state.remaining(now) })
            .collect()
    }

    /// Crea o edita un timer. Al editar se conserva el estado; si estaba parado, toma la nueva duración.
    pub async fn upsert(&self, config: TimerConfig) -> Result<()> {
        config.validate()?;
        let stored = {
            let mut timers = self.lock();
            match timers.iter_mut().find(|t| t.config.id == config.id) {
                Some(t) => {
                    if t.state.status == Status::Idle {
                        t.state.remaining_ms = config.start_ms();
                    }
                    t.config = config;
                    t.clone()
                }
                None => {
                    let t = StoredTimer { state: TimerState::idle(config.start_ms()), config };
                    timers.push(t.clone());
                    t
                }
            }
        };
        self.db.save_timers(&[stored]).await?;
        self.publish();
        Ok(())
    }

    pub async fn delete(&self, id: &str) -> Result<bool> {
        self.lock().retain(|t| t.config.id != id);
        let existed = self.db.delete_timer(id).await?;
        self.publish();
        Ok(existed)
    }

    /// Controla un timer (iniciar, pausar, reanudar, reiniciar, sumar/restar tiempo).
    pub async fn control(&self, id: &str, op: Control) -> Result<()> {
        let now = self.clock.now_ms();
        let stored = {
            let mut timers = self.lock();
            let t = timers
                .iter_mut()
                .find(|t| t.config.id == id)
                .ok_or_else(|| AppError::Invalid("el timer no existe".into()))?;
            let start = t.config.start_ms();
            match op {
                Control::Start => t.state.start(now, start),
                Control::Pause => t.state.pause(now),
                Control::Resume => t.state.resume(now),
                Control::Reset => t.state.reset(start),
                Control::AddSeconds(s) => {
                    t.state.add(now, s.saturating_mul(1000), t.config.max_ms());
                }
            }
            t.clone()
        };
        self.db.save_timers(&[stored]).await?;
        self.publish();
        Ok(())
    }

    /// Aplica un evento en vivo a los timers en marcha. Devuelve si algún timer cambió.
    pub fn apply_event(&self, ev: &LiveEvent) -> bool {
        let now = self.clock.now_ms();
        let mut changed = false;
        for t in self.lock().iter_mut().filter(|t| matches!(t.state.status, Status::Running | Status::Paused)) {
            let secs = extension_seconds(&t.config, &mut t.state.leftovers, ev);
            if secs > 0 {
                let delta = i64::try_from(secs).unwrap_or(i64::MAX / 1000).saturating_mul(1000);
                changed |= t.state.add(now, delta, t.config.max_ms());
            }
        }
        if changed {
            self.publish_dirty.store(true, Ordering::Relaxed);
            self.save_dirty.store(true, Ordering::Relaxed);
        }
        changed
    }

    /// Marca como terminados los timers cuyo tiempo se agotó. Devuelve sus ids.
    pub fn tick(&self) -> Vec<String> {
        let now = self.clock.now_ms();
        let ended: Vec<String> = self
            .lock()
            .iter_mut()
            .filter_map(|t| t.state.tick(now).then(|| t.config.id.clone()))
            .collect();
        if !ended.is_empty() {
            self.publish_dirty.store(true, Ordering::Relaxed);
            self.save_dirty.store(true, Ordering::Relaxed);
        }
        ended
    }

    /// Estado que reciben los overlays de timer. Incluye la hora de fin absoluta: el overlay corre
    /// en la misma PC, así que cuenta hacia atrás con su propio reloj sin pedir actualizaciones.
    pub fn snapshot(&self) -> Value {
        let now = self.clock.now_ms();
        let timers: Vec<Value> = self
            .lock()
            .iter()
            .map(|t| {
                json!({
                    "id": t.config.id,
                    "name": t.config.name,
                    "status": t.state.status,
                    "remainingMs": t.state.remaining(now),
                    "endsAtMs": if t.state.status == Status::Running { t.state.ends_at_ms } else { 0 },
                    "startSeconds": t.config.start_seconds,
                })
            })
            .collect();
        json!({ "timers": timers, "serverNow": now })
    }

    fn publish(&self) {
        self.hub.publish_retained(CHANNEL, self.snapshot());
    }

    pub async fn flush(&self) {
        if self.save_dirty.swap(false, Ordering::Relaxed) {
            let all = self.lock().clone();
            if let Err(e) = self.db.save_timers(&all).await {
                tracing::error!(error = %e, "no se pudo guardar el estado de los timers");
                self.save_dirty.store(true, Ordering::Relaxed);
            }
        }
    }

    /// Arranca las tareas de fondo: extensiones desde el bus y vigilancia del fin de cada timer.
    pub fn spawn(self: &Arc<Self>, bus: &EventBus, timing: TimerTiming) {
        let this = Arc::clone(self);
        let mut events = bus.subscribe();
        tokio::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(ev) => {
                        this.apply_event(&ev);
                    }
                    Err(RecvError::Lagged(n)) => tracing::warn!(missed = n, "los timers se quedaron atrás en el bus"),
                    Err(RecvError::Closed) => break,
                }
            }
        });

        let this = Arc::clone(self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(timing.tick_every);
            let ticks_per_save = (timing.save_every.as_millis() / timing.tick_every.as_millis().max(1)).max(1);
            let mut n: u128 = 0;
            loop {
                tick.tick().await;
                let ended = this.tick();
                if this.publish_dirty.swap(false, Ordering::Relaxed) {
                    this.publish();
                }
                if let Some(sink) = this.sink.get() {
                    for id in ended {
                        sink.fire(SystemEvent::TimerEnded(id)).await;
                    }
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
