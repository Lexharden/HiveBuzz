//! Motor de reglas declarativo: trigger → condiciones → acciones.

pub mod engine;
pub mod eval;
pub mod matcher;
pub mod model;
pub mod sample;
pub mod template;

use crate::error::{AppError, Result};
use eval::parse_hhmm;
use model::{Rule, Trigger};

const MAX_STEPS: usize = 50;
const MAX_COST: u64 = 1_000_000_000;
const MAX_DELAY_MS: u64 = 60 * 60 * 1000;

/// Nombre de una llamada a la API local: corto y sin caracteres que compliquen una URL o un JSON.
pub fn valid_api_name(name: &str) -> bool {
    (1..=64).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Valida una regla antes de guardarla. Devuelve el primer problema encontrado.
pub fn validate_rule(rule: &Rule) -> Result<()> {
    let bad = |m: &str| Err(AppError::Invalid(format!("regla «{}»: {m}", rule.name)));
    if rule.id.trim().is_empty() {
        return bad("falta el id");
    }
    if rule.name.trim().is_empty() {
        return Err(AppError::Invalid("la regla necesita un nombre".into()));
    }
    if !(0.0..=100.0).contains(&rule.conditions.probability) {
        return bad("la probabilidad debe estar entre 0 y 100");
    }
    if let Some(s) = &rule.conditions.schedule {
        if parse_hhmm(&s.from).is_none() || parse_hhmm(&s.to).is_none() {
            return bad("el horario debe tener formato HH:MM");
        }
        if s.days.iter().any(|d| *d > 6) {
            return bad("los días van de 0 (lunes) a 6 (domingo)");
        }
    }
    match &rule.trigger {
        Trigger::Like { every } if *every == 0 => return bad("«cada N likes» necesita N ≥ 1"),
        Trigger::Command { command } if command.trim().trim_start_matches('!').is_empty() => {
            return bad("el comando está vacío");
        }
        Trigger::Keyword { keywords, .. } if keywords.iter().all(|k| k.trim().is_empty()) => {
            return bad("indica al menos una palabra clave");
        }
        Trigger::Api { name } if !valid_api_name(name) => {
            return bad("el nombre de la llamada a la API debe tener 1–64 letras, números, guiones o guiones bajos");
        }
        _ => {}
    }
    if let Some(cost) = rule.cost_points {
        if cost == 0 || cost > MAX_COST {
            return bad("el coste debe estar entre 1 y 1 000 000 000 puntos");
        }
        if matches!(rule.trigger, Trigger::GoalReached { .. } | Trigger::TimerEnded { .. } | Trigger::Api { .. }) {
            return bad("una recompensa con coste necesita un disparador de un espectador (comando, palabra clave, regalo…)");
        }
    }
    if rule.plan.steps.is_empty() {
        return bad("necesita al menos una acción");
    }
    if rule.plan.steps.len() > MAX_STEPS {
        return bad("demasiadas acciones (máximo 50)");
    }
    for (i, step) in rule.plan.steps.iter().enumerate() {
        if step.delay_ms > MAX_DELAY_MS {
            return bad(&format!("el retardo de la acción {} supera 1 hora", i + 1));
        }
        if step.action.kind.trim().is_empty() {
            return bad(&format!("la acción {} no tiene tipo", i + 1));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::{ActionPlan, ActionSpec, Conditions, Schedule, Step};
    use serde_json::json;

    fn valid() -> Rule {
        Rule {
            id: "r".into(),
            name: "Rosa".into(),
            enabled: true,
            trigger: Trigger::Follow,
            conditions: Conditions::default(),
            plan: ActionPlan {
                mode: Default::default(),
                steps: vec![Step { delay_ms: 0, action: ActionSpec::new("tts", json!({"text": "hola"})) }],
            },
            priority: None,
            ttl_ms: 1000,
            cost_points: None,
        }
    }

    #[test]
    fn a_valid_rule_passes() {
        assert!(validate_rule(&valid()).is_ok());
        let mut reward = valid();
        reward.trigger = Trigger::Command { command: "!sonido".into() };
        reward.cost_points = Some(200);
        assert!(validate_rule(&reward).is_ok());
    }

    #[test]
    fn rejects_each_kind_of_bad_rule() {
        let mut cases: Vec<(&str, Rule)> = Vec::new();
        let mut r = valid(); r.id = " ".into(); cases.push(("id", r));
        let mut r = valid(); r.name = "".into(); cases.push(("nombre", r));
        let mut r = valid(); r.conditions.probability = 101.0; cases.push(("prob alta", r));
        let mut r = valid(); r.conditions.probability = -1.0; cases.push(("prob baja", r));
        let mut r = valid(); r.conditions.schedule = Some(Schedule { days: vec![], from: "25:00".into(), to: "10:00".into() }); cases.push(("horario", r));
        let mut r = valid(); r.conditions.schedule = Some(Schedule { days: vec![7], from: "10:00".into(), to: "11:00".into() }); cases.push(("día", r));
        let mut r = valid(); r.trigger = Trigger::Like { every: 0 }; cases.push(("likes", r));
        let mut r = valid(); r.trigger = Trigger::Command { command: "!".into() }; cases.push(("comando", r));
        let mut r = valid(); r.trigger = Trigger::Keyword { keywords: vec![" ".into()], whole_word: false }; cases.push(("keywords", r));
        let mut r = valid(); r.plan.steps.clear(); cases.push(("sin acciones", r));
        let mut r = valid(); r.plan.steps[0].delay_ms = MAX_DELAY_MS + 1; cases.push(("retardo", r));
        let mut r = valid(); r.plan.steps[0].action.kind = " ".into(); cases.push(("tipo", r));
        let mut r = valid(); r.plan.steps = (0..51).map(|_| r.plan.steps[0].clone()).collect(); cases.push(("muchas", r));
        let mut r = valid(); r.cost_points = Some(0); cases.push(("coste cero", r));
        let mut r = valid(); r.cost_points = Some(MAX_COST + 1); cases.push(("coste enorme", r));
        let mut r = valid(); r.cost_points = Some(10); r.trigger = Trigger::GoalReached { goal_id: "g".into() }; cases.push(("coste sin espectador", r));
        for (what, rule) in cases {
            assert!(validate_rule(&rule).is_err(), "debía rechazar: {what}");
        }
    }
}
