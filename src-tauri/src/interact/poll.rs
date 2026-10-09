//! Encuestas por chat: el streamer lanza una pregunta con opciones y los espectadores votan
//! escribiendo el número (`1`, `!1`, `!voto 2`). Un voto por persona (puede cambiarlo).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde::Serialize;
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;

use crate::actions::clock::Clock;
use crate::bot::service::BotService;
use crate::bus::EventBus;
use crate::error::{AppError, Result};
use crate::events::{EventType, LiveEvent};
use crate::overlay::OverlayHub;

pub const CHANNEL: &str = "poll";
pub const MAX_OPTIONS: usize = 8;
const MAX_QUESTION_CHARS: usize = 120;
const MAX_OPTION_CHARS: usize = 60;
const MIN_DURATION_S: u64 = 5;
const MAX_DURATION_S: u64 = 3_600;
/// Cada cuánto se refresca el overlay mientras llegan votos.
pub const PUBLISH_EVERY: Duration = Duration::from_millis(300);

/// Interpreta un mensaje de chat como un voto. Devuelve el índice (desde 0) de la opción.
/// Acepta `2`, `!2`, `# 2` no; solo número suelto, con `!` opcional, o `!voto N` / `!vote N`.
pub fn parse_vote(text: &str, options: usize) -> Option<usize> {
    let t = text.trim().to_lowercase();
    let t = t.trim_start_matches('!');
    let num = match t.split_once(char::is_whitespace) {
        None => t,
        Some(("voto" | "vote" | "v", rest)) => rest.trim(),
        Some(_) => return None,
    };
    if num.is_empty() || num.len() > 2 || !num.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let n: usize = num.parse().ok()?;
    (1..=options).contains(&n).then(|| n - 1)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionResult {
    pub label: String,
    pub votes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PollView {
    pub id: u64,
    pub question: String,
    pub options: Vec<OptionResult>,
    pub total: u64,
    pub ends_at_ms: i64,
    pub ended: bool,
    /// Índices ganadores (más de uno si hay empate); vacío si nadie votó.
    pub winners: Vec<usize>,
}

struct Active {
    id: u64,
    question: String,
    options: Vec<String>,
    ends_at_ms: i64,
    votes: HashMap<String, usize>,
    dirty: bool,
    ended: bool,
}

impl Active {
    fn view(&self) -> PollView {
        let mut counts = vec![0u64; self.options.len()];
        for &i in self.votes.values() {
            if let Some(c) = counts.get_mut(i) {
                *c += 1;
            }
        }
        let top = counts.iter().copied().max().unwrap_or(0);
        let winners = if self.ended && top > 0 { counts.iter().enumerate().filter(|(_, c)| **c == top).map(|(i, _)| i).collect() } else { Vec::new() };
        PollView {
            id: self.id,
            question: self.question.clone(),
            options: self.options.iter().zip(&counts).map(|(l, v)| OptionResult { label: l.clone(), votes: *v }).collect(),
            total: counts.iter().sum(),
            ends_at_ms: self.ends_at_ms,
            ended: self.ended,
            winners,
        }
    }
}

pub struct PollService {
    active: Mutex<Option<Active>>,
    next_id: Mutex<u64>,
    hub: OverlayHub,
    bot: Arc<BotService>,
    clock: Arc<dyn Clock>,
}

fn clean(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).collect::<String>().trim().chars().take(max).collect()
}

impl PollService {
    pub fn new(hub: OverlayHub, bot: Arc<BotService>, clock: Arc<dyn Clock>) -> Arc<Self> {
        Arc::new(Self { active: Mutex::new(None), next_id: Mutex::new(0), hub, bot, clock })
    }

    /// Lanza una encuesta (sustituye a la que hubiera). Devuelve su estado inicial.
    pub fn start(&self, question: &str, options: &[String], duration_s: u64) -> Result<PollView> {
        let question = clean(question, MAX_QUESTION_CHARS);
        if question.is_empty() {
            return Err(AppError::Invalid("la encuesta necesita una pregunta".into()));
        }
        let options: Vec<String> = options.iter().map(|o| clean(o, MAX_OPTION_CHARS)).filter(|o| !o.is_empty()).collect();
        if !(2..=MAX_OPTIONS).contains(&options.len()) {
            return Err(AppError::Invalid(format!("la encuesta necesita entre 2 y {MAX_OPTIONS} opciones")));
        }
        let duration_s = duration_s.clamp(MIN_DURATION_S, MAX_DURATION_S);
        let id = {
            let mut n = self.next_id.lock().unwrap_or_else(PoisonError::into_inner);
            *n += 1;
            *n
        };
        let ends_at_ms = self.clock.now_ms() + i64::try_from(duration_s * 1_000).unwrap_or(i64::MAX);
        let poll = Active { id, question, options, ends_at_ms, votes: HashMap::new(), dirty: false, ended: false };
        let view = poll.view();
        let say = Self::intro(&view);
        *self.active.lock().unwrap_or_else(PoisonError::into_inner) = Some(poll);
        self.publish(&view);
        self.bot.say(&say, "encuesta");
        Ok(view)
    }

    fn intro(v: &PollView) -> String {
        let opts = v.options.iter().enumerate().map(|(i, o)| format!("{}) {}", i + 1, o.label)).collect::<Vec<_>>().join("  ");
        format!("📊 {} — vota escribiendo el número: {opts}", v.question)
    }

    fn publish(&self, v: &PollView) {
        self.hub.publish_retained(CHANNEL, json!({ "kind": "poll", "poll": v }));
    }

    /// Termina la encuesta activa ya (o devuelve `None` si no hay).
    pub fn stop(&self) -> Option<PollView> {
        let mut guard = self.active.lock().unwrap_or_else(PoisonError::into_inner);
        let poll = guard.as_mut().filter(|p| !p.ended)?;
        poll.ended = true;
        let view = poll.view();
        drop(guard);
        self.publish(&view);
        self.bot.say(&Self::result_text(&view), "encuesta");
        Some(view)
    }

    /// Quita la encuesta del overlay.
    pub fn clear(&self) {
        *self.active.lock().unwrap_or_else(PoisonError::into_inner) = None;
        self.hub.publish_retained(CHANNEL, json!({ "kind": "none" }));
    }

    pub fn current(&self) -> Option<PollView> {
        self.active.lock().unwrap_or_else(PoisonError::into_inner).as_ref().map(Active::view)
    }

    fn result_text(v: &PollView) -> String {
        match v.winners.as_slice() {
            [] => format!("📊 Encuesta cerrada: «{}». Nadie votó.", v.question),
            [w] => {
                let o = &v.options[*w];
                format!("📊 Resultado de «{}»: {} con {} de {} votos 🏆", v.question, o.label, o.votes, v.total)
            }
            many => {
                let names = many.iter().map(|i| v.options[*i].label.as_str()).collect::<Vec<_>>().join(" y ");
                format!("📊 Empate en «{}»: {names} ({} votos cada una)", v.question, v.options[many[0]].votes)
            }
        }
    }

    /// Registra el voto de un mensaje de chat. Devuelve `true` si cambió algo.
    pub fn on_event(&self, ev: &LiveEvent) -> bool {
        if ev.kind != EventType::Chat {
            return false;
        }
        let Some(chat) = &ev.chat else { return false };
        let key = if ev.user.id.is_empty() { ev.user.unique_id.clone() } else { ev.user.id.clone() };
        if key.is_empty() {
            return false;
        }
        let mut guard = self.active.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(poll) = guard.as_mut().filter(|p| !p.ended) else { return false };
        let Some(choice) = parse_vote(&chat.text, poll.options.len()) else { return false };
        if poll.votes.insert(key, choice) == Some(choice) {
            return false;
        }
        poll.dirty = true;
        true
    }

    /// Cierra la encuesta si venció y refresca el overlay si hubo votos nuevos.
    pub fn tick(&self) {
        let now = self.clock.now_ms();
        let mut guard = self.active.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(poll) = guard.as_mut().filter(|p| !p.ended) else { return };
        if now >= poll.ends_at_ms {
            poll.ended = true;
            let view = poll.view();
            drop(guard);
            self.publish(&view);
            self.bot.say(&Self::result_text(&view), "encuesta");
        } else if poll.dirty {
            poll.dirty = false;
            let view = poll.view();
            drop(guard);
            self.publish(&view);
        }
    }

    pub fn spawn(self: &Arc<Self>, bus: &EventBus) {
        let (svc, mut events) = (Arc::clone(self), bus.subscribe());
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(PUBLISH_EVERY);
            loop {
                tokio::select! {
                    ev = events.recv() => match ev {
                        Ok(ev) => { svc.on_event(&ev); }
                        Err(RecvError::Lagged(n)) => tracing::warn!(missed = n, "las encuestas se quedaron atrás en el bus"),
                        Err(RecvError::Closed) => break,
                    },
                    _ = tick.tick() => svc.tick(),
                }
            }
        });
    }
}

#[cfg(test)]
mod tests;
