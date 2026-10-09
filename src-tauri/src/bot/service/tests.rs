use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Mutex as StdMutex;

use async_trait::async_trait;

use super::*;
use crate::bot::model::BotCommand;
use crate::bot::outbox::{ChatSender, Outbox, OutboxLimits};
use crate::events::testing::sample_event;
use crate::events::Chat;
use crate::rules::engine::{DeniedReport, FiredReport};

struct Clock(AtomicI64);

impl crate::actions::clock::Clock for Clock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::Relaxed)
    }
}

#[derive(Default)]
struct Sent(StdMutex<Vec<String>>);

struct Spy(Arc<Sent>);

#[async_trait]
impl ChatSender for Spy {
    async fn send_chat(&self, text: String) -> Result<()> {
        self.0 .0.lock().unwrap().push(text);
        Ok(())
    }
}

async fn rig() -> (Arc<BotService>, Arc<Sent>) {
    let clock = Arc::new(Clock(AtomicI64::new(1_000_000)));
    let db = Db::open_memory().await.unwrap();
    let points = PointsService::new(db.clone(), clock.clone());
    let sent = Arc::new(Sent::default());
    let outbox = Outbox::start(Arc::new(Spy(sent.clone())), clock.clone(), Duration::from_millis(1), OutboxLimits::default());
    let bot = BotService::new(db, clock, points, outbox);
    let mut cfg = BotConfig { enabled: true, min_interval_ms: 1, ..BotConfig::default() };
    cfg.commands.push(BotCommand { id: "1".into(), enabled: true, names: vec!["discord".into()], responses: vec!["discord.gg/x".into()], conditions: Default::default() });
    bot.set_config(cfg).await.unwrap();
    (bot, sent)
}

fn chat(id: &str, platform: Platform, text: &str) -> LiveEvent {
    let mut e = sample_event(id);
    e.platform = platform;
    e.chat = Some(Chat { text: text.into(), emotes: None });
    e
}

async fn settle() {
    tokio::time::sleep(Duration::from_millis(120)).await;
}

#[tokio::test]
async fn the_bot_answers_tiktok_commands() {
    let (bot, sent) = rig().await;
    bot.on_event(&chat("k1", Platform::Tiktok, "!discord")).await;
    settle().await;
    assert_eq!(*sent.0.lock().unwrap(), ["discord.gg/x"]);
}

#[tokio::test]
async fn the_bot_stays_silent_for_twitch_because_it_can_only_write_in_tiktok() {
    let (bot, sent) = rig().await;
    bot.on_event(&chat("t1", Platform::Twitch, "!discord")).await;
    settle().await;
    assert!(sent.0.lock().unwrap().is_empty(), "no debe contestar en el chat de TikTok a alguien de Twitch");
}

#[tokio::test]
async fn reward_confirmations_for_twitch_viewers_never_reach_the_tiktok_chat() {
    let (bot, sent) = rig().await;
    let fired = |platform| FiredReport {
        rule_id: "r".into(),
        rule_name: "Premio".into(),
        ts: 1,
        queued: true,
        user: Some("ana".into()),
        nickname: Some("Ana".into()),
        cost: Some(10),
        points_left: Some(5),
        platform,
    };
    bot.on_redeemed(&fired(Platform::Twitch));
    bot.on_denied(&DeniedReport { rule_id: "r".into(), rule_name: "Premio".into(), user: "ana".into(), nickname: "Ana".into(), cost: 10, have: 1, platform: Platform::Twitch });
    settle().await;
    assert!(sent.0.lock().unwrap().is_empty());
    bot.on_redeemed(&fired(Platform::Tiktok));
    settle().await;
    assert_eq!(sent.0.lock().unwrap().len(), 1, "en TikTok sí se confirma");
}
