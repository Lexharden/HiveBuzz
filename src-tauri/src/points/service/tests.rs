use std::sync::atomic::{AtomicI64, Ordering};

use super::*;
use crate::events::testing::sample_event;
use crate::events::{Chat, EventType, Gift};
use crate::session::SessionService;

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

struct Rig {
    svc: Arc<PointsService>,
    clock: Arc<ManualClock>,
    db: Db,
}

async fn rig() -> Rig {
    let db = Db::open_memory().await.expect("db");
    let clock = Arc::new(ManualClock(AtomicI64::new(1_000_000)));
    let svc = PointsService::new(db.clone(), clock.clone());
    Rig { svc, clock, db }
}

fn chat(id: &str, user: &str, text: &str) -> crate::events::LiveEvent {
    let mut e = sample_event(id);
    e.user.id = user.into();
    e.user.unique_id = format!("u{user}");
    e.chat = Some(Chat { text: text.into(), emotes: None });
    e
}

fn gift(id: &str, user: &str, coins: u64) -> crate::events::LiveEvent {
    let mut e = sample_event(id);
    e.kind = EventType::Gift;
    e.chat = None;
    e.user.id = user.into();
    e.user.unique_id = format!("u{user}");
    e.gift = Some(Gift { id: 1, name: "Rose".into(), coins, count: 1, streakable: false, image: String::new() });
    e
}

#[tokio::test]
async fn events_become_points_after_a_flush() {
    let r = rig().await;
    r.svc.on_event(&chat("1", "7", "hola"));
    r.svc.on_event(&gift("2", "7", 100));
    assert_eq!(r.db.balance("7").await.expect("bal"), 0, "aún no se ha volcado");
    assert_eq!(r.svc.balance("7").await.expect("bal"), 102, "consultar el saldo vuelca antes");
    let v = r.svc.viewer("7").await.expect("v").expect("existe");
    assert_eq!((v.comments, v.coins_gifted), (1, 100));
}

#[tokio::test]
async fn simulated_events_are_ignored_unless_enabled() {
    let r = rig().await;
    r.svc.on_event(&chat("sim-1", "9", "hola"));
    assert_eq!(r.svc.viewer("9").await.expect("v"), None);
    r.svc.set_config(PointsConfig { count_simulated: true, ..Default::default() }).await.expect("cfg");
    r.svc.on_event(&chat("sim-2", "9", "hola"));
    assert!(r.svc.viewer("9").await.expect("v").is_some());
}

#[tokio::test]
async fn spending_is_atomic_and_leaves_a_history_trail() {
    let r = rig().await;
    r.svc.on_event(&gift("1", "7", 100));
    assert_eq!(r.svc.spend("7", 60, "Sonido").await.expect("spend"), Some(40));
    assert_eq!(r.svc.spend("7", 60, "Sonido").await.expect("spend"), None);
    assert_eq!(r.svc.balance("7").await.expect("bal"), 40);
    let h = r.svc.history("7", 10).await.expect("h");
    assert_eq!((h[0].delta, h[0].reason.as_str()), (-60, "spend:Sonido"));
    r.svc.refund("7", 60, "Sonido").await.expect("refund");
    assert_eq!(r.svc.balance("7").await.expect("bal"), 100);
    assert!(r.svc.history("7", 1).await.expect("h")[0].reason.starts_with("refund:"));
}

#[tokio::test]
async fn watch_points_are_given_once_per_interval_to_present_viewers() {
    let r = rig().await;
    r.svc.on_event(&chat("1", "7", "hola")); // presente; 2 puntos por el comentario
    assert_eq!(r.svc.watch_tick_if_due(), 0, "aún no pasó el intervalo");
    r.clock.advance(5 * 60_000);
    assert_eq!(r.svc.watch_tick_if_due(), 1);
    assert_eq!(r.svc.watch_tick_if_due(), 0, "ya se repartió en este intervalo");
    assert_eq!(r.svc.balance("7").await.expect("bal"), 2 + 5);
    assert_eq!(r.svc.viewer("7").await.expect("v").expect("v").watch_minutes, 5);
}

#[tokio::test]
async fn viewers_who_left_stop_earning_watch_points() {
    let r = rig().await;
    r.svc.on_event(&chat("1", "7", "hola"));
    r.clock.advance(30 * 60_000);
    assert_eq!(r.svc.watch_tick_if_due(), 0, "lleva media hora sin señales");
}

#[tokio::test]
async fn a_new_session_clears_presence() {
    let r = rig().await;
    r.svc.on_event(&chat("1", "7", "hola"));
    r.svc.on_session_started();
    r.clock.advance(5 * 60_000);
    assert_eq!(r.svc.watch_tick_if_due(), 0);
}

#[tokio::test]
async fn config_is_saved_sanitized_and_reloaded() {
    let r = rig().await;
    let saved = r.svc.set_config(PointsConfig { watch_interval_minutes: 0, points_command: "!MIS_PUNTOS".into(), ..Default::default() }).await.expect("cfg");
    assert_eq!((saved.watch_interval_minutes, saved.points_command.as_str()), (1, "mis_puntos"));
    let fresh = PointsService::new(r.db.clone(), r.clock.clone());
    fresh.load_config().await.expect("load");
    assert_eq!(fresh.config(), saved);
    r.db.set_setting(KEY_POINTS_CONFIG, "{roto").await.expect("raw");
    fresh.load_config().await.expect("no falla");
    assert_eq!(fresh.config(), PointsConfig::default());
}

#[tokio::test]
async fn manual_adjustment_by_username_creates_the_viewer_if_needed() {
    let r = rig().await;
    let v = r.svc.adjust_by_unique("@NuevoUsuario", 250).await.expect("adjust");
    assert_eq!((v.unique_id.as_str(), v.points), ("nuevousuario", 250));
    let v = r.svc.adjust_by_unique("nuevousuario", -100).await.expect("adjust");
    assert_eq!(v.points, 150);
    assert!(r.svc.adjust_by_unique("  ", 5).await.is_err());
    // Cuando esa persona aparece de verdad, se une a su registro.
    let mut e = chat("1", "999", "hola");
    e.user.unique_id = "nuevousuario".into();
    r.svc.on_event(&e);
    assert_eq!(r.svc.balance("999").await.expect("bal"), 152);
    assert!(r.svc.viewer("unique:nuevousuario").await.expect("v").is_none());
}

#[tokio::test]
async fn csv_export_and_import_through_the_service() {
    let r = rig().await;
    r.svc.on_event(&gift("1", "7", 300));
    let csv = r.svc.export_csv().await.expect("export");
    r.svc.clear_all().await.expect("clear");
    assert_eq!(r.svc.list("", SortKey::Points, 10, 0).await.expect("list").1, 0);
    let report = r.svc.import_csv(&csv, ImportMode::Replace).await.expect("import");
    assert_eq!((report.created, report.errors.len()), (1, 0));
    assert_eq!(r.svc.viewer_by_unique("u7").await.expect("v").expect("v").points, 300);
}

#[tokio::test]
async fn background_tasks_persist_events_without_anyone_asking() {
    let r = rig().await;
    let bus = EventBus::new(16);
    let sessions = SessionService::new();
    r.svc.spawn(&bus, sessions.subscribe(), PointsTiming { flush_every: Duration::from_millis(15), watch_check_every: Duration::from_millis(15), maintenance_every: Duration::from_millis(50) });
    bus.publish(chat("1", "7", "hola"));
    for _ in 0..100 {
        if r.db.balance("7").await.expect("bal") == 2 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    panic!("el volcado periódico no guardó los puntos");
}

#[tokio::test]
async fn failed_flushes_keep_the_awards_for_the_next_attempt() {
    let r = rig().await;
    r.svc.on_event(&gift("1", "7", 50));
    r.db.exec_for_tests("ALTER TABLE viewers RENAME TO viewers_bak").await;
    assert!(r.svc.flush().await.is_err());
    r.svc.on_event(&gift("2", "7", 25));
    r.db.exec_for_tests("ALTER TABLE viewers_bak RENAME TO viewers").await;
    r.svc.flush().await.expect("reintento");
    assert_eq!(r.db.balance("7").await.expect("bal"), 75);
}
