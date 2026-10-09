use super::*;
use crate::events::testing::sample_event;
use crate::events::Gift;
use crate::session::SessionService;

fn gift(id: &str, user: &str, nick: &str, coins: u64, count: u32) -> LiveEvent {
    let mut e = sample_event(id);
    e.kind = EventType::Gift;
    e.chat = None;
    e.user.id = user.into();
    e.user.unique_id = format!("u_{user}");
    e.user.nickname = nick.into();
    e.gift = Some(Gift { id: 1, name: "Rose".into(), coins, count, streakable: false, image: String::new() });
    e
}

struct Rig {
    svc: Arc<LeaderboardService>,
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
    let svc = LeaderboardService::new(db.clone(), hub.clone(), Arc::new(move || d.lock().expect("lock").clone()));
    let bus = EventBus::new(64);
    let sessions = SessionService::new();
    svc.spawn(&bus, sessions.subscribe(), LeaderboardTiming { flush_every: Duration::from_millis(15) });
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

fn names(v: &[DonorEntry]) -> Vec<(&str, u64)> {
    v.iter().map(|e| (e.nickname.as_str(), e.coins)).collect()
}

#[test]
fn only_gifts_with_coins_and_an_identifiable_user_are_donations() {
    assert!(donation_from(&gift("1", "a", "Ana", 5, 1), "2026-10-08").is_some());
    assert!(donation_from(&gift("1", "a", "Ana", 0, 1), "2026-10-08").is_none());
    assert!(donation_from(&gift("1", "", "Anon", 5, 1), "2026-10-08").is_none());
    let mut chat = sample_event("c");
    chat.kind = EventType::Chat;
    assert!(donation_from(&chat, "2026-10-08").is_none());
    let mut no_nick = gift("1", "a", "", 5, 1);
    no_nick.user.nickname.clear();
    assert_eq!(donation_from(&no_nick, "d").expect("don").nickname, "u_a", "sin apodo usa el @usuario");
}

#[tokio::test]
async fn simulated_gifts_show_in_the_session_but_never_reach_the_database() {
    let db = Db::open_memory().await.expect("db");
    let svc = LeaderboardService::new(db.clone(), OverlayHub::new(8), Arc::new(|| "2026-10-08".to_string()));
    svc.record(&gift("sim-1", "s", "Sim", 500, 1));
    svc.record(&gift("123", "r", "Real", 40, 1));
    assert_eq!(names(&svc.top(Scope::Session, 10).await.expect("s")), [("Sim", 500), ("Real", 40)]);
    assert_eq!(names(&svc.top(Scope::All, 10).await.expect("all")), [("Real", 40)]);
    assert_eq!(names(&svc.top(Scope::Day, 10).await.expect("day")), [("Real", 40)]);
}

#[test]
fn ranking_orders_by_coins_then_by_username() {
    let e = |id: &str, coins| DonorEntry { user_id: id.into(), unique_id: id.into(), nickname: id.into(), avatar: String::new(), coins, gifts: 1 };
    let r = ranked(vec![e("b", 10), e("a", 10), e("c", 50), e("d", 1)], 3);
    assert_eq!(r.iter().map(|x| x.unique_id.as_str()).collect::<Vec<_>>(), ["c", "a", "b"]);
}

#[tokio::test]
async fn session_ranking_sums_gifts_per_user_and_keeps_the_latest_name() {
    let r = rig().await;
    r.svc.record(&gift("1", "a", "Ana", 100, 1));
    r.svc.record(&gift("2", "b", "Beto", 300, 2));
    r.svc.record(&gift("3", "a", "Ana la Grande", 250, 1));
    let top = r.svc.top(Scope::Session, 10).await.expect("top");
    assert_eq!(names(&top), [("Ana la Grande", 350), ("Beto", 300)]);
    assert_eq!(top[0].gifts, 2);
    assert_eq!(r.svc.top(Scope::Session, 1).await.expect("top").len(), 1);
}

#[tokio::test]
async fn day_and_all_time_rankings_come_from_the_database() {
    let r = rig().await;
    r.svc.record(&gift("1", "a", "Ana", 100, 1));
    r.svc.record(&gift("2", "b", "Beto", 500, 1));
    set_day(&r, "2026-10-09");
    r.svc.record(&gift("3", "a", "Ana", 450, 1));
    r.svc.record(&gift("4", "c", "Cata", 20, 1));

    let today = r.svc.top(Scope::Day, 10).await.expect("day");
    assert_eq!(names(&today), [("Ana", 450), ("Cata", 20)]);
    let all = r.svc.top(Scope::All, 10).await.expect("all");
    assert_eq!(names(&all), [("Ana", 550), ("Beto", 500), ("Cata", 20)]);
    assert_eq!(all[0].gifts, 2);
    let yesterday = r.db.top_donors_day("2026-10-08", 10).await.expect("ayer");
    assert_eq!(names(&yesterday), [("Beto", 500), ("Ana", 100)]);
}

#[tokio::test]
async fn all_time_uses_the_most_recent_nickname_and_avatar() {
    let r = rig().await;
    let mut old = gift("1", "a", "Viejo", 10, 1);
    old.user.avatar = "https://x/old.png".into();
    r.svc.record(&old);
    set_day(&r, "2026-10-09");
    let mut new = gift("2", "a", "Nuevo", 10, 1);
    new.user.avatar = "https://x/new.png".into();
    r.svc.record(&new);
    let all = r.svc.top(Scope::All, 5).await.expect("all");
    assert_eq!((all[0].nickname.as_str(), all[0].avatar.as_str(), all[0].coins), ("Nuevo", "https://x/new.png", 20));
}

#[tokio::test]
async fn resetting_the_session_keeps_day_and_history() {
    let r = rig().await;
    r.svc.record(&gift("1", "a", "Ana", 100, 1));
    r.svc.reset_session();
    assert!(r.svc.top(Scope::Session, 10).await.expect("top").is_empty());
    assert_eq!(names(&r.svc.top(Scope::Day, 10).await.expect("day")), [("Ana", 100)]);
    assert_eq!(names(&r.svc.top(Scope::All, 10).await.expect("all")), [("Ana", 100)]);
}

#[tokio::test]
async fn clearing_history_empties_day_and_all_but_not_the_session() {
    let r = rig().await;
    r.svc.record(&gift("1", "a", "Ana", 100, 1));
    r.svc.flush().await.expect("flush");
    r.svc.clear_history().await.expect("clear");
    assert!(r.svc.top(Scope::Day, 10).await.expect("day").is_empty());
    assert!(r.svc.top(Scope::All, 10).await.expect("all").is_empty());
    assert_eq!(r.svc.top(Scope::Session, 10).await.expect("s").len(), 1);
}

#[tokio::test]
async fn bus_gifts_are_recorded_persisted_and_published_to_the_overlay() {
    let r = rig().await;
    r.bus.publish(gift("1", "a", "Ana", 100, 1));
    r.bus.publish(gift("2", "b", "Beto", 40, 1));
    eventually(|| r.hub.retained_for("leaderboard").is_some_and(|m| m.data["all"].as_array().is_some_and(|a| a.len() == 2))).await;
    let data = r.hub.retained_for("leaderboard").expect("ret").data.clone();
    assert_eq!(data["session"][0]["nickname"], "Ana");
    assert_eq!(data["session"][0]["coins"], 100);
    assert_eq!(data["day"][1]["nickname"], "Beto");
    assert_eq!(data["all"][0]["userId"], "a");
    // Quedó en disco aunque nadie consultara.
    assert_eq!(r.db.top_donors_all(5).await.expect("all").len(), 2);
}

#[tokio::test]
async fn a_new_live_session_clears_only_the_session_board() {
    let r = rig().await;
    r.bus.publish(gift("1", "a", "Ana", 100, 1));
    eventually(|| r.hub.retained_for("leaderboard").is_some_and(|m| m.data["session"].as_array().is_some_and(|a| a.len() == 1))).await;
    r.sessions.start_new();
    eventually(|| r.hub.retained_for("leaderboard").is_some_and(|m| m.data["session"].as_array().is_some_and(Vec::is_empty))).await;
    assert_eq!(r.hub.retained_for("leaderboard").expect("ret").data["all"].as_array().expect("arr").len(), 1);
}

#[tokio::test]
async fn failed_database_writes_are_retried_without_losing_donations() {
    // Sin `spawn`: así ningún volcador en segundo plano compite con el test.
    let db = Db::open_memory().await.expect("db");
    let svc = LeaderboardService::new(db.clone(), OverlayHub::new(8), Arc::new(|| "2026-10-08".to_string()));
    svc.record(&gift("1", "a", "Ana", 100, 1));
    // Se rompe la tabla: el volcado falla y las donaciones deben quedar pendientes.
    sqlx_exec(&db, "ALTER TABLE donor_daily RENAME TO donor_daily_bak").await;
    assert!(svc.flush().await.is_err());
    svc.record(&gift("2", "a", "Ana", 50, 1));
    sqlx_exec(&db, "ALTER TABLE donor_daily_bak RENAME TO donor_daily").await;
    svc.flush().await.expect("reintento");
    assert_eq!(names(&db.top_donors_all(5).await.expect("all")), [("Ana", 150)]);
}

#[tokio::test]
async fn a_closed_session_channel_does_not_stop_recording() {
    let db = Db::open_memory().await.expect("db");
    let svc = LeaderboardService::new(db, OverlayHub::new(8), Arc::new(|| "2026-10-08".to_string()));
    let bus = EventBus::new(8);
    let (tx, rx) = broadcast::channel::<SessionStarted>(1);
    drop(tx);
    svc.spawn(&bus, rx, LeaderboardTiming { flush_every: Duration::from_millis(15) });
    bus.publish(gift("1", "a", "Ana", 10, 1));
    eventually(|| !svc.session().is_empty()).await;
}

#[test]
fn local_today_has_the_iso_shape() {
    let d = LeaderboardService::local_today();
    assert_eq!(d.len(), 10);
    assert_eq!(d.matches('-').count(), 2);
}

/// Ejecuta SQL crudo contra la base de pruebas.
async fn sqlx_exec(db: &Db, sql: &str) {
    db.exec_for_tests(sql).await;
}
