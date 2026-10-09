//! `webhook`: petición HTTP a un servicio externo (IFTTT, Home Assistant, Streamer.bot…).
//!
//! Parámetros: `url` (con variables, codificadas para URL), `method` (GET por defecto; POST/PUT/
//! PATCH/DELETE), `headers` (objeto de textos), y el cuerpo como `body` (texto con variables) o
//! `bodyJson` (JSON cuyos textos admiten variables; es la opción segura para apodos con comillas).

use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::{HeaderName, HeaderValue};
use reqwest::{Client, Method, Url};
use serde_json::{Map, Value};

use super::{render_json, render_url, require_str};
use crate::actions::{ActionContext, ActionExecutor};
use crate::error::{AppError, Result};

const TIMEOUT: Duration = Duration::from_secs(10);
const MAX_HEADERS: usize = 20;

pub struct WebhookExecutor {
    client: Client,
}

impl WebhookExecutor {
    pub fn new() -> Result<Self> {
        let client = Client::builder()
            .timeout(TIMEOUT)
            .redirect(reqwest::redirect::Policy::limited(3))
            .user_agent(concat!("HiveBuzz/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| AppError::Invalid(format!("cliente HTTP: {e}")))?;
        Ok(Self { client })
    }
}

fn parse_method(params: &Map<String, Value>) -> Result<Method> {
    let m = params.get("method").and_then(Value::as_str).unwrap_or("GET").trim().to_ascii_uppercase();
    match m.as_str() {
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" => Method::from_bytes(m.as_bytes()).map_err(|e| AppError::Invalid(e.to_string())),
        other => Err(AppError::Invalid(format!("método HTTP no admitido: «{other}»"))),
    }
}

fn check_url(url: &Url) -> Result<()> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(AppError::Invalid("la URL debe empezar por http:// o https://".into()));
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err(AppError::Invalid("la URL no tiene host".into()));
    }
    Ok(())
}

fn parse_headers(params: &Map<String, Value>, ctx: Option<&ActionContext>) -> Result<Vec<(HeaderName, HeaderValue)>> {
    let Some(v) = params.get("headers").filter(|v| !v.is_null()) else { return Ok(Vec::new()) };
    let obj = v.as_object().ok_or_else(|| AppError::Invalid("«headers» debe ser un objeto".into()))?;
    if obj.len() > MAX_HEADERS {
        return Err(AppError::Invalid(format!("demasiadas cabeceras (máximo {MAX_HEADERS})")));
    }
    obj.iter()
        .map(|(k, v)| {
            let raw = v.as_str().ok_or_else(|| AppError::Invalid(format!("la cabecera «{k}» debe ser un texto")))?;
            let text = ctx.map_or_else(|| raw.to_string(), |c| c.render(raw));
            let name = HeaderName::from_bytes(k.trim().as_bytes()).map_err(|_| AppError::Invalid(format!("nombre de cabecera inválido: «{k}»")))?;
            let value = HeaderValue::from_str(&text).map_err(|_| AppError::Invalid(format!("valor inválido en la cabecera «{k}»")))?;
            Ok((name, value))
        })
        .collect()
}

#[async_trait]
impl ActionExecutor for WebhookExecutor {
    fn kind(&self) -> &'static str {
        "webhook"
    }

    fn validate(&self, params: &Map<String, Value>) -> Result<()> {
        let raw = require_str(params, "url")?;
        // Con variables aún sin resolver solo se puede comprobar el esquema.
        let lower = raw.to_ascii_lowercase();
        if !(lower.starts_with("http://") || lower.starts_with("https://")) {
            return Err(AppError::Invalid("la URL debe empezar por http:// o https://".into()));
        }
        parse_method(params)?;
        parse_headers(params, None)?;
        Ok(())
    }

    async fn execute(&self, ctx: &ActionContext, params: &Map<String, Value>) -> Result<()> {
        let url = Url::parse(&render_url(ctx, require_str(params, "url")?)).map_err(|e| AppError::Invalid(format!("URL inválida: {e}")))?;
        check_url(&url)?;
        let method = parse_method(params)?;
        let mut req = self.client.request(method, url.clone());
        for (k, v) in parse_headers(params, Some(ctx))? {
            req = req.header(k, v);
        }
        if let Some(body) = params.get("bodyJson").filter(|v| !v.is_null()) {
            req = req.header(reqwest::header::CONTENT_TYPE, "application/json").body(serde_json::to_string(&render_json(ctx, body))?);
        } else if let Some(body) = params.get("body").and_then(Value::as_str).filter(|b| !b.is_empty()) {
            req = req.body(ctx.render(body));
        }
        let resp = req.send().await.map_err(|e| AppError::Invalid(format!("webhook a {}: {}", url.host_str().unwrap_or("?"), e.without_url())))?;
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            Err(AppError::Invalid(format!("el webhook respondió {status}")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::template::Vars;
    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn ctx(pairs: &[(&str, &str)]) -> ActionContext {
        let vars: Vars = pairs.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect();
        ActionContext { rule_id: "r".into(), vars }
    }

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap_or_default()
    }

    /// Servidor HTTP mínimo: responde con `status` y devuelve la petición completa recibida.
    async fn serve_once(status: &'static str) -> (u16, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let h = tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let n = s.read(&mut chunk).await.unwrap();
                buf.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&buf).to_string();
                if let Some(idx) = text.find("\r\n\r\n") {
                    let len = text[..idx]
                        .lines()
                        .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                        .unwrap_or(0);
                    if buf.len() >= idx + 4 + len {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            s.write_all(format!("HTTP/1.1 {status}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").as_bytes()).await.unwrap();
            String::from_utf8_lossy(&buf).to_string()
        });
        (port, h)
    }

    #[test]
    fn validates_scheme_method_and_headers() {
        let e = WebhookExecutor::new().unwrap();
        assert!(e.validate(&obj(json!({"url": "https://example.com/{user}"}))).is_ok());
        assert!(e.validate(&obj(json!({"url": "ftp://example.com"}))).is_err());
        assert!(e.validate(&obj(json!({"url": "file:///etc/passwd"}))).is_err());
        assert!(e.validate(&obj(json!({}))).is_err());
        assert!(e.validate(&obj(json!({"url": "http://a.b", "method": "TRACE"}))).is_err());
        assert!(e.validate(&obj(json!({"url": "http://a.b", "headers": {"bad name": "x"}}))).is_err());
        assert!(e.validate(&obj(json!({"url": "http://a.b", "headers": {"X-Key": 5}}))).is_err());
    }

    #[test]
    fn url_variables_are_percent_encoded() {
        let c = ctx(&[("nickname", "a&b=c #1")]);
        assert_eq!(render_url(&c, "http://x/?n={nickname}"), "http://x/?n=a%26b%3Dc%20%231");
    }

    #[tokio::test]
    async fn posts_json_with_rendered_variables_and_headers() {
        let (port, server) = serve_once("200 OK").await;
        let e = WebhookExecutor::new().unwrap();
        let params = obj(json!({
            "url": format!("http://127.0.0.1:{port}/hook?u={{user}}"),
            "method": "post",
            "headers": {"X-Token": "t-{user}"},
            "bodyJson": {"nick": "{nickname}", "n": 3}
        }));
        e.execute(&ctx(&[("user", "ana"), ("nickname", "A \"quoted\" Ana")]), &params).await.unwrap();
        let req = server.await.unwrap();
        assert!(req.starts_with("POST /hook?u=ana HTTP/1.1"), "{req}");
        assert!(req.to_ascii_lowercase().contains("x-token: t-ana"));
        assert!(req.contains(r#"{"n":3,"nick":"A \"quoted\" Ana"}"#), "{req}");
    }

    #[tokio::test]
    async fn non_success_status_is_an_error() {
        let (port, server) = serve_once("500 Internal Server Error").await;
        let e = WebhookExecutor::new().unwrap();
        let err = e.execute(&ctx(&[]), &obj(json!({"url": format!("http://127.0.0.1:{port}/")}))).await.unwrap_err();
        let _ = server.await;
        assert!(err.to_string().contains("500"), "{err}");
    }

    #[tokio::test]
    async fn refuses_non_http_schemes_at_runtime() {
        let e = WebhookExecutor::new().unwrap();
        // Una variable puede cambiar el esquema; se vuelve a comprobar tras resolver.
        let c = ctx(&[("scheme", "file")]);
        let err = e.execute(&c, &obj(json!({"url": "{scheme}:///etc/passwd"}))).await.unwrap_err();
        assert!(err.to_string().contains("http"), "{err}");
    }
}
