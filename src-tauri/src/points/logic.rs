//! Cuándo y cuántos puntos gana un espectador. Lógica pura: el reloj entra por parámetro.

use std::collections::HashMap;

use super::config::PointsConfig;
use crate::events::{EventType, LiveEvent};

const MINUTE_MS: i64 = 60_000;
/// Un espectador cuenta como «presente» durante al menos este tiempo tras su última señal.
const MIN_PRESENCE_MS: i64 = 10 * MINUTE_MS;

/// Quién es el espectador (lo mínimo para guardarlo en la base de datos).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Who {
    pub user_id: String,
    pub unique_id: String,
    pub nickname: String,
    pub avatar: String,
    pub is_subscriber: bool,
}

impl Who {
    pub fn from_event(ev: &LiveEvent) -> Option<Self> {
        let u = &ev.user;
        if u.id.trim().is_empty() {
            return None;
        }
        Some(Self {
            user_id: u.id.clone(),
            unique_id: u.unique_id.clone(),
            nickname: if u.nickname.is_empty() { u.unique_id.clone() } else { u.nickname.clone() },
            avatar: u.avatar.clone(),
            is_subscriber: u.is_subscriber,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    Watch,
    Comment,
    Like,
    Share,
    Follow,
    Subscribe,
    Gift,
    /// Canje de una recompensa.
    Spend(String),
    Manual,
    Import,
    /// Solo estadísticas (la acción no da puntos).
    Activity,
}

impl Reason {
    pub fn label(&self) -> String {
        match self {
            Self::Watch => "watch".into(),
            Self::Comment => "comment".into(),
            Self::Like => "like".into(),
            Self::Share => "share".into(),
            Self::Follow => "follow".into(),
            Self::Subscribe => "subscribe".into(),
            Self::Gift => "gift".into(),
            Self::Spend(name) => format!("spend:{name}"),
            Self::Manual => "manual".into(),
            Self::Import => "import".into(),
            Self::Activity => "activity".into(),
        }
    }
}

/// Incrementos de las estadísticas del espectador.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub comments: u64,
    pub likes: u64,
    pub shares: u64,
    pub coins_gifted: u64,
    pub watch_minutes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Award {
    pub who: Who,
    pub delta: i64,
    pub reason: Reason,
    pub stats: Stats,
    pub ts: i64,
}

struct Presence {
    who: Who,
    last_seen: i64,
}

#[derive(Default)]
pub struct Awarder {
    last_comment_ms: HashMap<String, i64>,
    like_acc: HashMap<String, u64>,
    presence: HashMap<String, Presence>,
}

/// Aplica el multiplicador de suscriptor y redondea al entero más cercano.
fn scaled(base: f64, cfg: &PointsConfig, who: &Who) -> i64 {
    let mult = if who.is_subscriber { cfg.subscriber_multiplier } else { 1.0 };
    #[allow(clippy::cast_possible_truncation)]
    let v = (base * mult).round().clamp(0.0, 1.0e12) as i64;
    v
}

fn to_f64(n: u64) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let f = n as f64;
    f
}

impl Awarder {
    pub fn on_event(&mut self, cfg: &PointsConfig, ev: &LiveEvent, now_ms: i64) -> Vec<Award> {
        if !cfg.enabled {
            return Vec::new();
        }
        let Some(who) = Who::from_event(ev) else {
            return Vec::new();
        };
        let mk = |delta: i64, reason: Reason, stats: Stats| Award { who: who.clone(), delta, reason, stats, ts: now_ms };

        // Cualquier señal de un espectador lo marca como presente (para «puntos por ver»).
        if matches!(ev.kind, EventType::Chat | EventType::Like | EventType::Gift | EventType::Follow | EventType::Share | EventType::Join | EventType::Subscribe | EventType::Emote) {
            self.presence.insert(who.user_id.clone(), Presence { who: who.clone(), last_seen: now_ms });
        }

        match ev.kind {
            EventType::Chat => {
                let Some(chat) = &ev.chat else { return Vec::new() };
                // Los comandos (`!puntos`…) no puntúan ni cuentan como comentario: evita farmear puntos.
                if chat.text.trim_start().starts_with('!') {
                    return Vec::new();
                }
                let stats = Stats { comments: 1, ..Stats::default() };
                let ready = self
                    .last_comment_ms
                    .get(&who.user_id)
                    .is_none_or(|last| now_ms.saturating_sub(*last) >= i64::try_from(cfg.comment_cooldown_ms).unwrap_or(i64::MAX));
                if ready && cfg.comment_points > 0 {
                    self.last_comment_ms.insert(who.user_id.clone(), now_ms);
                    vec![mk(scaled(to_f64(cfg.comment_points), cfg, &who), Reason::Comment, stats)]
                } else {
                    vec![mk(0, Reason::Activity, stats)]
                }
            }
            EventType::Like => {
                let count = ev.like.as_ref().map_or(0, |l| l.count);
                if count == 0 {
                    return Vec::new();
                }
                let acc = self.like_acc.entry(who.user_id.clone()).or_insert(0);
                *acc = acc.saturating_add(count);
                let steps = *acc / cfg.like_every.max(1);
                *acc %= cfg.like_every.max(1);
                let delta = scaled(to_f64(steps.saturating_mul(cfg.like_points)), cfg, &who);
                let stats = Stats { likes: count, ..Stats::default() };
                vec![mk(delta, if delta > 0 { Reason::Like } else { Reason::Activity }, stats)]
            }
            EventType::Share => vec![mk(scaled(to_f64(cfg.share_points), cfg, &who), Reason::Share, Stats { shares: 1, ..Stats::default() })],
            EventType::Follow => vec![mk(scaled(to_f64(cfg.follow_points), cfg, &who), Reason::Follow, Stats::default())],
            EventType::Subscribe => {
                // Quien se suscribe ya cuenta como suscriptor para el bonus de esta misma acción.
                let mut w = who.clone();
                w.is_subscriber = true;
                vec![Award { who: w.clone(), delta: scaled(to_f64(cfg.subscribe_points), cfg, &w), reason: Reason::Subscribe, stats: Stats::default(), ts: now_ms }]
            }
            EventType::Gift => {
                let coins = ev.gift.as_ref().map_or(0, |g| g.coins);
                if coins == 0 {
                    return Vec::new();
                }
                let delta = scaled(to_f64(coins) * cfg.points_per_coin, cfg, &who);
                vec![mk(delta, Reason::Gift, Stats { coins_gifted: coins, ..Stats::default() })]
            }
            _ => Vec::new(),
        }
    }

    /// Reparte los puntos por ver entre los espectadores presentes. Llamar cada `watch_interval_minutes`.
    pub fn on_watch_tick(&mut self, cfg: &PointsConfig, now_ms: i64) -> Vec<Award> {
        if !cfg.enabled || cfg.watch_points == 0 {
            return Vec::new();
        }
        let interval = i64::try_from(cfg.watch_interval_minutes).unwrap_or(1).saturating_mul(MINUTE_MS);
        let window = (interval.saturating_mul(2)).max(MIN_PRESENCE_MS);
        self.presence
            .values()
            .filter(|p| now_ms.saturating_sub(p.last_seen) <= window)
            .map(|p| Award {
                who: p.who.clone(),
                delta: scaled(to_f64(cfg.watch_points), cfg, &p.who),
                reason: Reason::Watch,
                stats: Stats { watch_minutes: cfg.watch_interval_minutes, ..Stats::default() },
                ts: now_ms,
            })
            .collect()
    }

    /// Descarta lo que ya no sirve (presencias antiguas, cooldowns vencidos) para no crecer sin límite.
    pub fn prune(&mut self, cfg: &PointsConfig, now_ms: i64) {
        let keep = (i64::try_from(cfg.watch_interval_minutes).unwrap_or(1).saturating_mul(MINUTE_MS).saturating_mul(2)).max(MIN_PRESENCE_MS);
        self.presence.retain(|_, p| now_ms.saturating_sub(p.last_seen) <= keep);
        let cd = i64::try_from(cfg.comment_cooldown_ms).unwrap_or(i64::MAX);
        self.last_comment_ms.retain(|_, t| now_ms.saturating_sub(*t) < cd);
    }

    /// Cuántos espectadores se consideran presentes ahora.
    pub fn present(&self, cfg: &PointsConfig, now_ms: i64) -> usize {
        let window = (i64::try_from(cfg.watch_interval_minutes).unwrap_or(1).saturating_mul(MINUTE_MS).saturating_mul(2)).max(MIN_PRESENCE_MS);
        self.presence.values().filter(|p| now_ms.saturating_sub(p.last_seen) <= window).count()
    }

    /// El LIVE terminó o empezó uno nuevo: nadie sigue «presente», y los acumulados de likes se reinician.
    pub fn reset_session(&mut self) {
        self.presence.clear();
        self.like_acc.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::testing::sample_event;
    use crate::events::{Chat, Gift, Like};

    fn cfg() -> PointsConfig {
        PointsConfig::default()
    }

    fn ev(kind: EventType) -> LiveEvent {
        let mut e = sample_event("e");
        e.kind = kind;
        e.chat = None;
        e
    }

    fn chat(text: &str) -> LiveEvent {
        let mut e = ev(EventType::Chat);
        e.chat = Some(Chat { text: text.into(), emotes: None });
        e
    }

    fn like(count: u64) -> LiveEvent {
        let mut e = ev(EventType::Like);
        e.like = Some(Like { count, total: 0 });
        e
    }

    fn gift(coins: u64) -> LiveEvent {
        let mut e = ev(EventType::Gift);
        e.gift = Some(Gift { id: 1, name: "Rose".into(), coins, count: 1, streakable: false, image: String::new() });
        e
    }

    fn deltas(a: &[Award]) -> Vec<i64> {
        a.iter().map(|x| x.delta).collect()
    }

    #[test]
    fn comments_give_points_with_an_anti_spam_cooldown() {
        let mut aw = Awarder::default();
        let c = cfg();
        let first = aw.on_event(&c, &chat("hola"), 0);
        assert_eq!((deltas(&first), first[0].reason.clone()), (vec![2], Reason::Comment));
        let spam = aw.on_event(&c, &chat("hola otra vez"), 5_000);
        assert_eq!(deltas(&spam), [0], "dentro del cooldown: sin puntos");
        assert_eq!(spam[0].reason, Reason::Activity);
        assert_eq!(spam[0].stats.comments, 1, "pero el comentario sí se cuenta");
        assert_eq!(deltas(&aw.on_event(&c, &chat("ya"), 30_000)), [2]);
    }

    #[test]
    fn commands_never_earn_points_or_count_as_comments() {
        let mut aw = Awarder::default();
        assert!(aw.on_event(&cfg(), &chat("!puntos"), 0).is_empty());
        assert!(aw.on_event(&cfg(), &chat("  !top"), 1).is_empty());
    }

    #[test]
    fn likes_accumulate_per_user_until_the_threshold() {
        let mut aw = Awarder::default();
        let c = cfg(); // 1 punto cada 50 likes
        assert_eq!(deltas(&aw.on_event(&c, &like(30), 0)), [0]);
        assert_eq!(deltas(&aw.on_event(&c, &like(30), 1)), [1], "60 → 1 paso, sobran 10");
        assert_eq!(deltas(&aw.on_event(&c, &like(140), 2)), [3], "150 → 3 pasos");
        let a = aw.on_event(&c, &like(5), 3);
        assert_eq!(a[0].stats.likes, 5);
    }

    #[test]
    fn follow_share_subscribe_and_gifts() {
        let mut aw = Awarder::default();
        let c = cfg();
        assert_eq!(deltas(&aw.on_event(&c, &ev(EventType::Follow), 0)), [50]);
        let share = aw.on_event(&c, &ev(EventType::Share), 0);
        assert_eq!((deltas(&share), share[0].stats.shares), (vec![20], 1));
        assert_eq!(deltas(&aw.on_event(&c, &ev(EventType::Subscribe), 0)), [500]);
        let g = aw.on_event(&c, &gift(120), 0);
        assert_eq!((deltas(&g), g[0].stats.coins_gifted, g[0].reason.clone()), (vec![120], 120, Reason::Gift));
    }

    #[test]
    fn fractional_points_per_coin_round() {
        let mut aw = Awarder::default();
        let c = PointsConfig { points_per_coin: 0.5, ..cfg() };
        assert_eq!(deltas(&aw.on_event(&c, &gift(101), 0)), [51]);
        let none = PointsConfig { points_per_coin: 0.0, ..cfg() };
        assert_eq!(deltas(&aw.on_event(&none, &gift(500), 0)), [0]);
    }

    #[test]
    fn subscribers_get_the_multiplier() {
        let mut aw = Awarder::default();
        let c = PointsConfig { subscriber_multiplier: 2.0, ..cfg() };
        let mut sub = ev(EventType::Follow);
        sub.user.is_subscriber = true;
        assert_eq!(deltas(&aw.on_event(&c, &sub, 0)), [100]);
        assert_eq!(deltas(&aw.on_event(&c, &ev(EventType::Follow), 0)), [50]);
        // Quien se suscribe cobra su propio bonus en esa acción.
        assert_eq!(deltas(&aw.on_event(&c, &ev(EventType::Subscribe), 0)), [1000]);
    }

    #[test]
    fn disabled_system_awards_nothing() {
        let mut aw = Awarder::default();
        let c = PointsConfig { enabled: false, ..cfg() };
        assert!(aw.on_event(&c, &gift(100), 0).is_empty());
        assert!(aw.on_watch_tick(&c, 0).is_empty());
    }

    #[test]
    fn anonymous_users_are_ignored() {
        let mut aw = Awarder::default();
        let mut e = gift(100);
        e.user.id = "  ".into();
        assert!(aw.on_event(&cfg(), &e, 0).is_empty());
    }

    #[test]
    fn watch_points_go_to_recently_active_viewers_only() {
        let mut aw = Awarder::default();
        let c = cfg(); // 5 puntos cada 5 min; ventana = 10 min
        aw.on_event(&c, &ev(EventType::Join), 0);
        let mut other = chat("hola");
        other.user.id = "2".into();
        other.user.unique_id = "beto".into();
        aw.on_event(&c, &other, 9 * MINUTE_MS);
        let tick = aw.on_watch_tick(&c, 11 * MINUTE_MS);
        assert_eq!(tick.len(), 1, "el primero lleva 11 min sin señales: ya no cuenta");
        assert_eq!((tick[0].who.unique_id.as_str(), tick[0].delta, tick[0].stats.watch_minutes), ("beto", 5, 5));
        assert_eq!(tick[0].reason, Reason::Watch);
    }

    #[test]
    fn a_join_alone_makes_a_viewer_present_without_earning_directly() {
        let mut aw = Awarder::default();
        assert!(aw.on_event(&cfg(), &ev(EventType::Join), 0).is_empty());
        assert_eq!(aw.present(&cfg(), 1), 1);
    }

    #[test]
    fn zero_watch_points_awards_nothing() {
        let mut aw = Awarder::default();
        let c = PointsConfig { watch_points: 0, ..cfg() };
        aw.on_event(&c, &chat("hola"), 0);
        assert!(aw.on_watch_tick(&c, 1).is_empty());
    }

    #[test]
    fn prune_and_reset_release_memory() {
        let mut aw = Awarder::default();
        let c = cfg();
        aw.on_event(&c, &chat("hola"), 0);
        aw.on_event(&c, &like(10), 0);
        aw.prune(&c, 60 * MINUTE_MS);
        assert_eq!(aw.present(&c, 60 * MINUTE_MS), 0);
        assert!(aw.last_comment_ms.is_empty());
        aw.on_event(&c, &chat("otra"), 61 * MINUTE_MS);
        aw.reset_session();
        assert_eq!(aw.present(&c, 61 * MINUTE_MS), 0);
        assert!(aw.like_acc.is_empty());
    }

    #[test]
    fn reason_labels_are_stable_strings_for_the_history() {
        assert_eq!(Reason::Spend("Sonido".into()).label(), "spend:Sonido");
        assert_eq!(Reason::Gift.label(), "gift");
    }
}
