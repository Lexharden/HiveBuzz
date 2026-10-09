//! Esquema normalizado de eventos. Espejo de `sidecar/src/types.ts`.

use serde::{Deserialize, Serialize};

/// De qué plataforma viene un evento o una conexión.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    #[default]
    Tiktok,
    Twitch,
}

impl Platform {
    pub const ALL: [Platform; 2] = [Platform::Tiktok, Platform::Twitch];

    pub fn as_str(self) -> &'static str {
        match self {
            Platform::Tiktok => "tiktok",
            Platform::Twitch => "twitch",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EventType {
    Gift,
    Chat,
    Like,
    Follow,
    Share,
    Subscribe,
    Join,
    Emote,
    LiveEnd,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub id: String,
    pub unique_id: String,
    pub nickname: String,
    pub avatar: String,
    pub is_moderator: bool,
    pub is_subscriber: bool,
    pub is_follower: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_level: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gifter_level: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Gift {
    pub id: i64,
    pub name: String,
    pub coins: u64,
    pub count: u32,
    pub streakable: bool,
    pub image: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Emote {
    pub id: String,
    pub image: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Chat {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emotes: Option<Vec<Emote>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Like {
    pub count: u64,
    pub total: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveEvent {
    /// Id del mensaje en su plataforma (`msgId` de TikTok, `id` del mensaje de Twitch); sirve para deduplicar.
    pub id: String,
    /// Plataforma de origen. La estampa la conexión; el sidecar de TikTok no la envía (por eso el valor por defecto).
    #[serde(default)]
    pub platform: Platform,
    #[serde(rename = "type")]
    pub kind: EventType,
    pub user: User,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gift: Option<Gift>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat: Option<Chat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub like: Option<Like>,
    /// Milisegundos desde epoch.
    pub ts: i64,
}

/// Prefijo del id de los eventos generados por el simulador.
pub const SIMULATED_ID_PREFIX: &str = "sim-";

impl LiveEvent {
    /// ¿Lo generó el simulador? Los almacenes persistentes (espectadores, ranking histórico) lo ignoran
    /// para que probar overlays no contamine los datos reales.
    pub fn is_simulated(&self) -> bool {
        self.id.starts_with(SIMULATED_ID_PREFIX)
    }
}

/// Utilidades compartidas por los tests de otros módulos.
#[cfg(test)]
pub mod testing {
    use super::*;

    pub fn sample_user() -> User {
        User {
            id: "1".into(),
            unique_id: "ana".into(),
            nickname: "Ana".into(),
            avatar: String::new(),
            is_moderator: false,
            is_subscriber: false,
            is_follower: false,
            team_level: None,
            gifter_level: None,
        }
    }

    pub fn sample_event(id: &str) -> LiveEvent {
        LiveEvent {
            id: id.into(),
            platform: Platform::Tiktok,
            kind: EventType::Chat,
            user: sample_user(),
            gift: None,
            chat: Some(Chat {
                text: "hola".into(),
                emotes: None,
            }),
            like: None,
            ts: 1_700_000_000_000,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_gift_event_from_ts_shape() {
        let json = r#"{
            "id": "123", "type": "gift", "ts": 1700000000000,
            "user": {"id":"1","uniqueId":"ana","nickname":"Ana","avatar":"",
                     "isModerator":false,"isSubscriber":true,"isFollower":true,"gifterLevel":7},
            "gift": {"id":5655,"name":"Rose","coins":1,"count":3,"streakable":true,"image":""}
        }"#;
        let ev: LiveEvent = serde_json::from_str(json).expect("debe parsear");
        assert_eq!(ev.kind, EventType::Gift);
        assert_eq!(ev.user.gifter_level, Some(7));
        assert_eq!(ev.gift.expect("gift").count, 3);
    }

    #[test]
    fn simulated_events_are_recognised_by_their_id() {
        let mut e = testing::sample_event("sim-123");
        assert!(e.is_simulated());
        e.id = "7123456789".into();
        assert!(!e.is_simulated());
    }

    #[test]
    fn live_end_uses_camel_case_and_omits_empty_options() {
        let ev = LiveEvent {
            id: "x".into(),
            platform: Platform::Tiktok,
            kind: EventType::LiveEnd,
            user: User {
                id: "1".into(),
                unique_id: "a".into(),
                nickname: "A".into(),
                avatar: String::new(),
                is_moderator: false,
                is_subscriber: false,
                is_follower: false,
                team_level: None,
                gifter_level: None,
            },
            gift: None,
            chat: None,
            like: None,
            ts: 1,
        };
        let v = serde_json::to_value(&ev).expect("serializa");
        assert_eq!(v["type"], "liveEnd");
        assert!(v.get("gift").is_none());
        assert!(v["user"].get("teamLevel").is_none());
    }
}
