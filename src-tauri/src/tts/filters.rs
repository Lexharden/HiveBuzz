//! Limpieza del texto antes de leerlo en voz alta: groserías, enlaces, emojis, repeticiones y
//! límite de caracteres. Todo es puro y se prueba sin audio.

use serde::{Deserialize, Serialize};

/// Qué hacer cuando el mensaje contiene una grosería.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProfanityMode {
    /// No leer el mensaje.
    #[default]
    Skip,
    /// Sustituir la palabra por «bip».
    Censor,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterConfig {
    #[serde(default = "yes")]
    pub profanity_enabled: bool,
    #[serde(default)]
    pub profanity_mode: ProfanityMode,
    /// Lista editable por el usuario. Se compara sin acentos, sin mayúsculas y sin letras repetidas.
    #[serde(default = "default_words")]
    pub profanity_words: Vec<String>,
    #[serde(default = "yes")]
    pub skip_links: bool,
    #[serde(default = "yes")]
    pub strip_emojis: bool,
    /// Máximo de caracteres iguales seguidos («holaaaaaa» → «holaaa»).
    #[serde(default = "default_run")]
    pub max_repeat: usize,
    #[serde(default = "default_max_chars")]
    pub max_chars: usize,
}

fn yes() -> bool {
    true
}
fn default_run() -> usize {
    3
}
fn default_max_chars() -> usize {
    200
}

impl Default for FilterConfig {
    fn default() -> Self {
        Self {
            profanity_enabled: true,
            profanity_mode: ProfanityMode::Skip,
            profanity_words: default_words(),
            skip_links: true,
            strip_emojis: true,
            max_repeat: 3,
            max_chars: 200,
        }
    }
}

/// Lista inicial, corta y editable. No pretende ser exhaustiva: cada streamer la ajusta.
fn default_words() -> Vec<String> {
    [
        "puta", "puto", "putos", "putas", "mierda", "pendejo", "pendeja", "cabron", "cabrona", "verga", "chingar",
        "chingada", "chingado", "joder", "coño", "culero", "culera", "marica", "maricon", "hijueputa", "gonorrea",
        "fuck", "fucking", "shit", "bitch", "asshole", "cunt", "dick", "bastard", "slut", "whore",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect()
}

/// Resultado de limpiar un texto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cleaned {
    /// Texto listo para leer.
    Say(String),
    /// No se debe leer (vacío, solo enlaces, grosería con modo `Skip`…).
    Skip(&'static str),
}

pub fn clean(text: &str, cfg: &FilterConfig) -> Cleaned {
    let mut s = text.trim().to_string();

    if cfg.skip_links {
        s = remove_links(&s);
    }
    if cfg.strip_emojis {
        s = strip_emojis(&s);
    }
    if cfg.max_repeat > 0 {
        s = collapse_repeats(&s, cfg.max_repeat);
    }
    s = collapse_spaces(&s);

    if cfg.profanity_enabled {
        let bad = Matcher::new(&cfg.profanity_words);
        match cfg.profanity_mode {
            ProfanityMode::Skip => {
                if bad.contains_any(&s) {
                    return Cleaned::Skip("grosería");
                }
            }
            ProfanityMode::Censor => s = bad.censor(&s),
        }
    }

    let s = truncate(&s, cfg.max_chars);
    if s.chars().all(|c| !c.is_alphanumeric()) {
        return Cleaned::Skip("sin texto legible");
    }
    Cleaned::Say(s)
}

// ---- Enlaces ------------------------------------------------------------------------------------

const TLDS: &[&str] = &[
    "com", "net", "org", "io", "tv", "gg", "me", "ly", "co", "es", "mx", "ar", "cl", "pe", "app", "xyz", "info", "dev",
    "link", "live", "to", "cc", "us", "uk", "ru", "de", "fr", "br", "ve", "ec", "bo", "uy",
];

fn is_link(token: &str) -> bool {
    let t = token
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '/' && c != ':' && c != '.')
        .to_lowercase();
    if t.contains("://") || t.starts_with("www.") {
        return true;
    }
    // dominio.tld o dominio.tld/ruta
    let host = t.split('/').next().unwrap_or("");
    match host.rsplit_once('.') {
        Some((name, tld)) => {
            !name.is_empty()
                && name.chars().any(char::is_alphanumeric)
                && name.chars().all(|c| c.is_alphanumeric() || c == '.' || c == '-')
                && TLDS.contains(&tld)
        }
        None => false,
    }
}

pub fn remove_links(text: &str) -> String {
    text.split_whitespace().filter(|t| !is_link(t)).collect::<Vec<_>>().join(" ")
}

// ---- Emojis y repeticiones ------------------------------------------------------------------------

fn is_emoji(c: char) -> bool {
    matches!(c as u32,
        0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x2300..=0x23FF | 0x2B00..=0x2BFF
        | 0xFE00..=0xFE0F | 0x200D | 0x20E3 | 0xE0020..=0xE007F)
}

pub fn strip_emojis(text: &str) -> String {
    text.chars().filter(|c| !is_emoji(*c)).collect()
}

/// Recorta las rachas de un mismo carácter a `max_run`.
pub fn collapse_repeats(text: &str, max_run: usize) -> String {
    let mut out = String::with_capacity(text.len());
    let (mut last, mut run) = (None::<char>, 0usize);
    for c in text.chars() {
        if Some(c) == last {
            run += 1;
        } else {
            last = Some(c);
            run = 1;
        }
        if run <= max_run {
            out.push(c);
        }
    }
    out
}

fn collapse_spaces(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Recorta a `max` caracteres, sin partir una palabra si se puede evitar.
pub fn truncate(text: &str, max: usize) -> String {
    if max == 0 || text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    match cut.rfind(' ') {
        Some(i) if i > max / 2 => cut[..i].trim_end().to_string(),
        _ => cut.trim_end().to_string(),
    }
}

// ---- Groserías ------------------------------------------------------------------------------------

/// Minúsculas, sin acentos, con sustituciones típicas («p3nd3j0» → «pendejo») y sin letras repetidas.
fn normalize_word(w: &str) -> String {
    let mut out = String::with_capacity(w.len());
    let mut last = None::<char>;
    for c in w.chars().flat_map(char::to_lowercase) {
        let c = match c {
            'á' | 'à' | 'ä' | 'â' | '@' | '4' => 'a',
            'é' | 'è' | 'ë' | 'ê' | '3' => 'e',
            'í' | 'ì' | 'ï' | 'î' | '1' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' | '0' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            '$' | '5' => 's',
            other => other,
        };
        if Some(c) != last {
            out.push(c);
        }
        last = Some(c);
    }
    out
}

/// Letras, números y los símbolos que se usan para disfrazar letras («p@ta», «mi3rda»).
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '@' || c == '$'
}

/// Conserva «ñ» distinta de «n» en la lista del usuario pero la iguala al comparar.
fn fold_n(s: &str) -> String {
    s.replace(['ñ', 'Ñ'], "n")
}

struct Matcher {
    words: Vec<String>,
}

impl Matcher {
    fn new(list: &[String]) -> Self {
        let words = list
            .iter()
            .map(|w| normalize_word(&fold_n(w.trim())))
            .filter(|w| !w.is_empty())
            .collect();
        Self { words }
    }

    fn is_bad(&self, token: &str) -> bool {
        let norm = normalize_word(&fold_n(token));
        !norm.is_empty() && self.words.contains(&norm)
    }

    fn contains_any(&self, text: &str) -> bool {
        text.split(|c: char| !is_word_char(c)).any(|t| self.is_bad(t))
    }

    fn censor(&self, text: &str) -> String {
        text.split_whitespace()
            .map(|chunk| {
                // Se conserva la puntuación alrededor de la palabra.
                let core = chunk.trim_matches(|c: char| !is_word_char(c));
                if !core.is_empty() && self.is_bad(core) {
                    chunk.replacen(core, "bip", 1)
                } else {
                    chunk.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn say(text: &str) -> Cleaned {
        clean(text, &FilterConfig::default())
    }

    #[test]
    fn plain_text_passes_through() {
        assert_eq!(say("¡Hola a todos!"), Cleaned::Say("¡Hola a todos!".into()));
    }

    #[test]
    fn links_are_removed_in_all_common_forms() {
        for link in ["https://x.com/a?b=1", "http://localhost:3000", "www.sitio.net", "mi-canal.tv", "discord.gg/abc", "ejemplo.com/ruta"] {
            assert_eq!(say(&format!("mira {link} ahora")), Cleaned::Say("mira ahora".into()), "{link}");
        }
        assert_eq!(say("https://spam.com"), Cleaned::Skip("sin texto legible"));
    }

    #[test]
    fn ordinary_dots_are_not_links() {
        assert_eq!(say("hola.que tal"), Cleaned::Say("hola.que tal".into()));
        assert_eq!(say("son las 10.30 pm"), Cleaned::Say("son las 10.30 pm".into()));
        assert_eq!(say("fin."), Cleaned::Say("fin.".into()));
    }

    #[test]
    fn links_can_be_kept_when_configured() {
        let cfg = FilterConfig { skip_links: false, ..Default::default() };
        assert_eq!(clean("ve a ejemplo.com", &cfg), Cleaned::Say("ve a ejemplo.com".into()));
    }

    #[test]
    fn emojis_are_stripped_and_emoji_only_messages_skipped() {
        assert_eq!(say("buen directo 🔥🔥🔥 ❤️"), Cleaned::Say("buen directo".into()));
        assert_eq!(say("🔥🔥🔥"), Cleaned::Skip("sin texto legible"));
        assert_eq!(say("¿qué tal? 👋"), Cleaned::Say("¿qué tal?".into()));
    }

    #[test]
    fn repeated_characters_are_collapsed() {
        assert_eq!(say("holaaaaaaaa"), Cleaned::Say("holaaa".into()));
        assert_eq!(say("!!!!!!!! hola"), Cleaned::Say("!!! hola".into()));
        assert_eq!(collapse_repeats("aabbbbcc", 2), "aabbcc");
    }

    #[test]
    fn long_messages_are_cut_on_a_word_boundary() {
        let cfg = FilterConfig { max_chars: 20, ..Default::default() };
        match clean("uno dos tres cuatro cinco seis", &cfg) {
            Cleaned::Say(s) => {
                assert!(s.chars().count() <= 20);
                assert_eq!(s, "uno dos tres cuatro");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(truncate("corto", 10), "corto");
        assert_eq!(truncate("sinespaciosnada", 5), "sines");
        assert_eq!(truncate("lo que sea", 0), "lo que sea");
    }

    #[test]
    fn profanity_skips_the_whole_message_by_default() {
        for msg in ["eres un pendejo", "PENDEJO!", "p3nd3j0", "pendeeeejooo", "qué mierda", "what the FUCK"] {
            assert_eq!(say(msg), Cleaned::Skip("grosería"), "{msg}");
        }
    }

    #[test]
    fn profanity_matches_whole_words_only() {
        // «computadora» no contiene ninguna palabra de la lista como palabra completa.
        for ok in ["mi computadora", "Scunthorpe", "el pene", "assistant", "putamen"] {
            assert!(matches!(say(ok), Cleaned::Say(_)), "{ok}");
        }
    }

    #[test]
    fn profanity_can_be_censored_instead() {
        let cfg = FilterConfig { profanity_mode: ProfanityMode::Censor, ..Default::default() };
        assert_eq!(clean("qué mierda, amigo", &cfg), Cleaned::Say("qué bip, amigo".into()));
        assert_eq!(clean("eres un Pendejo.", &cfg), Cleaned::Say("eres un bip.".into()));
    }

    #[test]
    fn profanity_filter_can_be_disabled_and_the_list_edited() {
        let off = FilterConfig { profanity_enabled: false, ..Default::default() };
        assert_eq!(clean("pendejo", &off), Cleaned::Say("pendejo".into()));
        let custom = FilterConfig { profanity_words: vec!["spoiler".into(), "Ñoño".into()], ..Default::default() };
        assert_eq!(clean("spoiler!", &custom), Cleaned::Skip("grosería"));
        assert_eq!(clean("qué ñoño", &custom), Cleaned::Skip("grosería"));
        assert!(matches!(clean("pendejo", &custom), Cleaned::Say(_)), "la lista por defecto se reemplaza");
    }

    #[test]
    fn empty_and_punctuation_only_are_skipped() {
        assert_eq!(say(""), Cleaned::Skip("sin texto legible"));
        assert_eq!(say("   "), Cleaned::Skip("sin texto legible"));
        assert_eq!(say("?!?!"), Cleaned::Skip("sin texto legible"));
    }

    #[test]
    fn config_roundtrips_and_fills_defaults_from_partial_json() {
        let cfg: FilterConfig = serde_json::from_str(r#"{"maxChars": 50}"#).expect("parsea");
        assert_eq!(cfg.max_chars, 50);
        assert!(cfg.profanity_enabled && cfg.skip_links && cfg.strip_emojis);
        assert!(!cfg.profanity_words.is_empty());
    }
}
