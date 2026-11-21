//! Simulador de eventos falsos, para probar overlays y reglas sin estar en vivo.
//! Los eventos ya salen normalizados y se publican directamente en el bus.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use crate::bus::EventBus;
use std::sync::Arc;

use crate::events::{Chat, EventType, Gift, Like, LiveEvent, Platform, User};
use crate::gifts::catalog::Catalog;
use crate::gifts::service::GiftService;
use crate::gifts::GiftView;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SimKind {
    /// Regalo combinable (racha), p. ej. una Rosa ×N.
    Gift,
    /// Regalo grande NO combinable (Galaxy, León, Universo…).
    BigGift,
    Chat,
    Like,
    Follow,
    Share,
    Subscribe,
    Join,
}

/// (nombre, monedas por unidad, combinable)
const GIFTS: &[(&str, u64, bool)] = &[
    ("Rose", 1, true),
    ("TikTok", 1, true),
    ("Finger Heart", 5, true),
    ("Doughnut", 30, true),
];
const BIG_GIFTS: &[(&str, u64)] = &[("Galaxy", 1_000), ("Lion", 29_999), ("Universe", 34_999)];
const USERS: &[(&str, &str)] = &[
    ("luna_sim", "Luna"),
    ("pixel_sim", "Pixel"),
    ("nova_sim", "Nova"),
    ("trueno_sim", "Trueno"),
    ("miel_sim", "Miel"),
];
const MESSAGES: &[&str] = &[
    "¡Hola a todos!",
    "Qué buen directo 🔥",
    "!puntos",
    "jajaja eso estuvo genial",
    "saludos desde México",
    "!tts hola chat",
];

fn pick<T>(items: &[T]) -> &T {
    // `items` nunca está vacío (constantes de arriba).
    &items[rand::random_range(0..items.len())]
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn random_user() -> User {
    let (unique_id, nickname) = pick(USERS);
    User {
        id: format!("sim-{unique_id}"),
        unique_id: (*unique_id).to_string(),
        nickname: (*nickname).to_string(),
        avatar: String::new(),
        is_moderator: rand::random_bool(0.1),
        is_subscriber: rand::random_bool(0.3),
        is_follower: rand::random_bool(0.6),
        team_level: rand::random_bool(0.4).then(|| rand::random_range(1..=10)),
        gifter_level: rand::random_bool(0.4).then(|| rand::random_range(1..=40)),
    }
}

/// Evento falso de TikTok.
pub fn build_event(kind: SimKind) -> LiveEvent {
    build_event_for(kind, Platform::Tiktok)
}

/// Evento falso de la plataforma indicada. Twitch no tiene likes, shares ni entradas, y sus «regalos» son
/// bits: esas pruebas se adaptan a lo que Twitch realmente puede enviar.
pub fn build_event_for(kind: SimKind, platform: Platform) -> LiveEvent {
    build_event_from(kind, platform, &Catalog::default())
}

/// Un regalo real del catálogo para simular: pequeño (hasta 99 💎, combinable) o grande (desde 1000 💎).
fn real_gift(catalog: &Catalog, big: bool) -> Option<&GiftView> {
    let fits = |g: &&GiftView| g.id.is_some() && g.diamonds.is_some_and(|d| if big { d >= 1_000 } else { (1..=99).contains(&d) });
    let n = catalog.gifts.iter().filter(fits).count();
    if n == 0 {
        return None;
    }
    catalog.gifts.iter().filter(fits).nth(rand::random_range(0..n))
}

fn sim_gift(g: &GiftView, big: bool) -> Gift {
    let count: u32 = if big { 1 } else { rand::random_range(1..=30) };
    Gift {
        id: g.id.unwrap_or_default(),
        // Llega con el nombre que manda TikTok (el oficial, en inglés).
        name: g.name_en.clone().or_else(|| g.name_es.clone()).unwrap_or_else(|| g.display_name.clone()),
        coins: g.diamonds.unwrap_or(1).saturating_mul(u64::from(count)),
        count,
        streakable: !big,
        image: g.icon_url.clone(),
    }
}

/// Como [`build_event_for`], pero con regalos reales del catálogo (id, nombre, precio e icono) si los hay.
pub fn build_event_from(kind: SimKind, platform: Platform, catalog: &Catalog) -> LiveEvent {
    let kind = match (platform, kind) {
        (Platform::Twitch, SimKind::Like | SimKind::Share | SimKind::Join) => SimKind::Chat,
        (_, k) => k,
    };
    let twitch_bits = platform == Platform::Twitch && matches!(kind, SimKind::Gift | SimKind::BigGift);
    let real = match kind {
        SimKind::Gift | SimKind::BigGift if !twitch_bits => real_gift(catalog, kind == SimKind::BigGift).map(|g| sim_gift(g, kind == SimKind::BigGift)),
        _ => None,
    };
    let (event_type, gift, chat, like) = match kind {
        SimKind::Gift | SimKind::BigGift if twitch_bits => {
            let bits: u64 = if kind == SimKind::Gift { rand::random_range(1..=300) } else { rand::random_range(1_000..=10_000) };
            let gift = Gift { id: 0, name: "Bits".to_string(), coins: bits, count: 1, streakable: false, image: String::new() };
            (EventType::Gift, Some(gift), None, None)
        }
        SimKind::Gift | SimKind::BigGift if real.is_some() => (EventType::Gift, real, None, None),
        SimKind::Gift => {
            let (name, unit, _) = *pick(GIFTS);
            let count: u32 = rand::random_range(1..=30);
            let gift = Gift {
                id: 1000 + i64::try_from(unit).unwrap_or(0),
                name: name.to_string(),
                coins: unit * u64::from(count),
                count,
                streakable: true,
                image: String::new(),
            };
            (EventType::Gift, Some(gift), None, None)
        }
        SimKind::BigGift => {
            let (name, unit) = *pick(BIG_GIFTS);
            let gift = Gift {
                id: 9000 + i64::try_from(unit).unwrap_or(0),
                name: name.to_string(),
                coins: unit,
                count: 1,
                streakable: false,
                image: String::new(),
            };
            (EventType::Gift, Some(gift), None, None)
        }
        SimKind::Chat => {
            let chat = Chat {
                text: (*pick(MESSAGES)).to_string(),
                emotes: None,
            };
            (EventType::Chat, None, Some(chat), None)
        }
        SimKind::Like => {
            let count: u64 = rand::random_range(1..=15);
            let like = Like {
                count,
                total: rand::random_range(100..=5_000) + count,
            };
            (EventType::Like, None, None, Some(like))
        }
        SimKind::Follow => (EventType::Follow, None, None, None),
        SimKind::Share => (EventType::Share, None, None, None),
        SimKind::Subscribe => (EventType::Subscribe, None, None, None),
        SimKind::Join => (EventType::Join, None, None, None),
    };

    let mut user = random_user();
    if event_type == EventType::Subscribe {
        user.is_subscriber = true;
    }
    if platform == Platform::Twitch {
        // Twitch no expone niveles de equipo/donador ni si alguien te sigue desde el chat.
        user.team_level = None;
        user.gifter_level = None;
        user.is_follower = false;
        user.id = format!("sim-tw:{}", user.unique_id);
    }
    LiveEvent {
        id: format!("{}{}", crate::events::SIMULATED_ID_PREFIX, uuid::Uuid::new_v4()),
        platform,
        kind: event_type,
        user,
        gift,
        chat,
        like,
        ts: now_ms(),
    }
}

#[derive(Clone)]
pub struct Simulator {
    bus: EventBus,
    gifts: Option<Arc<GiftService>>,
}

impl Simulator {
    pub fn new(bus: EventBus) -> Self {
        Self { bus, gifts: None }
    }

    /// Simula regalos reales del catálogo (los mismos ids y nombres que mandaría TikTok).
    #[must_use]
    pub fn with_gifts(mut self, gifts: Arc<GiftService>) -> Self {
        self.gifts = Some(gifts);
        self
    }

    fn build(&self, kind: SimKind, platform: Platform) -> LiveEvent {
        match &self.gifts {
            Some(g) => build_event_from(kind, platform, &g.catalog()),
            None => build_event_for(kind, platform),
        }
    }

    pub fn emit(&self, kind: SimKind) -> LiveEvent {
        self.emit_for(kind, Platform::Tiktok)
    }

    pub fn emit_for(&self, kind: SimKind, platform: Platform) -> LiveEvent {
        let ev = self.build(kind, platform);
        self.bus.publish(ev.clone());
        ev
    }

    /// Emite `count` eventos variados, espaciados, para simular actividad real.
    pub fn burst(&self, count: u32, spacing: Duration) -> tokio::task::JoinHandle<()> {
        self.burst_for(count, spacing, Platform::Tiktok)
    }

    pub fn burst_for(&self, count: u32, spacing: Duration, platform: Platform) -> tokio::task::JoinHandle<()> {
        const MIX: &[SimKind] = &[
            SimKind::Chat,
            SimKind::Chat,
            SimKind::Like,
            SimKind::Gift,
            SimKind::Join,
            SimKind::Follow,
            SimKind::Share,
            SimKind::BigGift,
            SimKind::Subscribe,
        ];
        let this = self.clone();
        tokio::spawn(async move {
            for _ in 0..count {
                this.emit_for(*pick(MIX), platform);
                tokio::time::sleep(spacing).await;
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const ALL: &[SimKind] = &[
        SimKind::Gift,
        SimKind::BigGift,
        SimKind::Chat,
        SimKind::Like,
        SimKind::Follow,
        SimKind::Share,
        SimKind::Subscribe,
        SimKind::Join,
    ];

    #[test]
    fn every_kind_builds_a_consistent_event() {
        for &k in ALL {
            for _ in 0..50 {
                let ev = build_event(k);
                assert!(ev.id.starts_with("sim-"));
                assert!(!ev.user.unique_id.is_empty());
                match ev.kind {
                    EventType::Gift => {
                        let g = ev.gift.as_ref().expect("gift");
                        assert!(g.count >= 1 && g.coins >= g.count as u64);
                        assert_eq!(g.streakable, k == SimKind::Gift);
                        if !g.streakable {
                            assert_eq!(g.count, 1);
                        }
                    }
                    EventType::Chat => assert!(ev.chat.is_some()),
                    EventType::Like => assert!(ev.like.as_ref().expect("like").count >= 1),
                    EventType::Subscribe => assert!(ev.user.is_subscriber),
                    _ => {}
                }
            }
        }
    }

    #[test]
    fn gifts_come_from_the_catalog_when_there_is_one() {
        let catalog = crate::gifts::testing::catalog();
        for _ in 0..30 {
            let small = build_event_from(SimKind::Gift, Platform::Tiktok, &catalog).gift.expect("regalo");
            assert!([5655, 5566, 5582, 7000].contains(&small.id), "{small:?}");
            assert!(small.streakable && small.coins >= u64::from(small.count));
            let big = build_event_from(SimKind::BigGift, Platform::Tiktok, &catalog).gift.expect("regalo");
            assert_eq!((big.id, big.name.as_str(), big.coins, big.count), (7001, "Freestyle", 1800, 1));
            let bits = build_event_from(SimKind::Gift, Platform::Twitch, &catalog).gift.expect("bits");
            assert_eq!(bits.name, "Bits", "Twitch sigue mandando bits");
        }
    }

    #[test]
    fn twitch_events_only_contain_what_twitch_can_send() {
        for &k in ALL {
            for _ in 0..30 {
                let ev = build_event_for(k, Platform::Twitch);
                assert_eq!(ev.platform, Platform::Twitch);
                assert!(ev.id.starts_with("sim-") && ev.user.id.starts_with("sim-tw:"));
                assert!(ev.user.team_level.is_none() && ev.user.gifter_level.is_none() && !ev.user.is_follower);
                assert!(!matches!(ev.kind, EventType::Like | EventType::Share | EventType::Join), "{:?}", ev.kind);
                if let Some(g) = &ev.gift {
                    assert_eq!((g.name.as_str(), g.count, g.streakable), ("Bits", 1, false));
                }
            }
        }
        assert_eq!(build_event(SimKind::Chat).platform, Platform::Tiktok);
    }

    #[test]
    fn ids_are_unique() {
        let ids: HashSet<_> = (0..200).map(|_| build_event(SimKind::Chat).id).collect();
        assert_eq!(ids.len(), 200);
    }

    #[test]
    fn simkind_deserializes_from_camel_case() {
        let k: SimKind = serde_json::from_str("\"bigGift\"").expect("parse");
        assert_eq!(k, SimKind::BigGift);
    }

    #[tokio::test]
    async fn emit_publishes_to_the_bus() {
        let bus = EventBus::new(8);
        let mut rx = bus.subscribe();
        let sim = Simulator::new(bus);
        let sent = sim.emit(SimKind::Follow);
        assert_eq!(rx.recv().await.expect("event").id, sent.id);
    }

    #[tokio::test(start_paused = true)]
    async fn burst_emits_the_requested_count() {
        let bus = EventBus::new(64);
        let mut rx = bus.subscribe();
        let sim = Simulator::new(bus);
        sim.burst(5, Duration::from_millis(100)).await.expect("join");
        let mut n = 0;
        while rx.try_recv().is_ok() {
            n += 1;
        }
        assert_eq!(n, 5);
    }
}
