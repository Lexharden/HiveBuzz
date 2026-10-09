use super::*;
use crate::events::testing::sample_event;
use crate::events::{Gift, Like};

fn goal(kind: GoalKind, target: u64, on_reach: OnReach) -> Goal {
    Goal {
        id: "g".into(),
        name: "Meta".into(),
        kind,
        target,
        current: 0,
        on_reach,
        reset_on_session: false,
        reached_count: 0,
    }
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

#[test]
fn contributions_by_kind() {
    assert_eq!(contribution(&GoalKind::Likes, &like_ev(7)), 7);
    assert_eq!(contribution(&GoalKind::Follows, &ev(EventType::Follow)), 1);
    assert_eq!(contribution(&GoalKind::Shares, &ev(EventType::Share)), 1);
    assert_eq!(contribution(&GoalKind::Subscribers, &ev(EventType::Subscribe)), 1);
    assert_eq!(contribution(&GoalKind::Coins, &gift_ev("Rose", 1, 30, 3)), 30);
}

#[test]
fn a_goal_ignores_unrelated_events() {
    assert_eq!(contribution(&GoalKind::Likes, &ev(EventType::Follow)), 0);
    assert_eq!(contribution(&GoalKind::Follows, &like_ev(5)), 0);
    assert_eq!(contribution(&GoalKind::Coins, &ev(EventType::Chat)), 0);
}

#[test]
fn specific_gift_goals_count_quantity_and_match_by_name_and_or_id() {
    let by_name = GoalKind::Gift { gift_id: None, gift_name: Some(" rose ".into()) };
    let by_id = GoalKind::Gift { gift_id: Some(7), gift_name: None };
    let both = GoalKind::Gift { gift_id: Some(7), gift_name: Some("Lion".into()) };
    assert_eq!(contribution(&by_name, &gift_ev("Rose", 1, 1, 12)), 12, "cuenta unidades, no monedas");
    assert_eq!(contribution(&by_name, &gift_ev("Lion", 7, 99, 1)), 0);
    assert_eq!(contribution(&by_id, &gift_ev("Lion", 7, 99, 2)), 2);
    assert_eq!(contribution(&both, &gift_ev("Lion", 7, 1, 1)), 1);
    assert_eq!(contribution(&both, &gift_ev("Rose", 7, 1, 1)), 0);
    assert_eq!(contribution(&by_name, &ev(EventType::Chat)), 0);
}

#[test]
fn stop_policy_fires_once_and_keeps_counting() {
    let mut g = goal(GoalKind::Likes, 100, OnReach::Stop);
    assert_eq!(add(&mut g, 60), 0);
    assert_eq!(add(&mut g, 60), 1);
    assert_eq!((g.current, g.target, g.reached_count), (120, 100, 1));
    assert_eq!(add(&mut g, 500), 0, "ya estaba completa");
    assert_eq!(g.current, 620);
    assert_eq!(g.percent(), 100.0);
}

#[test]
fn reset_policy_keeps_the_remainder_and_can_fire_several_times() {
    let mut g = goal(GoalKind::Likes, 100, OnReach::Reset);
    assert_eq!(add(&mut g, 250), 2);
    assert_eq!((g.current, g.reached_count), (50, 2));
    assert_eq!(add(&mut g, 50), 1);
    assert_eq!(g.current, 0);
}

#[test]
fn extend_policy_raises_the_target() {
    let mut g = goal(GoalKind::Coins, 100, OnReach::Extend { add: 50 });
    assert_eq!(add(&mut g, 100), 1);
    assert_eq!((g.current, g.target), (100, 150));
    assert_eq!(add(&mut g, 50), 1);
    assert_eq!((g.current, g.target), (150, 200));
    // Un golpe grande cruza todos los objetivos intermedios: 200, 250, 300, … , 550.
    assert_eq!(add(&mut g, 400), 8);
    assert_eq!((g.current, g.target), (550, 600));
}

#[test]
fn degenerate_goals_cannot_loop_forever() {
    let mut tiny = goal(GoalKind::Likes, 1, OnReach::Reset);
    assert_eq!(add(&mut tiny, 1_000_000), MAX_REACHES_PER_ADD);
    let mut zero_extend = goal(GoalKind::Likes, 1, OnReach::Extend { add: 0 });
    assert!(add(&mut zero_extend, 10) <= MAX_REACHES_PER_ADD);
    let mut huge = goal(GoalKind::Likes, 10, OnReach::Stop);
    assert_eq!(add(&mut huge, u64::MAX), 1);
    assert_eq!(add(&mut huge, u64::MAX), 0, "saturating_add: no desborda");
}

#[test]
fn adding_zero_does_nothing() {
    let mut g = goal(GoalKind::Likes, 10, OnReach::Stop);
    assert_eq!(add(&mut g, 0), 0);
    assert_eq!(g.current, 0);
}

#[test]
fn adjust_supports_negative_deltas_without_firing() {
    let mut g = goal(GoalKind::Likes, 100, OnReach::Stop);
    assert_eq!(adjust(&mut g, 90), 0);
    assert_eq!(adjust(&mut g, -30), 0);
    assert_eq!(g.current, 60);
    assert_eq!(adjust(&mut g, -1_000), 0);
    assert_eq!(g.current, 0, "no baja de cero");
    assert_eq!(adjust(&mut g, 100), 1);
}

#[test]
fn percent_is_clamped() {
    let mut g = goal(GoalKind::Likes, 200, OnReach::Stop);
    g.current = 50;
    assert_eq!(g.percent(), 25.0);
    g.current = 999;
    assert_eq!(g.percent(), 100.0);
}

#[test]
fn validation() {
    let ok = goal(GoalKind::Likes, 10, OnReach::Stop);
    assert!(ok.validate().is_ok());
    assert!(Goal { id: " ".into(), ..ok.clone() }.validate().is_err());
    assert!(Goal { name: "".into(), ..ok.clone() }.validate().is_err());
    assert!(Goal { name: "x".repeat(81), ..ok.clone() }.validate().is_err());
    assert!(Goal { target: 0, ..ok.clone() }.validate().is_err());
    assert!(Goal { on_reach: OnReach::Extend { add: 0 }, ..ok.clone() }.validate().is_err());
    assert!(Goal { kind: GoalKind::Gift { gift_id: None, gift_name: None }, ..ok.clone() }.validate().is_err());
    assert!(Goal { kind: GoalKind::Gift { gift_id: None, gift_name: Some("  ".into()) }, ..ok.clone() }.validate().is_err());
    assert!(Goal { kind: GoalKind::Gift { gift_id: Some(5), gift_name: None }, ..ok }.validate().is_ok());
}

#[test]
fn json_shape_roundtrips_and_fills_defaults() {
    let g: Goal = serde_json::from_str(
        r#"{"id":"a","name":"Likes","kind":{"type":"gift","giftName":"Rose"},"target":50,"onReach":{"type":"extend","add":25}}"#,
    )
    .expect("parsea");
    assert_eq!(g.current, 0);
    assert!(!g.reset_on_session);
    assert_eq!(g.on_reach, OnReach::Extend { add: 25 });
    let back: Goal = serde_json::from_str(&serde_json::to_string(&g).expect("ser")).expect("de");
    assert_eq!(back, g);
    let minimal: Goal = serde_json::from_str(r#"{"id":"b","name":"x","kind":{"type":"likes"},"target":1}"#).expect("parsea");
    assert_eq!(minimal.on_reach, OnReach::Stop);
}
