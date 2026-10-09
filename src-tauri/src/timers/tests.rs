use super::*;
use crate::events::testing::sample_event;
use crate::events::{Gift, Like};

fn cfg(exts: Vec<Extension>) -> TimerConfig {
    TimerConfig { id: "t".into(), name: "Subathon".into(), start_seconds: 600, max_seconds: None, extensions: exts }
}

fn ext(source: ExtensionSource, seconds: u64) -> Extension {
    Extension { source, seconds }
}

fn running(now: i64, ms: i64) -> TimerState {
    let mut s = TimerState::idle(ms);
    s.start(now, ms);
    s
}

fn ev(kind: EventType) -> LiveEvent {
    let mut e = sample_event("e");
    e.kind = kind;
    e.chat = None;
    e
}

fn gift_ev(name: &str, id: i64, coins: u64, count: u32) -> LiveEvent {
    let mut e = ev(EventType::Gift);
    e.gift = Some(Gift { id, name: name.into(), coins, count, streakable: false, image: String::new() });
    e
}

fn like_ev(count: u64) -> LiveEvent {
    let mut e = ev(EventType::Like);
    e.like = Some(Like { count, total: 0 });
    e
}

// ---- Estado ----

#[test]
fn counts_down_from_the_moment_it_starts() {
    let s = running(1_000, 60_000);
    assert_eq!(s.remaining(1_000), 60_000);
    assert_eq!(s.remaining(31_000), 30_000);
    assert_eq!(s.remaining(999_999), 0, "nunca negativo");
}

#[test]
fn idle_timers_show_their_full_duration_and_do_not_move() {
    let s = TimerState::idle(5_000);
    assert_eq!(s.remaining(0), 5_000);
    assert_eq!(s.remaining(1_000_000), 5_000);
}

#[test]
fn pause_freezes_and_resume_continues_from_there() {
    let mut s = running(0, 60_000);
    s.pause(20_000);
    assert_eq!((s.status, s.remaining(500_000)), (Status::Paused, 40_000));
    s.resume(100_000);
    assert_eq!(s.status, Status::Running);
    assert_eq!(s.remaining(110_000), 30_000);
    // Pausar o reanudar de nuevo es inofensivo.
    s.resume(110_000);
    assert_eq!(s.remaining(110_000), 30_000);
}

#[test]
fn start_on_a_paused_timer_resumes_instead_of_restarting() {
    let mut s = running(0, 60_000);
    s.pause(10_000);
    s.start(50_000, 60_000);
    assert_eq!(s.remaining(50_000), 50_000);
}

#[test]
fn start_while_running_does_nothing() {
    let mut s = running(0, 60_000);
    s.start(30_000, 60_000);
    assert_eq!(s.remaining(30_000), 30_000);
}

#[test]
fn tick_ends_exactly_once() {
    let mut s = running(0, 1_000);
    assert!(!s.tick(999));
    assert!(s.tick(1_000));
    assert_eq!((s.status, s.remaining(5_000)), (Status::Ended, 0));
    assert!(!s.tick(6_000), "no vuelve a avisar");
}

#[test]
fn restarting_an_ended_timer_begins_a_new_countdown() {
    let mut s = running(0, 1_000);
    s.tick(2_000);
    s.start(10_000, 7_000);
    assert_eq!((s.status, s.remaining(10_000)), (Status::Running, 7_000));
}

#[test]
fn reset_returns_to_idle_with_the_initial_time() {
    let mut s = running(0, 60_000);
    s.leftovers = vec![5];
    s.reset(60_000);
    assert_eq!(s, TimerState::idle(60_000));
}

#[test]
fn add_extends_a_running_timer() {
    let mut s = running(0, 60_000);
    assert!(s.add(10_000, 30_000, None));
    assert_eq!(s.remaining(10_000), 80_000);
}

#[test]
fn add_extends_a_paused_timer_without_starting_it() {
    let mut s = running(0, 60_000);
    s.pause(10_000);
    assert!(s.add(20_000, 5_000, None));
    assert_eq!((s.status, s.remaining(99_000)), (Status::Paused, 55_000));
}

#[test]
fn idle_and_ended_timers_ignore_extensions() {
    let mut idle = TimerState::idle(60_000);
    assert!(!idle.add(0, 30_000, None));
    assert_eq!(idle.remaining(0), 60_000);
    let mut ended = running(0, 1_000);
    ended.tick(5_000);
    assert!(!ended.add(6_000, 30_000, None));
    assert_eq!(ended.status, Status::Ended);
}

#[test]
fn the_cap_limits_extensions() {
    let mut s = running(0, 50_000);
    assert!(s.add(0, 100_000, Some(80_000)));
    assert_eq!(s.remaining(0), 80_000);
    assert!(!s.add(0, 10_000, Some(80_000)), "ya está en el tope");
    assert_eq!(s.remaining(0), 80_000);
}

#[test]
fn a_cap_never_shortens_time_that_was_already_above_it() {
    let mut s = running(0, 120_000);
    assert!(!s.add(0, 5_000, Some(80_000)));
    assert_eq!(s.remaining(0), 120_000);
}

#[test]
fn negative_adds_shorten_and_can_run_it_out() {
    let mut s = running(0, 60_000);
    assert!(s.add(0, -20_000, None));
    assert_eq!(s.remaining(0), 40_000);
    assert!(s.add(0, -1_000_000, None));
    assert_eq!(s.remaining(0), 0);
    assert!(s.tick(0), "queda en 0 y termina en el siguiente tick");
}

#[test]
fn zero_delta_is_a_no_op() {
    let mut s = running(0, 60_000);
    assert!(!s.add(0, 0, None));
}

// ---- Extensiones ----

#[test]
fn coins_extension_accumulates_leftovers_across_gifts() {
    let c = cfg(vec![ext(ExtensionSource::Coins { per_coins: 100 }, 60)]);
    let mut left = Vec::new();
    assert_eq!(extension_seconds(&c, &mut left, &gift_ev("Rose", 1, 70, 1)), 0);
    assert_eq!(extension_seconds(&c, &mut left, &gift_ev("Rose", 1, 70, 1)), 60, "140 monedas → 1 paso, sobran 40");
    assert_eq!(left, [40]);
    assert_eq!(extension_seconds(&c, &mut left, &gift_ev("Lion", 2, 460, 1)), 300, "40+460 = 500 → 5 pasos");
    assert_eq!(left, [0]);
}

#[test]
fn likes_extension_accumulates_too() {
    let c = cfg(vec![ext(ExtensionSource::Likes { per_likes: 50 }, 10)]);
    let mut left = Vec::new();
    assert_eq!(extension_seconds(&c, &mut left, &like_ev(30)), 0);
    assert_eq!(extension_seconds(&c, &mut left, &like_ev(30)), 10);
    assert_eq!(left, [10]);
}

#[test]
fn follow_share_and_subscribe_add_a_fixed_amount_per_event() {
    let c = cfg(vec![
        ext(ExtensionSource::Follow, 30),
        ext(ExtensionSource::Share, 20),
        ext(ExtensionSource::Subscribe, 120),
    ]);
    let mut left = Vec::new();
    assert_eq!(extension_seconds(&c, &mut left, &ev(EventType::Follow)), 30);
    assert_eq!(extension_seconds(&c, &mut left, &ev(EventType::Share)), 20);
    assert_eq!(extension_seconds(&c, &mut left, &ev(EventType::Subscribe)), 120);
    assert_eq!(extension_seconds(&c, &mut left, &ev(EventType::Chat)), 0);
}

#[test]
fn specific_gift_extension_counts_units_and_matches_name_or_id() {
    let c = cfg(vec![ext(ExtensionSource::Gift { gift_id: None, gift_name: Some("galaxy".into()) }, 600)]);
    let mut left = Vec::new();
    assert_eq!(extension_seconds(&c, &mut left, &gift_ev("Galaxy", 9, 1000, 2)), 1200);
    assert_eq!(extension_seconds(&c, &mut left, &gift_ev("Rose", 1, 1, 50)), 0);
}

#[test]
fn several_extensions_can_apply_to_one_event() {
    let c = cfg(vec![
        ext(ExtensionSource::Coins { per_coins: 10 }, 1),
        ext(ExtensionSource::Gift { gift_id: Some(1), gift_name: None }, 5),
    ]);
    let mut left = Vec::new();
    assert_eq!(extension_seconds(&c, &mut left, &gift_ev("Rose", 1, 30, 3)), 3 + 15);
}

#[test]
fn leftovers_resize_when_the_config_changes() {
    let c = cfg(vec![ext(ExtensionSource::Follow, 1), ext(ExtensionSource::Coins { per_coins: 10 }, 1)]);
    let mut left = vec![7];
    extension_seconds(&c, &mut left, &ev(EventType::Chat));
    assert_eq!(left.len(), 2);
}

#[test]
fn huge_values_saturate_instead_of_overflowing() {
    let c = cfg(vec![ext(ExtensionSource::Coins { per_coins: 1 }, u64::MAX)]);
    let mut left = Vec::new();
    assert_eq!(extension_seconds(&c, &mut left, &gift_ev("Rose", 1, u64::MAX, 1)), u64::MAX);
}

// ---- Validación y serialización ----

#[test]
fn config_validation() {
    assert!(cfg(vec![]).validate().is_ok());
    let bad = |f: &dyn Fn(&mut TimerConfig)| {
        let mut c = cfg(vec![]);
        f(&mut c);
        c.validate().is_err()
    };
    assert!(bad(&|c| c.id = " ".into()));
    assert!(bad(&|c| c.name = "".into()));
    assert!(bad(&|c| c.start_seconds = 0));
    assert!(bad(&|c| c.start_seconds = MAX_START_SECONDS + 1));
    assert!(bad(&|c| c.max_seconds = Some(0)));
    assert!(bad(&|c| c.extensions = vec![ext(ExtensionSource::Follow, 0)]));
    assert!(bad(&|c| c.extensions = vec![ext(ExtensionSource::Coins { per_coins: 0 }, 5)]));
    assert!(bad(&|c| c.extensions = vec![ext(ExtensionSource::Likes { per_likes: 0 }, 5)]));
    assert!(bad(&|c| c.extensions = vec![ext(ExtensionSource::Gift { gift_id: None, gift_name: None }, 5)]));
    assert!(bad(&|c| c.extensions = (0..21).map(|_| ext(ExtensionSource::Follow, 1)).collect()));
}

#[test]
fn json_shape_roundtrips() {
    let c: TimerConfig = serde_json::from_str(
        r#"{"id":"t","name":"Sub","startSeconds":3600,"maxSeconds":7200,
            "extensions":[{"source":{"type":"coins","perCoins":100},"seconds":60},{"source":{"type":"follow"},"seconds":10}]}"#,
    )
    .expect("parsea");
    assert_eq!(c.extensions.len(), 2);
    assert_eq!(c.max_ms(), Some(7_200_000));
    assert_eq!(c.start_ms(), 3_600_000);
    let back: TimerConfig = serde_json::from_str(&serde_json::to_string(&c).expect("ser")).expect("de");
    assert_eq!(back, c);
}

#[test]
fn state_json_roundtrips_and_tolerates_missing_leftovers() {
    let s: TimerState = serde_json::from_str(r#"{"status":"paused","remainingMs":5000,"endsAtMs":0}"#).expect("parsea");
    assert_eq!((s.status, s.remaining_ms, s.leftovers.len()), (Status::Paused, 5000, 0));
}
