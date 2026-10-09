use super::*;
use crate::actions::clock::AppClock;
use crate::events::testing::sample_event;
use crate::events::{Gift, Like};

fn gift(id: &str, user: &str, name: &str, coins: u64, count: u32) -> LiveEvent {
    let mut ev = sample_event(id);
    ev.kind = EventType::Gift;
    ev.user.id = user.into();
    ev.user.unique_id = format!("u_{user}");
    ev.user.nickname = format!("Nick {user}");
    ev.gift = Some(Gift { id: 1, name: name.into(), coins, count, streakable: true, image: String::new() });
    ev
}

fn plain(id: &str, kind: EventType) -> LiveEvent {
    let mut ev = sample_event(id);
    ev.kind = kind;
    ev
}

async fn svc() -> (Arc<StatsService>, Db) {
    let db = Db::open_memory().await.unwrap();
    (StatsService::new(db.clone(), Arc::new(AppClock::new())), db)
}

#[tokio::test]
async fn aggregates_coins_gifts_donors_and_counters() {
    let (s, _) = svc().await;
    s.on_event(&gift("1", "a", "Rose", 1, 1));
    s.on_event(&gift("2", "a", "Rose", 10, 10));
    s.on_event(&gift("3", "b", "Galaxy", 1000, 1));
    s.on_event(&plain("4", EventType::Chat));
    s.on_event(&plain("5", EventType::Follow));
    s.on_event(&plain("6", EventType::Share));
    s.on_event(&plain("7", EventType::Subscribe));
    let mut like = plain("8", EventType::Like);
    like.like = Some(Like { count: 30, total: 30 });
    s.on_event(&like);
    s.on_viewers(40);
    s.on_viewers(25);
    s.on_viewers(55);
    let c = s.current();
    assert_eq!((c.coins, c.gifts_total, c.chats, c.likes, c.follows, c.shares, c.subscribers, c.peak_viewers), (1011, 12, 1, 30, 1, 1, 1, 55));
    assert_eq!(c.gifts.iter().map(|g| (g.name.as_str(), g.count, g.coins)).collect::<Vec<_>>(), [("Galaxy", 1, 1000), ("Rose", 11, 11)]);
    assert_eq!(c.donors.iter().map(|d| (d.user_id.as_str(), d.coins, d.gifts)).collect::<Vec<_>>(), [("b", 1000, 1), ("a", 11, 11)]);
    assert_eq!(c.donors[0].nickname, "Nick b");
}

#[tokio::test]
async fn simulated_events_do_not_count() {
    let (s, _) = svc().await;
    s.on_event(&gift("sim-1", "a", "Rose", 100, 1));
    assert_eq!(s.current().coins, 0);
}

#[tokio::test]
async fn flush_persists_only_non_empty_dirty_streams_and_lists_them() {
    let (s, db) = svc().await;
    s.flush().await.unwrap();
    assert!(db.list_stream_stats(10).await.unwrap().is_empty(), "una transmisión vacía no se guarda");
    s.on_event(&gift("1", "a", "Rose", 5, 1));
    let list = s.list(10).await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].coins, 5);
    // Con cambios se actualiza la misma fila.
    s.on_event(&gift("2", "a", "Rose", 5, 1));
    assert_eq!(s.list(10).await.unwrap()[0].coins, 10);
    assert_eq!(s.list(10).await.unwrap().len(), 1);
    let full = s.get(list[0].id).await.unwrap();
    assert_eq!(full.donors.len(), 1);
}

#[tokio::test]
async fn a_new_session_closes_the_previous_stream_and_starts_clean() {
    let (s, _) = svc().await;
    s.on_event(&gift("1", "a", "Rose", 5, 1));
    let first = s.current().id;
    tokio::time::sleep(Duration::from_millis(5)).await;
    s.new_session().await;
    assert_eq!(s.current().coins, 0);
    assert_ne!(s.current().id, first);
    let saved = s.get(first).await.unwrap();
    assert_eq!(saved.coins, 5);
    assert_eq!(s.list(10).await.unwrap().len(), 1, "la nueva, vacía, no aparece");
}

#[tokio::test]
async fn delete_removes_saved_streams_and_resets_the_current_one() {
    let (s, _) = svc().await;
    s.on_event(&gift("1", "a", "Rose", 5, 1));
    let id = s.list(10).await.unwrap()[0].id;
    assert!(s.delete(id).await.unwrap());
    assert!(s.list(10).await.unwrap().is_empty());
    assert_eq!(s.current().coins, 0, "borrar la en curso la reinicia");
    assert!(s.get(12345).await.is_err());
}

#[tokio::test]
async fn donors_are_capped_keeping_the_biggest() {
    let (s, _) = svc().await;
    for i in 0..(MAX_DONORS + 20) {
        s.on_event(&gift(&format!("g{i}"), &format!("{i}"), "Rose", (i as u64) + 1, 1));
    }
    let c = s.current();
    assert_eq!(c.donors.len(), MAX_DONORS);
    assert_eq!(c.donors[0].coins, (MAX_DONORS as u64) + 20);
}

#[tokio::test]
async fn spawn_wires_events_viewers_and_periodic_flush() {
    let (s, db) = svc().await;
    let bus = EventBus::new(64);
    let (vtx, vrx) = watch::channel(0u64);
    let sessions = crate::session::SessionService::new();
    s.spawn(&bus, vrx, sessions.subscribe(), Duration::from_millis(20));
    tokio::task::yield_now().await;
    bus.publish(gift("1", "a", "Rose", 7, 1));
    vtx.send_replace(33);
    tokio::time::sleep(Duration::from_millis(150)).await;
    let c = s.current();
    assert_eq!((c.coins, c.peak_viewers), (7, 33));
    assert_eq!(db.list_stream_stats(10).await.unwrap().len(), 1, "el volcado periódico guardó la transmisión");
}
