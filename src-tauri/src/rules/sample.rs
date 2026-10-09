//! Eventos de ejemplo que cumplen un trigger, para el botón «probar» de cada regla.

use super::model::Trigger;
use crate::events::{Chat, EventType, LiveEvent};
use crate::simulator::{build_event, SimKind};

/// Un evento (si el trigger se dispara con eventos en vivo) que satisface el trigger.
/// Los triggers internos (meta, timer) no tienen evento asociado.
pub fn sample_event_for(trigger: &Trigger) -> Option<LiveEvent> {
    let chat = |text: String| {
        let mut ev = build_event(SimKind::Chat);
        ev.chat = Some(Chat { text, emotes: None });
        ev
    };
    Some(match trigger {
        Trigger::Gift {
            gift_id,
            gift_name,
            min_coins,
        } => {
            let mut ev = build_event(SimKind::BigGift);
            if let Some(g) = ev.gift.as_mut() {
                if let Some(id) = gift_id {
                    g.id = *id;
                }
                if let Some(name) = gift_name {
                    g.name.clone_from(name);
                }
                if let Some(min) = min_coins {
                    g.coins = g.coins.max(*min);
                }
            }
            ev
        }
        Trigger::Follow => build_event(SimKind::Follow),
        Trigger::Share => build_event(SimKind::Share),
        Trigger::Subscribe => build_event(SimKind::Subscribe),
        Trigger::Join => build_event(SimKind::Join),
        Trigger::SubEmote => {
            let mut ev = build_event(SimKind::Chat);
            ev.kind = EventType::Emote;
            ev.chat = None;
            ev.user.is_subscriber = true;
            ev
        }
        Trigger::Like { every } => {
            let mut ev = build_event(SimKind::Like);
            if let Some(l) = ev.like.as_mut() {
                l.count = *every;
            }
            ev
        }
        Trigger::Command { command } => {
            chat(format!("!{} argumento de prueba", command.trim().trim_start_matches('!')))
        }
        Trigger::Keyword { keywords, .. } => chat(
            keywords
                .iter()
                .map(|k| k.trim())
                .find(|k| !k.is_empty())
                .unwrap_or("hola")
                .to_string(),
        ),
        Trigger::GoalReached { .. } | Trigger::TimerEnded { .. } | Trigger::Api { .. } => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::matcher::{match_trigger, TriggerInput};

    #[test]
    fn every_sample_event_satisfies_its_own_trigger() {
        let triggers = [
            Trigger::Gift { gift_id: Some(77), gift_name: Some("Corona".into()), min_coins: Some(50_000) },
            Trigger::Gift { gift_id: None, gift_name: None, min_coins: None },
            Trigger::Follow,
            Trigger::Share,
            Trigger::Subscribe,
            Trigger::Join,
            Trigger::SubEmote,
            Trigger::Like { every: 250 },
            Trigger::Command { command: "!sonido".into() },
            Trigger::Command { command: "tts".into() },
            Trigger::Keyword { keywords: vec![" ".into(), "Hola".into()], whole_word: true },
        ];
        for t in triggers {
            let ev = sample_event_for(&t).unwrap_or_else(|| panic!("sin muestra: {t:?}"));
            assert!(
                match_trigger(&t, &TriggerInput::Live(&ev), &mut 0).is_some(),
                "la muestra no cumple su trigger: {t:?}"
            );
        }
    }

    #[test]
    fn internal_triggers_have_no_live_sample() {
        assert!(sample_event_for(&Trigger::GoalReached { goal_id: "g".into() }).is_none());
        assert!(sample_event_for(&Trigger::TimerEnded { timer_id: "t".into() }).is_none());
    }
}
