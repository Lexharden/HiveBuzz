//! Protocolo NDJSON entre Rust y el sidecar (una línea JSON por mensaje).
//! Espejo de `sidecar/src/protocol.ts`.

use serde::{Deserialize, Serialize};

use crate::events::LiveEvent;

/// Estados de conexión visibles en la UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    Disconnected,
    WaitingLive,
    Connected,
    SignatureError,
    Reconnecting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

/// Sidecar → Rust (stdout).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SidecarMessage {
    /// El sidecar terminó de arrancar y acepta comandos.
    Ready,
    Event {
        event: Box<LiveEvent>,
    },
    #[serde(rename_all = "camelCase")]
    Status {
        state: ConnectionState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
        /// Número de intento de reconexión, si aplica.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attempt: Option<u32>,
        /// Milisegundos hasta el próximo intento, si aplica.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        retry_in_ms: Option<u64>,
    },
    Log {
        level: LogLevel,
        message: String,
    },
    /// Espectadores conectados ahora mismo.
    Viewers {
        count: u64,
    },
    /// Resultado de un `SendChat` (se corresponde por `request_id`).
    #[serde(rename_all = "camelCase")]
    ChatResult {
        request_id: String,
        ok: bool,
        #[serde(default)]
        error: Option<String>,
    },
}

/// Sesión de TikTok del usuario (las dos cookies). Sus valores son secretos: `Debug` no los muestra.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPayload {
    pub session_id: String,
    pub tt_target_idc: String,
}

impl std::fmt::Debug for SessionPayload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionPayload { .. }")
    }
}

/// Rust → sidecar (stdin). `Debug` es manual: oculta la API key de Euler y la sesión de TikTok.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "camelCase")]
pub enum SidecarCommand {
    #[serde(rename_all = "camelCase")]
    Connect {
        unique_id: String,
        /// API key propia de Euler Stream (opcional; viene del keyring).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        euler_api_key: Option<String>,
        /// Sesión de TikTok (opcional; solo hace falta para que el bot escriba en el chat).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<SessionPayload>,
    },
    Disconnect,
    #[serde(rename_all = "camelCase")]
    SendChat {
        request_id: String,
        text: String,
    },
    Shutdown,
}

impl std::fmt::Debug for SidecarCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connect { unique_id, euler_api_key, session } => f
                .debug_struct("Connect")
                .field("unique_id", unique_id)
                .field("euler_api_key", &euler_api_key.as_ref().map(|_| "***"))
                .field("session", &session.as_ref().map(|_| "***"))
                .finish(),
            Self::Disconnect => f.write_str("Disconnect"),
            Self::SendChat { request_id, text } => f.debug_struct("SendChat").field("request_id", request_id).field("text", text).finish(),
            Self::Shutdown => f.write_str("Shutdown"),
        }
    }
}

impl SidecarCommand {
    /// Serializa a una línea NDJSON terminada en `\n`.
    pub fn to_line(&self) -> Result<String, serde_json::Error> {
        let mut s = serde_json::to_string(self)?;
        s.push('\n');
        Ok(s)
    }
}

impl SidecarMessage {
    pub fn from_line(line: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(line.trim())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_status_message() {
        let m = SidecarMessage::from_line(
            r#"{"kind":"status","state":"reconnecting","attempt":2,"retryInMs":4000}"#,
        )
        .expect("parsea");
        assert_eq!(
            m,
            SidecarMessage::Status {
                state: ConnectionState::Reconnecting,
                detail: None,
                attempt: Some(2),
                retry_in_ms: Some(4000),
            }
        );
    }

    #[test]
    fn parses_ready_and_log() {
        assert_eq!(
            SidecarMessage::from_line(r#"{"kind":"ready"}"#).expect("ready"),
            SidecarMessage::Ready
        );
        let log = SidecarMessage::from_line(r#"{"kind":"log","level":"warn","message":"x"}"#)
            .expect("log");
        assert!(matches!(log, SidecarMessage::Log { level: LogLevel::Warn, .. }));
    }

    #[test]
    fn parses_viewers() {
        assert_eq!(
            SidecarMessage::from_line(r#"{"kind":"viewers","count":1234}"#).expect("viewers"),
            SidecarMessage::Viewers { count: 1234 }
        );
        assert!(SidecarMessage::from_line(r#"{"kind":"viewers","count":-1}"#).is_err());
    }

    #[test]
    fn unknown_kind_is_an_error_not_a_panic() {
        assert!(SidecarMessage::from_line(r#"{"kind":"nope"}"#).is_err());
        assert!(SidecarMessage::from_line("no es json").is_err());
    }

    #[test]
    fn parses_chat_results() {
        let ok = SidecarMessage::from_line(r#"{"kind":"chatResult","requestId":"r1","ok":true}"#).expect("ok");
        assert_eq!(ok, SidecarMessage::ChatResult { request_id: "r1".into(), ok: true, error: None });
        let err = SidecarMessage::from_line(r#"{"kind":"chatResult","requestId":"r2","ok":false,"error":"sin sesión"}"#).expect("err");
        assert_eq!(err, SidecarMessage::ChatResult { request_id: "r2".into(), ok: false, error: Some("sin sesión".into()) });
    }

    #[test]
    fn connect_carries_the_session_only_when_there_is_one_and_never_prints_it() {
        let session = SessionPayload { session_id: "SECRETO".into(), tt_target_idc: "useast1a".into() };
        let line = SidecarCommand::Connect { unique_id: "ana".into(), euler_api_key: None, session: Some(session.clone()) }
            .to_line()
            .expect("serializa");
        assert_eq!(line, "{\"cmd\":\"connect\",\"uniqueId\":\"ana\",\"session\":{\"sessionId\":\"SECRETO\",\"ttTargetIdc\":\"useast1a\"}}\n");
        assert!(!format!("{session:?}").contains("SECRETO"), "Debug no debe filtrar la cookie");
        let cmd = SidecarCommand::Connect { unique_id: "ana".into(), euler_api_key: Some("clave-euler".into()), session: Some(session) };
        let shown = format!("{cmd:?}");
        assert!(!shown.contains("SECRETO") && !shown.contains("clave-euler"), "Debug no debe filtrar secretos: {shown}");
        assert!(shown.contains("ana"));
    }

    #[test]
    fn send_chat_serializes_with_a_request_id() {
        let line = SidecarCommand::SendChat { request_id: "r1".into(), text: "hola \"mundo\"".into() }.to_line().expect("serializa");
        assert_eq!(line, "{\"cmd\":\"sendChat\",\"requestId\":\"r1\",\"text\":\"hola \\\"mundo\\\"\"}\n");
        assert_eq!(line.matches('\n').count(), 1, "una sola línea aunque el texto lleve comillas");
    }

    #[test]
    fn command_serializes_as_single_line() {
        let line = SidecarCommand::Connect {
            unique_id: "ana".into(),
            euler_api_key: None,
            session: None,
        }
        .to_line()
        .expect("serializa");
        assert_eq!(line, "{\"cmd\":\"connect\",\"uniqueId\":\"ana\"}\n");
        assert_eq!(
            SidecarCommand::Disconnect.to_line().expect("ok"),
            "{\"cmd\":\"disconnect\"}\n"
        );
    }
}
