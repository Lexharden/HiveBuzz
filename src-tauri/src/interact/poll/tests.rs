use std::sync::atomic::{AtomicI64, Ordering};

use async_trait::async_trait;

use super::*;
use crate::bot::outbox::{ChatSender, Outbox, OutboxLimits};
use crate::db::Db;
use crate::events::testing::sample_event;
use crate::events::Chat;
use crate::points::service::PointsService;

#[test]
fn votes_are_numbers_with_optional_prefixes() {
    for (text, expected) in [
        ("1", Some(0)),
        (" 2 ", Some(1)),
        ("!3", Some(2)),
        ("!voto 2", Some(1)),
        ("!VOTE 1", Some(0)),
        ("!v 3", Some(2)),
        ("voto 1", Some(0)),
        ("4", None),
        ("0", None),
        ("10", None),
        ("hola", None),
        ("1 2", None),
        ("quiero el 2", None),
        ("-1", None),
        ("1.5", None),
        ("", None),
        ("!voto", None),
        ("!voto dos", None),
        ("999", None),
    ] {
        assert_eq!(parse_vote(text, 3), expected, "{text:?}");
    }
}

struct Mute;

#[async_trait]
impl ChatSender for Mute {
    async fn send_chat(&self, _text: String) -> Result<()> {
        Ok(())
    }
}

struct TestClock(AtomicI64);

impl Clock for TestClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::Relaxed)
    }
}

struct Rig {
    poll: Arc<PollService>,
    hub: OverlayHub,
    clock: Arc<TestClock>,
}

async fn rig() -> Rig {
    let clock = Arc::new(TestClock(AtomicI64::new(1_000_000)));
    let db = Db::open_memory().await.expect("db");
    let hub = OverlayHub::new(16);
    let points = PointsService::new(db.clone(), clock.clone());
    let outbox = Outbox::start(Arc::new(Mute), clock.clone(), Duration::from_millis(1_000), OutboxLimits::default());
    let bot = BotService::new(db, clock.clone(), points, outbox);
    Rig { poll: PollService::new(hub.clone(), bot, clock.clone()), hub, clock }
}

fn opts(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_string()).collect()
}

fn vote(user: &str, text: &str) -> LiveEvent {
    let mut e = sample_event(&format!("v-{user}-{text}"));
    e.user.id = user.into();
    e.user.unique_id = user.into();
    e.chat = Some(Chat { text: text.into(), emotes: None });
    e
}

fn published(r: &Rig) -> serde_json::Value {
    r.hub.retained_for(CHANNEL).expect("retenido").data.clone()
}

#[tokio::test]
async fn starting_validates_and_publishes_the_initial_state() {
    let r = rig().await;
    assert!(r.poll.start("  ", &opts(&["a", "b"]), 60).is_err(), "sin pregunta");
    assert!(r.poll.start("¿?", &opts(&["a"]), 60).is_err(), "una sola opción");
    assert!(r.poll.start("¿?", &opts(&["a", " ", ""]), 60).is_err(), "las vacías no cuentan");
    let nine: Vec<String> = (1..=9).map(|i| i.to_string()).collect();
    assert!(r.poll.start("¿?", &nine, 60).is_err(), "demasiadas");
    assert!(r.hub.retained_for(CHANNEL).is_none(), "nada publicado si falló");

    let v = r.poll.start("¿Qué jugamos?", &opts(&["Minecraft", "Fortnite"]), 60).expect("start");
    assert_eq!((v.id, v.total, v.ended), (1, 0, false));
    assert_eq!(v.ends_at_ms, 1_060_000);
    let p = published(&r);
    assert_eq!(p["kind"], "poll");
    assert_eq!(p["poll"]["question"], "¿Qué jugamos?");
    assert_eq!(p["poll"]["options"][1]["label"], "Fortnite");
    assert_eq!(p["poll"]["endsAtMs"], 1_060_000);
}

#[tokio::test]
async fn the_duration_is_clamped() {
    let r = rig().await;
    assert_eq!(r.poll.start("q", &opts(&["a", "b"]), 0).expect("start").ends_at_ms, 1_000_000 + 5_000);
    assert_eq!(r.poll.start("q", &opts(&["a", "b"]), 999_999).expect("start").ends_at_ms, 1_000_000 + 3_600_000);
}

#[tokio::test]
async fn one_vote_per_person_which_they_can_change() {
    let r = rig().await;
    r.poll.start("q", &opts(&["a", "b", "c"]), 60).expect("start");
    assert!(r.poll.on_event(&vote("ana", "1")));
    assert!(!r.poll.on_event(&vote("ana", "1")), "el mismo voto otra vez no cambia nada");
    assert!(r.poll.on_event(&vote("ana", "!3")), "cambia de opinión");
    assert!(r.poll.on_event(&vote("beto", "3")));
    assert!(r.poll.on_event(&vote("cata", "!voto 2")));
    assert!(!r.poll.on_event(&vote("dani", "9")), "opción inexistente");
    assert!(!r.poll.on_event(&vote("dani", "hola")));
    let v = r.poll.current().expect("activa");
    assert_eq!(v.options.iter().map(|o| o.votes).collect::<Vec<_>>(), [0, 1, 2]);
    assert_eq!(v.total, 3);
}

#[tokio::test]
async fn non_chat_events_and_missing_polls_are_ignored() {
    let r = rig().await;
    assert!(!r.poll.on_event(&vote("ana", "1")), "sin encuesta");
    r.poll.start("q", &opts(&["a", "b"]), 60).expect("start");
    let mut gift = vote("ana", "1");
    gift.kind = EventType::Gift;
    assert!(!r.poll.on_event(&gift));
    let mut anon = vote("", "1");
    anon.user.unique_id = String::new();
    assert!(!r.poll.on_event(&anon), "sin identidad no se puede evitar el voto doble");
}

#[tokio::test]
async fn the_overlay_is_refreshed_only_when_votes_arrive() {
    let r = rig().await;
    r.poll.start("q", &opts(&["a", "b"]), 60).expect("start");
    let mut rx = r.hub.subscribe();
    r.poll.tick();
    assert!(rx.try_recv().is_err(), "sin votos nuevos no se publica");
    r.poll.on_event(&vote("ana", "2"));
    assert!(rx.try_recv().is_err(), "los votos se agrupan hasta el siguiente tick");
    r.poll.tick();
    let m = rx.try_recv().expect("publicado");
    assert_eq!(m.data["poll"]["options"][1]["votes"], 1);
    r.poll.tick();
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn it_ends_by_itself_and_reports_the_winner() {
    let r = rig().await;
    r.poll.start("q", &opts(&["a", "b"]), 30).expect("start");
    r.poll.on_event(&vote("ana", "2"));
    r.poll.on_event(&vote("beto", "2"));
    r.poll.on_event(&vote("cata", "1"));
    r.clock.0.store(1_000_000 + 29_999, Ordering::Relaxed);
    r.poll.tick();
    assert_eq!(published(&r)["poll"]["ended"], false);
    r.clock.0.store(1_000_000 + 30_000, Ordering::Relaxed);
    r.poll.tick();
    let p = published(&r);
    assert_eq!(p["poll"]["ended"], true);
    assert_eq!(p["poll"]["winners"], serde_json::json!([1]));
    assert_eq!(p["poll"]["total"], 3);
    assert!(!r.poll.on_event(&vote("dani", "1")), "ya cerrada, no admite votos");
}

#[tokio::test]
async fn ties_and_empty_polls_are_reported_honestly() {
    let r = rig().await;
    r.poll.start("q", &opts(&["a", "b", "c"]), 30).expect("start");
    r.poll.on_event(&vote("ana", "1"));
    r.poll.on_event(&vote("beto", "3"));
    let v = r.poll.stop().expect("activa");
    assert_eq!(v.winners, [0, 2]);
    assert!(PollService::result_text(&v).contains("Empate"));
    assert!(r.poll.stop().is_none(), "ya estaba cerrada");

    r.poll.start("q2", &opts(&["a", "b"]), 30).expect("start");
    let v = r.poll.stop().expect("activa");
    assert!(v.winners.is_empty());
    assert!(PollService::result_text(&v).contains("Nadie votó"));
}

#[tokio::test]
async fn a_new_poll_replaces_the_old_one_and_clear_empties_the_overlay() {
    let r = rig().await;
    r.poll.start("uno", &opts(&["a", "b"]), 30).expect("start");
    r.poll.on_event(&vote("ana", "1"));
    let v = r.poll.start("dos", &opts(&["x", "y"]), 30).expect("start");
    assert_eq!((v.id, v.total), (2, 0), "los votos viejos no pasan a la nueva");
    assert_eq!(r.poll.current().expect("activa").question, "dos");
    r.poll.clear();
    assert!(r.poll.current().is_none());
    assert_eq!(published(&r)["kind"], "none");
}

#[tokio::test]
async fn question_and_options_are_cleaned_and_bounded() {
    let r = rig().await;
    let long = "x".repeat(500);
    let v = r.poll.start(&format!("hola\n{long}"), &opts(&["a\tb", &long]), 30).expect("start");
    assert_eq!(v.question.chars().count(), MAX_QUESTION_CHARS);
    assert!(!v.question.contains('\n'));
    assert_eq!(v.options[0].label, "ab");
    assert_eq!(v.options[1].label.chars().count(), MAX_OPTION_CHARS);
}
