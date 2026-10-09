//! Acciones internas: modificar una meta y controlar un timer.
//!
//! Las cantidades pueden ser un número o una plantilla con variables (p. ej. `"{coins}"` para sumar
//! tantos segundos como monedas tenga el regalo que disparó la regla).

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Map, Value};

use super::require_str;
use crate::actions::{ActionContext, ActionExecutor};
use crate::error::{AppError, Result};
use crate::goals::service::GoalService;
use crate::timers::service::{Control, TimerService};

/// Lee una cantidad entera con signo: número JSON o texto con variables que se resuelve y se parsea.
fn amount(ctx: &ActionContext, params: &Map<String, Value>, key: &str) -> Result<i64> {
    let bad = || AppError::Invalid(format!("«{key}» debe ser un número entero (o una variable numérica)"));
    match params.get(key) {
        Some(Value::Number(n)) => {
            let f = n.as_f64().filter(|f| f.is_finite() && f.fract() == 0.0).ok_or_else(bad)?;
            #[allow(clippy::cast_possible_truncation)]
            Ok(f as i64)
        }
        Some(Value::String(s)) => {
            let rendered = ctx.render(s);
            rendered.trim().parse::<i64>().map_err(|_| AppError::Invalid(format!(
                "«{key}» resolvió a «{}», que no es un número entero",
                rendered.trim()
            )))
        }
        _ => Err(bad()),
    }
}

/// Valida en el guardado: número, o texto con una variable (no se puede resolver hasta ejecutar).
fn validate_amount(params: &Map<String, Value>, key: &str) -> Result<()> {
    match params.get(key) {
        Some(Value::Number(n)) if n.as_f64().is_some_and(|f| f.is_finite() && f.fract() == 0.0) => Ok(()),
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(()),
        _ => Err(AppError::Invalid(format!("«{key}» debe ser un número entero o una variable"))),
    }
}

// ---- goalAdjust ------------------------------------------------------------------------------------

/// `goalAdjust`: `goalId`, `op` (`add` por defecto, `set`, `reset`) y `amount`.
pub struct GoalAdjustExecutor {
    goals: Arc<GoalService>,
}

impl GoalAdjustExecutor {
    pub fn new(goals: Arc<GoalService>) -> Self {
        Self { goals }
    }
}

fn goal_op(params: &Map<String, Value>) -> &str {
    params.get("op").and_then(Value::as_str).unwrap_or("add")
}

#[async_trait]
impl ActionExecutor for GoalAdjustExecutor {
    fn kind(&self) -> &'static str {
        "goalAdjust"
    }

    fn validate(&self, params: &Map<String, Value>) -> Result<()> {
        require_str(params, "goalId")?;
        match goal_op(params) {
            "add" | "set" => validate_amount(params, "amount"),
            "reset" => Ok(()),
            other => Err(AppError::Invalid(format!("operación de meta desconocida: «{other}»"))),
        }
    }

    async fn execute(&self, ctx: &ActionContext, params: &Map<String, Value>) -> Result<()> {
        let id = ctx.render(require_str(params, "goalId")?);
        match goal_op(params) {
            "add" => self.goals.adjust(&id, amount(ctx, params, "amount")?).await,
            "set" => {
                let v = amount(ctx, params, "amount")?;
                let v = u64::try_from(v).map_err(|_| AppError::Invalid("el progreso no puede ser negativo".into()))?;
                self.goals.set_current(&id, v).await
            }
            "reset" => self.goals.reset(&id).await,
            other => Err(AppError::Invalid(format!("operación de meta desconocida: «{other}»"))),
        }
    }
}

// ---- timerControl ----------------------------------------------------------------------------------

/// `timerControl`: `timerId`, `op` (`start`, `pause`, `resume`, `reset`, `add`) y `seconds` (para `add`).
pub struct TimerControlExecutor {
    timers: Arc<TimerService>,
}

impl TimerControlExecutor {
    pub fn new(timers: Arc<TimerService>) -> Self {
        Self { timers }
    }
}

#[async_trait]
impl ActionExecutor for TimerControlExecutor {
    fn kind(&self) -> &'static str {
        "timerControl"
    }

    fn validate(&self, params: &Map<String, Value>) -> Result<()> {
        require_str(params, "timerId")?;
        match require_str(params, "op")? {
            "start" | "pause" | "resume" | "reset" => Ok(()),
            "add" => validate_amount(params, "seconds"),
            other => Err(AppError::Invalid(format!("operación de timer desconocida: «{other}»"))),
        }
    }

    async fn execute(&self, ctx: &ActionContext, params: &Map<String, Value>) -> Result<()> {
        let id = ctx.render(require_str(params, "timerId")?);
        let op = match require_str(params, "op")? {
            "start" => Control::Start,
            "pause" => Control::Pause,
            "resume" => Control::Resume,
            "reset" => Control::Reset,
            "add" => Control::AddSeconds(amount(ctx, params, "seconds")?),
            other => return Err(AppError::Invalid(format!("operación de timer desconocida: «{other}»"))),
        };
        self.timers.control(&id, op).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::clock::AppClock;
    use crate::db::Db;
    use crate::goals::{Goal, GoalKind, OnReach};
    use crate::overlay::OverlayHub;
    use crate::timers::service::TimerService;
    use crate::timers::{Status, TimerConfig};
    use serde_json::json;

    fn params(v: Value) -> Map<String, Value> {
        v.as_object().cloned().expect("objeto")
    }

    fn ctx(vars: &[(&str, &str)]) -> ActionContext {
        ActionContext { rule_id: "r".into(), vars: vars.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect() }
    }

    async fn goals() -> Arc<GoalService> {
        let svc = GoalService::new(Db::open_memory().await.expect("db"), OverlayHub::new(8));
        svc.upsert(Goal {
            id: "g".into(),
            name: "Meta".into(),
            kind: GoalKind::Likes,
            target: 100,
            current: 0,
            on_reach: OnReach::Stop,
            reset_on_session: false,
            reached_count: 0,
        })
        .await
        .expect("upsert");
        svc
    }

    async fn timers() -> Arc<TimerService> {
        let svc = TimerService::new(Db::open_memory().await.expect("db"), OverlayHub::new(8), Arc::new(AppClock::new()));
        svc.upsert(TimerConfig { id: "t".into(), name: "T".into(), start_seconds: 600, max_seconds: None, extensions: vec![] })
            .await
            .expect("upsert");
        svc
    }

    /// Segundos restantes redondeados (el timer usa el reloj real y descuenta milisegundos entre pasos).
    fn secs(svc: &TimerService) -> i64 {
        (svc.list()[0].remaining_ms + 500) / 1000
    }

    fn goal_current(svc: &GoalService) -> u64 {
        svc.list()[0].current
    }

    #[tokio::test]
    async fn goal_add_set_and_reset() {
        let svc = goals().await;
        let exec = GoalAdjustExecutor::new(svc.clone());
        exec.execute(&ctx(&[]), &params(json!({"goalId": "g", "amount": 40}))).await.expect("add");
        exec.execute(&ctx(&[]), &params(json!({"goalId": "g", "op": "add", "amount": -15}))).await.expect("sub");
        assert_eq!(goal_current(&svc), 25);
        exec.execute(&ctx(&[]), &params(json!({"goalId": "g", "op": "set", "amount": 77}))).await.expect("set");
        assert_eq!(goal_current(&svc), 77);
        exec.execute(&ctx(&[]), &params(json!({"goalId": "g", "op": "reset"}))).await.expect("reset");
        assert_eq!(goal_current(&svc), 0);
    }

    #[tokio::test]
    async fn goal_amounts_can_come_from_variables() {
        let svc = goals().await;
        let exec = GoalAdjustExecutor::new(svc.clone());
        exec.execute(&ctx(&[("coins", "30")]), &params(json!({"goalId": "g", "amount": "{coins}"}))).await.expect("add");
        assert_eq!(goal_current(&svc), 30);
        let e = exec.execute(&ctx(&[("coins", "mucho")]), &params(json!({"goalId": "g", "amount": "{coins}"}))).await.expect_err("no numérico");
        assert!(e.to_string().contains("mucho"));
        assert!(exec.execute(&ctx(&[]), &params(json!({"goalId": "g", "amount": "{coins}"}))).await.is_err(), "variable sin resolver");
    }

    #[tokio::test]
    async fn goal_errors_are_clear() {
        let svc = goals().await;
        let exec = GoalAdjustExecutor::new(svc);
        assert!(exec.execute(&ctx(&[]), &params(json!({"goalId": "nada", "amount": 1}))).await.is_err());
        assert!(exec.execute(&ctx(&[]), &params(json!({"goalId": "g", "op": "set", "amount": -5}))).await.is_err());
        assert!(exec.execute(&ctx(&[]), &params(json!({"goalId": "g", "amount": 1.5}))).await.is_err());
    }

    #[tokio::test]
    async fn goal_validation() {
        let exec = GoalAdjustExecutor::new(goals().await);
        assert!(exec.validate(&params(json!({"goalId": "g", "amount": 5}))).is_ok());
        assert!(exec.validate(&params(json!({"goalId": "g", "amount": "{coins}"}))).is_ok());
        assert!(exec.validate(&params(json!({"goalId": "g", "op": "reset"}))).is_ok());
        for bad in [json!({}), json!({"goalId": "g"}), json!({"goalId": "g", "amount": 1.5}), json!({"goalId": "g", "op": "explotar", "amount": 1}), json!({"goalId": "g", "amount": ""})] {
            assert!(exec.validate(&params(bad.clone())).is_err(), "{bad}");
        }
    }

    #[tokio::test]
    async fn timer_start_add_pause_resume_reset() {
        let svc = timers().await;
        let exec = TimerControlExecutor::new(svc.clone());
        let run = |p: Value| {
            let exec = &exec;
            async move { exec.execute(&ctx(&[("coins", "45")]), &params(p)).await }
        };
        run(json!({"timerId": "t", "op": "start"})).await.expect("start");
        run(json!({"timerId": "t", "op": "add", "seconds": 60})).await.expect("add");
        assert_eq!(secs(&svc), 660);
        run(json!({"timerId": "t", "op": "add", "seconds": "{coins}"})).await.expect("add var");
        assert_eq!(secs(&svc), 705);
        run(json!({"timerId": "t", "op": "add", "seconds": -5})).await.expect("sub");
        assert_eq!(secs(&svc), 700);
        run(json!({"timerId": "t", "op": "pause"})).await.expect("pause");
        assert_eq!(svc.list()[0].status, Status::Paused);
        run(json!({"timerId": "t", "op": "resume"})).await.expect("resume");
        assert_eq!(svc.list()[0].status, Status::Running);
        run(json!({"timerId": "t", "op": "reset"})).await.expect("reset");
        assert_eq!((svc.list()[0].status, svc.list()[0].remaining_ms), (Status::Idle, 600_000));
        assert!(run(json!({"timerId": "nada", "op": "start"})).await.is_err());
    }

    #[tokio::test]
    async fn timer_validation() {
        let exec = TimerControlExecutor::new(timers().await);
        assert!(exec.validate(&params(json!({"timerId": "t", "op": "start"}))).is_ok());
        assert!(exec.validate(&params(json!({"timerId": "t", "op": "add", "seconds": 30}))).is_ok());
        assert!(exec.validate(&params(json!({"timerId": "t", "op": "add", "seconds": "{coins}"}))).is_ok());
        for bad in [json!({}), json!({"timerId": "t"}), json!({"timerId": "t", "op": "add"}), json!({"timerId": "t", "op": "borrar"}), json!({"op": "start"})] {
            assert!(exec.validate(&params(bad.clone())).is_err(), "{bad}");
        }
    }
}
