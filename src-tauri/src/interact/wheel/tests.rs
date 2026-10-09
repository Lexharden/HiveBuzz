use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::{Map, Value};

use super::*;
use crate::actions::clock::{AppClock, Clock};
use crate::actions::queue::QueueConfig;
use crate::actions::store::MemoryJobStore;
use crate::actions::{ActionContext, ActionExecutor, ExecutorRegistry};
use crate::bot::outbox::{ChatSender, Outbox, OutboxLimits};
use crate::points::service::PointsService;
use crate::rules::model::{PlanMode, Step};

fn seg(id: &str, weight: u32) -> Segment {
    Segment { id: id.into(), label: format!("Premio {id}"), weight, color: "#ff0000".into(), plan: empty_plan() }
}

#[test]
fn pick_follows_the_weights_exactly_at_the_boundaries() {
    let s = [seg("a", 1), seg("b", 3), seg("c", 0), seg("d", 6)];
    // total 10: a=[0,.1) b=[.1,.4) c=∅ d=[.4,1)
    for (roll, expected) in [(0.0, 0), (0.099, 0), (0.1, 1), (0.399, 1), (0.4, 3), (0.999_999, 3)] {
        assert_eq!(pick(&s, roll), Some(expected), "roll={roll}");
    }
}

#[test]
fn zero_weight_prizes_never_win_and_empty_wheels_give_nothing() {
    let s = [seg("a", 0), seg("b", 5)];
    for i in 0..100 {
        assert_eq!(pick(&s, f64::from(i) / 100.0), Some(1));
    }
    assert_eq!(pick(&[], 0.5), None);
    assert_eq!(pick(&[seg("a", 0)], 0.5), None);
    assert_eq!(pick(&s, 5.0), Some(1), "rolls fuera de rango se acotan");
    assert_eq!(pick(&s, -1.0), Some(1));
    assert_eq!(pick(&s, f64::NAN), Some(1));
}

#[test]
fn the_distribution_converges_to_the_weights() {
    let s = [seg("a", 1), seg("b", 2), seg("c", 7)];
    let mut counts = [0u32; 3];
    for i in 0..10_000 {
        let roll = f64::from(i) / 10_000.0;
        counts[pick(&s, roll).expect("pick")] += 1;
    }
    assert_eq!(counts, [1_000, 2_000, 7_000]);
}

#[test]
fn validation_catches_bad_wheels() {
    let ok = WheelConfig { segments: vec![seg("a", 1), seg("b", 1)], ..Default::default() };
    assert!(ok.validate().is_ok());
    assert!(WheelConfig::default().validate().is_ok(), "sin premios es válido (aún no configurada)");
    let dup = WheelConfig { segments: vec![seg("a", 1), seg("a", 1)], ..Default::default() };
    assert!(dup.validate().is_err());
    let no_id = WheelConfig { segments: vec![seg(" ", 1)], ..Default::default() };
    assert!(no_id.validate().is_err());
    let mut unnamed = seg("a", 1);
    unnamed.label = String::new();
    assert!(WheelConfig { segments: vec![unnamed], ..Default::default() }.validate().is_err());
    assert!(WheelConfig { segments: vec![seg("a", 0)], ..Default::default() }.validate().is_err());
    let many = WheelConfig { segments: (0..25).map(|i| seg(&i.to_string(), 1)).collect(), ..Default::default() };
    assert!(many.validate().is_err());
}

#[test]
fn sanitizing_fixes_what_it_can() {
    let mut s = seg("a", 99_999);
    s.label = format!("  {}  ", "x".repeat(100));
    s.color = "rojo".into();
    let c = WheelConfig { segments: vec![s], spin_ms: 1, announce: String::new() }.sanitized();
    assert_eq!(c.segments[0].label.chars().count(), 60);
    assert_eq!(c.segments[0].weight, MAX_WEIGHT);
    assert_eq!(c.segments[0].color, "#f59e0b");
    assert_eq!(c.spin_ms, 1_000);
}

// ---- Servicio ----

struct Recorder(Arc<Mutex<Vec<String>>>);

#[async_trait]
impl ActionExecutor for Recorder {
    fn kind(&self) -> &'static str {
        "rec"
    }
    async fn execute(&self, ctx: &ActionContext, params: &Map<String, Value>) -> Result<()> {
        let label = ctx.render(params.get("label").and_then(Value::as_str).unwrap_or(""));
        self.0.lock().expect("lock").push(label);
        Ok(())
    }
}

struct Mute;

#[async_trait]
impl ChatSender for Mute {
    async fn send_chat(&self, _text: String) -> Result<()> {
        Ok(())
    }
}

struct Rig {
    wheel: Arc<WheelService>,
    hub: OverlayHub,
    ran: Arc<Mutex<Vec<String>>>,
    bot: Arc<BotService>,
}

fn prize(label: &str) -> ActionPlan {
    ActionPlan {
        mode: PlanMode::Sequence,
        steps: vec![Step { delay_ms: 0, action: crate::rules::model::ActionSpec::new("rec", json!({ "label": label })) }],
    }
}

async fn rig(roll: f64) -> Rig {
    let ran: Arc<Mutex<Vec<String>>> = Arc::default();
    let mut registry = ExecutorRegistry::new();
    registry.register(Arc::new(Recorder(ran.clone())));
    let clock: Arc<dyn Clock> = Arc::new(AppClock::new());
    let queue = Arc::new(ActionQueue::start(registry, Arc::new(MemoryJobStore::default()), clock.clone(), QueueConfig::default()));
    let db = Db::open_memory().await.expect("db");
    let hub = OverlayHub::new(16);
    let points = PointsService::new(db.clone(), clock.clone());
    let outbox = Outbox::start(Arc::new(Mute), clock.clone(), Duration::from_millis(1_000), OutboxLimits::default());
    let bot = BotService::new(db.clone(), clock, points, outbox);
    let wheel = WheelService::with_rand(db, hub.clone(), bot.clone(), Arc::new(move || roll), Duration::from_millis(30));
    wheel.attach_queue(queue);
    Rig { wheel, hub, ran, bot }
}

/// Pone una duración de giro corta saltándose el mínimo de 1 s que impone set_config.
fn fast(r: &Rig, spin_ms: u64) {
    r.wheel.cfg.write().expect("lock").spin_ms = spin_ms;
}

fn config() -> WheelConfig {
    let mut a = seg("a", 1);
    a.plan = prize("ganó A para {nickname}: {prize}");
    let mut b = seg("b", 1);
    b.plan = prize("ganó B");
    WheelConfig { segments: vec![a, b], spin_ms: 2_000, announce: "🎡 {nickname}: {prize}".into() }
}

fn vars() -> Vars {
    [("nickname".to_string(), "Ana".to_string()), ("user".to_string(), "ana".to_string())].into()
}

#[tokio::test]
async fn spinning_publishes_the_result_waits_and_then_runs_the_prize() {
    let r = rig(0.1).await;
    r.wheel.set_config(config()).await.expect("config");
    fast(&r, 200);
    let mut rx = r.hub.subscribe();
    let wheel = r.wheel.clone();
    let task = tokio::spawn(async move { wheel.spin(vars()).await });

    let msg = rx.recv().await.expect("mensaje");
    assert_eq!(msg.channel, CHANNEL);
    assert_eq!(msg.data["kind"], "spin");
    assert_eq!(msg.data["winner"], 0);
    assert_eq!(msg.data["durationMs"], 200);
    assert_eq!(msg.data["user"], "Ana");
    assert_eq!(msg.data["segments"].as_array().map(Vec::len), Some(2));
    assert!(r.ran.lock().expect("lock").is_empty(), "el premio no se ejecuta mientras gira");

    tokio::time::sleep(Duration::from_millis(80)).await;
    assert!(r.ran.lock().expect("lock").is_empty(), "ni a mitad de la animación");
    let res = task.await.expect("join").expect("spin");
    assert_eq!((res.index, res.label.as_str()), (0, "Premio a"));
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(r.ran.lock().expect("lock").as_slice(), ["ganó A para Ana: Premio a"]);
    assert!(r.bot.log().is_empty() || r.bot.log().iter().all(|e| e.source == "ruleta"));
}

#[tokio::test]
async fn the_random_source_decides_the_winner() {
    let r = rig(0.9).await;
    r.wheel.set_config(config()).await.expect("config");
    fast(&r, 10);
    let res = r.wheel.spin(vars()).await.expect("spin");
    assert_eq!(res.label, "Premio b");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(r.ran.lock().expect("lock").as_slice(), ["ganó B"]);
}

#[tokio::test]
async fn an_empty_wheel_refuses_to_spin() {
    let r = rig(0.5).await;
    let err = r.wheel.spin(vars()).await.expect_err("sin premios");
    assert!(err.to_string().contains("no tiene premios"), "{err}");
    assert!(r.hub.retained_for(CHANNEL).is_some() || r.wheel.config().segments.is_empty());
}

#[tokio::test]
async fn saving_the_config_publishes_an_idle_wheel_and_persists_it() {
    let r = rig(0.5).await;
    r.wheel.set_config(config()).await.expect("config");
    let idle = r.hub.retained_for(CHANNEL).expect("retenido");
    assert_eq!(idle.data["kind"], "idle");
    assert_eq!(idle.data["segments"][1]["label"], "Premio b");
    assert!(r.wheel.set_config(WheelConfig { segments: vec![seg("x", 0)], ..Default::default() }).await.is_err());
    assert_eq!(r.wheel.config().segments.len(), 2, "una config inválida no cambia nada");
    // Se recarga de la base de datos.
    r.wheel.cfg.write().expect("lock").segments.clear();
    r.wheel.load().await.expect("load");
    assert_eq!(r.wheel.config().segments.len(), 2);
}

#[tokio::test]
async fn without_an_announcement_template_the_bot_stays_quiet() {
    let r = rig(0.1).await;
    let mut cfg = config();
    cfg.announce = String::new();
    r.wheel.set_config(cfg).await.expect("config");
    fast(&r, 10);
    r.wheel.spin(vars()).await.expect("spin");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(r.bot.log().is_empty());
}
