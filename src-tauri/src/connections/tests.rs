use std::sync::Mutex;

use async_trait::async_trait;
use tokio::sync::mpsc;

use super::*;
use crate::bus::EventBus;
use crate::error::Result;
use crate::events::testing::sample_event;
use crate::source::protocol::ConnectionState;
use crate::source::{ConnectTarget, LiveSource, SourceMessage};

#[derive(Default)]
struct Fake {
    connected: Mutex<Vec<String>>,
}

#[async_trait]
impl LiveSource for Fake {
    async fn connect(&self, t: ConnectTarget) -> Result<()> {
        self.connected.lock().unwrap().push(t.unique_id);
        Ok(())
    }
    async fn disconnect(&self) -> Result<()> {
        Ok(())
    }
}

struct Rig {
    manager: ConnectionManager,
    tiktok_tx: mpsc::Sender<SourceMessage>,
    twitch_tx: mpsc::Sender<SourceMessage>,
    bus: EventBus,
    twitch_src: Arc<Fake>,
    tiktok_src: Arc<Fake>,
}

fn rig() -> Rig {
    let bus = EventBus::new(32);
    let (tiktok_src, twitch_src) = (Arc::new(Fake::default()), Arc::new(Fake::default()));
    let tiktok = ConnectionService::with_platform(tiktok_src.clone(), Platform::Tiktok);
    let twitch = ConnectionService::with_platform(twitch_src.clone(), Platform::Twitch);
    let (ttx, trx) = mpsc::channel(16);
    let (wtx, wrx) = mpsc::channel(16);
    tiktok.spawn_pump(trx, bus.clone());
    twitch.spawn_pump(wrx, bus.clone());
    Rig { manager: ConnectionManager::new(tiktok, twitch), tiktok_tx: ttx, twitch_tx: wtx, bus, twitch_src, tiktok_src }
}

#[tokio::test]
async fn each_connection_stamps_its_own_platform_on_every_event() {
    let r = rig();
    let mut events = r.bus.subscribe();
    // El conector se equivoca (o no sabe) de plataforma: la conexión manda.
    let mut wrong = sample_event("t1");
    wrong.platform = Platform::Tiktok;
    r.twitch_tx.send(SourceMessage::Event(Box::new(wrong))).await.unwrap();
    r.tiktok_tx.send(SourceMessage::Event(Box::new(sample_event("k1")))).await.unwrap();
    let a = events.recv().await.unwrap();
    let b = events.recv().await.unwrap();
    let by_id = |id: &str| [&a, &b].into_iter().find(|e| e.id == id).unwrap().platform;
    assert_eq!(by_id("t1"), Platform::Twitch);
    assert_eq!(by_id("k1"), Platform::Tiktok);
}

#[tokio::test]
async fn each_platform_normalizes_names_its_own_way() {
    let r = rig();
    // «Pokimane» es un canal de Twitch válido; «mi-canal» no es un usuario de TikTok.
    r.manager.service(Platform::Twitch).connect("https://twitch.tv/Pokimane", None, None).await.unwrap();
    assert!(r.manager.service(Platform::Tiktok).connect("mi-canal", None, None).await.is_err());
    r.manager.service(Platform::Tiktok).connect("@ana_99", None, None).await.unwrap();
    assert_eq!(*r.twitch_src.connected.lock().unwrap(), ["pokimane"]);
    assert_eq!(*r.tiktok_src.connected.lock().unwrap(), ["ana_99"]);
    assert!(r.manager.service(Platform::Twitch).connect("a b", None, None).await.is_err());
}

#[tokio::test]
async fn statuses_are_reported_per_platform() {
    let r = rig();
    let mut st = r.manager.service(Platform::Twitch).subscribe_status();
    r.twitch_tx.send(SourceMessage::Status(StatusUpdate::new(ConnectionState::Connected))).await.unwrap();
    st.changed().await.unwrap();
    let all = r.manager.statuses();
    assert_eq!(all.len(), 2);
    let state_of = |p: Platform| all.iter().find(|s| s.platform == p).unwrap().status.state;
    assert_eq!(state_of(Platform::Twitch), ConnectionState::Connected);
    assert_eq!(state_of(Platform::Tiktok), ConnectionState::Disconnected);
    let json = serde_json::to_value(&all[1]).unwrap();
    assert_eq!(json["platform"], "twitch");
    assert_eq!(json["state"], "connected", "el estado va al nivel de la plataforma (flatten)");
}

#[tokio::test]
async fn viewers_are_added_up_across_platforms() {
    let r = rig();
    let mut total = r.manager.spawn_total_viewers();
    r.tiktok_tx.send(SourceMessage::Viewers(100)).await.unwrap();
    total.changed().await.unwrap();
    assert_eq!(*total.borrow_and_update(), 100);
    r.twitch_tx.send(SourceMessage::Viewers(25)).await.unwrap();
    total.changed().await.unwrap();
    assert_eq!(*total.borrow_and_update(), 125);
    // Twitch se queda sin directo: solo cuenta TikTok.
    r.twitch_tx.send(SourceMessage::Viewers(0)).await.unwrap();
    total.changed().await.unwrap();
    assert_eq!(*total.borrow_and_update(), 100);
}
