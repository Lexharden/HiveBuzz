//! Jarrón de regalos: lleva su propia cuenta de monedas (no depende de ninguna meta), la guarda
//! y la publica retenida al overlay. El objetivo y lo que pasa al llenarse salen de la
//! configuración del overlay `jar`, así que se editan junto al resto de sus opciones.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::broadcast::error::RecvError;

use crate::bus::EventBus;
use crate::db::Db;
use crate::error::Result;
use crate::events::{EventType, LiveEvent};
use crate::overlay::OverlayHub;
use crate::overlay_config::OverlayConfigService;

/// Canal retenido que lee el overlay.
pub const CHANNEL: &str = "jar";
const KEY_STATE: &str = "jar.state";
const OVERLAY_ID: &str = "jar";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JarState {
    /// Monedas en la vuelta actual.
    pub coins: u64,
    /// Veces que se ha llenado y vaciado (solo con «vaciar al llenarse»).
    pub laps: u32,
}

/// Qué hace el jarrón al alcanzar el objetivo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnFull {
    /// Se queda lleno y sigue contando monedas.
    Keep,
    /// Se vacía, suma una vuelta y conserva el sobrante.
    Reset,
}

/// Suma `amount` monedas (negativo resta, sin bajar de 0). Con `Reset` y un objetivo > 0, cada vez
/// que se llega al objetivo se vacía y se cuenta una vuelta. Devuelve las vueltas ganadas.
pub fn apply(state: &mut JarState, amount: i64, target: u64, on_full: OnFull) -> u32 {
    state.coins = if amount >= 0 { state.coins.saturating_add(amount.unsigned_abs()) } else { state.coins.saturating_sub(amount.unsigned_abs()) };
    let mut gained = 0;
    if on_full == OnFull::Reset && target > 0 {
        while state.coins >= target {
            state.coins -= target;
            state.laps = state.laps.saturating_add(1);
            gained += 1;
        }
    }
    gained
}

pub struct JarService {
    state: Mutex<JarState>,
    save_dirty: AtomicBool,
    db: Db,
    hub: OverlayHub,
    config: OverlayConfigService,
}

impl JarService {
    pub fn new(db: Db, hub: OverlayHub, config: OverlayConfigService) -> Arc<Self> {
        Arc::new(Self { state: Mutex::new(JarState::default()), save_dirty: AtomicBool::new(false), db, hub, config })
    }

    fn lock(&self) -> MutexGuard<'_, JarState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Carga lo guardado (un valor ilegible se ignora) y lo publica.
    pub async fn load(&self) -> Result<()> {
        if let Some(raw) = self.db.get_setting(KEY_STATE).await? {
            match serde_json::from_str::<JarState>(&raw) {
                Ok(s) => *self.lock() = s,
                Err(e) => tracing::warn!(error = %e, "estado del jarrón ilegible; se empieza vacío"),
            }
        }
        self.publish();
        Ok(())
    }

    pub fn state(&self) -> JarState {
        *self.lock()
    }

    /// Objetivo y política de llenado actuales según la configuración del overlay.
    async fn settings(&self) -> (u64, OnFull, u64) {
        let cfg = self.config.get(OVERLAY_ID).await.unwrap_or_default();
        let target = cfg.get("target").and_then(Value::as_u64).unwrap_or(0);
        let on_full = if cfg.get("onFull").and_then(Value::as_str) == Some("reset") { OnFull::Reset } else { OnFull::Keep };
        let min_coins = cfg.get("minCoins").and_then(Value::as_u64).unwrap_or(0);
        (target, on_full, min_coins)
    }

    /// Suma o resta monedas a mano (botones de la app y prueba).
    pub async fn add(&self, delta: i64) {
        let (target, on_full, _) = self.settings().await;
        apply(&mut self.lock(), delta, target, on_full);
        self.changed();
    }

    /// Fija las monedas de la vuelta actual.
    pub fn set(&self, coins: u64) {
        self.lock().coins = coins;
        self.changed();
    }

    /// Vacía el jarrón y borra las vueltas.
    pub fn reset(&self) {
        *self.lock() = JarState::default();
        self.changed();
    }

    /// Cuenta los regalos del bus. Los de menos de `minCoins` no entran.
    async fn on_event(&self, ev: &LiveEvent) {
        if ev.kind != EventType::Gift {
            return;
        }
        let Some(gift) = ev.gift.as_ref() else { return };
        let (target, on_full, min_coins) = self.settings().await;
        // `coins` ya es el valor total del regalo (unidad × cantidad).
        if gift.coins == 0 || gift.coins < min_coins {
            return;
        }
        let amount = i64::try_from(gift.coins).unwrap_or(i64::MAX);
        apply(&mut self.lock(), amount, target, on_full);
        self.changed();
    }

    fn changed(&self) {
        self.save_dirty.store(true, Ordering::Relaxed);
        self.publish();
    }

    fn publish(&self) {
        let s = self.state();
        self.hub.publish_retained(CHANNEL, json!({ "coins": s.coins, "laps": s.laps }));
    }

    /// Guarda el estado si cambió (también al cerrar la app).
    pub async fn flush(&self) {
        if self.save_dirty.swap(false, Ordering::Relaxed) {
            let raw = serde_json::to_string(&self.state()).unwrap_or_default();
            if let Err(e) = self.db.set_setting(KEY_STATE, &raw).await {
                tracing::error!(error = %e, "no se pudo guardar el jarrón");
                self.save_dirty.store(true, Ordering::Relaxed);
            }
        }
    }

    pub fn spawn(self: &Arc<Self>, bus: &EventBus, save_every: Duration) {
        let this = Arc::clone(self);
        let mut events = bus.subscribe();
        tokio::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(ev) => this.on_event(&ev).await,
                    Err(RecvError::Lagged(n)) => tracing::warn!(missed = n, "el jarrón se quedó atrás en el bus"),
                    Err(RecvError::Closed) => break,
                }
            }
        });
        let this = Arc::clone(self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(save_every);
            loop {
                tick.tick().await;
                this.flush().await;
            }
        });
    }
}

#[cfg(test)]
mod tests;
