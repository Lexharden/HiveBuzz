//! IRC de Twitch: análisis de líneas con etiquetas y conversión a `LiveEvent`.

use std::collections::HashMap;

use super::USER_ID_PREFIX;
use crate::events::{Chat, Emote, EventType, Gift, LiveEvent, Platform, User};

/// Una línea IRC ya separada.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IrcMsg {
    pub tags: HashMap<String, String>,
    pub prefix: String,
    pub command: String,
    pub params: Vec<String>,
    pub trailing: Option<String>,
}

fn unescape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut it = v.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some(':') => out.push(';'),
            Some('s') => out.push(' '),
            Some('r') => out.push('\r'),
            Some('n') => out.push('\n'),
            Some('\\') => out.push('\\'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// Analiza una línea (sin el `\r\n`). Devuelve `None` si está vacía o mal formada.
pub fn parse(line: &str) -> Option<IrcMsg> {
    let mut rest = line.trim_end_matches(['\r', '\n']);
    if rest.trim().is_empty() {
        return None;
    }
    let mut msg = IrcMsg::default();
    if let Some(stripped) = rest.strip_prefix('@') {
        let (tags, tail) = stripped.split_once(' ')?;
        for pair in tags.split(';') {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            if !k.is_empty() {
                msg.tags.insert(k.to_string(), unescape(v));
            }
        }
        rest = tail.trim_start();
    }
    if let Some(stripped) = rest.strip_prefix(':') {
        let (prefix, tail) = stripped.split_once(' ')?;
        msg.prefix = prefix.to_string();
        rest = tail.trim_start();
    }
    let (head, trailing) = match rest.split_once(" :") {
        Some((h, t)) => (h, Some(t.to_string())),
        None => (rest, None),
    };
    let mut parts = head.split_whitespace();
    msg.command = parts.next()?.to_string();
    msg.params = parts.map(str::to_string).collect();
    msg.trailing = trailing;
    Some(msg)
}

impl IrcMsg {
    fn tag(&self, k: &str) -> &str {
        self.tags.get(k).map_or("", String::as_str)
    }

    /// Login del autor según el prefijo `login!login@login.tmi.twitch.tv`.
    fn prefix_login(&self) -> &str {
        self.prefix.split('!').next().unwrap_or("")
    }
}

fn badges(msg: &IrcMsg) -> Vec<&str> {
    msg.tag("badges").split(',').filter_map(|b| b.split('/').next()).filter(|b| !b.is_empty()).collect()
}

fn ts_of(msg: &IrcMsg, now_ms: i64) -> i64 {
    msg.tag("tmi-sent-ts").parse::<i64>().ok().filter(|t| *t > 0).unwrap_or(now_ms)
}

/// Usuario a partir de las etiquetas del mensaje. `None` si no hay un id numérico.
fn user_of(msg: &IrcMsg, login: &str, display: &str, user_id: &str) -> Option<User> {
    if user_id.is_empty() || !user_id.chars().all(|c| c.is_ascii_digit()) || login.is_empty() {
        return None;
    }
    let b = badges(msg);
    let broadcaster = b.contains(&"broadcaster");
    Some(User {
        id: format!("{USER_ID_PREFIX}{user_id}"),
        unique_id: login.to_string(),
        nickname: if display.is_empty() { login.to_string() } else { display.to_string() },
        avatar: String::new(),
        is_moderator: b.contains(&"moderator") || broadcaster,
        is_subscriber: b.contains(&"subscriber") || b.contains(&"founder"),
        // IRC no dice si te sigue; por eso este rol no se aplica en Twitch.
        is_follower: false,
        team_level: None,
        gifter_level: None,
    })
}

/// Emotes de la etiqueta `emotes` (`25:0-4,12-16/1902:6-10`), sin repetir ids.
fn emotes_of(msg: &IrcMsg) -> Option<Vec<Emote>> {
    let mut out: Vec<Emote> = Vec::new();
    for part in msg.tag("emotes").split('/') {
        let id = part.split(':').next().unwrap_or("");
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || out.iter().any(|e| e.id == id) {
            continue;
        }
        out.push(Emote { id: id.to_string(), image: format!("https://static-cdn.jtvnw.net/emoticons/v2/{id}/default/dark/1.0") });
    }
    (!out.is_empty()).then_some(out)
}

fn base_event(id: String, kind: EventType, user: User, ts: i64) -> LiveEvent {
    LiveEvent { id, platform: Platform::Twitch, kind, user, gift: None, chat: None, like: None, ts }
}

/// Convierte una línea en 0, 1 o 2 eventos: un `PRIVMSG` da el chat y, si trae bits, también un regalo.
/// Un `USERNOTICE` de suscripción da un `Subscribe`. Lo demás no genera eventos.
pub fn to_events(msg: &IrcMsg, now_ms: i64) -> Vec<LiveEvent> {
    match msg.command.as_str() {
        "PRIVMSG" => privmsg(msg, now_ms),
        "USERNOTICE" => usernotice(msg, now_ms).into_iter().collect(),
        _ => Vec::new(),
    }
}

fn privmsg(msg: &IrcMsg, now_ms: i64) -> Vec<LiveEvent> {
    let id = msg.tag("id");
    let Some(user) = user_of(msg, msg.prefix_login(), msg.tag("display-name"), msg.tag("user-id")) else { return Vec::new() };
    let Some(text) = msg.trailing.as_deref() else { return Vec::new() };
    if id.is_empty() {
        return Vec::new();
    }
    // «/me hola» llega como «\u{1}ACTION hola\u{1}».
    let text = text
        .strip_prefix('\u{1}')
        .and_then(|t| t.strip_suffix('\u{1}'))
        .map_or(text, |t| t.strip_prefix("ACTION ").unwrap_or(t))
        .trim();
    let ts = ts_of(msg, now_ms);
    let mut out = Vec::new();
    if !text.is_empty() {
        let mut ev = base_event(id.to_string(), EventType::Chat, user.clone(), ts);
        ev.chat = Some(Chat { text: text.to_string(), emotes: emotes_of(msg) });
        out.push(ev);
    }
    // Un cheer: los bits viajan en la misma línea del chat.
    if let Some(bits) = msg.tag("bits").parse::<u64>().ok().filter(|b| *b > 0) {
        let mut ev = base_event(format!("{id}-bits"), EventType::Gift, user, ts);
        ev.gift = Some(Gift { id: 0, name: "Bits".into(), coins: bits, count: 1, streakable: false, image: String::new() });
        out.push(ev);
    }
    out
}

fn usernotice(msg: &IrcMsg, now_ms: i64) -> Option<LiveEvent> {
    let id = msg.tag("id");
    if id.is_empty() {
        return None;
    }
    let ts = ts_of(msg, now_ms);
    let mut user = match msg.tag("msg-id") {
        // Suscripción propia o renovación: el autor es quien se suscribe.
        "sub" | "resub" => user_of(msg, msg.tag("login"), msg.tag("display-name"), msg.tag("user-id"))?,
        // Regalo de una suscripción: «se suscribe» el destinatario.
        "subgift" | "anonsubgift" => user_of(
            msg,
            msg.tag("msg-param-recipient-user-name"),
            msg.tag("msg-param-recipient-display-name"),
            msg.tag("msg-param-recipient-id"),
        )?,
        // Los regalos masivos (`submysterygift`) van seguidos de un `subgift` por destinatario: se cuentan ahí.
        _ => return None,
    };
    user.is_subscriber = true;
    // Las insignias del mensaje son las de quien REGALA, no las del destinatario.
    if msg.tag("msg-id").ends_with("subgift") {
        user.is_moderator = false;
    }
    Some(base_event(id.to_string(), EventType::Subscribe, user, ts))
}

#[cfg(test)]
mod tests;
