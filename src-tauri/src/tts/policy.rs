//! Configuración del TTS y política de lectura del chat: qué mensajes se leen y con qué voz.
//! Todo es puro (el reloj y los datos entran por parámetro) para poder probarlo sin audio.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::filters::{clean, Cleaned, FilterConfig};
use crate::events::{EventType, LiveEvent};
use crate::rules::model::Role;
use crate::rules::template::{event_vars, Vars};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VoiceMode {
    /// Siempre la voz predeterminada.
    #[default]
    Single,
    /// Una voz según el rol (moderador > suscriptor > seguidor).
    ByRole,
    /// Una voz de la lista, fija para cada usuario (siempre la misma para la misma persona).
    RandomPerUser,
}

/// Qué hacer con la lectura en curso cuando el streamer habla.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MicGuardMode {
    /// Pausa y, al callar, retrocede un poco para repetir la palabra que se cortó.
    #[default]
    RepeatWord,
    /// Pausa y, al callar, vuelve a leer el mensaje desde el principio.
    RepeatMessage,
    /// Corta el mensaje y pasa al siguiente (que espera a que el streamer calle).
    Skip,
}

/// No hablar encima del streamer: el micrófono pausa o salta la lectura mientras habla.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MicGuard {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub mode: MicGuardMode,
    /// Nombre del micrófono; vacío = el predeterminado del sistema.
    #[serde(default)]
    pub device: Option<String>,
    /// Sensibilidad: nivel (dBFS) a partir del cual se considera que hablas. Más bajo = más sensible.
    #[serde(default = "default_mic_threshold")]
    pub threshold_db: f32,
    /// Silencio necesario para retomar la lectura.
    #[serde(default = "default_mic_hold")]
    pub hold_ms: u64,
}

fn default_mic_threshold() -> f32 {
    -40.0
}
fn default_mic_hold() -> u64 {
    800
}

impl Default for MicGuard {
    fn default() -> Self {
        serde_json::from_str("{}").unwrap_or_else(|_| unreachable!("todos los campos tienen valor por defecto"))
    }
}

impl MicGuard {
    pub fn sanitized(mut self) -> Self {
        self.threshold_db = if self.threshold_db.is_finite() { self.threshold_db.clamp(-80.0, -5.0) } else { default_mic_threshold() };
        self.hold_ms = self.hold_ms.clamp(200, 5_000);
        self.device = self.device.map(|d| d.trim().to_string()).filter(|d| !d.is_empty());
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleVoices {
    #[serde(default)]
    pub moderator: Option<String>,
    #[serde(default)]
    pub subscriber: Option<String>,
    #[serde(default)]
    pub follower: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsConfig {
    /// Lee el chat automáticamente (las acciones `tts` de las reglas funcionan igualmente).
    #[serde(default)]
    pub enabled: bool,
    /// Si se indica (p. ej. `tts`), solo se leen los mensajes `!tts texto`.
    #[serde(default)]
    pub command: Option<String>,
    /// Basta con uno de estos roles. Vacío = todos.
    #[serde(default)]
    pub roles_any: Vec<Role>,
    #[serde(default)]
    pub min_team_level: Option<u32>,
    #[serde(default)]
    pub min_gifter_level: Option<u32>,
    /// Solo quienes hayan enviado un regalo en los últimos N minutos.
    #[serde(default)]
    pub recent_donors_minutes: Option<u32>,
    #[serde(default = "default_user_cooldown")]
    pub user_cooldown_ms: u64,
    /// `@usuario` a los que nunca se lee.
    #[serde(default)]
    pub ignore_users: Vec<String>,
    /// Sin comando configurado, no leer mensajes que empiecen por `!` (son comandos de otros bots).
    #[serde(default = "yes")]
    pub ignore_bang_commands: bool,
    /// Plantilla de lo que se dice. Variables: `{nickname}`, `{user}`, `{text}`…
    #[serde(default = "default_template")]
    pub template: String,
    /// Un mensaje que espere más que esto en la cola se descarta (el chat caduca rápido).
    #[serde(default = "default_max_wait")]
    pub max_wait_ms: u64,
    #[serde(default)]
    pub filters: FilterConfig,
    #[serde(default)]
    pub voice_mode: VoiceMode,
    /// `motor:voz`, p. ej. `piper:es_MX-claude-high`, `sapi:Microsoft Sabina Desktop`, `edge:es-MX-DaliaNeural`.
    #[serde(default)]
    pub default_voice: Option<String>,
    #[serde(default)]
    pub role_voices: RoleVoices,
    #[serde(default)]
    pub random_voices: Vec<String>,
    /// Velocidad, 0.5–2.0.
    #[serde(default = "default_rate")]
    pub rate: f64,
    /// Volumen, 0–100.
    #[serde(default = "default_volume")]
    pub volume: u8,
    /// Ruta de `piper(.exe)`; vacío = el que instala la app.
    #[serde(default)]
    pub piper_path: Option<String>,
    /// Carpeta de voces `.onnx` de Piper; vacío = la de la app.
    #[serde(default)]
    pub piper_voices_dir: Option<String>,
    #[serde(default)]
    pub mic_guard: MicGuard,
}

fn yes() -> bool {
    true
}
fn default_user_cooldown() -> u64 {
    5_000
}
fn default_template() -> String {
    "{nickname} dice: {text}".to_string()
}
fn default_max_wait() -> u64 {
    30_000
}
fn default_rate() -> f64 {
    1.0
}
fn default_volume() -> u8 {
    100
}

impl Default for TtsConfig {
    fn default() -> Self {
        // Pasar por serde garantiza una única fuente de verdad para los valores por defecto.
        serde_json::from_str("{}").unwrap_or_else(|_| unreachable!("todos los campos tienen valor por defecto"))
    }
}

impl TtsConfig {
    /// Corrige valores fuera de rango (por ediciones manuales o importaciones).
    pub fn sanitized(mut self) -> Self {
        self.rate = if self.rate.is_finite() { self.rate.clamp(0.5, 2.0) } else { 1.0 };
        self.volume = self.volume.min(100);
        self.command = self
            .command
            .map(|c| c.trim().trim_start_matches('!').to_lowercase())
            .filter(|c| !c.is_empty());
        self.ignore_users = self
            .ignore_users
            .iter()
            .map(|u| u.trim().trim_start_matches('@').to_lowercase())
            .filter(|u| !u.is_empty())
            .collect();
        if self.template.trim().is_empty() {
            self.template = default_template();
        }
        self.max_wait_ms = self.max_wait_ms.clamp(1_000, 600_000);
        self.mic_guard = self.mic_guard.sanitized();
        self
    }
}

/// Voz disponible en algún motor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceInfo {
    /// `motor:nombre`
    pub id: String,
    pub engine: String,
    pub name: String,
    /// Código de idioma si se conoce (`es-MX`).
    pub lang: Option<String>,
}

// ---- Decisión sobre un mensaje de chat ---------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum ChatDecision {
    Skip(&'static str),
    /// Leer: `text` ya está limpio; `vars` sirve para la plantilla y la elección de voz.
    Speak { text: String, vars: Vars },
}

/// Estado que la política necesita recordar entre mensajes.
#[derive(Debug, Default)]
pub struct ChatMemory {
    /// user_id → último regalo (ms).
    pub last_gift_ms: HashMap<String, i64>,
    /// user_id → última vez que se leyó un mensaje suyo (ms).
    pub last_spoken_ms: HashMap<String, i64>,
}

impl ChatMemory {
    /// Anota quién regaló (para el filtro «donadores recientes»).
    pub fn observe(&mut self, ev: &LiveEvent) {
        if ev.kind == EventType::Gift {
            self.last_gift_ms.insert(ev.user.id.clone(), ev.ts);
        }
    }

    /// Evita que los mapas crezcan sin límite en LIVEs largos.
    pub fn prune(&mut self, now_ms: i64, keep_ms: i64) {
        self.last_gift_ms.retain(|_, t| now_ms.saturating_sub(*t) < keep_ms);
        self.last_spoken_ms.retain(|_, t| now_ms.saturating_sub(*t) < keep_ms);
    }
}

pub fn decide_chat(cfg: &TtsConfig, ev: &LiveEvent, mem: &mut ChatMemory, now_ms: i64) -> ChatDecision {
    if !cfg.enabled || ev.kind != EventType::Chat {
        return ChatDecision::Skip("desactivado");
    }
    let Some(chat) = &ev.chat else {
        return ChatDecision::Skip("sin texto");
    };
    let u = &ev.user;
    if cfg.ignore_users.iter().any(|i| i.eq_ignore_ascii_case(&u.unique_id)) {
        return ChatDecision::Skip("usuario ignorado");
    }

    // Texto a leer, según el modo.
    let raw = chat.text.trim();
    let text = match &cfg.command {
        Some(cmd) => {
            let (head, args) = raw.split_once(char::is_whitespace).map_or((raw, ""), |(h, a)| (h, a.trim()));
            match head.strip_prefix('!') {
                Some(typed) if typed.eq_ignore_ascii_case(cmd) && !args.is_empty() => args,
                _ => return ChatDecision::Skip("no es el comando"),
            }
        }
        None => {
            if cfg.ignore_bang_commands && raw.starts_with('!') {
                return ChatDecision::Skip("comando de otro bot");
            }
            raw
        }
    };

    // Quién puede.
    let role_ok = cfg.roles_any.is_empty()
        || cfg.roles_any.iter().any(|r| match r {
            Role::Moderator => u.is_moderator,
            Role::Subscriber => u.is_subscriber,
            Role::Follower => u.is_follower,
        });
    if !role_ok
        || cfg.min_team_level.is_some_and(|m| u.team_level.unwrap_or(0) < m)
        || cfg.min_gifter_level.is_some_and(|m| u.gifter_level.unwrap_or(0) < m)
    {
        return ChatDecision::Skip("no cumple el rol o nivel");
    }
    if let Some(minutes) = cfg.recent_donors_minutes {
        let window = i64::from(minutes) * 60_000;
        let recent = mem.last_gift_ms.get(&u.id).is_some_and(|t| now_ms.saturating_sub(*t) <= window);
        if !recent {
            return ChatDecision::Skip("no es donador reciente");
        }
    }

    // Texto limpio (groserías, enlaces, emojis, longitud).
    let spoken = match clean(text, &cfg.filters) {
        Cleaned::Say(s) => s,
        Cleaned::Skip(reason) => return ChatDecision::Skip(reason),
    };

    // Cooldown por usuario: solo se consume si el mensaje va a leerse.
    let span = i64::try_from(cfg.user_cooldown_ms).unwrap_or(i64::MAX);
    if span > 0 {
        if let Some(last) = mem.last_spoken_ms.get(&u.id) {
            if now_ms.saturating_sub(*last) < span {
                return ChatDecision::Skip("cooldown del usuario");
            }
        }
    }
    mem.last_spoken_ms.insert(u.id.clone(), now_ms);

    let mut vars = event_vars(ev);
    vars.insert("text".into(), spoken.clone());
    ChatDecision::Speak { text: spoken, vars }
}

// ---- Elección de voz -----------------------------------------------------------------------------

/// FNV-1a: estable entre ejecuciones (a diferencia de `DefaultHasher`), así cada usuario
/// conserva su voz aunque se reinicie la app.
fn stable_hash(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

fn flag(vars: &Vars, key: &str) -> bool {
    vars.get(key).is_some_and(|v| v == "true")
}

/// Decide la voz: la pedida explícitamente > la del modo (rol / aleatoria por usuario) >
/// la predeterminada > la primera disponible. Solo devuelve voces que existan.
pub fn resolve_voice(cfg: &TtsConfig, vars: &Vars, available: &[VoiceInfo], explicit: Option<&str>) -> Option<String> {
    let exists = |id: &str| available.iter().any(|v| v.id == id);
    let pick = |id: Option<&String>| id.filter(|i| exists(i)).cloned();

    if let Some(e) = explicit.filter(|e| exists(e)) {
        return Some(e.to_string());
    }
    let by_mode = match cfg.voice_mode {
        VoiceMode::Single => None,
        VoiceMode::ByRole => {
            if flag(vars, "ismoderator") {
                pick(cfg.role_voices.moderator.as_ref())
            } else {
                None
            }
            .or_else(|| if flag(vars, "issubscriber") { pick(cfg.role_voices.subscriber.as_ref()) } else { None })
            .or_else(|| if flag(vars, "isfollower") { pick(cfg.role_voices.follower.as_ref()) } else { None })
        }
        VoiceMode::RandomPerUser => {
            let pool: Vec<&String> = cfg.random_voices.iter().filter(|v| exists(v)).collect();
            let key = vars.get("userid").or_else(|| vars.get("user"));
            match (pool.len(), key) {
                (0, _) | (_, None) => None,
                (n, Some(k)) => pool.get((stable_hash(k) % n as u64) as usize).map(|v| (*v).clone()),
            }
        }
    };
    by_mode
        .or_else(|| pick(cfg.default_voice.as_ref()))
        .or_else(|| available.first().map(|v| v.id.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::testing::sample_event;
    use crate::events::{Chat, Gift};

    fn chat(text: &str) -> LiveEvent {
        let mut e = sample_event("c");
        e.chat = Some(Chat { text: text.into(), emotes: None });
        e
    }

    fn cfg() -> TtsConfig {
        TtsConfig { enabled: true, user_cooldown_ms: 0, ..Default::default() }
    }

    fn decide(c: &TtsConfig, ev: &LiveEvent) -> ChatDecision {
        decide_chat(c, ev, &mut ChatMemory::default(), 1_000_000)
    }

    fn skip_reason(d: ChatDecision) -> &'static str {
        match d {
            ChatDecision::Skip(r) => r,
            ChatDecision::Speak { text, .. } => panic!("se esperaba Skip, se leería: {text}"),
        }
    }

    #[test]
    fn disabled_reads_nothing() {
        let c = TtsConfig { enabled: false, ..cfg() };
        assert_eq!(skip_reason(decide(&c, &chat("hola"))), "desactivado");
    }

    #[test]
    fn reads_all_chat_by_default_and_exposes_the_text_variable() {
        match decide(&cfg(), &chat("hola a todos")) {
            ChatDecision::Speak { text, vars } => {
                assert_eq!(text, "hola a todos");
                assert_eq!(vars["text"], "hola a todos");
                assert_eq!(vars["nickname"], "Ana");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn bang_messages_from_other_bots_are_ignored() {
        assert_eq!(skip_reason(decide(&cfg(), &chat("!puntos"))), "comando de otro bot");
        let c = TtsConfig { ignore_bang_commands: false, ..cfg() };
        assert!(matches!(decide(&c, &chat("!puntos")), ChatDecision::Speak { .. }));
    }

    #[test]
    fn command_mode_reads_only_the_arguments() {
        let c = TtsConfig { command: Some("tts".into()), ..cfg() }.sanitized();
        match decide(&c, &chat("!TTS  buenas noches ")) {
            ChatDecision::Speak { text, .. } => assert_eq!(text, "buenas noches"),
            other => panic!("{other:?}"),
        }
        assert_eq!(skip_reason(decide(&c, &chat("hola"))), "no es el comando");
        assert_eq!(skip_reason(decide(&c, &chat("!tts"))), "no es el comando");
        assert_eq!(skip_reason(decide(&c, &chat("!ttsx hola"))), "no es el comando");
    }

    #[test]
    fn role_and_level_filters() {
        let c = TtsConfig { roles_any: vec![Role::Moderator, Role::Subscriber], ..cfg() };
        let mut ev = chat("hola");
        assert_eq!(skip_reason(decide(&c, &ev)), "no cumple el rol o nivel");
        ev.user.is_subscriber = true;
        assert!(matches!(decide(&c, &ev), ChatDecision::Speak { .. }));

        let lvl = TtsConfig { min_gifter_level: Some(10), min_team_level: Some(2), ..cfg() };
        let mut ev = chat("hola");
        ev.user.gifter_level = Some(10);
        assert!(matches!(decide(&lvl, &ev), ChatDecision::Skip(_)), "falta nivel de equipo");
        ev.user.team_level = Some(2);
        assert!(matches!(decide(&lvl, &ev), ChatDecision::Speak { .. }));
    }

    #[test]
    fn recent_donors_filter_uses_the_gift_window() {
        let c = TtsConfig { recent_donors_minutes: Some(10), ..cfg() };
        let mut mem = ChatMemory::default();
        let ev = chat("gracias por el directo");
        let now = 10_000_000;
        assert_eq!(skip_reason(decide_chat(&c, &ev, &mut mem, now)), "no es donador reciente");

        let mut gift = sample_event("g");
        gift.kind = EventType::Gift;
        gift.gift = Some(Gift { id: 1, name: "Rose".into(), coins: 1, count: 1, streakable: false, image: String::new() });
        gift.ts = now - 9 * 60_000;
        mem.observe(&gift);
        assert!(matches!(decide_chat(&c, &ev, &mut mem, now), ChatDecision::Speak { .. }));
        assert_eq!(skip_reason(decide_chat(&c, &ev, &mut mem, now + 2 * 60_000)), "no es donador reciente");
    }

    #[test]
    fn user_cooldown_only_counts_messages_that_were_read() {
        let c = TtsConfig { user_cooldown_ms: 5_000, ..cfg() };
        let mut mem = ChatMemory::default();
        assert!(matches!(decide_chat(&c, &chat("uno"), &mut mem, 0), ChatDecision::Speak { .. }));
        assert_eq!(skip_reason(decide_chat(&c, &chat("dos"), &mut mem, 4_999)), "cooldown del usuario");
        assert!(matches!(decide_chat(&c, &chat("tres"), &mut mem, 5_000), ChatDecision::Speak { .. }));
        // Un mensaje filtrado no gasta el turno del usuario.
        let mut mem = ChatMemory::default();
        assert_eq!(skip_reason(decide_chat(&c, &chat("eres un pendejo"), &mut mem, 0)), "grosería");
        assert!(matches!(decide_chat(&c, &chat("hola"), &mut mem, 1), ChatDecision::Speak { .. }));
    }

    #[test]
    fn ignored_users_are_never_read() {
        let c = TtsConfig { ignore_users: vec!["@ANA".into()], ..cfg() }.sanitized();
        assert_eq!(skip_reason(decide(&c, &chat("hola"))), "usuario ignorado");
    }

    #[test]
    fn filters_are_applied_before_reading() {
        match decide(&cfg(), &chat("mira https://spam.com 🔥🔥 holaaaaaaa")) {
            ChatDecision::Speak { text, .. } => assert_eq!(text, "mira holaaa"),
            other => panic!("{other:?}"),
        }
        assert_eq!(skip_reason(decide(&cfg(), &chat("https://spam.com"))), "sin texto legible");
    }

    #[test]
    fn non_chat_events_are_not_read() {
        let mut ev = sample_event("f");
        ev.kind = EventType::Follow;
        assert_eq!(skip_reason(decide(&cfg(), &ev)), "desactivado");
    }

    #[test]
    fn memory_is_pruned() {
        let mut mem = ChatMemory::default();
        mem.last_gift_ms.insert("viejo".into(), 0);
        mem.last_gift_ms.insert("nuevo".into(), 9_000);
        mem.last_spoken_ms.insert("viejo".into(), 0);
        mem.prune(10_000, 5_000);
        assert_eq!(mem.last_gift_ms.len(), 1);
        assert!(mem.last_spoken_ms.is_empty());
    }

    #[test]
    fn sanitized_fixes_out_of_range_values() {
        let c = TtsConfig {
            rate: 9.0,
            volume: 250,
            command: Some(" !TTS ".into()),
            template: "  ".into(),
            max_wait_ms: 1,
            ..Default::default()
        }
        .sanitized();
        assert_eq!((c.rate, c.volume), (2.0, 100));
        assert_eq!(c.command.as_deref(), Some("tts"));
        assert_eq!(c.template, "{nickname} dice: {text}");
        assert_eq!(c.max_wait_ms, 1_000);
        assert_eq!(TtsConfig { rate: f64::NAN, ..Default::default() }.sanitized().rate, 1.0);
        assert_eq!(TtsConfig { command: Some("!".into()), ..Default::default() }.sanitized().command, None);
    }

    #[test]
    fn config_defaults_and_partial_json() {
        let d = TtsConfig::default();
        assert!(!d.enabled && d.rate == 1.0 && d.volume == 100 && d.ignore_bang_commands);
        let c: TtsConfig = serde_json::from_str(r#"{"enabled": true, "voiceMode": "byRole"}"#).expect("parsea");
        assert!(c.enabled);
        assert_eq!(c.voice_mode, VoiceMode::ByRole);
        assert_eq!(c.template, "{nickname} dice: {text}");
    }

    // ---- Voces ----

    fn voices() -> Vec<VoiceInfo> {
        ["piper:a", "piper:b", "sapi:c", "edge:d"]
            .iter()
            .map(|id| {
                let (engine, name) = id.split_once(':').expect("id");
                VoiceInfo { id: (*id).into(), engine: engine.into(), name: name.into(), lang: None }
            })
            .collect()
    }

    fn vars(pairs: &[(&str, &str)]) -> Vars {
        pairs.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect()
    }

    #[test]
    fn explicit_voice_wins_when_it_exists() {
        let c = TtsConfig { default_voice: Some("piper:a".into()), ..Default::default() };
        assert_eq!(resolve_voice(&c, &vars(&[]), &voices(), Some("sapi:c")).as_deref(), Some("sapi:c"));
        assert_eq!(resolve_voice(&c, &vars(&[]), &voices(), Some("fantasma:x")).as_deref(), Some("piper:a"));
    }

    #[test]
    fn falls_back_to_default_then_to_the_first_available() {
        let c = TtsConfig { default_voice: Some("edge:d".into()), ..Default::default() };
        assert_eq!(resolve_voice(&c, &vars(&[]), &voices(), None).as_deref(), Some("edge:d"));
        let missing = TtsConfig { default_voice: Some("piper:no-existe".into()), ..Default::default() };
        assert_eq!(resolve_voice(&missing, &vars(&[]), &voices(), None).as_deref(), Some("piper:a"));
        assert_eq!(resolve_voice(&missing, &vars(&[]), &[], None), None);
    }

    #[test]
    fn by_role_prefers_moderator_over_subscriber_over_follower() {
        let c = TtsConfig {
            voice_mode: VoiceMode::ByRole,
            default_voice: Some("piper:a".into()),
            role_voices: RoleVoices {
                moderator: Some("piper:b".into()),
                subscriber: Some("sapi:c".into()),
                follower: Some("edge:d".into()),
            },
            ..Default::default()
        };
        let pick = |v: &[(&str, &str)]| resolve_voice(&c, &vars(v), &voices(), None);
        assert_eq!(pick(&[("ismoderator", "true"), ("issubscriber", "true")]).as_deref(), Some("piper:b"));
        assert_eq!(pick(&[("issubscriber", "true"), ("isfollower", "true")]).as_deref(), Some("sapi:c"));
        assert_eq!(pick(&[("isfollower", "true")]).as_deref(), Some("edge:d"));
        assert_eq!(pick(&[("isfollower", "false")]).as_deref(), Some("piper:a"));
    }

    #[test]
    fn random_per_user_is_stable_and_spreads_users() {
        let c = TtsConfig {
            voice_mode: VoiceMode::RandomPerUser,
            random_voices: vec!["piper:a".into(), "piper:b".into(), "sapi:c".into(), "ghost:x".into()],
            ..Default::default()
        };
        let voice_of = |id: &str| resolve_voice(&c, &vars(&[("userid", id)]), &voices(), None).expect("voz");
        assert_eq!(voice_of("user-42"), voice_of("user-42"), "siempre la misma voz para la misma persona");
        let distinct: std::collections::HashSet<_> = (0..200).map(|i| voice_of(&format!("u{i}"))).collect();
        assert_eq!(distinct.len(), 3, "usa todas las voces válidas y descarta las inexistentes: {distinct:?}");
        assert!(!distinct.contains("ghost:x"));
    }

    #[test]
    fn stable_hash_is_deterministic_across_runs() {
        // Valor fijo: si cambia, los usuarios cambiarían de voz tras actualizar la app.
        assert_eq!(stable_hash("user-42"), stable_hash("user-42"));
        assert_eq!(stable_hash(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(stable_hash("a"), 0xaf63_dc4c_8601_ec8c);
    }
}
