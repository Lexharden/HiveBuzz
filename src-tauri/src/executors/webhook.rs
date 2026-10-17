//! `webhook`: petición HTTP a un servicio externo (IFTTT, Home Assistant, Streamer.bot…).
//!
//! Parámetros: `url` (con variables, codificadas para URL), `method` (GET por defecto; POST/PUT/
//! PATCH/DELETE), `headers` (objeto de textos), y el cuerpo como `body` (texto con variables) o
//! `bodyJson` (JSON cuyos textos admiten variables; es la opción segura para apodos con comillas).
//! Opcional: `retries` (0 a 3, por defecto 0) reintenta ante fallos de red, 429 y 5xx con espera
//! creciente; no se activa solo porque repetir un POST puede duplicar el efecto en el destino.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use reqwest::header::{HeaderName, HeaderValue};
use reqwest::{Client, Method, Url};
use serde::Serialize;
use serde_json::{Map, Value};

use super::{opt_number, render_json, render_url, require_str};
use crate::actions::{ActionContext, ActionExecutor};
use crate::error::{AppError, Result};

const TIMEOUT: Duration = Duration::from_secs(10);
const MAX_HEADERS: usize = 20;
const MAX_RETRIES: u32 = 3;
const RETRY_BASE: Duration = Duration::from_millis(500);
/// Cuánto de la respuesta se guarda para mostrarla en la prueba.
const MAX_BODY_PREVIEW: usize = 500;

/// Resultado de una petición, para el botón «Probar» de la interfaz.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookReport {
    pub status: u16,
    pub ok: bool,
    pub elapsed_ms: u64,
    /// Peticiones hechas (más de una si hubo reintentos).
    pub attempts: u32,
    /// Primeros caracteres de la respuesta.
    pub body: String,
}

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
        retries_of(params)?;
        Ok(())
    }

    async fn execute(&self, ctx: &ActionContext, params: &Map<String, Value>) -> Result<()> {
        let report = self.send(ctx, params, false).await?;
        if report.ok {
            Ok(())
        } else {
            Err(AppError::Invalid(format!("el webhook respondió {}", report.status)))
        }
    }
}

fn retries_of(params: &Map<String, Value>) -> Result<u32> {
    // `opt_number` ya garantiza un valor entre 0 y MAX_RETRIES.
    Ok(opt_number(params, "retries", 0.0, f64::from(MAX_RETRIES))?.map_or(0, |n| n as u32))
}

fn is_transient(status: reqwest::StatusCode) -> bool {
    status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS
}

impl WebhookExecutor {
    /// Hace la petición (con los reintentos pedidos) y devuelve lo que pasó. Un estado HTTP de error
    /// no es un `Err`: lo es solo no poder hablar con el servidor.
    pub async fn send(&self, ctx: &ActionContext, params: &Map<String, Value>, capture_body: bool) -> Result<WebhookReport> {
        let url = Url::parse(&render_url(ctx, require_str(params, "url")?)).map_err(|e| AppError::Invalid(format!("URL inválida: {e}")))?;
        check_url(&url)?;
        let method = parse_method(params)?;
        let headers = parse_headers(params, Some(ctx))?;
        // (texto, es JSON)
        let body: Option<(String, bool)> = if let Some(b) = params.get("bodyJson").filter(|v| !v.is_null()) {
            Some((serde_json::to_string(&render_json(ctx, b))?, true))
        } else {
            params.get("body").and_then(Value::as_str).filter(|b| !b.is_empty()).map(|b| (ctx.render(b), false))
        };
        let retries = retries_of(params)?;
        let started = Instant::now();
        let mut attempt = 0;
        loop {
            attempt += 1;
            let mut req = self.client.request(method.clone(), url.clone());
            for (k, v) in &headers {
                req = req.header(k.clone(), v.clone());
            }
            if let Some((text, json)) = &body {
                if *json {
                    req = req.header(reqwest::header::CONTENT_TYPE, "application/json");
                }
                req = req.body(text.clone());
            }
            let last = attempt > retries;
            match req.send().await {
                Ok(resp) if last || !is_transient(resp.status()) => {
                    let status = resp.status();
                    let body = if capture_body { preview(resp).await } else { String::new() };
                    return Ok(WebhookReport {
                        status: status.as_u16(),
                        ok: status.is_success(),
                        elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                        attempts: attempt,
                        body,
                    });
                }
                Err(e) if last => {
                    return Err(AppError::Invalid(format!("webhook a {}: {}", url.host_str().unwrap_or("?"), e.without_url())));
                }
                _ => {}
            }
            tokio::time::sleep(RETRY_BASE * 2u32.pow(attempt - 1)).await;
        }
    }
}

/// Lee como mucho `MAX_BODY_PREVIEW` caracteres de la respuesta (sin bajar cuerpos enormes).
async fn preview(mut resp: reqwest::Response) -> String {
    let mut bytes = Vec::new();
    while bytes.len() < MAX_BODY_PREVIEW * 4 {
        match resp.chunk().await {
            Ok(Some(c)) => bytes.extend_from_slice(&c),
            _ => break,
        }
    }
    String::from_utf8_lossy(&bytes).chars().take(MAX_BODY_PREVIEW).collect()
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

    /// Responde a cada conexión con el siguiente (estado, cuerpo) de la lista; devuelve cuántas atendió.
    async fn serve_seq(replies: Vec<(&'static str, &'static str)>) -> (u16, tokio::task::JoinHandle<usize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let h = tokio::spawn(async move {
            let mut served = 0;
            for (status, body) in replies {
                let (mut s, _) = listener.accept().await.unwrap();
                let mut chunk = [0u8; 4096];
                let _ = s.read(&mut chunk).await.unwrap();
                let head = format!("HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
                s.write_all(head.as_bytes()).await.unwrap();
                served += 1;
            }
            served
        });
        (port, h)
    }

    #[test]
    fn validates_retries_range() {
        let e = WebhookExecutor::new().unwrap();
        assert!(e.validate(&obj(json!({"url": "http://a.b", "retries": 3}))).is_ok());
        assert!(e.validate(&obj(json!({"url": "http://a.b", "retries": 4}))).is_err());
        assert!(e.validate(&obj(json!({"url": "http://a.b", "retries": -1}))).is_err());
        assert!(e.validate(&obj(json!({"url": "http://a.b", "retries": "2"}))).is_err());
    }

    #[tokio::test]
    async fn retries_server_errors_then_succeeds() {
        let (port, server) = serve_seq(vec![("503 Service Unavailable", ""), ("200 OK", "listo")]).await;
        let e = WebhookExecutor::new().unwrap();
        let params = obj(json!({"url": format!("http://127.0.0.1:{port}/"), "retries": 2}));
        let r = e.send(&ctx(&[]), &params, true).await.unwrap();
        assert_eq!((r.status, r.ok, r.attempts, r.body.as_str()), (200, true, 2, "listo"));
        assert_eq!(server.await.unwrap(), 2);
    }

    #[tokio::test]
    async fn gives_up_after_the_retries_and_reports_the_last_status() {
        let (port, server) = serve_seq(vec![("500 Internal Server Error", ""), ("500 Internal Server Error", "")]).await;
        let e = WebhookExecutor::new().unwrap();
        let params = obj(json!({"url": format!("http://127.0.0.1:{port}/"), "retries": 1}));
        let r = e.send(&ctx(&[]), &params, false).await.unwrap();
        assert_eq!((r.status, r.ok, r.attempts), (500, false, 2));
        assert_eq!(server.await.unwrap(), 2);
    }

    #[tokio::test]
    async fn client_errors_are_not_retried() {
        let (port, server) = serve_seq(vec![("404 Not Found", "nada")]).await;
        let e = WebhookExecutor::new().unwrap();
        let params = obj(json!({"url": format!("http://127.0.0.1:{port}/"), "retries": 3}));
        let r = e.send(&ctx(&[]), &params, true).await.unwrap();
        assert_eq!((r.status, r.attempts, r.body.as_str()), (404, 1, "nada"));
        assert_eq!(server.await.unwrap(), 1);
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
