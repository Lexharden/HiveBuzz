use serde_json::{json, Map};

use super::*;
use crate::events::testing::sample_event;
use crate::events::Gift;

fn gift_ev(coins: u64) -> LiveEvent {
    let mut e = sample_event("g");
    e.kind = EventType::Gift;
    e.chat = None;
    e.gift = Some(Gift { id: 1, name: "Rose".into(), coins, count: 1, streakable: false, image: String::new() });
    e
}

struct Rig {
    svc: Arc<JarService>,
    hub: OverlayHub,
    bus: EventBus,
    cfg: OverlayConfigService,
    db: Db,
}

async fn rig() -> Rig {
    let db = Db::open_memory().await.expect("db");
    let hub = OverlayHub::new(32);
    let cfg = OverlayConfigService::new(db.clone(), hub.clone());
    let svc = JarService::new(db.clone(), hub.clone(), cfg.clone());
    let bus = EventBus::new(32);
    svc.spawn(&bus, Duration::from_millis(30));
    Rig { svc, hub, bus, cfg, db }
}

fn patch(v: Value) -> Map<String, Value> {
    v.as_object().cloned().expect("objeto")
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

#[test]
fn keep_stays_full_and_keeps_counting() {
    let mut s = JarState::default();
    assert_eq!(apply(&mut s, 150, 100, OnFull::Keep), 0);
    assert_eq!((s.coins, s.laps), (150, 0));
}

#[test]
fn reset_empties_at_the_target_and_keeps_the_remainder() {
    let mut s = JarState::default();
    assert_eq!(apply(&mut s, 250, 100, OnFull::Reset), 2);
    assert_eq!((s.coins, s.laps), (50, 2));
}

#[test]
fn reset_without_a_target_never_loops() {
    let mut s = JarState::default();
    assert_eq!(apply(&mut s, 10, 0, OnFull::Reset), 0);
    assert_eq!(s.coins, 10);
}

#[test]
fn subtracting_never_goes_below_zero() {
    let mut s = JarState { coins: 30, laps: 0 };
    apply(&mut s, -100, 100, OnFull::Keep);
    assert_eq!(s.coins, 0);
}

#[tokio::test]
async fn gifts_from_the_bus_fill_the_jar_without_any_goal() {
    let r = rig().await;
    r.bus.publish(gift_ev(40));
    r.bus.publish(gift_ev(60));
    eventually(|| r.svc.state().coins == 100).await;
    assert_eq!(r.hub.retained_for(CHANNEL).expect("retenido").data["coins"], 100);
}

#[tokio::test]
async fn small_gifts_and_other_events_are_ignored() {
    let r = rig().await;
    r.cfg.set("jar", &patch(json!({ "minCoins": 10 }))).await.expect("cfg");
    let mut chat = sample_event("c");
    chat.kind = EventType::Chat;
    r.bus.publish(chat);
    r.bus.publish(gift_ev(5));
    r.bus.publish(gift_ev(20));
    eventually(|| r.svc.state().coins == 20).await;
}

#[tokio::test]
async fn on_full_reset_uses_the_overlay_target() {
    let r = rig().await;
    r.cfg.set("jar", &patch(json!({ "target": 100, "onFull": "reset" }))).await.expect("cfg");
    r.svc.add(130).await;
    assert_eq!((r.svc.state().coins, r.svc.state().laps), (30, 1));
}

#[tokio::test]
async fn set_and_reset_publish_and_survive_a_restart() {
    let r = rig().await;
    r.svc.set(70);
    assert_eq!(r.hub.retained_for(CHANNEL).expect("ret").data["coins"], 70);
    r.svc.flush().await;
    let again = JarService::new(r.db.clone(), r.hub.clone(), r.cfg.clone());
    again.load().await.expect("load");
    assert_eq!(again.state().coins, 70);
    again.reset();
    assert_eq!(again.state(), JarState::default());
}

#[tokio::test]
async fn an_unreadable_saved_state_starts_empty() {
    let r = rig().await;
    r.db.set_setting(KEY_STATE, "{roto").await.expect("raw");
    r.svc.load().await.expect("load");
    assert_eq!(r.svc.state(), JarState::default());
}
