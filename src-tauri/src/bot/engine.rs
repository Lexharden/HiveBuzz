//! Qué responde el bot a cada evento. Lógica pura: reutiliza el matcher y las condiciones de las
//! reglas (cooldowns, roles, niveles, horario, probabilidad), así el comportamiento es idéntico.

use std::collections::HashMap;

use chrono::NaiveDateTime;

use super::model::{BotConfig, MAX_CHAT_CHARS};
use crate::events::{EventType, LiveEvent};
use crate::rules::eval::{evaluate, Decision, EvalEnv, RuleState};
use crate::rules::matcher::TriggerInput;
use crate::rules::model::{ActionPlan, Conditions, Rule, Trigger};
use crate::rules::template::{render, Vars};

/// Contexto de una evaluación.
pub struct ReplyCtx<'a> {
    /// Saldo del espectador, si alguna plantilla usa `{points}`.
    pub points: Option<u64>,
    pub currency: &'a str,
    pub now_ms: i64,
    pub local: NaiveDateTime,
    /// Número aleatorio en [0, 1): para la probabilidad y para elegir entre variantes.
    pub rand: &'a (dyn Fn() -> f64 + Sync),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub text: String,
    /// De dónde sale (para el registro del bot): `comando discord`, `palabra clave`, `gracias (regalo)`…
    pub source: String,
}

/// ¿Alguna plantilla de la configuración usa `{points}`? (para no consultar la base de datos sin necesidad)
pub fn uses_points(cfg: &BotConfig) -> bool {
    let has = |s: &str| s.to_ascii_lowercase().contains("{points}");
    cfg.commands.iter().any(|c| c.responses.iter().any(|r| has(r)))
        || cfg.keyword_replies.iter().any(|k| k.responses.iter().any(|r| has(r)))
        || [&cfg.thanks.gift, &cfg.thanks.follow, &cfg.thanks.share, &cfg.thanks.subscribe].iter().any(|t| has(&t.template))
}

#[derive(Default)]
pub struct BotEngine {
    states: HashMap<String, RuleState>,
}

/// Regla «de mentira» para reutilizar `evaluate` (no tiene acciones: solo importan trigger y condiciones).
fn pseudo_rule(id: &str, trigger: Trigger, conditions: &Conditions) -> Rule {
    Rule {
        id: id.to_string(),
        name: id.to_string(),
        enabled: true,
        trigger,
        conditions: conditions.clone(),
        plan: ActionPlan { mode: Default::default(), steps: Vec::new() },
        priority: None,
        ttl_ms: 0,
        cost_points: None,
    }
}

fn pick<'a>(options: &'a [String], rand: &(dyn Fn() -> f64 + Sync)) -> Option<&'a String> {
    let usable: Vec<&String> = options.iter().filter(|s| !s.trim().is_empty()).collect();
    if usable.is_empty() {
        return None;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let i = ((rand().clamp(0.0, 0.999_999) * usable.len() as f64) as usize).min(usable.len() - 1);
    usable.get(i).copied()
}

/// Recorta a `MAX_CHAT_CHARS` caracteres (sin partir un emoji).
pub fn fit(text: &str) -> String {
    text.chars().take(MAX_CHAT_CHARS).collect::<String>().trim_end().to_string()
}

impl BotEngine {
    fn fires(&mut self, key: &str, rule: &Rule, ev: &LiveEvent, ctx: &ReplyCtx) -> Option<Vars> {
        let state = self.states.entry(key.to_string()).or_default();
        let env = EvalEnv { now_ms: ctx.now_ms, local: ctx.local, roll: (ctx.rand)(), balance: None };
        match evaluate(rule, &TriggerInput::Live(ev), state, &env) {
            Decision::Fire { vars, .. } => Some(vars),
            _ => None,
        }
    }

    fn finish(&self, template: &str, mut vars: Vars, ctx: &ReplyCtx, source: String) -> Reply {
        vars.insert("currency".into(), ctx.currency.to_string());
        if let Some(p) = ctx.points {
            vars.insert("points".into(), p.to_string());
        }
        Reply { text: fit(&render(template, &vars)), source }
    }

    /// ¿Pasa este comando integrado (`!puntos`, `!top`) su cooldown? Consume el turno si pasa.
    pub fn allow_builtin(&mut self, key: &str, command: &str, ev: &LiveEvent, ctx: &ReplyCtx) -> bool {
        let cond = Conditions { user_cooldown_ms: 10_000, global_cooldown_ms: 1_000, ..Default::default() };
        let rule = pseudo_rule(key, Trigger::Command { command: command.to_string() }, &cond);
        self.fires(key, &rule, ev, ctx).is_some()
    }

    pub fn on_event(&mut self, cfg: &BotConfig, ev: &LiveEvent, ctx: &ReplyCtx) -> Vec<Reply> {
        if !cfg.enabled {
            return Vec::new();
        }
        match ev.kind {
            EventType::Chat => self.on_chat(cfg, ev, ctx).into_iter().collect(),
            EventType::Gift | EventType::Follow | EventType::Share | EventType::Subscribe => self.on_thanks(cfg, ev, ctx).into_iter().collect(),
            _ => Vec::new(),
        }
    }

    fn on_chat(&mut self, cfg: &BotConfig, ev: &LiveEvent, ctx: &ReplyCtx) -> Option<Reply> {
        // Un comando por mensaje: el primero que coincida y pase sus condiciones.
        for c in cfg.commands.iter().filter(|c| c.enabled) {
            for name in &c.names {
                let key = format!("cmd:{}", c.id);
                let rule = pseudo_rule(&key, Trigger::Command { command: name.clone() }, &c.conditions);
                if let Some(vars) = self.fires(&key, &rule, ev, ctx) {
                    let template = pick(&c.responses, ctx.rand)?;
                    return Some(self.finish(template, vars, ctx, format!("comando !{name}")));
                }
            }
        }
        // Si no era un comando, probar las palabras clave (una respuesta por mensaje).
        for k in cfg.keyword_replies.iter().filter(|k| k.enabled) {
            let key = format!("kw:{}", k.id);
            let rule = pseudo_rule(&key, Trigger::Keyword { keywords: k.keywords.clone(), whole_word: k.whole_word }, &k.conditions);
            if let Some(vars) = self.fires(&key, &rule, ev, ctx) {
                let template = pick(&k.responses, ctx.rand)?;
                return Some(self.finish(template, vars, ctx, "palabra clave".into()));
            }
        }
        None
    }

    fn on_thanks(&mut self, cfg: &BotConfig, ev: &LiveEvent, ctx: &ReplyCtx) -> Option<Reply> {
        let (key, thanks, trigger, label) = match ev.kind {
            EventType::Gift => ("thanks:gift", &cfg.thanks.gift, Trigger::Gift { gift_id: None, gift_name: None, min_coins: Some(cfg.thanks.gift.min_coins) }, "gracias (regalo)"),
            EventType::Follow => ("thanks:follow", &cfg.thanks.follow, Trigger::Follow, "gracias (follow)"),
            EventType::Share => ("thanks:share", &cfg.thanks.share, Trigger::Share, "gracias (share)"),
            EventType::Subscribe => ("thanks:subscribe", &cfg.thanks.subscribe, Trigger::Subscribe, "gracias (suscripción)"),
            _ => return None,
        };
        if !thanks.enabled || thanks.template.trim().is_empty() {
            return None;
        }
        let cond = Conditions { user_cooldown_ms: thanks.user_cooldown_ms, ..Default::default() };
        let rule = pseudo_rule(key, trigger, &cond);
        let vars = self.fires(key, &rule, ev, ctx)?;
        Some(self.finish(&thanks.template, vars, ctx, label.into()))
    }

    /// Renderiza una plantilla con variables ya resueltas (para los mensajes integrados y las acciones).
    pub fn render_with(&self, template: &str, mut vars: Vars, currency: &str) -> String {
        vars.insert("currency".into(), currency.to_string());
        fit(&render(template, &vars))
    }
}

#[cfg(test)]
mod tests;
