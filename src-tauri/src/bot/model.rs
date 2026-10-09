//! Configuración del chatbot: comandos, respuestas por palabra clave, mensajes temporizados,
//! agradecimientos y las plantillas de los comandos de puntos y recompensas.

use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};
use crate::rules::model::Conditions;

/// Máximo de caracteres de un mensaje de chat de TikTok.
pub const MAX_CHAT_CHARS: usize = 150;
const MAX_ENTRIES: usize = 200;
const MAX_VARIANTS: usize = 20;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BotCommand {
    pub id: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Nombre y alias, con o sin `!` (`discord`, `!ds`). El primero es el principal.
    pub names: Vec<String>,
    /// Posibles respuestas; si hay varias, se elige una al azar.
    pub responses: Vec<String>,
    #[serde(default)]
    pub conditions: Conditions,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeywordReply {
    pub id: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    pub keywords: Vec<String>,
    #[serde(default = "yes")]
    pub whole_word: bool,
    pub responses: Vec<String>,
    #[serde(default)]
    pub conditions: Conditions,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimedMessage {
    pub id: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    pub text: String,
    /// Cada cuántos minutos se dice.
    pub every_minutes: u32,
    /// No se dice si desde la última vez hubo menos de N mensajes en el chat (no hablar a una sala vacía).
    #[serde(default)]
    pub min_chat_messages: u32,
}

/// Agradecimiento automático por un tipo de evento.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thanks {
    #[serde(default)]
    pub enabled: bool,
    pub template: String,
    /// Solo regalos: valor total mínimo para agradecerlo.
    #[serde(default)]
    pub min_coins: u64,
    /// Tiempo mínimo entre agradecimientos al mismo usuario.
    #[serde(default = "default_thanks_cooldown")]
    pub user_cooldown_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThanksConfig {
    #[serde(default = "thanks_gift")]
    pub gift: Thanks,
    #[serde(default = "thanks_follow")]
    pub follow: Thanks,
    #[serde(default = "thanks_share")]
    pub share: Thanks,
    #[serde(default = "thanks_subscribe")]
    pub subscribe: Thanks,
}

impl Default for ThanksConfig {
    fn default() -> Self {
        Self { gift: thanks_gift(), follow: thanks_follow(), share: thanks_share(), subscribe: thanks_subscribe() }
    }
}

/// Plantillas de los mensajes integrados (puntos, top y recompensas).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuiltinReplies {
    #[serde(default = "default_points_reply")]
    pub points: String,
    #[serde(default = "default_top_reply")]
    pub top: String,
    #[serde(default = "default_redeemed")]
    pub redeemed: String,
    #[serde(default = "default_denied")]
    pub denied: String,
}

impl Default for BuiltinReplies {
    fn default() -> Self {
        Self { points: default_points_reply(), top: default_top_reply(), redeemed: default_redeemed(), denied: default_denied() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BotConfig {
    /// Interruptor general. Por defecto apagado: el bot escribe en tu chat, mejor que lo actives tú.
    #[serde(default)]
    pub enabled: bool,
    /// Tiempo mínimo entre mensajes del bot (TikTok limita el ritmo de escritura).
    #[serde(default = "default_min_interval")]
    pub min_interval_ms: u64,
    #[serde(default)]
    pub commands: Vec<BotCommand>,
    #[serde(default)]
    pub keyword_replies: Vec<KeywordReply>,
    #[serde(default)]
    pub timed_messages: Vec<TimedMessage>,
    #[serde(default)]
    pub thanks: ThanksConfig,
    #[serde(default)]
    pub builtin: BuiltinReplies,
}

fn yes() -> bool {
    true
}
fn default_min_interval() -> u64 {
    2_000
}
fn default_thanks_cooldown() -> u64 {
    30_000
}
fn thanks_gift() -> Thanks {
    Thanks { enabled: false, template: "¡Gracias {nickname} por {count}× {gift}! 🎁".into(), min_coins: 1, user_cooldown_ms: 5_000 }
}
fn thanks_follow() -> Thanks {
    Thanks { enabled: false, template: "¡Gracias por seguirme, {nickname}! 💛".into(), min_coins: 0, user_cooldown_ms: 600_000 }
}
fn thanks_share() -> Thanks {
    Thanks { enabled: false, template: "¡Gracias por compartir el LIVE, {nickname}! 🙌".into(), min_coins: 0, user_cooldown_ms: 600_000 }
}
fn thanks_subscribe() -> Thanks {
    Thanks { enabled: false, template: "¡Bienvenido a los suscriptores, {nickname}! 🌟".into(), min_coins: 0, user_cooldown_ms: 600_000 }
}
fn default_points_reply() -> String {
    "@{user}, tienes {points} {currency}".into()
}
fn default_top_reply() -> String {
    "🏆 Top {currency}: {top}".into()
}
fn default_redeemed() -> String {
    "@{user} canjeó «{reward}» ({cost} {currency}). Te quedan {points}.".into()
}
fn default_denied() -> String {
    "@{user}, «{reward}» cuesta {cost} {currency} y tienes {points}.".into()
}

impl Default for BotConfig {
    fn default() -> Self {
        serde_json::from_str("{}").unwrap_or_else(|_| unreachable!("todos los campos tienen valor por defecto"))
    }
}

fn clean_name(raw: &str) -> String {
    raw.trim().trim_start_matches('!').to_lowercase().chars().filter(|c| c.is_alphanumeric() || *c == '_').collect()
}

fn check_texts(what: &str, texts: &[String]) -> Result<()> {
    if texts.is_empty() || texts.iter().all(|t| t.trim().is_empty()) {
        return Err(AppError::Invalid(format!("{what}: escribe al menos una respuesta")));
    }
    if texts.len() > MAX_VARIANTS {
        return Err(AppError::Invalid(format!("{what}: demasiadas respuestas (máximo {MAX_VARIANTS})")));
    }
    Ok(())
}

impl BotConfig {
    /// Normaliza lo que se puede arreglar sin molestar al usuario (nombres de comando, espacios).
    pub fn sanitized(mut self) -> Self {
        self.min_interval_ms = self.min_interval_ms.clamp(1_000, 600_000);
        for c in &mut self.commands {
            c.names = c.names.iter().map(|n| clean_name(n)).filter(|n| !n.is_empty()).collect();
            c.responses.retain(|r| !r.trim().is_empty());
        }
        for k in &mut self.keyword_replies {
            k.keywords = k.keywords.iter().map(|w| w.trim().to_lowercase()).filter(|w| !w.is_empty()).collect();
            k.responses.retain(|r| !r.trim().is_empty());
        }
        self
    }

    /// Valida la configuración antes de guardarla. Devuelve el primer problema encontrado.
    pub fn validate(&self) -> Result<()> {
        if self.commands.len() > MAX_ENTRIES || self.keyword_replies.len() > MAX_ENTRIES || self.timed_messages.len() > MAX_ENTRIES {
            return Err(AppError::Invalid(format!("demasiadas entradas (máximo {MAX_ENTRIES} de cada tipo)")));
        }
        let mut seen = std::collections::HashSet::new();
        for c in &self.commands {
            let label = format!("comando «{}»", c.names.first().map_or("?", String::as_str));
            if c.id.trim().is_empty() {
                return Err(AppError::Invalid(format!("{label}: falta el id")));
            }
            if c.names.is_empty() {
                return Err(AppError::Invalid("hay un comando sin nombre".into()));
            }
            check_texts(&label, &c.responses)?;
            for n in &c.names {
                if !seen.insert(n.clone()) {
                    return Err(AppError::Invalid(format!("el comando «!{n}» está repetido")));
                }
            }
            c.conditions_valid(&label)?;
        }
        for k in &self.keyword_replies {
            if k.id.trim().is_empty() {
                return Err(AppError::Invalid("una respuesta por palabra clave no tiene id".into()));
            }
            if k.keywords.is_empty() {
                return Err(AppError::Invalid("una respuesta por palabra clave no tiene palabras".into()));
            }
            check_texts("palabra clave", &k.responses)?;
        }
        for t in &self.timed_messages {
            if t.id.trim().is_empty() {
                return Err(AppError::Invalid("un mensaje temporizado no tiene id".into()));
            }
            if t.text.trim().is_empty() {
                return Err(AppError::Invalid("un mensaje temporizado está vacío".into()));
            }
            if !(1..=720).contains(&t.every_minutes) {
                return Err(AppError::Invalid("el intervalo de un mensaje temporizado debe estar entre 1 y 720 minutos".into()));
            }
        }
        for (name, t) in [("regalos", &self.thanks.gift), ("follows", &self.thanks.follow), ("shares", &self.thanks.share), ("suscripciones", &self.thanks.subscribe)] {
            if t.enabled && t.template.trim().is_empty() {
                return Err(AppError::Invalid(format!("el agradecimiento de {name} está activo pero vacío")));
            }
        }
        Ok(())
    }
}

impl BotCommand {
    fn conditions_valid(&self, label: &str) -> Result<()> {
        if !(0.0..=100.0).contains(&self.conditions.probability) {
            return Err(AppError::Invalid(format!("{label}: la probabilidad debe estar entre 0 y 100")));
        }
        if let Some(s) = &self.conditions.schedule {
            if crate::rules::eval::parse_hhmm(&s.from).is_none() || crate::rules::eval::parse_hhmm(&s.to).is_none() {
                return Err(AppError::Invalid(format!("{label}: el horario debe tener formato HH:MM")));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(id: &str, names: &[&str], resp: &[&str]) -> BotCommand {
        BotCommand {
            id: id.into(),
            enabled: true,
            names: names.iter().map(|s| (*s).to_string()).collect(),
            responses: resp.iter().map(|s| (*s).to_string()).collect(),
            conditions: Conditions::default(),
        }
    }

    #[test]
    fn defaults_are_safe_and_off() {
        let c = BotConfig::default();
        assert!(!c.enabled, "el bot nace apagado");
        assert_eq!(c.min_interval_ms, 2_000);
        assert!(!c.thanks.gift.enabled && !c.thanks.follow.enabled);
        assert!(c.builtin.points.contains("{points}"));
        assert!(c.validate().is_ok());
    }

    #[test]
    fn partial_json_fills_the_rest() {
        let c: BotConfig = serde_json::from_str(r#"{"enabled": true, "thanks": {"gift": {"enabled": true, "template": "gracias"}}}"#).expect("parsea");
        assert!(c.enabled && c.thanks.gift.enabled);
        assert_eq!(c.thanks.gift.user_cooldown_ms, 30_000);
        assert!(!c.thanks.follow.enabled, "lo no indicado conserva su valor de fábrica");
    }

    #[test]
    fn sanitizing_cleans_names_and_drops_empty_responses() {
        let mut cfg = BotConfig::default();
        cfg.commands.push(cmd("1", &[" !Discord ", "ds!", "  "], &["a", "   "]));
        cfg.keyword_replies.push(KeywordReply {
            id: "k".into(),
            enabled: true,
            keywords: vec![" HOLA ".into(), "".into()],
            whole_word: true,
            responses: vec!["x".into(), "".into()],
            conditions: Conditions::default(),
        });
        cfg.min_interval_ms = 1;
        let c = cfg.sanitized();
        assert_eq!(c.commands[0].names, ["discord", "ds"]);
        assert_eq!(c.commands[0].responses, ["a"]);
        assert_eq!(c.keyword_replies[0].keywords, ["hola"]);
        assert_eq!(c.min_interval_ms, 1_000);
    }

    #[test]
    fn validation_rejects_each_kind_of_problem() {
        let base = || BotConfig::default();
        let mut cases: Vec<(&str, BotConfig)> = Vec::new();
        let mut c = base(); c.commands.push(cmd("1", &[], &["x"])); cases.push(("sin nombre", c));
        let mut c = base(); c.commands.push(cmd("1", &["a"], &[])); cases.push(("sin respuesta", c));
        let mut c = base(); c.commands.push(cmd("", &["a"], &["x"])); cases.push(("sin id", c));
        let mut c = base(); c.commands.push(cmd("1", &["a"], &["x"])); c.commands.push(cmd("2", &["a"], &["y"])); cases.push(("repetido", c));
        let mut c = base(); c.commands.push(cmd("1", &["a", "b"], &["x"])); c.commands.push(cmd("2", &["b"], &["y"])); cases.push(("alias repetido", c));
        let mut c = base(); let mut k = cmd("1", &["a"], &["x"]); k.conditions.probability = 150.0; c.commands.push(k); cases.push(("prob", c));
        let mut c = base(); c.keyword_replies.push(KeywordReply { id: "k".into(), enabled: true, keywords: vec![], whole_word: true, responses: vec!["x".into()], conditions: Conditions::default() }); cases.push(("sin palabras", c));
        let mut c = base(); c.timed_messages.push(TimedMessage { id: "t".into(), enabled: true, text: "".into(), every_minutes: 5, min_chat_messages: 0 }); cases.push(("temporizado vacío", c));
        let mut c = base(); c.timed_messages.push(TimedMessage { id: "t".into(), enabled: true, text: "hola".into(), every_minutes: 0, min_chat_messages: 0 }); cases.push(("intervalo 0", c));
        let mut c = base(); c.thanks.follow.enabled = true; c.thanks.follow.template = " ".into(); cases.push(("gracias vacío", c));
        let mut c = base(); c.commands = (0..201).map(|i| cmd(&i.to_string(), &[&format!("c{i}")], &["x"])).collect(); cases.push(("demasiados", c));
        for (what, cfg) in cases {
            assert!(cfg.validate().is_err(), "debía rechazar: {what}");
        }
    }

    #[test]
    fn a_complete_valid_config_passes() {
        let mut c = BotConfig::default();
        c.commands.push(cmd("1", &["discord", "ds"], &["Únete: discord.gg/x", "https://discord.gg/x"]));
        c.timed_messages.push(TimedMessage { id: "t".into(), enabled: true, text: "¡Sígueme!".into(), every_minutes: 10, min_chat_messages: 3 });
        assert!(c.sanitized().validate().is_ok());
    }
}
