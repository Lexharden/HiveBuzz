//! Planificador de mensajes temporizados. Lógica pura: se le da la hora y dice qué toca decir.

use std::collections::HashMap;

use super::model::TimedMessage;

#[derive(Default)]
pub struct TimedScheduler {
    /// Última vez (ms) que se dijo cada mensaje, o el momento en que se vio por primera vez.
    last: HashMap<String, i64>,
    /// Mensajes de chat de espectadores desde que se dijo cada uno.
    chat_since: HashMap<String, u32>,
}

impl TimedScheduler {
    pub fn note_chat(&mut self) {
        for n in self.chat_since.values_mut() {
            *n = n.saturating_add(1);
        }
    }

    /// Textos que toca decir ahora. Un mensaje nuevo espera su primer intervalo completo; uno al que
    /// le faltan mensajes de chat se pospone (no se pierde su turno: se dice en cuanto haya actividad).
    pub fn due(&mut self, messages: &[TimedMessage], now_ms: i64) -> Vec<String> {
        // Olvidar lo que ya no existe.
        self.last.retain(|id, _| messages.iter().any(|m| &m.id == id));
        self.chat_since.retain(|id, _| messages.iter().any(|m| &m.id == id));
        let mut out = Vec::new();
        for m in messages.iter().filter(|m| m.enabled) {
            let last = *self.last.entry(m.id.clone()).or_insert(now_ms);
            self.chat_since.entry(m.id.clone()).or_insert(0);
            let every = i64::from(m.every_minutes) * 60_000;
            if now_ms - last < every {
                continue;
            }
            if self.chat_since.get(&m.id).copied().unwrap_or(0) < m.min_chat_messages {
                continue;
            }
            self.last.insert(m.id.clone(), now_ms);
            self.chat_since.insert(m.id.clone(), 0);
            out.push(m.text.clone());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(id: &str, every: u32, min_chat: u32) -> TimedMessage {
        TimedMessage { id: id.into(), enabled: true, text: format!("texto {id}"), every_minutes: every, min_chat_messages: min_chat }
    }

    const MIN: i64 = 60_000;

    #[test]
    fn a_new_message_waits_one_full_interval() {
        let mut s = TimedScheduler::default();
        let m = [msg("a", 5, 0)];
        assert!(s.due(&m, 1_000 * MIN).is_empty());
        assert!(s.due(&m, 1_004 * MIN).is_empty());
        assert_eq!(s.due(&m, 1_005 * MIN), ["texto a"]);
        assert!(s.due(&m, 1_006 * MIN).is_empty());
        assert_eq!(s.due(&m, 1_010 * MIN), ["texto a"]);
    }

    #[test]
    fn it_waits_for_chat_activity_and_then_speaks_at_once() {
        let mut s = TimedScheduler::default();
        let m = [msg("a", 1, 2)];
        s.due(&m, 0);
        assert!(s.due(&m, 2 * MIN).is_empty(), "sala muda");
        s.note_chat();
        assert!(s.due(&m, 3 * MIN).is_empty(), "solo 1 mensaje");
        s.note_chat();
        assert_eq!(s.due(&m, 3 * MIN + 1), ["texto a"]);
        assert!(s.due(&m, 3 * MIN + 2).is_empty());
    }

    #[test]
    fn disabled_and_removed_messages_are_ignored_and_forgotten() {
        let mut s = TimedScheduler::default();
        let mut off = msg("off", 1, 0);
        off.enabled = false;
        s.due(&[off.clone(), msg("on", 1, 0)], 0);
        let out = s.due(&[off, msg("on", 1, 0)], MIN);
        assert_eq!(out, ["texto on"]);
        s.due(&[], 2 * MIN);
        assert!(s.last.is_empty() && s.chat_since.is_empty());
    }

    #[test]
    fn several_messages_can_be_due_together() {
        let mut s = TimedScheduler::default();
        let m = [msg("a", 1, 0), msg("b", 1, 0)];
        s.due(&m, 0);
        assert_eq!(s.due(&m, MIN), ["texto a", "texto b"]);
    }
}
