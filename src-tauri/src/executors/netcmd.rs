//! Comandos genéricos por red para juegos y mods de terceros:
//! - `tcpSend`: `host`, `port`, `message` (plantilla) o `messageJson`, `newline` (por defecto sí).
//! - `wsSend`: `url` (`ws://` o `wss://`) y `message` o `messageJson`.
//!
//! Sin respuesta que esperar: se conecta, envía, cierra. El mensaje admite variables; si el destino
//! espera JSON, usa `messageJson` (los textos se insertan ya escapados).

use std::time::Duration;

use async_trait::async_trait;
use futures_util::SinkExt;
use serde_json::{Map, Value};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;

use super::{opt_bool, render_json, require_str};
use crate::actions::{ActionContext, ActionExecutor};
use crate::error::{AppError, Result};

const NET_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_MESSAGE_BYTES: usize = 8 * 1024;

/// Cuerpo a enviar: `messageJson` (serializado compacto) o `message` (texto con variables).
fn build_message(ctx: &ActionContext, params: &Map<String, Value>) -> Result<String> {
    let text = match params.get("messageJson").filter(|v| !v.is_null()) {
        Some(v) => serde_json::to_string(&render_json(ctx, v))?,
        None => ctx.render(require_str(params, "message")?),
    };
    if text.len() > MAX_MESSAGE_BYTES {
        return Err(AppError::Invalid(format!("el mensaje supera {MAX_MESSAGE_BYTES} bytes")));
    }
    Ok(text)
}

fn has_message(params: &Map<String, Value>) -> Result<()> {
    if params.get("messageJson").is_some_and(|v| !v.is_null()) {
        return Ok(());
    }
    require_str(params, "message").map(|_| ())
}

fn port_of(params: &Map<String, Value>) -> Result<u16> {
    params
        .get("port")
        .and_then(Value::as_u64)
        .and_then(|p| u16::try_from(p).ok())
        .filter(|p| *p != 0)
        .ok_or_else(|| AppError::Invalid("«port» debe ser un número entre 1 y 65535".into()))
}

pub struct TcpSendExecutor;

#[async_trait]
impl ActionExecutor for TcpSendExecutor {
    fn kind(&self) -> &'static str {
        "tcpSend"
    }

    fn validate(&self, params: &Map<String, Value>) -> Result<()> {
        require_str(params, "host")?;
        port_of(params)?;
        opt_bool(params, "newline", true)?;
        has_message(params)
    }

    async fn execute(&self, ctx: &ActionContext, params: &Map<String, Value>) -> Result<()> {
        let host = require_str(params, "host")?;
        let port = port_of(params)?;
        let mut payload = build_message(ctx, params)?.into_bytes();
        if opt_bool(params, "newline", true)? {
            payload.push(b'\n');
        }
        let io = async {
            let mut s = TcpStream::connect((host, port)).await?;
            s.write_all(&payload).await?;
            s.flush().await?;
            s.shutdown().await
        };
        timeout(NET_TIMEOUT, io)
            .await
            .map_err(|_| AppError::Invalid(format!("tiempo agotado conectando con {host}:{port}")))?
            .map_err(|e| AppError::Invalid(format!("TCP {host}:{port}: {e}")))
    }
}

pub struct WsSendExecutor;

#[async_trait]
impl ActionExecutor for WsSendExecutor {
    fn kind(&self) -> &'static str {
        "wsSend"
    }

    fn validate(&self, params: &Map<String, Value>) -> Result<()> {
        let url = require_str(params, "url")?.to_ascii_lowercase();
        if !(url.starts_with("ws://") || url.starts_with("wss://")) {
            return Err(AppError::Invalid("la URL debe empezar por ws:// o wss://".into()));
        }
        has_message(params)
    }

    async fn execute(&self, ctx: &ActionContext, params: &Map<String, Value>) -> Result<()> {
        let url = require_str(params, "url")?;
        if !(url.to_ascii_lowercase().starts_with("ws://") || url.to_ascii_lowercase().starts_with("wss://")) {
            return Err(AppError::Invalid("la URL debe empezar por ws:// o wss://".into()));
        }
        let text = build_message(ctx, params)?;
        let io = async {
            let (mut ws, _) = tokio_tungstenite::connect_async(url).await.map_err(|e| e.to_string())?;
            ws.send(Message::text(text)).await.map_err(|e| e.to_string())?;
            let _ = ws.close(None).await;
            Ok::<(), String>(())
        };
        timeout(NET_TIMEOUT, io)
            .await
            .map_err(|_| AppError::Invalid("tiempo agotado conectando por WebSocket".into()))?
            .map_err(|e| AppError::Invalid(format!("WebSocket: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::template::Vars;
    use futures_util::StreamExt;
    use serde_json::json;
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpListener;

    fn ctx(pairs: &[(&str, &str)]) -> ActionContext {
        let vars: Vars = pairs.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect();
        ActionContext { rule_id: "r".into(), vars }
    }

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap_or_default()
    }

    #[test]
    fn validates_required_fields() {
        assert!(TcpSendExecutor.validate(&obj(json!({"host": "127.0.0.1", "port": 9, "message": "x"}))).is_ok());
        assert!(TcpSendExecutor.validate(&obj(json!({"host": "127.0.0.1", "port": 0, "message": "x"}))).is_err());
        assert!(TcpSendExecutor.validate(&obj(json!({"host": "127.0.0.1", "port": 70000, "message": "x"}))).is_err());
        assert!(TcpSendExecutor.validate(&obj(json!({"host": "", "port": 9, "message": "x"}))).is_err());
        assert!(TcpSendExecutor.validate(&obj(json!({"host": "h", "port": 9}))).is_err());
        assert!(WsSendExecutor.validate(&obj(json!({"url": "ws://h:1", "message": "x"}))).is_ok());
        assert!(WsSendExecutor.validate(&obj(json!({"url": "http://h:1", "message": "x"}))).is_err());
    }

    #[tokio::test]
    async fn tcp_sends_rendered_message_with_newline() {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        let srv = tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let mut got = String::new();
            s.read_to_string(&mut got).await.unwrap();
            got
        });
        let p = obj(json!({"host": "127.0.0.1", "port": port, "message": "spawn {nickname} x{count}"}));
        TcpSendExecutor.execute(&ctx(&[("nickname", "Ana"), ("count", "3")]), &p).await.unwrap();
        assert_eq!(srv.await.unwrap(), "spawn Ana x3\n");
    }

    #[tokio::test]
    async fn tcp_json_message_escapes_variables() {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        let srv = tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let mut got = String::new();
            s.read_to_string(&mut got).await.unwrap();
            got
        });
        let p = obj(json!({"host": "127.0.0.1", "port": port, "newline": false, "messageJson": {"who": "{nickname}"}}));
        TcpSendExecutor.execute(&ctx(&[("nickname", "a\"b")]), &p).await.unwrap();
        let got = srv.await.unwrap();
        assert_eq!(serde_json::from_str::<Value>(&got).unwrap(), json!({"who": "a\"b"}));
    }

    #[tokio::test]
    async fn tcp_connection_refused_is_an_error() {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        let p = obj(json!({"host": "127.0.0.1", "port": port, "message": "x"}));
        assert!(TcpSendExecutor.execute(&ctx(&[]), &p).await.is_err());
    }

    #[tokio::test]
    async fn ws_sends_a_text_frame() {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        let srv = tokio::spawn(async move {
            let (s, _) = l.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(s).await.unwrap();
            ws.next().await.unwrap().unwrap().into_text().unwrap().to_string()
        });
        let p = obj(json!({"url": format!("ws://127.0.0.1:{port}"), "message": "hola {user}"}));
        WsSendExecutor.execute(&ctx(&[("user", "ana")]), &p).await.unwrap();
        assert_eq!(srv.await.unwrap(), "hola ana");
    }

    #[tokio::test]
    async fn oversized_messages_are_rejected() {
        let p = obj(json!({"host": "127.0.0.1", "port": 9, "message": "x".repeat(MAX_MESSAGE_BYTES + 1)}));
        let err = TcpSendExecutor.execute(&ctx(&[]), &p).await.unwrap_err();
        assert!(err.to_string().contains("bytes"));
    }
}
