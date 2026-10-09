//! Timers / subathon: cuenta regresiva que los regalos (y follows, shares, suscripciones, likes)
//! pueden extender. La lógica de estados es pura (el reloj entra por parámetro); el servicio
//! (`service`) la conecta al bus, a los overlays y a las reglas.

pub mod service;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};
use crate::events::{EventType, LiveEvent};

const MAX_START_SECONDS: u64 = 7 * 24 * 3600;
const MAX_EXTENSIONS: usize = 20;

/// Qué extiende el timer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ExtensionSource {
    /// Cada `per_coins` monedas regaladas.
    #[serde(rename_all = "camelCase")]
    Coins { per_coins: u64 },
    /// Cada `per_likes` likes.
    #[serde(rename_all = "camelCase")]
    Likes { per_likes: u64 },
    Follow,
    Share,
    Subscribe,
    /// Cada unidad de un regalo concreto.
    #[serde(rename_all = "camelCase")]
    Gift {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        gift_id: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        gift_name: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Extension {
    pub source: ExtensionSource,
    /// Segundos que suma cada vez.
    pub seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimerConfig {
    pub id: String,
    pub name: String,
    /// Duración inicial.
    pub start_seconds: u64,
    /// Tope del tiempo restante (la extensión nunca lo supera). `None` = sin tope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_seconds: Option<u64>,
    #[serde(default)]
    pub extensions: Vec<Extension>,
}

impl TimerConfig {
    pub fn validate(&self) -> Result<()> {
        if self.id.trim().is_empty() {
            return Err(AppError::Invalid("el timer necesita un id".into()));
        }
        if self.name.trim().is_empty() || self.name.chars().count() > 80 {
            return Err(AppError::Invalid("el nombre del timer debe tener entre 1 y 80 caracteres".into()));
        }
        if self.start_seconds == 0 || self.start_seconds > MAX_START_SECONDS {
            return Err(AppError::Invalid("la duración inicial debe estar entre 1 segundo y 7 días".into()));
        }
        if self.max_seconds.is_some_and(|m| m == 0 || m > MAX_START_SECONDS) {
            return Err(AppError::Invalid("el tope debe estar entre 1 segundo y 7 días".into()));
        }
        if self.extensions.len() > MAX_EXTENSIONS {
            return Err(AppError::Invalid("demasiadas extensiones (máximo 20)".into()));
        }
        for e in &self.extensions {
            if e.seconds == 0 {
                return Err(AppError::Invalid("cada extensión debe sumar al menos 1 segundo".into()));
            }
            match &e.source {
                ExtensionSource::Coins { per_coins: 0 } | ExtensionSource::Likes { per_likes: 0 } => {
                    return Err(AppError::Invalid("«cada N» necesita N ≥ 1".into()));
                }
                ExtensionSource::Gift { gift_id: None, gift_name } if gift_name.as_deref().is_none_or(|n| n.trim().is_empty()) => {
                    return Err(AppError::Invalid("indica el nombre o el id del regalo".into()));
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub fn start_ms(&self) -> i64 {
        secs_to_ms(self.start_seconds)
    }

    pub fn max_ms(&self) -> Option<i64> {
        self.max_seconds.map(secs_to_ms)
    }
}

fn secs_to_ms(s: u64) -> i64 {
    i64::try_from(s).unwrap_or(i64::MAX / 1000).saturating_mul(1000)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    #[default]
    Idle,
    Running,
    Paused,
    Ended,
}

/// Estado de un timer. `remaining_ms` vale cuando NO está corriendo; si corre, manda `ends_at_ms`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimerState {
    pub status: Status,
    pub remaining_ms: i64,
    pub ends_at_ms: i64,
    /// Sobrantes de las extensiones «cada N» (monedas o likes que aún no completan un paso),
    /// una entrada por extensión.
    #[serde(default)]
    pub leftovers: Vec<u64>,
}

impl TimerState {
    pub fn idle(initial_ms: i64) -> Self {
        Self { status: Status::Idle, remaining_ms: initial_ms, ends_at_ms: 0, leftovers: Vec::new() }
    }

    pub fn remaining(&self, now_ms: i64) -> i64 {
        match self.status {
            Status::Running => (self.ends_at_ms - now_ms).max(0),
            _ => self.remaining_ms.max(0),
        }
    }

    /// Inicia desde cero (si estaba parado o terminado) o reanuda (si estaba en pausa).
    pub fn start(&mut self, now_ms: i64, initial_ms: i64) {
        match self.status {
            Status::Running => {}
            Status::Paused => self.resume(now_ms),
            Status::Idle | Status::Ended => {
                self.status = Status::Running;
                self.remaining_ms = initial_ms;
                self.ends_at_ms = now_ms.saturating_add(initial_ms);
            }
        }
    }

    pub fn pause(&mut self, now_ms: i64) {
        if self.status == Status::Running {
            self.remaining_ms = self.remaining(now_ms);
            self.status = Status::Paused;
        }
    }

    pub fn resume(&mut self, now_ms: i64) {
        if self.status == Status::Paused {
            self.ends_at_ms = now_ms.saturating_add(self.remaining_ms);
            self.status = Status::Running;
        }
    }

    pub fn reset(&mut self, initial_ms: i64) {
        *self = Self::idle(initial_ms);
    }

    /// Suma (o resta) tiempo, respetando el tope. Solo afecta a un timer en marcha o en pausa:
    /// uno parado o terminado no se «revive» con regalos. Devuelve si cambió algo.
    pub fn add(&mut self, now_ms: i64, delta_ms: i64, cap_ms: Option<i64>) -> bool {
        if !matches!(self.status, Status::Running | Status::Paused) || delta_ms == 0 {
            return false;
        }
        let before = self.remaining(now_ms);
        let mut after = before.saturating_add(delta_ms).max(0);
        if let Some(cap) = cap_ms {
            // El tope no acorta un tiempo que ya lo superaba por otra vía (p. ej. ajuste manual).
            after = after.min(cap.max(before));
        }
        if after == before {
            return false;
        }
        match self.status {
            Status::Running => self.ends_at_ms = now_ms.saturating_add(after),
            _ => self.remaining_ms = after,
        }
        true
    }

    /// Pasa a `Ended` si el tiempo se agotó. Devuelve `true` solo en el instante en que termina.
    pub fn tick(&mut self, now_ms: i64) -> bool {
        if self.status == Status::Running && self.remaining(now_ms) == 0 {
            self.status = Status::Ended;
            self.remaining_ms = 0;
            return true;
        }
        false
    }
}

/// Segundos que un evento suma a un timer. Actualiza los sobrantes de las extensiones «cada N».
pub fn extension_seconds(cfg: &TimerConfig, leftovers: &mut Vec<u64>, ev: &LiveEvent) -> u64 {
    if leftovers.len() != cfg.extensions.len() {
        leftovers.resize(cfg.extensions.len(), 0);
    }
    let mut total: u64 = 0;
    for (i, ext) in cfg.extensions.iter().enumerate() {
        let steps = match (&ext.source, ev.kind) {
            (ExtensionSource::Follow, EventType::Follow) | (ExtensionSource::Share, EventType::Share) | (ExtensionSource::Subscribe, EventType::Subscribe) => 1,
            (ExtensionSource::Coins { per_coins }, EventType::Gift) => {
                accumulate(&mut leftovers[i], ev.gift.as_ref().map_or(0, |g| g.coins), *per_coins)
            }
            (ExtensionSource::Likes { per_likes }, EventType::Like) => {
                accumulate(&mut leftovers[i], ev.like.as_ref().map_or(0, |l| l.count), *per_likes)
            }
            (ExtensionSource::Gift { gift_id, gift_name }, EventType::Gift) => ev.gift.as_ref().map_or(0, |g| {
                let id_ok = gift_id.is_none_or(|id| id == g.id);
                let name_ok = gift_name.as_deref().is_none_or(|n| n.trim().eq_ignore_ascii_case(g.name.trim()));
                if id_ok && name_ok {
                    u64::from(g.count)
                } else {
                    0
                }
            }),
            _ => 0,
        };
        total = total.saturating_add(steps.saturating_mul(ext.seconds));
    }
    total
}

/// Suma `amount` al sobrante y devuelve cuántos pasos completos de `per` se alcanzaron.
fn accumulate(leftover: &mut u64, amount: u64, per: u64) -> u64 {
    if per == 0 {
        return 0;
    }
    let acc = leftover.saturating_add(amount);
    *leftover = acc % per;
    acc / per
}

#[cfg(test)]
mod tests;
