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
}

/// Rust → sidecar (stdin). `Debug` es manual: oculta la API key de Euler.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "camelCase")]
pub enum SidecarCommand {
    #[serde(rename_all = "camelCase")]
    Connect {
        unique_id: String,
        /// API key propia de Euler Stream (opcional; viene del keyring).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        euler_api_key: Option<String>,
    },
    Disconnect,
    Shutdown,
}

impl std::fmt::Debug for SidecarCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connect { unique_id, euler_api_key } => f
                .debug_struct("Connect")
                .field("unique_id", unique_id)
                .field("euler_api_key", &euler_api_key.as_ref().map(|_| "***"))
                .finish(),
            Self::Disconnect => f.write_str("Disconnect"),
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
    fn connect_never_prints_the_euler_key() {
        let cmd = SidecarCommand::Connect { unique_id: "ana".into(), euler_api_key: Some("clave-euler".into()) };
        let shown = format!("{cmd:?}");
        assert!(!shown.contains("clave-euler"), "Debug no debe filtrar secretos: {shown}");
        assert!(shown.contains("ana"));
    }

    #[test]
    fn command_serializes_as_single_line() {
        let line = SidecarCommand::Connect {
            unique_id: "ana".into(),
            euler_api_key: None,
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
