use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Map, Value};

use super::*;
use crate::actions::clock::AppClock;
use crate::actions::queue::QueueConfig;
use crate::actions::store::MemoryJobStore;
use crate::actions::{ActionContext, ActionExecutor};
use crate::events::testing::sample_event;
use crate::events::{Chat, Gift, Like};
use crate::rules::model::{ActionPlan, ActionSpec, Conditions, PlanMode, Step, Trigger};

type Out = Arc<Mutex<Vec<String>>>;

struct Recorder(Out);

#[async_trait]
impl ActionExecutor for Recorder {
    fn kind(&self) -> &'static str {
        "rec"
    }
    fn validate(&self, params: &Map<String, Value>) -> Result<()> {
        if params.contains_key("label") {
            Ok(())
        } else {
            Err(AppError::Invalid("falta label".into()))
        }
    }
    async fn execute(&self, ctx: &ActionContext, params: &Map<String, Value>) -> Result<()> {
        let label = ctx.render(params.get("label").and_then(Value::as_str).unwrap_or(""));
        self.0.lock().expect("lock").push(label);
        Ok(())
    }
}

struct Rig {
    engine: Arc<RuleEngine>,
    bus: EventBus,
    out: Out,
    db: Db,
}

/// Acción lenta y serial: sirve para mantener ocupada la cola y forzar el descarte.
struct Slow;

#[async_trait]
impl ActionExecutor for Slow {
    fn kind(&self) -> &'static str {
        "slow"
    }
    fn concurrency(&self) -> crate::actions::Concurrency {
        crate::actions::Concurrency::Serial("slow")
    }
    async fn execute(&self, _ctx: &ActionContext, _params: &Map<String, Value>) -> Result<()> {
        tokio::time::sleep(Duration::from_millis(400)).await;
        Ok(())
    }
}

async fn rig() -> Rig {
    rig_with(QueueConfig::default()).await
}

async fn rig_with(cfg: QueueConfig) -> Rig {
    let out: Out = Arc::default();
    let mut registry = ExecutorRegistry::new();
    registry.register(Arc::new(Recorder(out.clone())));
    registry.register(Arc::new(Slow));
    let clock: Arc<dyn Clock> = Arc::new(AppClock::new());
    let queue = Arc::new(ActionQueue::start(
        registry.clone(),
        Arc::new(MemoryJobStore::default()),
        clock.clone(),
        cfg,
    ));
    let db = Db::open_memory().await.expect("db");
    let engine = RuleEngine::new(db.clone(), queue, registry, clock);
    let bus = EventBus::new(64);
    engine.spawn(&bus);
    Rig { engine, bus, out, db }
}

fn rule(id: &str, trigger: Trigger, conditions: Conditions, label: &str) -> Rule {
    Rule {
        id: id.into(),
        name: format!("regla {id}"),
        enabled: true,
        trigger,
        conditions,
        plan: ActionPlan {
            mode: PlanMode::Sequence,
            steps: vec![Step { delay_ms: 0, action: ActionSpec::new("rec", json!({ "label": label })) }],
        },
        priority: None,
        ttl_ms: 60_000,
            cost_points: None,
    }
}

fn follow_ev(id: &str, user_id: &str) -> LiveEvent {
    let mut e = sample_event(id);
    e.kind = EventType::Follow;
    e.chat = None;
    e.user.id = user_id.into();
    e
}

async fn wait_for(out: &Out, n: usize) -> Vec<String> {
    for _ in 0..100 {
        if out.lock().expect("lock").len() >= n {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // Margen para detectar ejecuciones de más.
    tokio::time::sleep(Duration::from_millis(80)).await;
    out.lock().expect("lock").clone()
}

#[tokio::test]
async fn a_matching_event_runs_the_action_with_rendered_variables() {
    let r = rig().await;
    r.engine.upsert(rule("a", Trigger::Follow, Conditions::default(), "Gracias {nickname}")).await.expect("upsert");
    r.bus.publish(follow_ev("1", "u1"));
    assert_eq!(wait_for(&r.out, 1).await, ["Gracias Ana"]);
}

#[tokio::test]
async fn non_matching_events_do_nothing() {
    let r = rig().await;
    r.engine.upsert(rule("a", Trigger::Share, Conditions::default(), "x")).await.expect("upsert");
    r.bus.publish(follow_ev("1", "u1"));
    assert!(wait_for(&r.out, 1).await.is_empty());
}

#[tokio::test]
async fn user_cooldown_applies_per_user_through_the_engine() {
    let r = rig().await;
    let c = Conditions { user_cooldown_ms: 60_000, ..Default::default() };
    r.engine.upsert(rule("a", Trigger::Follow, c, "{user}")).await.expect("upsert");
    let mut bob = follow_ev("3", "u2");
    bob.user.unique_id = "bob".into();
    r.bus.publish(follow_ev("1", "u1"));
    r.bus.publish(follow_ev("2", "u1")); // mismo usuario: bloqueado
    r.bus.publish(bob);
    let mut got = wait_for(&r.out, 2).await;
    got.sort();
    assert_eq!(got, ["ana", "bob"]);
}

#[tokio::test]
async fn command_arguments_are_available_as_variables() {
    let r = rig().await;
    r.engine.upsert(rule("a", Trigger::Command { command: "tts".into() }, Conditions::default(), "dice {args}")).await.expect("upsert");
    let mut ev = sample_event("c1");
    ev.chat = Some(Chat { text: "!tts buenas noches".into(), emotes: None });
    r.bus.publish(ev);
    assert_eq!(wait_for(&r.out, 1).await, ["dice buenas noches"]);
}

#[tokio::test]
async fn gift_rule_sees_gift_variables() {
    let r = rig().await;
    let t = Trigger::Gift { gift_id: None, gift_name: Some("Rose".into()), min_coins: Some(5) };
    r.engine.upsert(rule("a", t, Conditions::default(), "{count}x {gift} = {coins}")).await.expect("upsert");
    let mut ev = sample_event("g");
    ev.kind = EventType::Gift;
    ev.chat = None;
    ev.gift = Some(Gift { id: 1, name: "Rose".into(), coins: 7, count: 7, streakable: true, image: String::new() });
    r.bus.publish(ev);
    assert_eq!(wait_for(&r.out, 1).await, ["7x Rose = 7"]);
}

#[tokio::test]
async fn like_rule_fires_once_per_threshold() {
    let r = rig().await;
    r.engine.upsert(rule("a", Trigger::Like { every: 100 }, Conditions::default(), "meta")).await.expect("upsert");
    for (i, count) in [60u64, 60, 60, 60].into_iter().enumerate() {
        let mut ev = sample_event(&format!("l{i}"));
        ev.kind = EventType::Like;
        ev.chat = None;
        ev.like = Some(Like { count, total: 0 });
        r.bus.publish(ev);
    }
    // 240 likes → cruza 100 y 200.
    assert_eq!(wait_for(&r.out, 2).await.len(), 2);
}

#[tokio::test]
async fn disabling_and_deleting_stop_the_rule() {
    let r = rig().await;
    r.engine.upsert(rule("a", Trigger::Follow, Conditions::default(), "x")).await.expect("upsert");
    r.engine.set_enabled("a", false).await.expect("disable");
    r.bus.publish(follow_ev("1", "u1"));
    assert!(wait_for(&r.out, 1).await.is_empty());
    r.engine.set_enabled("a", true).await.expect("enable");
    r.bus.publish(follow_ev("2", "u9"));
    assert_eq!(wait_for(&r.out, 1).await.len(), 1);
    assert!(r.engine.delete("a").await.expect("delete"));
    assert!(r.engine.list().is_empty());
    assert!(!r.engine.delete("a").await.expect("delete again"));
}

#[tokio::test]
async fn invalid_rules_are_rejected_and_not_saved() {
    let r = rig().await;
    let mut unknown = rule("a", Trigger::Follow, Conditions::default(), "x");
    unknown.plan.steps[0].action.kind = "noexiste".into();
    assert!(r.engine.upsert(unknown).await.is_err());

    let mut bad_params = rule("b", Trigger::Follow, Conditions::default(), "x");
    bad_params.plan.steps[0].action.params.clear();
    assert!(r.engine.upsert(bad_params).await.is_err());

    let mut no_name = rule("c", Trigger::Follow, Conditions::default(), "x");
    no_name.name.clear();
    assert!(r.engine.upsert(no_name).await.is_err());

    assert!(r.engine.list().is_empty());
    assert!(r.db.list_rules().await.expect("list").is_empty());
}

#[tokio::test]
async fn upserting_twice_edits_in_place() {
    let r = rig().await;
    r.engine.upsert(rule("a", Trigger::Follow, Conditions::default(), "uno")).await.expect("upsert");
    r.engine.upsert(rule("a", Trigger::Follow, Conditions::default(), "dos")).await.expect("upsert");
    assert_eq!(r.engine.list().len(), 1);
    r.bus.publish(follow_ev("1", "u1"));
    assert_eq!(wait_for(&r.out, 1).await, ["dos"]);
}

#[tokio::test]
async fn test_button_ignores_conditions_and_cooldowns() {
    let r = rig().await;
    let c = Conditions { probability: 0.0, global_cooldown_ms: 3_600_000, ..Default::default() };
    r.engine.upsert(rule("a", Trigger::Command { command: "hola".into() }, c, "args={args}")).await.expect("upsert");
    assert_eq!(r.engine.test_rule("a").await.expect("test"), Outcome::Queued);
    assert_eq!(r.engine.test_rule("a").await.expect("test"), Outcome::Queued);
    let got = wait_for(&r.out, 2).await;
    assert_eq!(got.len(), 2);
    assert!(got[0].starts_with("args=argumento"), "{got:?}");
    assert!(r.engine.test_rule("nope").await.is_err());
}

#[tokio::test]
async fn internal_triggers_fire_through_fire_system() {
    let r = rig().await;
    r.engine.upsert(rule("a", Trigger::GoalReached { goal_id: "g1".into() }, Conditions::default(), "meta {goal}")).await.expect("upsert");
    r.engine.upsert(rule("b", Trigger::TimerEnded { timer_id: "t1".into() }, Conditions::default(), "timer {timer}")).await.expect("upsert");
    assert_eq!(r.engine.fire_system(&SystemEvent::GoalReached("g1".into())).await, 1);
    assert_eq!(r.engine.fire_system(&SystemEvent::GoalReached("otra".into())).await, 0);
    assert_eq!(r.engine.fire_system(&SystemEvent::TimerEnded("t1".into())).await, 1);
    let mut got = wait_for(&r.out, 2).await;
    got.sort();
    assert_eq!(got, ["meta g1", "timer t1"]);
}

#[tokio::test]
async fn rules_survive_a_restart() {
    let r = rig().await;
    r.engine.upsert(rule("a", Trigger::Follow, Conditions::default(), "x")).await.expect("upsert");
    let fresh = RuleEngine::new(
        r.db.clone(),
        r.engine.queue.clone(),
        r.engine.registry.clone(),
        r.engine.clock.clone(),
    );
    assert_eq!(fresh.load().await.expect("load"), 1);
    assert_eq!(fresh.list()[0].id, "a");
}

#[tokio::test]
async fn firing_is_reported_to_subscribers() {
    let r = rig().await;
    let mut rx = r.engine.subscribe_fired();
    r.engine.upsert(rule("a", Trigger::Follow, Conditions::default(), "x")).await.expect("upsert");
    r.bus.publish(follow_ev("1", "u1"));
    let report = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await.expect("a tiempo").expect("recv");
    assert_eq!(report.rule_id, "a");
    assert!(report.queued);
}

#[test]
fn bigger_gifts_get_higher_priority() {
    let gift = |coins| {
        let mut e = sample_event("g");
        e.kind = EventType::Gift;
        e.gift = Some(Gift { id: 1, name: "x".into(), coins, count: 1, streakable: false, image: String::new() });
        e
    };
    let p = |coins| auto_priority(&TriggerInput::Live(&gift(coins)));
    assert!(p(1) < p(10) && p(10) < p(100) && p(100) < p(1_000) && p(1_000) < p(10_000) && p(10_000) < p(34_999));
    assert_eq!(auto_priority(&TriggerInput::Live(&follow_ev("f", "u"))), 0);
    assert!(auto_priority(&TriggerInput::GoalReached("g")) > 0);
}

// ---- Recompensas con coste en puntos ----

use super::{DeniedReport, PointsGate};
use std::collections::HashMap;

#[derive(Default)]
struct FakeGate {
    balances: Mutex<HashMap<String, u64>>,
    refunds: Mutex<Vec<(String, u64)>>,
}

impl FakeGate {
    fn with(user: &str, points: u64) -> Arc<Self> {
        let g = Arc::new(Self::default());
        g.balances.lock().expect("lock").insert(user.into(), points);
        g
    }
    fn balance_of(&self, user: &str) -> u64 {
        self.balances.lock().expect("lock").get(user).copied().unwrap_or(0)
    }
}

#[async_trait]
impl PointsGate for FakeGate {
    async fn balance(&self, user_id: &str) -> Result<u64> {
        Ok(self.balance_of(user_id))
    }
    async fn spend(&self, user_id: &str, cost: u64, _reward: &str) -> Result<Option<u64>> {
        let mut b = self.balances.lock().expect("lock");
        let have = b.get(user_id).copied().unwrap_or(0);
        if have < cost {
            return Ok(None);
        }
        b.insert(user_id.into(), have - cost);
        Ok(Some(have - cost))
    }
    async fn refund(&self, user_id: &str, amount: u64, _reward: &str) -> Result<()> {
        *self.balances.lock().expect("lock").entry(user_id.into()).or_insert(0) += amount;
        self.refunds.lock().expect("lock").push((user_id.into(), amount));
        Ok(())
    }
}

fn reward(id: &str, command: &str, cost: u64, label: &str) -> Rule {
    Rule { cost_points: Some(cost), ..rule(id, Trigger::Command { command: command.into() }, Conditions::default(), label) }
}

fn say(user_id: &str, text: &str) -> LiveEvent {
    let mut e = chat(text);
    e.id = format!("m-{}", uuid::Uuid::new_v4());
    e.user.id = user_id.into();
    e.user.unique_id = format!("u{user_id}");
    e
}

fn chat(text: &str) -> LiveEvent {
    let mut e = sample_event("c");
    e.chat = Some(Chat { text: text.into(), emotes: None });
    e
}

#[tokio::test]
async fn redeeming_a_reward_charges_points_runs_the_action_and_reports_it() {
    let r = rig().await;
    let gate = FakeGate::with("7", 100);
    r.engine.attach_points(gate.clone());
    r.engine.upsert(reward("a", "sonido", 60, "suena para {nickname}")).await.expect("upsert");
    let mut fired = r.engine.subscribe_fired();

    r.bus.publish(say("7", "!sonido"));
    assert_eq!(wait_for(&r.out, 1).await, ["suena para Ana"]);
    assert_eq!(gate.balance_of("7"), 40);
    let report = tokio::time::timeout(Duration::from_secs(2), fired.recv()).await.expect("a tiempo").expect("recv");
    assert_eq!((report.cost, report.points_left, report.user.as_deref()), (Some(60), Some(40), Some("u7")));
    assert!(report.queued);
}

#[tokio::test]
async fn not_enough_points_means_no_action_no_charge_and_a_denial() {
    let r = rig().await;
    let gate = FakeGate::with("7", 40);
    r.engine.attach_points(gate.clone());
    r.engine.upsert(reward("a", "sonido", 60, "no debe sonar")).await.expect("upsert");
    let mut denied = r.engine.subscribe_denied();

    r.bus.publish(say("7", "!sonido"));
    let d: DeniedReport = tokio::time::timeout(Duration::from_secs(2), denied.recv()).await.expect("a tiempo").expect("recv");
    assert_eq!((d.cost, d.have, d.user.as_str(), d.rule_name.as_str()), (60, 40, "u7", "regla a"));
    assert!(wait_for(&r.out, 1).await.is_empty());
    assert_eq!(gate.balance_of("7"), 40, "no se cobra nada");
}

#[tokio::test]
async fn a_failed_attempt_does_not_burn_the_cooldown() {
    let r = rig().await;
    let gate = FakeGate::with("7", 10);
    r.engine.attach_points(gate.clone());
    let mut rw = reward("a", "sonido", 60, "ok");
    rw.conditions.user_cooldown_ms = 3_600_000;
    r.engine.upsert(rw).await.expect("upsert");

    r.bus.publish(say("7", "!sonido")); // sin saldo
    tokio::time::sleep(Duration::from_millis(100)).await;
    gate.balances.lock().expect("lock").insert("7".into(), 100);
    r.bus.publish(say("7", "!sonido")); // ya puede, y el cooldown sigue intacto
    assert_eq!(wait_for(&r.out, 1).await, ["ok"]);
}

#[tokio::test]
async fn without_a_points_system_costly_rules_never_fire_and_nobody_is_told_off() {
    let r = rig().await;
    r.engine.upsert(reward("a", "sonido", 60, "no")).await.expect("upsert");
    let mut denied = r.engine.subscribe_denied();
    r.bus.publish(say("7", "!sonido"));
    assert!(wait_for(&r.out, 1).await.is_empty());
    assert!(denied.try_recv().is_err());
}

#[tokio::test]
async fn costs_apply_per_user_and_free_rules_are_unaffected() {
    let r = rig().await;
    let gate = FakeGate::with("7", 100);
    r.engine.attach_points(gate.clone());
    r.engine.upsert(reward("a", "caro", 80, "caro")).await.expect("upsert");
    r.engine.upsert(rule("b", Trigger::Command { command: "gratis".into() }, Conditions::default(), "gratis")).await.expect("upsert");
    r.bus.publish(say("8", "!caro")); // otro usuario sin puntos
    r.bus.publish(say("8", "!gratis"));
    r.bus.publish(say("7", "!caro"));
    let mut got = wait_for(&r.out, 2).await;
    got.sort();
    assert_eq!(got, ["caro", "gratis"]);
    assert_eq!(gate.balance_of("7"), 20);
}

#[tokio::test]
async fn points_are_refunded_when_the_queue_drops_the_reward() {
    // Cola de 1 hueco: una acción lenta corriendo + otra de mayor prioridad esperando ⇒ la recompensa se descarta.
    let r = rig_with(QueueConfig { max_pending: 1, ..QueueConfig::default() }).await;
    let gate = FakeGate::with("7", 100);
    r.engine.attach_points(gate.clone());
    let slow = |id: &str, trigger: Trigger, priority: i32| Rule {
        priority: Some(priority),
        plan: ActionPlan { mode: PlanMode::Sequence, steps: vec![Step { delay_ms: 0, action: ActionSpec::new("slow", json!({})) }] },
        ..rule(id, trigger, Conditions::default(), "x")
    };
    r.engine.upsert(slow("running", Trigger::Share, 50)).await.expect("upsert");
    r.engine.upsert(slow("waiting", Trigger::Follow, 100)).await.expect("upsert");
    let mut rw = reward("reward", "sonido", 60, "x");
    rw.plan = ActionPlan { mode: PlanMode::Sequence, steps: vec![Step { delay_ms: 0, action: ActionSpec::new("slow", json!({})) }] };
    rw.priority = Some(0);
    r.engine.upsert(rw).await.expect("upsert");

    let mut share = sample_event("s");
    share.kind = EventType::Share;
    share.chat = None;
    let mut follow = sample_event("f");
    follow.kind = EventType::Follow;
    follow.chat = None;
    r.bus.publish(share);
    tokio::time::sleep(Duration::from_millis(100)).await; // empieza a correr
    r.bus.publish(follow);
    tokio::time::sleep(Duration::from_millis(100)).await; // queda en espera: la cola (1) está llena
    r.bus.publish(say("7", "!sonido"));

    for _ in 0..50 {
        if !gate.refunds.lock().expect("lock").is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(*gate.refunds.lock().expect("lock"), [("7".to_string(), 60)]);
    assert_eq!(gate.balance_of("7"), 100, "los puntos volvieron a su dueño");
}

#[tokio::test]
async fn test_button_never_charges() {
    let r = rig().await;
    let gate = FakeGate::with("7", 0);
    r.engine.attach_points(gate.clone());
    r.engine.upsert(reward("a", "sonido", 60, "probando")).await.expect("upsert");
    assert_eq!(r.engine.test_rule("a").await.expect("test"), Outcome::Queued);
    assert_eq!(wait_for(&r.out, 1).await, ["probando"]);
    assert_eq!(gate.balance_of("7"), 0);
}
#[tokio::test]
async fn api_calls_fire_matching_enabled_rules_with_their_variables() {
    let r = rig().await;
    r.engine.upsert(rule("a", Trigger::Api { name: "Gracias".into() }, Conditions::default(), "hola {user} ({api}/{event})")).await.expect("upsert");
    r.engine.upsert(rule("b", Trigger::Api { name: "otra".into() }, Conditions::default(), "no")).await.expect("upsert");
    let mut off = rule("c", Trigger::Api { name: "gracias".into() }, Conditions::default(), "apagada");
    off.enabled = false;
    r.engine.upsert(off).await.expect("upsert");

    let vars: Vars = [("user".to_string(), "ana".to_string())].into();
    assert_eq!(r.engine.fire_api("gracias", vars).await, 1, "no distingue mayúsculas y salta las apagadas");
    assert_eq!(wait_for(&r.out, 1).await, ["hola ana (gracias/api)"]);
    assert_eq!(r.engine.fire_api("inexistente", Vars::new()).await, 0);
}

#[tokio::test]
async fn api_rules_never_fire_from_live_events_and_validate_the_name() {
    let r = rig().await;
    r.engine.upsert(rule("a", Trigger::Api { name: "x".into() }, Conditions::default(), "no debe")).await.expect("upsert");
    assert!(r.engine.upsert(rule("b", Trigger::Api { name: "con espacio".into() }, Conditions::default(), "x")).await.is_err());
    assert!(r.engine.upsert(rule("c", Trigger::Api { name: String::new() }, Conditions::default(), "x")).await.is_err());
    let mut costly = rule("d", Trigger::Api { name: "pago".into() }, Conditions::default(), "x");
    costly.cost_points = Some(10);
    assert!(r.engine.upsert(costly).await.is_err(), "una recompensa necesita un espectador");
    for ev in [sample_event("e1"), crate::simulator::build_event(crate::simulator::SimKind::Follow)] {
        assert_eq!(r.engine.handle(live(&ev)).await, 0);
    }
    assert!(r.engine.test_rule("a").await.is_ok(), "probar una regla de API usa un evento de prueba");
}
