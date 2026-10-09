//! Metas: barras de progreso de likes, follows, shares, suscripciones, monedas o de un regalo
//! concreto. La lógica de avance es pura; el servicio (`service`) la conecta al bus y a los overlays.

pub mod service;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};
use crate::events::{EventType, LiveEvent};

/// Tope de veces que una sola aportación puede cruzar la meta (evita bucles con metas diminutas).
const MAX_REACHES_PER_ADD: u32 = 50;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum GoalKind {
    Likes,
    Follows,
    Shares,
    Subscribers,
    /// Monedas totales regaladas.
    Coins,
    /// Cantidad de un regalo concreto (por id y/o nombre).
    #[serde(rename_all = "camelCase")]
    Gift {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        gift_id: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        gift_name: Option<String>,
    },
}

/// Qué pasa al alcanzar la meta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum OnReach {
    /// Se queda completa (sigue contando por encima del objetivo, pero avisa una sola vez).
    #[default]
    Stop,
    /// Vuelve a empezar conservando el sobrante.
    Reset,
    /// El objetivo sube `add` y se sigue acumulando.
    Extend { add: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Goal {
    pub id: String,
    pub name: String,
    pub kind: GoalKind,
    pub target: u64,
    #[serde(default)]
    pub current: u64,
    #[serde(default)]
    pub on_reach: OnReach,
    /// Empieza de cero al comenzar una sesión (LIVE) nueva.
    #[serde(default)]
    pub reset_on_session: bool,
    #[serde(default)]
    pub reached_count: u32,
}

impl Goal {
    pub fn validate(&self) -> Result<()> {
        if self.id.trim().is_empty() {
            return Err(AppError::Invalid("la meta necesita un id".into()));
        }
        if self.name.trim().is_empty() || self.name.chars().count() > 80 {
            return Err(AppError::Invalid("el nombre de la meta debe tener entre 1 y 80 caracteres".into()));
        }
        if self.target == 0 {
            return Err(AppError::Invalid("el objetivo debe ser mayor que 0".into()));
        }
        if let GoalKind::Gift { gift_id: None, gift_name } = &self.kind {
            if gift_name.as_deref().is_none_or(|n| n.trim().is_empty()) {
                return Err(AppError::Invalid("indica el nombre o el id del regalo".into()));
            }
        }
        if matches!(self.on_reach, OnReach::Extend { add: 0 }) {
            return Err(AppError::Invalid("«ampliar» necesita sumar al menos 1".into()));
        }
        Ok(())
    }

    /// Porcentaje de avance, 0–100 (acotado, con 2 decimales).
    pub fn percent(&self) -> f64 {
        #[allow(clippy::cast_precision_loss)]
        let p = self.current as f64 / self.target.max(1) as f64 * 100.0;
        (p.clamp(0.0, 100.0) * 100.0).round() / 100.0
    }
}

/// Cuánto aporta un evento a una meta.
pub fn contribution(kind: &GoalKind, ev: &LiveEvent) -> u64 {
    match (kind, ev.kind) {
        (GoalKind::Likes, EventType::Like) => ev.like.as_ref().map_or(0, |l| l.count),
        (GoalKind::Follows, EventType::Follow) | (GoalKind::Shares, EventType::Share) | (GoalKind::Subscribers, EventType::Subscribe) => 1,
        (GoalKind::Coins, EventType::Gift) => ev.gift.as_ref().map_or(0, |g| g.coins),
        (GoalKind::Gift { gift_id, gift_name }, EventType::Gift) => ev.gift.as_ref().map_or(0, |g| {
            let id_ok = gift_id.is_none_or(|id| id == g.id);
            let name_ok = gift_name.as_deref().is_none_or(|n| n.trim().eq_ignore_ascii_case(g.name.trim()));
            if id_ok && name_ok {
                u64::from(g.count)
            } else {
                0
            }
        }),
        _ => 0,
    }
}

/// Suma `amount` a la meta. Devuelve cuántas veces se alcanzó el objetivo con esta aportación.
pub fn add(goal: &mut Goal, amount: u64) -> u32 {
    if amount == 0 {
        return 0;
    }
    let before = goal.current;
    goal.current = goal.current.saturating_add(amount);
    let reached = match goal.on_reach {
        OnReach::Stop => u32::from(before < goal.target && goal.current >= goal.target),
        OnReach::Reset => {
            let mut n = 0;
            while goal.current >= goal.target && n < MAX_REACHES_PER_ADD {
                goal.current -= goal.target;
                n += 1;
            }
            n
        }
        OnReach::Extend { add } => {
            let mut n = 0;
            while goal.current >= goal.target && n < MAX_REACHES_PER_ADD {
                goal.target = goal.target.saturating_add(add.max(1));
                n += 1;
            }
            n
        }
    };
    goal.reached_count = goal.reached_count.saturating_add(reached);
    reached
}

/// Ajuste manual con signo (acción «modificar meta»). Restar nunca dispara la meta.
pub fn adjust(goal: &mut Goal, delta: i64) -> u32 {
    if delta >= 0 {
        add(goal, delta.unsigned_abs())
    } else {
        goal.current = goal.current.saturating_sub(delta.unsigned_abs());
        0
    }
}

#[cfg(test)]
mod tests;
