//! Registro en memoria para la pestaña de logs de la UI: un `Layer` de `tracing` que guarda las
//! últimas entradas en un anillo. Los secretos no se registran en ningún sitio de la app; como red de seguridad,
//! cada mensaje pasa por [`redact`] (tokens, contraseñas, `Bearer…`) antes de guardarse, de modo que ni la pantalla,
//! ni `hivebuzz.log`, ni el archivo que se exporta para soporte pueden llevarlos. Además cada mensaje se acota a una
//! longitud razonable.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

const CAPACITY: usize = 2_000;
const MAX_MESSAGE_CHARS: usize = 1_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub ts_ms: i64,
    /// `ERROR`, `WARN`, `INFO`, `DEBUG` o `TRACE`.
    pub level: String,
    pub target: String,
    pub message: String,
}

#[derive(Clone)]
pub struct LogBuffer {
    inner: Arc<Mutex<VecDeque<LogEntry>>>,
    capacity: usize,
    file: Arc<Mutex<Option<FileSink>>>,
}

/// Tamaño a partir del cual `hivebuzz.log` pasa a `hivebuzz.log.1` (solo se conserva una copia).
const MAX_LOG_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// Archivo de registro en disco: lo que queda tras un cierre inesperado o un reinicio, para soporte.
struct FileSink {
    path: PathBuf,
    file: File,
    written: u64,
}

impl FileSink {
    fn open(path: PathBuf) -> std::io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let written = file.metadata().map_or(0, |m| m.len());
        Ok(Self { path, file, written })
    }

    fn write_line(&mut self, line: &str) {
        if self.written >= MAX_LOG_FILE_BYTES {
            let mut old = self.path.as_os_str().to_owned();
            old.push(".1");
            let _ = std::fs::rename(&self.path, PathBuf::from(old));
            match OpenOptions::new().create(true).append(true).open(&self.path) {
                Ok(f) => {
                    self.file = f;
                    self.written = 0;
                }
                Err(_) => return,
            }
        }
        if self.file.write_all(line.as_bytes()).is_ok() {
            self.written += line.len() as u64;
        }
    }
}

/// Gravedad como número (mayor = más grave) para filtrar por «nivel mínimo».
fn severity(level: &str) -> u8 {
    match level {
        "ERROR" => 4,
        "WARN" => 3,
        "INFO" => 2,
        "DEBUG" => 1,
        _ => 0,
    }
}

impl LogBuffer {
    pub fn new(capacity: usize) -> Self {
        Self { inner: Arc::new(Mutex::new(VecDeque::with_capacity(capacity.min(CAPACITY)))), capacity: capacity.max(1), file: Arc::default() }
    }

    /// Empieza a guardar también en `path` (INFO o más grave). Un fallo al abrir no es grave: se
    /// sigue con el anillo en memoria.
    pub fn enable_file(&self, path: PathBuf) {
        match FileSink::open(path) {
            Ok(sink) => *self.file.lock().unwrap_or_else(PoisonError::into_inner) = Some(sink),
            Err(e) => tracing::warn!(error = %e, "no se pudo abrir el archivo de registro"),
        }
    }

    pub fn push(&self, entry: LogEntry) {
        if severity(&entry.level) >= severity("INFO") {
            if let Some(sink) = self.file.lock().unwrap_or_else(PoisonError::into_inner).as_mut() {
                sink.write_line(&format!("{} {:5} {}: {}
", entry.ts_ms, entry.level, entry.target, entry.message));
            }
        }
        let mut q = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if q.len() >= self.capacity {
            q.pop_front();
        }
        q.push_back(entry);
    }

    /// Entradas de nivel `min_level` o más grave, de la más vieja a la más nueva (las últimas `limit`).
    pub fn snapshot(&self, min_level: &str, limit: usize) -> Vec<LogEntry> {
        let min = severity(&min_level.to_ascii_uppercase());
        let q = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let mut out: Vec<LogEntry> = q.iter().rev().filter(|e| severity(&e.level) >= min).take(limit).cloned().collect();
        out.reverse();
        out
    }

    pub fn clear(&self) {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner).clear();
    }

    /// Texto plano para guardar en un archivo (por si el usuario lo adjunta a un reporte).
    pub fn to_text(&self) -> String {
        let q = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let mut out = String::new();
        for e in q.iter() {
            let _ = writeln!(out, "{} {:5} {}: {}", e.ts_ms, e.level, e.target, e.message);
        }
        out
    }
}

/// El anillo compartido de toda la app (el subscriber se instala antes que el estado).
pub fn global() -> &'static LogBuffer {
    static BUFFER: OnceLock<LogBuffer> = OnceLock::new();
    BUFFER.get_or_init(|| LogBuffer::new(CAPACITY))
}

pub struct BufferLayer(pub LogBuffer);

#[derive(Default)]
struct MessageVisitor {
    message: String,
    fields: String,
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.message, "{value:?}");
        } else {
            let _ = write!(self.fields, " {}={value:?}", field.name());
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            let _ = write!(self.fields, " {}={value}", field.name());
        }
    }
}

impl<S: Subscriber> Layer<S> for BufferLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        // TRACE y DEBUG de dependencias ruidosas no aportan al usuario.
        if *meta.level() == Level::TRACE {
            return;
        }
        let mut v = MessageVisitor::default();
        event.record(&mut v);
        let mut message = redact(&format!("{}{}", v.message, v.fields));
        if message.chars().count() > MAX_MESSAGE_CHARS {
            message = message.chars().take(MAX_MESSAGE_CHARS).collect::<String>() + "…";
        }
        let ts_ms = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
        self.0.push(LogEntry { ts_ms, level: meta.level().to_string(), target: meta.target().to_string(), message });
    }
}

/// Texto que sustituye a un secreto.
const MASK: &str = "***";
/// Un valor más corto que esto no se enmascara: no puede ser un token (evita tapar `token=5`).
const MIN_SECRET_CHARS: usize = 6;
/// Nombres de parámetro cuyo valor es secreto. Se busca como sufijo, así que `access_token=`, `overlay_token=` y
/// `client_secret=` entran sin listarlos uno a uno; `tokens=` no (después del nombre debe venir `=` o `:`).
const SECRET_KEYS: [&str; 6] = ["token", "secret", "password", "apikey", "api_key", "sessionid"];

/// Fin de un valor que empieza en `start`: hasta un separador habitual de URL, JSON, cabeceras o texto.
fn value_end(s: &str, start: usize) -> usize {
    s[start..]
        .char_indices()
        .find(|(_, c)| c.is_whitespace() || matches!(c, '&' | '"' | '\'' | ',' | ';' | ')' | ']' | '}' | '<' | '>' | '\\'))
        .map_or(s.len(), |(i, _)| start + i)
}

/// Enmascara los secretos de un texto de registro: `token=…`, `"access_token":"…"`, `password: …`, `Bearer …` y
/// `oauth:…`. Es una defensa en profundidad: la app no debería registrar nunca un secreto, pero un olvido no debe
/// acabar en un archivo que el usuario envía a soporte.
pub fn redact(input: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let bytes = input.as_bytes();
    let mut spans: Vec<(usize, usize)> = Vec::new();

    for key in SECRET_KEYS {
        for (pos, _) in lower.match_indices(key) {
            let mut i = pos + key.len();
            if matches!(bytes.get(i), Some(b'"' | b'\'')) {
                i += 1;
            }
            if !matches!(bytes.get(i), Some(b'=' | b':')) {
                continue;
            }
            i += 1;
            while matches!(bytes.get(i), Some(b' ')) {
                i += 1;
            }
            if matches!(bytes.get(i), Some(b'"' | b'\'')) {
                i += 1;
            }
            let end = value_end(input, i);
            if input[i..end].chars().count() >= MIN_SECRET_CHARS {
                spans.push((i, end));
            }
        }
    }
    for prefix in ["bearer ", "oauth:"] {
        for (pos, _) in lower.match_indices(prefix) {
            let start = pos + prefix.len();
            let end = value_end(input, start);
            if input[start..end].chars().count() >= MIN_SECRET_CHARS {
                spans.push((start, end));
            }
        }
    }
    if spans.is_empty() {
        return input.to_string();
    }

    spans.sort_unstable();
    let mut out = String::with_capacity(input.len());
    let mut at = 0;
    for (start, end) in spans {
        if end <= at {
            continue;
        }
        out.push_str(&input[at..start.max(at)]);
        out.push_str(MASK);
        at = end;
    }
    out.push_str(&input[at..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::layer::SubscriberExt;

    fn capture(f: impl FnOnce()) -> Vec<LogEntry> {
        let buf = LogBuffer::new(10);
        let sub = tracing_subscriber::registry().with(BufferLayer(buf.clone()));
        tracing::subscriber::with_default(sub, f);
        buf.snapshot("debug", 100)
    }

    #[test]
    fn records_message_fields_and_level() {
        let logs = capture(|| {
            tracing::warn!(user = "ana", n = 3, "algo pasó");
        });
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].level, "WARN");
        assert_eq!(logs[0].message, "algo pasó user=ana n=3");
    }

    #[test]
    fn trace_is_skipped_and_filtering_by_level_works() {
        let buf = LogBuffer::new(10);
        let sub = tracing_subscriber::registry().with(BufferLayer(buf.clone()));
        tracing::subscriber::with_default(sub, || {
            tracing::trace!("ruido");
            tracing::debug!("d");
            tracing::info!("i");
            tracing::error!("e");
        });
        assert_eq!(buf.snapshot("trace", 10).len(), 3);
        let warn_up: Vec<String> = buf.snapshot("WARN", 10).into_iter().map(|e| e.message).collect();
        assert_eq!(warn_up, ["e"]);
        let last_two: Vec<String> = buf.snapshot("debug", 2).into_iter().map(|e| e.message).collect();
        assert_eq!(last_two, ["i", "e"], "las últimas, en orden cronológico");
    }

    #[test]
    fn the_ring_drops_the_oldest_and_can_be_cleared() {
        let buf = LogBuffer::new(3);
        for i in 0..5 {
            buf.push(LogEntry { ts_ms: i, level: "INFO".into(), target: "t".into(), message: format!("m{i}") });
        }
        let msgs: Vec<String> = buf.snapshot("info", 10).into_iter().map(|e| e.message).collect();
        assert_eq!(msgs, ["m2", "m3", "m4"]);
        assert!(buf.to_text().contains("m4"));
        buf.clear();
        assert!(buf.snapshot("info", 10).is_empty());
    }

    #[test]
    fn very_long_messages_are_truncated() {
        let logs = capture(|| tracing::info!("{}", "x".repeat(5_000)));
        assert!(logs[0].message.chars().count() <= MAX_MESSAGE_CHARS + 1);
        assert!(logs[0].message.ends_with('…'));
    }

    #[test]
    fn file_sink_keeps_info_and_above_and_rotates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hivebuzz.log");
        let buf = LogBuffer::new(10);
        buf.enable_file(path.clone());
        let entry = |level: &str, message: &str| LogEntry { ts_ms: 1, level: level.into(), target: "t".into(), message: message.into() };
        buf.push(entry("DEBUG", "ruido"));
        buf.push(entry("INFO", "hola"));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("hola") && !text.contains("ruido"), "{text}");

        // Pasado el límite, el archivo viejo se aparta y se empieza otro.
        buf.push(entry("WARN", &"x".repeat(usize::try_from(MAX_LOG_FILE_BYTES).unwrap())));
        buf.push(entry("ERROR", "nuevo"));
        let fresh = std::fs::read_to_string(&path).unwrap();
        assert!(fresh.contains("nuevo") && !fresh.contains("hola"), "{fresh}");
        assert!(dir.path().join("hivebuzz.log.1").is_file());
    }

    #[test]
    fn redact_hides_secrets_in_urls_headers_and_json() {
        let hex = "0123456789abcdef0123456789abcdef";
        for (input, secret) in [
            (format!("GET /overlay/chat?token={hex}&lang=en"), hex),
            (format!("falló https://x.com/api?access_token={hex}"), hex),
            (format!("overlay_token={hex}"), hex),
            ("Authorization: Bearer abcdef123456".to_string(), "abcdef123456"),
            ("PASS oauth:abcdef1234567890".to_string(), "abcdef1234567890"),
            (r#"{"refresh_token":"abcdef123456","x":1}"#.to_string(), "abcdef123456"),
            (r#"{"client_secret": "abcdef123456"}"#.to_string(), "abcdef123456"),
            ("password: hunter22hunter".to_string(), "hunter22hunter"),
            ("api_key=SECRETVALUE99".to_string(), "SECRETVALUE99"),
        ] {
            let out = redact(&input);
            assert!(!out.contains(secret), "{input} → {out}");
            assert!(out.contains(MASK), "{input} → {out}");
        }
    }

    #[test]
    fn redact_keeps_the_rest_of_the_message_readable() {
        assert_eq!(redact("GET /overlay/chat?token=0123456789abcdef&lang=en ok"), "GET /overlay/chat?token=***&lang=en ok");
        assert_eq!(redact(r#"{"access_token":"abcdef123456","expires_in":3600}"#), r#"{"access_token":"***","expires_in":3600}"#);
        assert_eq!(redact("Authorization: Bearer abcdef123456 (reintento 2)"), "Authorization: Bearer *** (reintento 2)");
    }

    #[test]
    fn redact_does_not_touch_ordinary_messages() {
        for msg in [
            "conectado a @ana",
            "tokens=5 usados",
            "token=5",
            "el token es válido",
            "status code=404 en /api",
            "secret sauce",
            "Twitch: reconectando canal=ana motivo=timeout intento=3",
        ] {
            assert_eq!(redact(msg), msg);
        }
    }

    #[test]
    fn redact_handles_repeats_overlaps_and_non_ascii() {
        let out = redact("a token=abcdef123456 b token=ghijkl654321 c Bearer token=zzzzzz999999 ñ ✓");
        assert!(!out.contains("abcdef") && !out.contains("ghijkl") && !out.contains("zzzzzz"), "{out}");
        assert!(out.ends_with("ñ ✓"), "{out}");
        assert_eq!(redact(""), "");
        assert_eq!(redact("token=abcdef123456"), "token=***");
    }

    #[test]
    fn secrets_never_reach_the_buffer_or_the_exported_text() {
        let logs = capture(|| {
            tracing::warn!(url = "http://127.0.0.1:17890/overlay/chat?token=0123456789abcdef", "petición rara");
        });
        assert!(!logs[0].message.contains("0123456789abcdef"), "{}", logs[0].message);
        assert!(logs[0].message.contains("token=***"), "{}", logs[0].message);
    }
}
