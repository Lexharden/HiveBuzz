//! Acciones de interacción: `spinWheel` y `startPoll`.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Map, Value};

use super::require_str;
use crate::actions::{ActionContext, ActionExecutor, Concurrency};
use crate::error::{AppError, Result};
use crate::interact::poll::{PollService, MAX_OPTIONS};
use crate::interact::wheel::WheelService;

/// `spinWheel`: sin parámetros. Los giros pedidos a la vez se hacen uno tras otro.
pub struct SpinWheelExecutor {
    wheel: Arc<WheelService>,
}

impl SpinWheelExecutor {
    pub fn new(wheel: Arc<WheelService>) -> Self {
        Self { wheel }
    }
}

#[async_trait]
impl ActionExecutor for SpinWheelExecutor {
    fn kind(&self) -> &'static str {
        "spinWheel"
    }

    fn concurrency(&self) -> Concurrency {
        Concurrency::Serial("wheel")
    }

    async fn execute(&self, ctx: &ActionContext, _params: &Map<String, Value>) -> Result<()> {
        self.wheel.spin(ctx.vars.clone()).await.map(|_| ())
    }
}

/// Opciones como lista JSON o como texto separado por `|` o saltos de línea.
fn parse_options(params: &Map<String, Value>) -> Vec<String> {
    match params.get("options") {
        Some(Value::Array(list)) => list.iter().filter_map(Value::as_str).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
        Some(Value::String(text)) => text.split(['|', '\n']).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
        _ => Vec::new(),
    }
}

/// `startPoll`: `question`, `options` y `durationSec` (60 por defecto).
pub struct StartPollExecutor {
    polls: Arc<PollService>,
}

impl StartPollExecutor {
    pub fn new(polls: Arc<PollService>) -> Self {
        Self { polls }
    }
}

#[async_trait]
impl ActionExecutor for StartPollExecutor {
    fn kind(&self) -> &'static str {
        "startPoll"
    }

    fn validate(&self, params: &Map<String, Value>) -> Result<()> {
        require_str(params, "question")?;
        let n = parse_options(params).len();
        if !(2..=MAX_OPTIONS).contains(&n) {
            return Err(AppError::Invalid(format!("la encuesta necesita entre 2 y {MAX_OPTIONS} opciones")));
        }
        if let Some(d) = params.get("durationSec") {
            if !d.as_f64().is_some_and(|d| d.is_finite() && d > 0.0) {
                return Err(AppError::Invalid("«durationSec» debe ser un número positivo".into()));
            }
        }
        Ok(())
    }

    async fn execute(&self, ctx: &ActionContext, params: &Map<String, Value>) -> Result<()> {
        let question = ctx.render(require_str(params, "question")?);
        let options: Vec<String> = parse_options(params).iter().map(|o| ctx.render(o)).collect();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let duration = params.get("durationSec").and_then(Value::as_f64).map_or(60, |d| d.max(0.0) as u64);
        self.polls.start(&question, &options, duration).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn p(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap_or_default()
    }

    #[test]
    fn options_come_from_a_list_or_from_delimited_text() {
        assert_eq!(parse_options(&p(json!({ "options": ["a", " b ", "", 3] }))), ["a", "b"]);
        assert_eq!(parse_options(&p(json!({ "options": "a | b\nc||" }))), ["a", "b", "c"]);
        assert!(parse_options(&p(json!({}))).is_empty());
        assert!(parse_options(&p(json!({ "options": 5 }))).is_empty());
    }
}
