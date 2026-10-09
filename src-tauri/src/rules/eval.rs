//! Evaluación completa de una regla: trigger + condiciones (roles, horario, cooldowns,
//! probabilidad). El reloj y el azar entran por parámetro para poder probarlo.

use std::collections::HashMap;

use chrono::{Datelike, NaiveDateTime, Timelike};

use super::matcher::{match_trigger, TriggerInput};
use super::model::{Conditions, Role, Rule, Schedule};
use super::template::{event_vars, Vars};

/// Estado mutable de una regla (cooldowns y acumulador de likes).
#[derive(Debug, Default)]
pub struct RuleState {
    last_global_ms: Option<i64>,
    last_by_user: HashMap<String, i64>,
    like_acc: u64,
}

impl RuleState {
    /// Descarta cooldowns de usuario ya vencidos (evita crecer sin límite en LIVEs largos).
    pub fn prune(&mut self, now_ms: i64, user_cooldown_ms: u64) {
        let span = i64::try_from(user_cooldown_ms).unwrap_or(i64::MAX);
        self.last_by_user.retain(|_, last| now_ms.saturating_sub(*last) < span);
    }

    pub fn tracked_users(&self) -> usize {
        self.last_by_user.len()
    }
}

pub struct EvalEnv {
    pub now_ms: i64,
    /// Hora local (para el horario).
    pub local: NaiveDateTime,
    /// Número aleatorio en [0, 1) para la probabilidad.
    pub roll: f64,
    /// Saldo de puntos del espectador que dispara el evento. `None` = desconocido (sin sistema de
    /// puntos o sin usuario): una regla con coste no puede dispararse sin saber el saldo.
    pub balance: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blocked {
    Disabled,
    Role,
    Schedule,
    GlobalCooldown,
    UserCooldown,
    /// El espectador no tiene puntos suficientes para el coste de la regla.
    Points,
    Probability,
}

#[derive(Debug, PartialEq)]
pub enum Decision {
    NoMatch,
    Blocked(Blocked),
    Fire { times: u32, vars: Vars },
}

pub fn evaluate(rule: &Rule, input: &TriggerInput, state: &mut RuleState, env: &EvalEnv) -> Decision {
    if !rule.enabled {
        return Decision::Blocked(Blocked::Disabled);
    }
    let Some(hit) = match_trigger(&rule.trigger, input, &mut state.like_acc) else {
        return Decision::NoMatch;
    };

    let c = &rule.conditions;
    let user = match input {
        TriggerInput::Live(ev) => Some(&ev.user),
        _ => None,
    };

    if let Some(u) = user {
        if !roles_allow(c, u) {
            return Decision::Blocked(Blocked::Role);
        }
    }
    if let Some(s) = &c.schedule {
        if !schedule_allows(s, env.local) {
            return Decision::Blocked(Blocked::Schedule);
        }
    }

    let user_key = user.map(|u| u.id.clone());
    if c.global_cooldown_ms > 0 {
        if let Some(last) = state.last_global_ms {
            if env.now_ms.saturating_sub(last) < ms(c.global_cooldown_ms) {
                return Decision::Blocked(Blocked::GlobalCooldown);
            }
        }
    }
    if c.user_cooldown_ms > 0 {
        if let Some(last) = user_key.as_ref().and_then(|k| state.last_by_user.get(k)) {
            if env.now_ms.saturating_sub(*last) < ms(c.user_cooldown_ms) {
                return Decision::Blocked(Blocked::UserCooldown);
            }
        }
    }
    // El coste se comprueba antes de consumir nada: quien no puede pagar no gasta cooldown ni turno.
    // (Los disparadores internos —meta, timer— no tienen a quién cobrar.)
    if let (Some(cost), Some(_)) = (rule.cost_points, user) {
        if env.balance.is_none_or(|b| b < cost) {
            return Decision::Blocked(Blocked::Points);
        }
    }
    // La tirada va después de los cooldowns y antes de consumirlos: una tirada fallida no gasta turno.
    if c.probability < 100.0 && env.roll * 100.0 >= c.probability.max(0.0) {
        return Decision::Blocked(Blocked::Probability);
    }

    state.last_global_ms = Some(env.now_ms);
    if let Some(k) = user_key {
        state.last_by_user.insert(k, env.now_ms);
    }

    let mut vars = match input {
        TriggerInput::Live(ev) => event_vars(ev),
        TriggerInput::GoalReached(id) => HashMap::from([("goal".to_string(), (*id).to_string())]),
        TriggerInput::TimerEnded(id) => HashMap::from([("timer".to_string(), (*id).to_string())]),
    };
    vars.extend(hit.vars);
    Decision::Fire { times: hit.times, vars }
}

fn ms(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

fn roles_allow(c: &Conditions, u: &crate::events::User) -> bool {
    let role_ok = c.roles_any.is_empty()
        || c.roles_any.iter().any(|r| match r {
            Role::Moderator => u.is_moderator,
            Role::Subscriber => u.is_subscriber,
            Role::Follower => u.is_follower,
        });
    role_ok
        && c.min_team_level.is_none_or(|m| u.team_level.unwrap_or(0) >= m)
        && c.min_gifter_level.is_none_or(|m| u.gifter_level.unwrap_or(0) >= m)
}

/// "HH:MM" → minutos desde medianoche.
pub fn parse_hhmm(s: &str) -> Option<u32> {
    let (h, m) = s.trim().split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

/// Un horario mal escrito bloquea la regla (falla cerrado); `validate_rule` lo avisa al guardarla.
pub fn schedule_allows(s: &Schedule, local: NaiveDateTime) -> bool {
    let (Some(from), Some(to)) = (parse_hhmm(&s.from), parse_hhmm(&s.to)) else {
        return false;
    };
    let now = local.hour() * 60 + local.minute();
    let today = u8::try_from(local.weekday().num_days_from_monday()).unwrap_or(0);
    let day_ok = |d: u8| s.days.is_empty() || s.days.contains(&d);

    if from == to {
        return day_ok(today); // ventana de 24 h
    }
    if from < to {
        return now >= from && now < to && day_ok(today);
    }
    // Cruza la medianoche: la parte de la madrugada pertenece al día anterior.
    if now >= from {
        day_ok(today)
    } else if now < to {
        day_ok((today + 6) % 7)
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::testing::sample_event;
    use crate::events::{Chat, EventType, Gift};
    use crate::rules::model::{ActionPlan, Trigger};
    use chrono::NaiveDate;

    fn rule(trigger: Trigger, conditions: Conditions) -> Rule {
        Rule {
            id: "r".into(),
            name: "r".into(),
            enabled: true,
            trigger,
            conditions,
            plan: ActionPlan { mode: Default::default(), steps: vec![] },
            priority: None,
            ttl_ms: 1000,
            cost_points: None,
        }
    }

    fn follow() -> LiveEventHolder {
        let mut e = sample_event("f");
        e.kind = EventType::Follow;
        e.chat = None;
        LiveEventHolder(e)
    }

    struct LiveEventHolder(crate::events::LiveEvent);

    fn env(now_ms: i64) -> EvalEnv {
        EvalEnv {
            now_ms,
            local: NaiveDate::from_ymd_opt(2026, 10, 8).expect("fecha").and_hms_opt(12, 0, 0).expect("hora"),
            roll: 0.0,
            balance: None,
        }
    }

    fn fire(r: &Rule, ev: &crate::events::LiveEvent, st: &mut RuleState, e: &EvalEnv) -> Decision {
        evaluate(r, &TriggerInput::Live(ev), st, e)
    }

    #[test]
    fn fires_and_exposes_event_variables() {
        let r = rule(Trigger::Follow, Conditions::default());
        let ev = follow().0;
        match fire(&r, &ev, &mut RuleState::default(), &env(0)) {
            Decision::Fire { times, vars } => {
                assert_eq!(times, 1);
                assert_eq!(vars["nickname"], "Ana");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn disabled_rules_never_fire() {
        let mut r = rule(Trigger::Follow, Conditions::default());
        r.enabled = false;
        assert_eq!(fire(&r, &follow().0, &mut RuleState::default(), &env(0)), Decision::Blocked(Blocked::Disabled));
    }

    #[test]
    fn non_matching_events_are_no_match() {
        let r = rule(Trigger::Share, Conditions::default());
        assert_eq!(fire(&r, &follow().0, &mut RuleState::default(), &env(0)), Decision::NoMatch);
    }

    #[test]
    fn global_cooldown_blocks_until_it_expires() {
        let r = rule(Trigger::Follow, Conditions { global_cooldown_ms: 1000, ..Default::default() });
        let (ev, mut st) = (follow().0, RuleState::default());
        assert!(matches!(fire(&r, &ev, &mut st, &env(0)), Decision::Fire { .. }));
        assert_eq!(fire(&r, &ev, &mut st, &env(999)), Decision::Blocked(Blocked::GlobalCooldown));
        assert!(matches!(fire(&r, &ev, &mut st, &env(1000)), Decision::Fire { .. }));
    }

    #[test]
    fn user_cooldown_is_per_user() {
        let r = rule(Trigger::Follow, Conditions { user_cooldown_ms: 5000, ..Default::default() });
        let mut st = RuleState::default();
        let ana = follow().0;
        let mut bob = follow().0;
        bob.user.id = "2".into();
        assert!(matches!(fire(&r, &ana, &mut st, &env(0)), Decision::Fire { .. }));
        assert_eq!(fire(&r, &ana, &mut st, &env(100)), Decision::Blocked(Blocked::UserCooldown));
        assert!(matches!(fire(&r, &bob, &mut st, &env(100)), Decision::Fire { .. }));
        assert!(matches!(fire(&r, &ana, &mut st, &env(5000)), Decision::Fire { .. }));
    }

    #[test]
    fn blocked_attempts_do_not_consume_the_cooldown() {
        let r = rule(
            Trigger::Follow,
            Conditions { global_cooldown_ms: 1000, probability: 50.0, ..Default::default() },
        );
        let (ev, mut st) = (follow().0, RuleState::default());
        // Tirada fallida (0.9 ≥ 0.5): no se gasta el cooldown.
        let fail = EvalEnv { roll: 0.9, ..env(0) };
        assert_eq!(fire(&r, &ev, &mut st, &fail), Decision::Blocked(Blocked::Probability));
        assert!(matches!(fire(&r, &ev, &mut st, &env(1)), Decision::Fire { .. }));
    }

    #[test]
    fn probability_edges() {
        let ev = follow().0;
        let never = rule(Trigger::Follow, Conditions { probability: 0.0, ..Default::default() });
        let always = rule(Trigger::Follow, Conditions { probability: 100.0, ..Default::default() });
        let high_roll = EvalEnv { roll: 0.999_999, ..env(0) };
        assert_eq!(fire(&never, &ev, &mut RuleState::default(), &env(0)), Decision::Blocked(Blocked::Probability));
        assert!(matches!(fire(&always, &ev, &mut RuleState::default(), &high_roll), Decision::Fire { .. }));
        let half = rule(Trigger::Follow, Conditions { probability: 50.0, ..Default::default() });
        assert!(matches!(fire(&half, &ev, &mut RuleState::default(), &EvalEnv { roll: 0.49, ..env(0) }), Decision::Fire { .. }));
        assert_eq!(
            fire(&half, &ev, &mut RuleState::default(), &EvalEnv { roll: 0.5, ..env(0) }),
            Decision::Blocked(Blocked::Probability)
        );
    }

    #[test]
    fn roles_any_requires_at_least_one() {
        let c = Conditions { roles_any: vec![Role::Moderator, Role::Subscriber], ..Default::default() };
        let r = rule(Trigger::Follow, c);
        let mut ev = follow().0;
        assert_eq!(fire(&r, &ev, &mut RuleState::default(), &env(0)), Decision::Blocked(Blocked::Role));
        ev.user.is_subscriber = true;
        assert!(matches!(fire(&r, &ev, &mut RuleState::default(), &env(0)), Decision::Fire { .. }));
    }

    #[test]
    fn level_requirements() {
        let c = Conditions { min_team_level: Some(3), min_gifter_level: Some(10), ..Default::default() };
        let r = rule(Trigger::Follow, c);
        let mut ev = follow().0;
        assert_eq!(fire(&r, &ev, &mut RuleState::default(), &env(0)), Decision::Blocked(Blocked::Role));
        ev.user.team_level = Some(3);
        ev.user.gifter_level = Some(9);
        assert_eq!(fire(&r, &ev, &mut RuleState::default(), &env(0)), Decision::Blocked(Blocked::Role));
        ev.user.gifter_level = Some(10);
        assert!(matches!(fire(&r, &ev, &mut RuleState::default(), &env(0)), Decision::Fire { .. }));
    }

    #[test]
    fn like_rule_fires_after_enough_likes_and_keeps_the_remainder() {
        let r = rule(Trigger::Like { every: 100 }, Conditions::default());
        let mut st = RuleState::default();
        let mut ev = sample_event("l");
        ev.kind = EventType::Like;
        ev.chat = None;
        ev.like = Some(crate::events::Like { count: 70, total: 0 });
        assert_eq!(fire(&r, &ev, &mut st, &env(0)), Decision::NoMatch);
        assert!(matches!(fire(&r, &ev, &mut st, &env(1)), Decision::Fire { times: 1, .. }));
    }

    #[test]
    fn command_vars_reach_the_firing() {
        let r = rule(Trigger::Command { command: "tts".into() }, Conditions::default());
        let mut ev = sample_event("c");
        ev.chat = Some(Chat { text: "!tts hola".into(), emotes: None });
        match fire(&r, &ev, &mut RuleState::default(), &env(0)) {
            Decision::Fire { vars, .. } => assert_eq!(vars["args"], "hola"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn gift_vars_are_available() {
        let r = rule(Trigger::Gift { gift_id: None, gift_name: None, min_coins: None }, Conditions::default());
        let mut ev = sample_event("g");
        ev.kind = EventType::Gift;
        ev.gift = Some(Gift { id: 1, name: "Rose".into(), coins: 5, count: 5, streakable: true, image: String::new() });
        match fire(&r, &ev, &mut RuleState::default(), &env(0)) {
            Decision::Fire { vars, .. } => {
                assert_eq!(vars["gift"], "Rose");
                assert_eq!(vars["coins"], "5");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn system_triggers_skip_user_conditions() {
        let c = Conditions { roles_any: vec![Role::Moderator], user_cooldown_ms: 1000, ..Default::default() };
        let r = rule(Trigger::GoalReached { goal_id: "g".into() }, c);
        let mut st = RuleState::default();
        let d = evaluate(&r, &TriggerInput::GoalReached("g"), &mut st, &env(0));
        match d {
            Decision::Fire { vars, .. } => assert_eq!(vars["goal"], "g"),
            other => panic!("{other:?}"),
        }
    }

    // ---- Coste en puntos (recompensas) ----

    fn costly(cost: u64, conditions: Conditions) -> Rule {
        Rule { cost_points: Some(cost), ..rule(Trigger::Follow, conditions) }
    }

    fn with_balance(b: Option<u64>) -> EvalEnv {
        EvalEnv { balance: b, ..env(0) }
    }

    #[test]
    fn a_costly_rule_needs_enough_points() {
        let r = costly(100, Conditions::default());
        let ev = follow().0;
        assert_eq!(fire(&r, &ev, &mut RuleState::default(), &with_balance(None)), Decision::Blocked(Blocked::Points));
        assert_eq!(fire(&r, &ev, &mut RuleState::default(), &with_balance(Some(99))), Decision::Blocked(Blocked::Points));
        assert!(matches!(fire(&r, &ev, &mut RuleState::default(), &with_balance(Some(100))), Decision::Fire { .. }));
        assert!(matches!(fire(&r, &ev, &mut RuleState::default(), &with_balance(Some(5_000))), Decision::Fire { .. }));
    }

    #[test]
    fn rules_without_cost_ignore_the_balance() {
        let r = rule(Trigger::Follow, Conditions::default());
        assert!(matches!(fire(&r, &follow().0, &mut RuleState::default(), &with_balance(None)), Decision::Fire { .. }));
        assert!(matches!(fire(&r, &follow().0, &mut RuleState::default(), &with_balance(Some(0))), Decision::Fire { .. }));
    }

    #[test]
    fn being_unable_to_pay_does_not_consume_cooldowns() {
        let c = Conditions { global_cooldown_ms: 10_000, user_cooldown_ms: 10_000, ..Default::default() };
        let r = costly(50, c);
        let (ev, mut st) = (follow().0, RuleState::default());
        assert_eq!(fire(&r, &ev, &mut st, &with_balance(Some(10))), Decision::Blocked(Blocked::Points));
        // Puede reintentar de inmediato al tener saldo: no se gastó ningún turno.
        assert!(matches!(fire(&r, &ev, &mut st, &with_balance(Some(60))), Decision::Fire { .. }));
    }

    #[test]
    fn an_active_cooldown_wins_over_the_points_check() {
        // Así quien insiste en pleno cooldown no recibe además un «no tienes puntos».
        let r = costly(50, Conditions { user_cooldown_ms: 10_000, ..Default::default() });
        let (ev, mut st) = (follow().0, RuleState::default());
        assert!(matches!(fire(&r, &ev, &mut st, &with_balance(Some(100))), Decision::Fire { .. }));
        assert_eq!(fire(&r, &ev, &mut st, &with_balance(Some(0))), Decision::Blocked(Blocked::UserCooldown));
    }

    #[test]
    fn internal_triggers_have_nobody_to_charge() {
        let r = Rule { cost_points: Some(50), trigger: Trigger::GoalReached { goal_id: "g".into() }, ..rule(Trigger::Follow, Conditions::default()) };
        assert!(matches!(evaluate(&r, &TriggerInput::GoalReached("g"), &mut RuleState::default(), &with_balance(None)), Decision::Fire { .. }));
    }

    #[test]
    fn prune_drops_expired_user_cooldowns() {
        let mut st = RuleState::default();
        st.last_by_user.insert("a".into(), 0);
        st.last_by_user.insert("b".into(), 900);
        st.prune(1000, 500);
        assert_eq!(st.tracked_users(), 1);
    }

    // ---- Horario ----

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, mo, d).expect("fecha").and_hms_opt(h, mi, 0).expect("hora")
    }

    fn sched(days: &[u8], from: &str, to: &str) -> Schedule {
        Schedule { days: days.to_vec(), from: from.into(), to: to.into() }
    }

    #[test]
    fn parse_hhmm_validates() {
        assert_eq!(parse_hhmm("00:00"), Some(0));
        assert_eq!(parse_hhmm("23:59"), Some(1439));
        assert_eq!(parse_hhmm(" 9:05 "), Some(545));
        for bad in ["24:00", "12:60", "12", "ab:cd", "", "12:3:4"] {
            assert_eq!(parse_hhmm(bad), None, "{bad}");
        }
    }

    #[test]
    fn daytime_window() {
        let s = sched(&[], "09:00", "17:00");
        assert!(schedule_allows(&s, at(2026, 10, 8, 9, 0)));
        assert!(schedule_allows(&s, at(2026, 10, 8, 16, 59)));
        assert!(!schedule_allows(&s, at(2026, 10, 8, 17, 0)));
        assert!(!schedule_allows(&s, at(2026, 10, 8, 8, 59)));
    }

    #[test]
    fn day_filter_uses_monday_zero() {
        // 2026-10-08 es jueves (3); 2026-10-05 es lunes (0).
        let s = sched(&[0], "00:00", "23:59");
        assert!(schedule_allows(&s, at(2026, 10, 5, 12, 0)));
        assert!(!schedule_allows(&s, at(2026, 10, 8, 12, 0)));
    }

    #[test]
    fn window_crossing_midnight_belongs_to_the_starting_day() {
        // Viernes (4) de 22:00 a 03:00.
        let s = sched(&[4], "22:00", "03:00");
        assert!(schedule_allows(&s, at(2026, 10, 9, 23, 0))); // viernes noche
        assert!(schedule_allows(&s, at(2026, 10, 10, 2, 0))); // sábado madrugada
        assert!(!schedule_allows(&s, at(2026, 10, 9, 2, 0))); // viernes madrugada = jueves noche
        assert!(!schedule_allows(&s, at(2026, 10, 10, 23, 0))); // sábado noche
        assert!(!schedule_allows(&s, at(2026, 10, 9, 12, 0)));
    }

    #[test]
    fn equal_bounds_mean_all_day_and_bad_times_fail_closed() {
        assert!(schedule_allows(&sched(&[], "10:00", "10:00"), at(2026, 10, 8, 3, 0)));
        assert!(!schedule_allows(&sched(&[], "ab", "10:00"), at(2026, 10, 8, 3, 0)));
    }

    #[test]
    fn schedule_blocks_the_rule() {
        let c = Conditions { schedule: Some(sched(&[], "20:00", "23:00")), ..Default::default() };
        let r = rule(Trigger::Follow, c);
        assert_eq!(fire(&r, &follow().0, &mut RuleState::default(), &env(0)), Decision::Blocked(Blocked::Schedule));
    }
}
