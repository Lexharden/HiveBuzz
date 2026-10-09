use chrono::NaiveDate;

use super::*;
use crate::bot::model::{BotCommand, KeywordReply};
use crate::events::testing::sample_event;
use crate::events::{Chat, Gift};
use crate::rules::model::Role;

fn local() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 10, 8).expect("fecha").and_hms_opt(12, 0, 0).expect("hora")
}

fn ctx<'a>(now: i64, points: Option<u64>, rand: &'a (dyn Fn() -> f64 + Sync)) -> ReplyCtx<'a> {
    ReplyCtx { points, currency: "puntos", now_ms: now, local: local(), rand }
}

fn zero() -> f64 {
    0.0
}

fn chat(text: &str) -> LiveEvent {
    let mut e = sample_event("c");
    e.chat = Some(Chat { text: text.into(), emotes: None });
    e
}

fn ev(kind: EventType) -> LiveEvent {
    let mut e = sample_event("e");
    e.kind = kind;
    e.chat = None;
    e
}

fn gift(coins: u64, count: u32) -> LiveEvent {
    let mut e = ev(EventType::Gift);
    e.gift = Some(Gift { id: 1, name: "Rose".into(), coins, count, streakable: false, image: String::new() });
    e
}

fn command(id: &str, names: &[&str], responses: &[&str]) -> BotCommand {
    BotCommand {
        id: id.into(),
        enabled: true,
        names: names.iter().map(|s| (*s).to_string()).collect(),
        responses: responses.iter().map(|s| (*s).to_string()).collect(),
        conditions: Conditions::default(),
    }
}

fn cfg_with(commands: Vec<BotCommand>) -> BotConfig {
    BotConfig { enabled: true, commands, ..BotConfig::default() }
}

fn run(engine: &mut BotEngine, cfg: &BotConfig, ev: &LiveEvent, now: i64) -> Vec<Reply> {
    engine.on_event(cfg, ev, &ctx(now, None, &zero))
}

#[test]
fn a_command_and_its_aliases_get_the_reply() {
    let cfg = cfg_with(vec![command("d", &["discord", "ds"], &["Únete: discord.gg/x"])]);
    let mut b = BotEngine::default();
    for text in ["!discord", "!DS", "!discord por favor"] {
        let r = run(&mut b, &cfg, &chat(text), 0);
        assert_eq!(r.len(), 1, "{text}");
        assert_eq!(r[0].text, "Únete: discord.gg/x");
    }
    assert!(run(&mut b, &cfg, &chat("discord"), 0).is_empty(), "sin «!» no es un comando");
    assert!(run(&mut b, &cfg, &chat("!otro"), 0).is_empty());
}

#[test]
fn replies_can_use_variables_and_arguments() {
    let cfg = cfg_with(vec![command("s", &["saludo"], &["¡Hola {nickname} ({user})! Dijiste: {args}"])]);
    let r = run(&mut BotEngine::default(), &cfg, &chat("!saludo buenas tardes"), 0);
    assert_eq!(r[0].text, "¡Hola Ana (ana)! Dijiste: buenas tardes");
    assert_eq!(r[0].source, "comando !saludo");
}

#[test]
fn points_and_currency_variables() {
    let cfg = cfg_with(vec![command("p", &["mispuntos"], &["{user} tiene {points} {currency}"])]);
    let rand = zero;
    let r = BotEngine::default().on_event(&cfg, &chat("!mispuntos"), &ctx(0, Some(1234), &rand));
    assert_eq!(r[0].text, "ana tiene 1234 puntos");
    assert!(uses_points(&cfg));
    assert!(!uses_points(&cfg_with(vec![command("x", &["x"], &["hola"])])));
}

#[test]
fn several_variants_are_chosen_by_the_random_source() {
    let cfg = cfg_with(vec![command("d", &["d"], &["uno", "dos", "tres"])]);
    for (rand_value, expected) in [(0.0, "uno"), (0.34, "dos"), (0.67, "tres"), (0.999, "tres")] {
        let rand = move || rand_value;
        let r = BotEngine::default().on_event(&cfg, &chat("!d"), &ctx(0, None, &rand));
        assert_eq!(r[0].text, expected, "rand={rand_value}");
    }
}

#[test]
fn empty_variants_are_never_picked() {
    let cfg = cfg_with(vec![command("d", &["d"], &["", "  ", "real"])]);
    for v in [0.0, 0.5, 0.99] {
        let rand = move || v;
        assert_eq!(BotEngine::default().on_event(&cfg, &chat("!d"), &ctx(0, None, &rand))[0].text, "real");
    }
}

#[test]
fn cooldowns_apply_per_command_and_per_user() {
    let mut c = command("d", &["d"], &["x"]);
    c.conditions.user_cooldown_ms = 10_000;
    c.conditions.global_cooldown_ms = 2_000;
    let cfg = cfg_with(vec![c]);
    let mut b = BotEngine::default();
    let ana = chat("!d");
    let mut beto = chat("!d");
    beto.user.id = "2".into();
    assert_eq!(run(&mut b, &cfg, &ana, 0).len(), 1);
    assert!(run(&mut b, &cfg, &beto, 1_000).is_empty(), "cooldown global");
    assert_eq!(run(&mut b, &cfg, &beto, 2_000).len(), 1);
    assert!(run(&mut b, &cfg, &ana, 5_000).is_empty(), "cooldown de ana");
    assert_eq!(run(&mut b, &cfg, &ana, 10_000).len(), 1);
}

#[test]
fn commands_can_be_limited_to_roles() {
    let mut c = command("mod", &["silencio"], &["ok"]);
    c.conditions.roles_any = vec![Role::Moderator];
    let cfg = cfg_with(vec![c]);
    let mut b = BotEngine::default();
    assert!(run(&mut b, &cfg, &chat("!silencio"), 0).is_empty());
    let mut m = chat("!silencio");
    m.user.is_moderator = true;
    assert_eq!(run(&mut b, &cfg, &m, 0).len(), 1);
}

#[test]
fn disabled_bot_or_entry_stays_quiet() {
    let mut cfg = cfg_with(vec![command("d", &["d"], &["x"])]);
    cfg.enabled = false;
    assert!(run(&mut BotEngine::default(), &cfg, &chat("!d"), 0).is_empty());
    cfg.enabled = true;
    cfg.commands[0].enabled = false;
    assert!(run(&mut BotEngine::default(), &cfg, &chat("!d"), 0).is_empty());
}

#[test]
fn only_one_command_answers_a_message() {
    let cfg = cfg_with(vec![command("a", &["x"], &["primero"]), command("b", &["x"], &["segundo"])]);
    // (la validación impide repetir nombres; aun así el motor no debe contestar dos veces)
    let r = run(&mut BotEngine::default(), &cfg, &chat("!x"), 0);
    assert_eq!(r.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), ["primero"]);
}

#[test]
fn keyword_replies_work_when_no_command_matched() {
    let mut cfg = cfg_with(vec![command("d", &["d"], &["comando"])]);
    cfg.keyword_replies.push(KeywordReply {
        id: "k".into(),
        enabled: true,
        keywords: vec!["precio".into(), "cuánto cuesta".into()],
        whole_word: true,
        responses: vec!["Mira !tienda".into()],
        conditions: Conditions::default(),
    });
    let mut b = BotEngine::default();
    let r = run(&mut b, &cfg, &chat("¿Cuál es el PRECIO?"), 0);
    assert_eq!((r[0].text.as_str(), r[0].source.as_str()), ("Mira !tienda", "palabra clave"));
    assert!(run(&mut b, &cfg, &chat("preciosa vista"), 0).is_empty(), "palabra completa");
    // Un comando gana a la palabra clave y no se responde dos veces.
    let both = run(&mut b, &cfg, &chat("!d precio"), 0);
    assert_eq!(both.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), ["comando"]);
}

#[test]
fn probability_is_honoured() {
    let mut c = command("d", &["d"], &["x"]);
    c.conditions.probability = 30.0;
    let cfg = cfg_with(vec![c]);
    let lucky = || 0.1;
    let unlucky = || 0.9;
    assert_eq!(BotEngine::default().on_event(&cfg, &chat("!d"), &ctx(0, None, &lucky)).len(), 1);
    assert!(BotEngine::default().on_event(&cfg, &chat("!d"), &ctx(0, None, &unlucky)).is_empty());
}

// ---- Agradecimientos ----

fn thanks_cfg() -> BotConfig {
    let mut c = BotConfig { enabled: true, ..BotConfig::default() };
    c.thanks.gift.enabled = true;
    c.thanks.gift.min_coins = 10;
    c.thanks.gift.user_cooldown_ms = 5_000;
    c.thanks.follow.enabled = true;
    c
}

#[test]
fn gifts_are_thanked_above_the_minimum_with_their_variables() {
    let cfg = thanks_cfg();
    let mut b = BotEngine::default();
    assert!(run(&mut b, &cfg, &gift(5, 1), 0).is_empty(), "por debajo del mínimo");
    let r = run(&mut b, &cfg, &gift(30, 3), 1);
    assert_eq!(r[0].text, "¡Gracias Ana por 3× Rose! 🎁");
    assert_eq!(r[0].source, "gracias (regalo)");
}

#[test]
fn thanks_are_rate_limited_per_user() {
    let cfg = thanks_cfg();
    let mut b = BotEngine::default();
    assert_eq!(run(&mut b, &cfg, &gift(30, 1), 0).len(), 1);
    assert!(run(&mut b, &cfg, &gift(30, 1), 4_999).is_empty());
    assert_eq!(run(&mut b, &cfg, &gift(30, 1), 5_000).len(), 1);
    let mut other = gift(30, 1);
    other.user.id = "9".into();
    assert_eq!(run(&mut b, &cfg, &other, 5_001).len(), 1, "otro usuario sí");
}

#[test]
fn follows_are_thanked_only_if_enabled_and_shares_stay_quiet_by_default() {
    let cfg = thanks_cfg();
    let mut b = BotEngine::default();
    assert_eq!(run(&mut b, &cfg, &ev(EventType::Follow), 0)[0].text, "¡Gracias por seguirme, Ana! 💛");
    assert!(run(&mut b, &cfg, &ev(EventType::Share), 0).is_empty());
    assert!(run(&mut b, &cfg, &ev(EventType::Subscribe), 0).is_empty());
}

#[test]
fn an_active_thanks_with_an_empty_template_does_nothing() {
    let mut cfg = thanks_cfg();
    cfg.thanks.follow.template = "  ".into();
    assert!(run(&mut BotEngine::default(), &cfg, &ev(EventType::Follow), 0).is_empty());
}

#[test]
fn other_events_never_trigger_replies() {
    let cfg = thanks_cfg();
    for k in [EventType::Join, EventType::Like, EventType::Emote, EventType::LiveEnd] {
        assert!(run(&mut BotEngine::default(), &cfg, &ev(k), 0).is_empty(), "{k:?}");
    }
}

// ---- Límites y utilidades ----

#[test]
fn replies_are_trimmed_to_the_chat_limit_without_cutting_emoji() {
    let long = "🔥".repeat(300);
    let cfg = cfg_with(vec![command("d", &["d"], &[&long])]);
    let r = run(&mut BotEngine::default(), &cfg, &chat("!d"), 0);
    assert_eq!(r[0].text.chars().count(), MAX_CHAT_CHARS);
    assert_eq!(fit("  hola  "), "  hola", "solo se recorta el final");
}

#[test]
fn builtin_commands_have_their_own_short_cooldown() {
    let mut b = BotEngine::default();
    let rand = zero;
    let c = |now| ctx(now, None, &rand);
    let e = chat("!puntos");
    assert!(b.allow_builtin("builtin:points", "puntos", &e, &c(0)));
    assert!(!b.allow_builtin("builtin:points", "puntos", &e, &c(9_000)), "10 s por usuario");
    assert!(b.allow_builtin("builtin:points", "puntos", &e, &c(10_000)));
    assert!(!b.allow_builtin("builtin:points", "puntos", &chat("hola"), &c(20_000)), "no es el comando");
}

#[test]
fn render_with_fills_currency() {
    let b = BotEngine::default();
    let vars: Vars = [("user".to_string(), "ana".to_string())].into();
    assert_eq!(b.render_with("@{user}: {currency}", vars, "abejas"), "@ana: abejas");
}
