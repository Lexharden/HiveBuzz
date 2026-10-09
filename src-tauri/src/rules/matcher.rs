//! ¿Un evento dispara este trigger? Lógica pura, sin estado salvo el acumulador de likes.

use super::model::Trigger;
use super::template::Vars;
use crate::events::{EventType, LiveEvent};

/// Lo que puede disparar una regla: un evento en vivo o un evento interno del sistema.
#[derive(Debug, Clone, Copy)]
pub enum TriggerInput<'a> {
    Live(&'a LiveEvent),
    GoalReached(&'a str),
    TimerEnded(&'a str),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TriggerMatch {
    /// Cuántas veces dispara (un golpe grande de likes puede cruzar varios umbrales).
    pub times: u32,
    /// Variables propias del trigger (`args`, `command`, `keyword`…).
    pub vars: Vars,
}

/// Tope de disparos por un solo evento de likes.
const MAX_LIKE_FIRINGS: u64 = 10;

pub fn match_trigger(trigger: &Trigger, input: &TriggerInput, like_acc: &mut u64) -> Option<TriggerMatch> {
    match (trigger, input) {
        (Trigger::GoalReached { goal_id }, TriggerInput::GoalReached(id)) => {
            (goal_id == id).then(|| one(Vars::new()))
        }
        (Trigger::TimerEnded { timer_id }, TriggerInput::TimerEnded(id)) => {
            (timer_id == id).then(|| one(Vars::new()))
        }
        (_, TriggerInput::Live(ev)) => match_live(trigger, ev, like_acc),
        _ => None,
    }
}

fn one(vars: Vars) -> TriggerMatch {
    TriggerMatch { times: 1, vars }
}

fn match_live(trigger: &Trigger, ev: &LiveEvent, like_acc: &mut u64) -> Option<TriggerMatch> {
    match trigger {
        Trigger::Gift {
            gift_id,
            gift_name,
            min_coins,
        } => {
            if ev.kind != EventType::Gift {
                return None;
            }
            let gift = ev.gift.as_ref()?;
            let ok = gift_id.is_none_or(|id| id == gift.id)
                && gift_name
                    .as_deref()
                    .is_none_or(|n| n.trim().eq_ignore_ascii_case(gift.name.trim()))
                && min_coins.is_none_or(|m| gift.coins >= m);
            ok.then(|| one(Vars::new()))
        }
        Trigger::Follow => (ev.kind == EventType::Follow).then(|| one(Vars::new())),
        Trigger::Share => (ev.kind == EventType::Share).then(|| one(Vars::new())),
        Trigger::Subscribe => (ev.kind == EventType::Subscribe).then(|| one(Vars::new())),
        Trigger::Join => (ev.kind == EventType::Join).then(|| one(Vars::new())),
        Trigger::SubEmote => {
            (ev.kind == EventType::Emote && ev.user.is_subscriber).then(|| one(Vars::new()))
        }
        Trigger::Like { every } => {
            if ev.kind != EventType::Like || *every == 0 {
                return None;
            }
            *like_acc = like_acc.saturating_add(ev.like.as_ref()?.count);
            let times = (*like_acc / every).min(MAX_LIKE_FIRINGS);
            if times == 0 {
                return None;
            }
            *like_acc %= every;
            Some(TriggerMatch {
                times: u32::try_from(times).unwrap_or(1),
                vars: Vars::new(),
            })
        }
        Trigger::Command { command } => {
            if ev.kind != EventType::Chat {
                return None;
            }
            let text = ev.chat.as_ref()?.text.trim();
            let (head, args) = match text.split_once(char::is_whitespace) {
                Some((h, a)) => (h, a.trim()),
                None => (text, ""),
            };
            let typed = head.strip_prefix('!')?;
            let wanted = command.trim().trim_start_matches('!');
            if wanted.is_empty() || !typed.eq_ignore_ascii_case(wanted) {
                return None;
            }
            let mut vars = Vars::new();
            vars.insert("command".into(), wanted.to_lowercase());
            vars.insert("args".into(), args.to_string());
            Some(one(vars))
        }
        Trigger::Keyword { keywords, whole_word } => {
            if ev.kind != EventType::Chat {
                return None;
            }
            let text = ev.chat.as_ref()?.text.to_lowercase();
            let found = keywords
                .iter()
                .map(|k| k.trim().to_lowercase())
                .filter(|k| !k.is_empty())
                .find(|k| {
                    if *whole_word {
                        text.split(|c: char| !c.is_alphanumeric()).any(|w| w == k)
                    } else {
                        text.contains(k.as_str())
                    }
                })?;
            let mut vars = Vars::new();
            vars.insert("keyword".into(), found);
            Some(one(vars))
        }
        Trigger::GoalReached { .. } | Trigger::TimerEnded { .. } | Trigger::Api { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::testing::sample_event;
    use crate::events::{Chat, Gift, Like};

    fn gift_ev(name: &str, id: i64, coins: u64) -> LiveEvent {
        let mut e = sample_event("g");
        e.kind = EventType::Gift;
        e.chat = None;
        e.gift = Some(Gift {
            id,
            name: name.into(),
            coins,
            count: 1,
            streakable: false,
            image: String::new(),
        });
        e
    }

    fn chat_ev(text: &str) -> LiveEvent {
        let mut e = sample_event("c");
        e.chat = Some(Chat {
            text: text.into(),
            emotes: None,
        });
        e
    }

    fn like_ev(count: u64) -> LiveEvent {
        let mut e = sample_event("l");
        e.kind = EventType::Like;
        e.chat = None;
        e.like = Some(Like { count, total: 0 });
        e
    }

    fn m(t: &Trigger, ev: &LiveEvent) -> Option<TriggerMatch> {
        match_trigger(t, &TriggerInput::Live(ev), &mut 0)
    }

    #[test]
    fn gift_by_name_id_and_min_coins() {
        let galaxy = gift_ev("Galaxy", 9, 1000);
        let by_name = Trigger::Gift { gift_id: None, gift_name: Some("galaxy".into()), min_coins: None };
        let by_id = Trigger::Gift { gift_id: Some(9), gift_name: None, min_coins: None };
        let by_coins = Trigger::Gift { gift_id: None, gift_name: None, min_coins: Some(1000) };
        let too_much = Trigger::Gift { gift_id: None, gift_name: None, min_coins: Some(1001) };
        let wrong_id = Trigger::Gift { gift_id: Some(8), gift_name: None, min_coins: None };
        assert!(m(&by_name, &galaxy).is_some());
        assert!(m(&by_id, &galaxy).is_some());
        assert!(m(&by_coins, &galaxy).is_some());
        assert!(m(&too_much, &galaxy).is_none());
        assert!(m(&wrong_id, &galaxy).is_none());
    }

    #[test]
    fn gift_criteria_combine_with_and() {
        let t = Trigger::Gift { gift_id: None, gift_name: Some("Rose".into()), min_coins: Some(10) };
        assert!(m(&t, &gift_ev("Rose", 1, 10)).is_some());
        assert!(m(&t, &gift_ev("Rose", 1, 9)).is_none());
        assert!(m(&t, &gift_ev("Lion", 1, 99)).is_none());
    }

    #[test]
    fn gift_trigger_ignores_non_gift_events() {
        let t = Trigger::Gift { gift_id: None, gift_name: None, min_coins: None };
        assert!(m(&t, &chat_ev("hola")).is_none());
    }

    #[test]
    fn simple_event_triggers() {
        let mut e = sample_event("x");
        e.chat = None;
        for (kind, trig) in [
            (EventType::Follow, Trigger::Follow),
            (EventType::Share, Trigger::Share),
            (EventType::Subscribe, Trigger::Subscribe),
            (EventType::Join, Trigger::Join),
        ] {
            e.kind = kind;
            assert!(m(&trig, &e).is_some());
            assert!(m(&Trigger::Follow, &chat_ev("x")).is_none());
        }
    }

    #[test]
    fn sub_emote_requires_a_subscriber() {
        let mut e = sample_event("x");
        e.kind = EventType::Emote;
        e.user.is_subscriber = false;
        assert!(m(&Trigger::SubEmote, &e).is_none());
        e.user.is_subscriber = true;
        assert!(m(&Trigger::SubEmote, &e).is_some());
    }

    #[test]
    fn likes_accumulate_and_fire_every_n() {
        let t = Trigger::Like { every: 100 };
        let mut acc = 0;
        let go = |n, acc: &mut u64| match_trigger(&t, &TriggerInput::Live(&like_ev(n)), acc);
        assert!(go(60, &mut acc).is_none());
        assert_eq!(acc, 60);
        let hit = go(50, &mut acc).expect("cruza 100");
        assert_eq!(hit.times, 1);
        assert_eq!(acc, 10);
        // Un golpe grande cruza varios umbrales, con tope.
        assert_eq!(go(250, &mut acc).expect("hit").times, 2);
        assert_eq!(acc, 60);
        assert_eq!(go(100_000, &mut acc).expect("hit").times, 10);
    }

    #[test]
    fn like_every_zero_never_fires() {
        let mut acc = 0;
        assert!(match_trigger(&Trigger::Like { every: 0 }, &TriggerInput::Live(&like_ev(5)), &mut acc).is_none());
    }

    #[test]
    fn command_matches_with_or_without_bang_in_the_rule() {
        for configured in ["sonido", "!sonido", "  !SONIDO "] {
            let t = Trigger::Command { command: configured.into() };
            let hit = m(&t, &chat_ev("!Sonido  hola mundo ")).unwrap_or_else(|| panic!("{configured}"));
            assert_eq!(hit.vars["args"], "hola mundo");
            assert_eq!(hit.vars["command"], "sonido");
        }
    }

    #[test]
    fn command_requires_the_bang_in_chat_and_exact_word() {
        let t = Trigger::Command { command: "tts".into() };
        assert!(m(&t, &chat_ev("tts hola")).is_none());
        assert!(m(&t, &chat_ev("!ttsx hola")).is_none());
        assert!(m(&t, &chat_ev("hola !tts")).is_none());
        assert_eq!(m(&t, &chat_ev("!tts")).expect("hit").vars["args"], "");
    }

    #[test]
    fn empty_command_never_matches() {
        let t = Trigger::Command { command: "!".into() };
        assert!(m(&t, &chat_ev("!")).is_none());
    }

    #[test]
    fn keyword_substring_and_whole_word() {
        let sub = Trigger::Keyword { keywords: vec!["hola".into()], whole_word: false };
        let word = Trigger::Keyword { keywords: vec!["hola".into()], whole_word: true };
        assert!(m(&sub, &chat_ev("holaaa a todos")).is_some());
        assert!(m(&word, &chat_ev("holaaa a todos")).is_none());
        assert!(m(&word, &chat_ev("¡Hola, a todos!")).is_some());
        assert_eq!(m(&sub, &chat_ev("HOLA")).expect("hit").vars["keyword"], "hola");
    }

    #[test]
    fn empty_keywords_never_match() {
        let t = Trigger::Keyword { keywords: vec![" ".into()], whole_word: false };
        assert!(m(&t, &chat_ev("cualquier cosa")).is_none());
    }

    #[test]
    fn system_triggers_match_by_id() {
        let goal = Trigger::GoalReached { goal_id: "g1".into() };
        let timer = Trigger::TimerEnded { timer_id: "t1".into() };
        let mut acc = 0;
        assert!(match_trigger(&goal, &TriggerInput::GoalReached("g1"), &mut acc).is_some());
        assert!(match_trigger(&goal, &TriggerInput::GoalReached("g2"), &mut acc).is_none());
        assert!(match_trigger(&goal, &TriggerInput::TimerEnded("g1"), &mut acc).is_none());
        assert!(match_trigger(&timer, &TriggerInput::TimerEnded("t1"), &mut acc).is_some());
        assert!(match_trigger(&Trigger::Follow, &TriggerInput::GoalReached("g1"), &mut acc).is_none());
    }
}
