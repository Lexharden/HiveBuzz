use async_trait::async_trait;
use tokio::sync::broadcast;

use super::*;
use crate::events::testing::sample_event;
use crate::events::{EventType, Like};
use crate::goals::{GoalKind, OnReach};
use crate::session::SessionService;

#[derive(Default)]
struct RecordingSink(Mutex<Vec<SystemEvent>>);

#[async_trait]
impl SystemSink for RecordingSink {
    async fn fire(&self, ev: SystemEvent) {
        self.0.lock().expect("lock").push(ev);
    }
}

struct Rig {
    svc: Arc<GoalService>,
    hub: OverlayHub,
    sink: Arc<RecordingSink>,
    bus: EventBus,
    db: Db,
    sessions: SessionService,
}

const FAST: GoalTiming = GoalTiming { publish_every: Duration::from_millis(15), save_every: Duration::from_millis(60) };

async fn rig() -> Rig {
    let db = Db::open_memory().await.expect("db");
    let hub = OverlayHub::new(32);
    let sink = Arc::new(RecordingSink::default());
    let svc = GoalService::new(db.clone(), hub.clone());
    svc.attach_sink(sink.clone());
    let bus = EventBus::new(64);
    let sessions = SessionService::new();
    svc.spawn(&bus, sessions.subscribe(), FAST);
    Rig { svc, hub, sink, bus, db, sessions }
}

fn goal(id: &str, kind: GoalKind, target: u64) -> Goal {
    Goal { id: id.into(), name: format!("Meta {id}"), kind, target, current: 0, on_reach: OnReach::Stop, reset_on_session: false, reached_count: 0 }
}

fn like_ev(id: &str, count: u64) -> LiveEvent {
    let mut e = sample_event(id);
    e.kind = EventType::Like;
    e.chat = None;
    e.like = Some(Like { count, total: 0 });
    e
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

fn current(r: &Rig, id: &str) -> u64 {
    r.svc.list().iter().find(|g| g.id == id).map_or(u64::MAX, |g| g.current)
}

#[tokio::test]
async fn bus_events_advance_matching_goals_and_publish_the_state() {
    let r = rig().await;
    r.svc.upsert(goal("likes", GoalKind::Likes, 100)).await.expect("upsert");
    r.svc.upsert(goal("follows", GoalKind::Follows, 10)).await.expect("upsert");
    r.bus.publish(like_ev("l1", 30));
    r.bus.publish(like_ev("l2", 25));
    eventually(|| current(&r, "likes") == 55).await;
    assert_eq!(current(&r, "follows"), 0);

    eventually(|| r.hub.retained_for("goals").is_some_and(|m| m.data["goals"][0]["current"] == 55)).await;
    let snap = r.hub.retained_for("goals").expect("retenida").data.clone();
    assert_eq!(snap["goals"][0]["kind"], "likes");
    assert_eq!(snap["goals"][0]["percent"], 55.0);
    assert_eq!(snap["goals"][1]["id"], "follows");
}

#[tokio::test]
async fn reaching_a_goal_fires_the_rule_event_exactly_once_per_crossing() {
    let r = rig().await;
    r.svc.upsert(goal("likes", GoalKind::Likes, 100)).await.expect("upsert");
    r.bus.publish(like_ev("a", 60));
    r.bus.publish(like_ev("b", 60));
    r.bus.publish(like_ev("c", 60)); // ya completa: no vuelve a avisar
    eventually(|| current(&r, "likes") == 180).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(fired(&r), [SystemEvent::GoalReached("likes".into())]);
}

#[tokio::test]
async fn reset_goals_fire_once_per_lap() {
    let r = rig().await;
    let mut g = goal("coins", GoalKind::Likes, 10);
    g.on_reach = OnReach::Reset;
    r.svc.upsert(g).await.expect("upsert");
    r.bus.publish(like_ev("a", 35));
    eventually(|| fired(&r).len() == 3).await;
    assert_eq!(current(&r, "coins"), 5);
}

#[tokio::test]
async fn upserting_an_existing_goal_keeps_its_live_progress() {
    let r = rig().await;
    r.svc.upsert(goal("likes", GoalKind::Likes, 100)).await.expect("upsert");
    r.svc.adjust("likes", 70).await.expect("adjust");
    // La UI guarda una copia vieja (progreso 0) con otro objetivo.
    let mut edited = goal("likes", GoalKind::Likes, 500);
    edited.name = "Editada".into();
    r.svc.upsert(edited).await.expect("upsert");
    let g = r.svc.list().remove(0);
    assert_eq!((g.current, g.target, g.name.as_str()), (70, 500, "Editada"));
}

#[tokio::test]
async fn invalid_goals_are_rejected() {
    let r = rig().await;
    assert!(r.svc.upsert(goal("x", GoalKind::Likes, 0)).await.is_err());
    assert!(r.svc.list().is_empty());
}

#[tokio::test]
async fn manual_adjust_set_and_reset() {
    let r = rig().await;
    r.svc.upsert(goal("g", GoalKind::Likes, 100)).await.expect("upsert");
    r.svc.adjust("g", 40).await.expect("adjust");
    r.svc.adjust("g", -10).await.expect("adjust");
    assert_eq!(current(&r, "g"), 30);
    r.svc.adjust("g", 100).await.expect("adjust");
    assert_eq!(fired(&r).len(), 1, "sumar desde una acción también alcanza la meta");
    r.svc.set_current("g", 5).await.expect("set");
    assert_eq!(current(&r, "g"), 5);
    r.svc.reset("g").await.expect("reset");
    assert_eq!(current(&r, "g"), 0);
    assert!(r.svc.adjust("nada", 1).await.is_err());
}

#[tokio::test]
async fn progress_is_saved_and_reloaded() {
    let r = rig().await;
    r.svc.upsert(goal("likes", GoalKind::Likes, 100)).await.expect("upsert");
    r.bus.publish(like_ev("a", 42));
    eventually(|| current(&r, "likes") == 42).await;
    r.svc.flush().await;

    let hub = OverlayHub::new(8);
    let fresh = GoalService::new(r.db.clone(), hub.clone());
    assert_eq!(fresh.load().await.expect("load"), 1);
    assert_eq!(fresh.list()[0].current, 42);
    assert_eq!(hub.retained_for("goals").expect("retenida").data["goals"][0]["current"], 42);
}

#[tokio::test]
async fn deleting_removes_the_goal_from_storage_and_overlays() {
    let r = rig().await;
    r.svc.upsert(goal("a", GoalKind::Likes, 10)).await.expect("upsert");
    assert!(r.svc.delete("a").await.expect("delete"));
    assert!(!r.svc.delete("a").await.expect("delete"));
    assert!(r.db.list_goals().await.expect("list").is_empty());
    assert_eq!(r.hub.retained_for("goals").expect("ret").data["goals"].as_array().expect("arr").len(), 0);
}

#[tokio::test]
async fn a_new_session_resets_only_goals_that_ask_for_it() {
    let r = rig().await;
    let mut resets = goal("r", GoalKind::Likes, 100);
    resets.reset_on_session = true;
    r.svc.upsert(resets).await.expect("upsert");
    r.svc.upsert(goal("keeps", GoalKind::Likes, 100)).await.expect("upsert");
    r.bus.publish(like_ev("a", 20));
    eventually(|| current(&r, "r") == 20 && current(&r, "keeps") == 20).await;
    r.sessions.start_new();
    eventually(|| current(&r, "r") == 0).await;
    assert_eq!(current(&r, "keeps"), 20);
}

#[tokio::test]
async fn closed_session_channel_does_not_spin_or_stop_event_handling() {
    let db = Db::open_memory().await.expect("db");
    let svc = GoalService::new(db, OverlayHub::new(8));
    let bus = EventBus::new(16);
    let (tx, rx) = broadcast::channel::<SessionStarted>(1);
    drop(tx); // canal de sesiones ya cerrado
    svc.spawn(&bus, rx, FAST);
    svc.upsert(goal("g", GoalKind::Likes, 100)).await.expect("upsert");
    bus.publish(like_ev("a", 9));
    eventually(|| svc.list()[0].current == 9).await;
}
