use std::sync::atomic::AtomicI64;

use async_trait::async_trait;

use super::*;
use crate::events::testing::sample_event;
use crate::events::{EventType, Gift};
use crate::timers::{Extension, ExtensionSource};

/// Reloj controlado a mano: los tests deciden cuándo «pasa el tiempo».
struct ManualClock(AtomicI64);

impl ManualClock {
    fn advance(&self, ms: i64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

#[derive(Default)]
struct RecordingSink(Mutex<Vec<SystemEvent>>);

#[async_trait]
impl SystemSink for RecordingSink {
    async fn fire(&self, ev: SystemEvent) {
        self.0.lock().expect("lock").push(ev);
    }
}

struct Rig {
    svc: Arc<TimerService>,
    clock: Arc<ManualClock>,
    hub: OverlayHub,
    sink: Arc<RecordingSink>,
    bus: EventBus,
    db: Db,
}

const FAST: TimerTiming = TimerTiming { tick_every: Duration::from_millis(15), save_every: Duration::from_millis(60) };
const T0: i64 = 1_000_000;

async fn rig() -> Rig {
    let db = Db::open_memory().await.expect("db");
    let hub = OverlayHub::new(32);
    let clock = Arc::new(ManualClock(AtomicI64::new(T0)));
    let sink = Arc::new(RecordingSink::default());
    let svc = TimerService::new(db.clone(), hub.clone(), clock.clone());
    svc.attach_sink(sink.clone());
    let bus = EventBus::new(64);
    svc.spawn(&bus, FAST);
    Rig { svc, clock, hub, sink, bus, db }
}

fn cfg(id: &str, start: u64, exts: Vec<Extension>) -> TimerConfig {
    TimerConfig { id: id.into(), name: format!("Timer {id}"), start_seconds: start, max_seconds: None, extensions: exts }
}

fn ext(source: ExtensionSource, seconds: u64) -> Extension {
    Extension { source, seconds }
}

fn view(r: &Rig, id: &str) -> TimerView {
    r.svc.list().into_iter().find(|t| t.config.id == id).expect("timer")
}

fn fired(r: &Rig) -> Vec<SystemEvent> {
    r.sink.0.lock().expect("lock").clone()
}

async fn eventually(mut cond: impl FnMut() -> bool) {
    for _ in 0..100 {
        if cond() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    panic!("la condición no se cumplió a tiempo");
}

fn follow() -> LiveEvent {
    let mut e = sample_event("f");
    e.kind = EventType::Follow;
    e.chat = None;
    e
}

fn gift(coins: u64) -> LiveEvent {
    let mut e = sample_event("g");
    e.kind = EventType::Gift;
    e.chat = None;
    e.gift = Some(Gift { id: 1, name: "Rose".into(), coins, count: 1, streakable: false, image: String::new() });
    e
}

#[tokio::test]
async fn a_new_timer_is_idle_with_its_full_duration() {
    let r = rig().await;
    r.svc.upsert(cfg("t", 600, vec![])).await.expect("upsert");
    let v = view(&r, "t");
    assert_eq!((v.status, v.remaining_ms), (Status::Idle, 600_000));
    let snap = r.hub.retained_for("timer").expect("retenido").data.clone();
    assert_eq!(snap["timers"][0]["status"], "idle");
    assert_eq!(snap["timers"][0]["endsAtMs"], 0);
}

#[tokio::test]
async fn running_timer_counts_down_and_ends_once_firing_the_rule_event() {
    let r = rig().await;
    r.svc.upsert(cfg("t", 60, vec![])).await.expect("upsert");
    r.svc.control("t", Control::Start).await.expect("start");
    assert_eq!(view(&r, "t").remaining_ms, 60_000);
    let snap = r.hub.retained_for("timer").expect("ret").data.clone();
    assert_eq!(snap["timers"][0]["endsAtMs"], T0 + 60_000);

    r.clock.advance(30_000);
    assert_eq!(view(&r, "t").remaining_ms, 30_000);
    r.clock.advance(31_000);
    eventually(|| view(&r, "t").status == Status::Ended).await;
    eventually(|| !fired(&r).is_empty()).await;
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(fired(&r), [SystemEvent::TimerEnded("t".into())], "solo una vez");
    eventually(|| r.hub.retained_for("timer").is_some_and(|m| m.data["timers"][0]["status"] == "ended")).await;
}

#[tokio::test]
async fn events_extend_a_running_timer_and_the_overlay_is_updated() {
    let r = rig().await;
    let exts = vec![ext(ExtensionSource::Follow, 30), ext(ExtensionSource::Coins { per_coins: 100 }, 60)];
    r.svc.upsert(cfg("t", 600, exts)).await.expect("upsert");
    r.svc.control("t", Control::Start).await.expect("start");
    r.bus.publish(follow());
    r.bus.publish(gift(250));
    eventually(|| view(&r, "t").remaining_ms == 600_000 + 30_000 + 120_000).await;
    eventually(|| r.hub.retained_for("timer").is_some_and(|m| m.data["timers"][0]["remainingMs"] == 750_000)).await;
}

#[tokio::test]
async fn the_cap_limits_extensions() {
    let r = rig().await;
    let mut c = cfg("t", 600, vec![ext(ExtensionSource::Follow, 400)]);
    c.max_seconds = Some(800);
    r.svc.upsert(c).await.expect("upsert");
    r.svc.control("t", Control::Start).await.expect("start");
    r.bus.publish(follow());
    r.bus.publish(follow());
    eventually(|| view(&r, "t").remaining_ms == 800_000).await;
}

#[tokio::test]
async fn idle_and_ended_timers_ignore_events_but_paused_ones_extend() {
    let r = rig().await;
    r.svc.upsert(cfg("t", 600, vec![ext(ExtensionSource::Follow, 30)])).await.expect("upsert");
    r.bus.publish(follow());
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(view(&r, "t").remaining_ms, 600_000, "parado: no se extiende");

    r.svc.control("t", Control::Start).await.expect("start");
    r.svc.control("t", Control::Pause).await.expect("pause");
    r.bus.publish(follow());
    eventually(|| view(&r, "t").remaining_ms == 630_000).await;
    assert_eq!(view(&r, "t").status, Status::Paused);
}

#[tokio::test]
async fn pause_resume_reset_and_manual_adjustments() {
    let r = rig().await;
    r.svc.upsert(cfg("t", 100, vec![])).await.expect("upsert");
    r.svc.control("t", Control::Start).await.expect("start");
    r.clock.advance(40_000);
    r.svc.control("t", Control::Pause).await.expect("pause");
    r.clock.advance(500_000);
    assert_eq!(view(&r, "t").remaining_ms, 60_000, "en pausa no corre");
    r.svc.control("t", Control::Resume).await.expect("resume");
    r.clock.advance(10_000);
    assert_eq!(view(&r, "t").remaining_ms, 50_000);
    r.svc.control("t", Control::AddSeconds(25)).await.expect("add");
    assert_eq!(view(&r, "t").remaining_ms, 75_000);
    r.svc.control("t", Control::AddSeconds(-5)).await.expect("sub");
    assert_eq!(view(&r, "t").remaining_ms, 70_000);
    r.svc.control("t", Control::Reset).await.expect("reset");
    let v = view(&r, "t");
    assert_eq!((v.status, v.remaining_ms), (Status::Idle, 100_000));
    assert!(r.svc.control("nada", Control::Start).await.is_err());
}

#[tokio::test]
async fn editing_keeps_the_state_and_idle_timers_take_the_new_duration() {
    let r = rig().await;
    r.svc.upsert(cfg("t", 100, vec![])).await.expect("upsert");
    r.svc.upsert(cfg("t", 200, vec![])).await.expect("upsert");
    assert_eq!(view(&r, "t").remaining_ms, 200_000);
    r.svc.control("t", Control::Start).await.expect("start");
    r.clock.advance(50_000);
    r.svc.upsert(cfg("t", 999, vec![])).await.expect("upsert");
    let v = view(&r, "t");
    assert_eq!((v.status, v.remaining_ms), (Status::Running, 150_000), "no se reinicia al editar");
    assert!(r.svc.upsert(cfg("x", 0, vec![])).await.is_err());
}

#[tokio::test]
async fn running_timers_survive_a_restart_and_expired_ones_end_silently() {
    let r = rig().await;
    r.svc.upsert(cfg("long", 3600, vec![])).await.expect("upsert");
    r.svc.upsert(cfg("short", 60, vec![])).await.expect("upsert");
    r.svc.control("long", Control::Start).await.expect("start");
    r.svc.control("short", Control::Start).await.expect("start");

    // «La app se cierra» y se reabre 5 minutos después.
    r.clock.advance(300_000);
    let hub = OverlayHub::new(8);
    let sink = Arc::new(RecordingSink::default());
    let fresh = TimerService::new(r.db.clone(), hub.clone(), r.clock.clone());
    fresh.attach_sink(sink.clone());
    assert_eq!(fresh.load().await.expect("load"), 2);
    let by_id = |id: &str| fresh.list().into_iter().find(|t| t.config.id == id).expect("timer");
    assert_eq!((by_id("long").status, by_id("long").remaining_ms), (Status::Running, 3_300_000));
    assert_eq!(by_id("short").status, Status::Ended);
    assert!(sink.0.lock().expect("lock").is_empty(), "no se celebra un fin que ocurrió con la app cerrada");
    assert!(hub.retained_for("timer").is_some());
}

#[tokio::test]
async fn extension_progress_is_flushed_to_storage() {
    let r = rig().await;
    r.svc.upsert(cfg("t", 600, vec![ext(ExtensionSource::Coins { per_coins: 100 }, 60)])).await.expect("upsert");
    r.svc.control("t", Control::Start).await.expect("start");
    r.bus.publish(gift(130));
    eventually(|| view(&r, "t").remaining_ms == 660_000).await;
    r.svc.flush().await;
    let saved = r.db.list_timers().await.expect("list");
    assert_eq!(saved[0].state.leftovers, [30], "también se guarda el sobrante de monedas");
    assert_eq!(saved[0].state.ends_at_ms, T0 + 660_000);
}

#[tokio::test]
async fn deleting_a_timer_removes_it_everywhere() {
    let r = rig().await;
    r.svc.upsert(cfg("t", 60, vec![])).await.expect("upsert");
    assert!(r.svc.delete("t").await.expect("delete"));
    assert!(!r.svc.delete("t").await.expect("delete"));
    assert!(r.db.list_timers().await.expect("list").is_empty());
    assert_eq!(r.hub.retained_for("timer").expect("ret").data["timers"].as_array().expect("arr").len(), 0);
}
