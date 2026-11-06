use super::*;
use crate::events::testing::sample_event;
use crate::events::Like;
use crate::session::SessionService;

fn like(id: &str, user: &str, nick: &str, count: u64) -> LiveEvent {
    let mut e = sample_event(id);
    e.kind = EventType::Like;
    e.chat = None;
    e.user.id = user.into();
    e.user.unique_id = format!("u_{user}");
    e.user.nickname = nick.into();
    e.like = Some(Like { count, total: 99_999 });
    e
}

struct Rig {
    svc: Arc<LikeBoardService>,
    hub: OverlayHub,
    bus: EventBus,
    db: Db,
    day: Arc<Mutex<String>>,
    sessions: SessionService,
}

async fn rig() -> Rig {
    let db = Db::open_memory().await.expect("db");
    let hub = OverlayHub::new(16);
    let day = Arc::new(Mutex::new("2026-10-08".to_string()));
    let d = Arc::clone(&day);
    let svc = LikeBoardService::new(db.clone(), hub.clone(), Arc::new(move || d.lock().expect("lock").clone()));
    let bus = EventBus::new(64);
    let sessions = SessionService::new();
    svc.spawn(&bus, sessions.subscribe(), LikeBoardTiming { flush_every: Duration::from_millis(15) });
    Rig { svc, hub, bus, db, day, sessions }
}

fn set_day(r: &Rig, d: &str) {
    *r.day.lock().expect("lock") = d.into();
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

fn names(v: &[LikeEntry]) -> Vec<(&str, u64)> {
    v.iter().map(|e| (e.nickname.as_str(), e.likes)).collect()
}

#[test]
fn only_likes_with_a_count_and_an_identifiable_user_are_taps() {
    assert!(tap_from(&like("1", "a", "Ana", 5), "2026-10-08").is_some());
    assert!(tap_from(&like("1", "a", "Ana", 0), "2026-10-08").is_none(), "sin cantidad");
    assert!(tap_from(&like("1", "", "Anon", 5), "2026-10-08").is_none(), "sin usuario");
    let mut chat = sample_event("c");
    chat.kind = EventType::Chat;
    assert!(tap_from(&chat, "2026-10-08").is_none());
    let mut no_like = like("1", "a", "Ana", 5);
    no_like.like = None;
    assert!(tap_from(&no_like, "d").is_none());
    let mut no_nick = like("1", "a", "", 5);
    no_nick.user.nickname.clear();
    assert_eq!(tap_from(&no_nick, "d").expect("tap").nickname, "u_a", "sin apodo usa el @usuario");
}

#[test]
fn the_room_total_is_never_added_to_a_user() {
    let t = tap_from(&like("1", "a", "Ana", 7), "d").expect("tap");
    assert_eq!(t.likes, 7, "cuenta `count`, no el `total` de la sala");
}

#[test]
fn ranking_orders_by_likes_then_by_username() {
    let e = |id: &str, likes| LikeEntry { user_id: id.into(), unique_id: id.into(), nickname: id.into(), avatar: String::new(), likes };
    let r = ranked(vec![e("b", 10), e("a", 10), e("c", 50), e("d", 1)], 3);
    assert_eq!(r.iter().map(|x| x.unique_id.as_str()).collect::<Vec<_>>(), ["c", "a", "b"]);
}

#[tokio::test]
async fn session_ranking_sums_per_user_and_keeps_the_latest_name() {
    let r = rig().await;
    r.svc.record(&like("1", "a", "Ana", 10));
    r.svc.record(&like("2", "b", "Beto", 30));
    r.svc.record(&like("3", "a", "Ana la Grande", 25));
    let top = r.svc.top(Scope::Session, 10).await.expect("top");
    assert_eq!(names(&top), [("Ana la Grande", 35), ("Beto", 30)]);
    assert_eq!(r.svc.top(Scope::Session, 1).await.expect("top").len(), 1);
}

#[tokio::test]
async fn simulated_likes_show_in_the_session_but_never_reach_the_database() {
    let db = Db::open_memory().await.expect("db");
    let svc = LikeBoardService::new(db, OverlayHub::new(8), Arc::new(|| "2026-10-08".to_string()));
    svc.record(&like("sim-1", "s", "Sim", 500));
    svc.record(&like("123", "r", "Real", 40));
    assert_eq!(names(&svc.top(Scope::Session, 10).await.expect("s")), [("Sim", 500), ("Real", 40)]);
    assert_eq!(names(&svc.top(Scope::Day, 10).await.expect("day")), [("Real", 40)]);
}

#[tokio::test]
async fn the_day_ranking_comes_from_the_database_and_changes_with_the_day() {
    let r = rig().await;
    r.svc.record(&like("1", "a", "Ana", 100));
    r.svc.record(&like("2", "b", "Beto", 500));
    set_day(&r, "2026-10-09");
    r.svc.record(&like("3", "a", "Ana", 450));
    r.svc.record(&like("4", "c", "Cata", 20));
    assert_eq!(names(&r.svc.top(Scope::Day, 10).await.expect("day")), [("Ana", 450), ("Cata", 20)]);
    assert_eq!(names(&r.db.top_likers_day("2026-10-08", 10).await.expect("ayer")), [("Beto", 500), ("Ana", 100)]);
}

#[tokio::test]
async fn resetting_the_session_keeps_the_day() {
    let r = rig().await;
    r.svc.record(&like("1", "a", "Ana", 100));
    r.svc.reset_session();
    assert!(r.svc.top(Scope::Session, 10).await.expect("top").is_empty());
    assert_eq!(names(&r.svc.top(Scope::Day, 10).await.expect("day")), [("Ana", 100)]);
}

#[tokio::test]
async fn clearing_history_empties_the_day_but_not_the_session() {
    let r = rig().await;
    r.svc.record(&like("1", "a", "Ana", 100));
    r.svc.flush().await.expect("flush");
    r.svc.clear_history().await.expect("clear");
    assert!(r.svc.top(Scope::Day, 10).await.expect("day").is_empty());
    assert_eq!(r.svc.top(Scope::Session, 10).await.expect("s").len(), 1);
}

#[tokio::test]
async fn bus_likes_are_recorded_persisted_and_published_to_the_overlay() {
    let r = rig().await;
    r.bus.publish(like("1", "a", "Ana", 100));
    r.bus.publish(like("2", "b", "Beto", 40));
    eventually(|| r.hub.retained_for("likeboard").is_some_and(|m| m.data["day"].as_array().is_some_and(|a| a.len() == 2))).await;
    let data = r.hub.retained_for("likeboard").expect("ret").data.clone();
    assert_eq!(data["session"][0]["nickname"], "Ana");
    assert_eq!(data["session"][0]["likes"], 100);
    assert_eq!(data["day"][1]["nickname"], "Beto");
    assert_eq!(data["day"][0]["userId"], "a");
    assert_eq!(r.db.top_likers_day("2026-10-08", 5).await.expect("día").len(), 2, "quedó en disco");
}

#[tokio::test]
async fn gifts_and_chat_do_not_move_the_like_board() {
    let r = rig().await;
    let mut chat = sample_event("c1");
    chat.kind = EventType::Chat;
    r.bus.publish(chat);
    r.bus.publish(like("2", "b", "Beto", 5));
    eventually(|| r.hub.retained_for("likeboard").is_some_and(|m| m.data["session"].as_array().is_some_and(|a| a.len() == 1))).await;
}

#[tokio::test]
async fn a_new_live_session_clears_only_the_session_board() {
    let r = rig().await;
    r.bus.publish(like("1", "a", "Ana", 100));
    eventually(|| r.hub.retained_for("likeboard").is_some_and(|m| m.data["session"].as_array().is_some_and(|a| a.len() == 1))).await;
    r.sessions.start_new();
    eventually(|| r.hub.retained_for("likeboard").is_some_and(|m| m.data["session"].as_array().is_some_and(Vec::is_empty))).await;
    assert_eq!(r.hub.retained_for("likeboard").expect("ret").data["day"].as_array().expect("arr").len(), 1);
}

#[tokio::test]
async fn failed_database_writes_are_retried_without_losing_likes() {
    let db = Db::open_memory().await.expect("db");
    let svc = LikeBoardService::new(db.clone(), OverlayHub::new(8), Arc::new(|| "2026-10-08".to_string()));
    svc.record(&like("1", "a", "Ana", 100));
    // Se rompe la tabla: el volcado falla y los likes deben quedar pendientes.
    db.exec_for_tests("ALTER TABLE like_daily RENAME TO like_daily_bak").await;
    assert!(svc.flush().await.is_err());
    svc.record(&like("2", "a", "Ana", 50));
    assert_eq!(svc.pending().len(), 1, "los pendientes del mismo usuario y día se fusionan");
    db.exec_for_tests("ALTER TABLE like_daily_bak RENAME TO like_daily").await;
    svc.flush().await.expect("reintento");
    assert_eq!(names(&db.top_likers_day("2026-10-08", 5).await.expect("día")), [("Ana", 150)]);
}

#[tokio::test]
async fn a_closed_session_channel_does_not_stop_recording() {
    let db = Db::open_memory().await.expect("db");
    let svc = LikeBoardService::new(db, OverlayHub::new(8), Arc::new(|| "2026-10-08".to_string()));
    let bus = EventBus::new(8);
    let (tx, rx) = broadcast::channel::<SessionStarted>(1);
    drop(tx);
    svc.spawn(&bus, rx, LikeBoardTiming { flush_every: Duration::from_millis(15) });
    bus.publish(like("1", "a", "Ana", 10));
    eventually(|| !svc.session().is_empty()).await;
}

#[tokio::test]
async fn the_session_memory_is_bounded_and_keeps_the_top_likers() {
    let db = Db::open_memory().await.expect("db");
    let svc = LikeBoardService::new(db, OverlayHub::new(8), Arc::new(|| "2026-10-08".to_string()));
    // Un usuario destacado y luego muchos con 1 like cada uno, hasta pasarse del tope.
    svc.record(&like("big", "star", "Star", 9_000));
    for i in 0..MAX_SESSION_USERS + 10 {
        svc.record(&like(&format!("sim-{i}"), &format!("u{i}"), "x", 1));
    }
    let len = svc.session().len();
    assert!(len <= MAX_SESSION_USERS, "memoria acotada, hay {len}");
    assert_eq!(svc.top(Scope::Session, 1).await.expect("top")[0].nickname, "Star", "el líder se conserva");
}

#[tokio::test]
async fn likes_saturate_instead_of_overflowing() {
    let db = Db::open_memory().await.expect("db");
    let svc = LikeBoardService::new(db, OverlayHub::new(8), Arc::new(|| "2026-10-08".to_string()));
    svc.record(&like("1", "a", "Ana", u64::MAX));
    svc.record(&like("2", "a", "Ana", 10));
    assert_eq!(svc.top(Scope::Session, 1).await.expect("top")[0].likes, u64::MAX);
}
