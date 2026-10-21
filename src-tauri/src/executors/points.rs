//! Acción de puntos: `pointsAdjust`.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Map, Value};

use crate::actions::{ActionContext, ActionExecutor};
use crate::error::{AppError, Result};
use crate::points::service::PointsService;

/// `pointsAdjust`: `amount` (entero con signo o variable) y `target` (`@usuario`; por defecto `{user}`).
pub struct PointsAdjustExecutor {
    points: Arc<PointsService>,
}

impl PointsAdjustExecutor {
    pub fn new(points: Arc<PointsService>) -> Self {
        Self { points }
    }
}

fn parse_amount(ctx: &ActionContext, params: &Map<String, Value>) -> Result<i64> {
    let bad = |what: String| AppError::Invalid(format!("«amount» debe ser un número entero (resolvió a «{what}»)"));
    match params.get("amount") {
        Some(Value::Number(n)) => n
            .as_f64()
            .filter(|f| f.is_finite() && f.fract() == 0.0 && f.abs() < 1e15)
            .map(|f| {
                #[allow(clippy::cast_possible_truncation)]
                let v = f as i64;
                v
            })
            .ok_or_else(|| bad(n.to_string())),
        Some(Value::String(s)) => {
            let r = ctx.render(s);
            r.trim().parse::<i64>().map_err(|_| bad(r.trim().to_string()))
        }
        _ => Err(AppError::Invalid("«amount» debe ser un número entero o una variable".into())),
    }
}

#[async_trait]
impl ActionExecutor for PointsAdjustExecutor {
    fn kind(&self) -> &'static str {
        "pointsAdjust"
    }

    fn validate(&self, params: &Map<String, Value>) -> Result<()> {
        match params.get("amount") {
            Some(Value::Number(n)) if n.as_f64().is_some_and(|f| f.is_finite() && f.fract() == 0.0) => Ok(()),
            Some(Value::String(s)) if !s.trim().is_empty() => Ok(()),
            _ => Err(AppError::Invalid("«amount» debe ser un número entero o una variable".into())),
        }
    }

    async fn execute(&self, ctx: &ActionContext, params: &Map<String, Value>) -> Result<()> {
        let target = params.get("target").and_then(Value::as_str).filter(|t| !t.trim().is_empty()).unwrap_or("{user}");
        let target = ctx.render(target);
        let target = target.trim().trim_start_matches('@');
        if target.is_empty() {
            return Err(AppError::Invalid("no hay a quién sumar o restar puntos".into()));
        }
        let delta = parse_amount(ctx, params)?;
        self.points.adjust_by_unique(target, delta).await.map(|_| ())
    }
}
