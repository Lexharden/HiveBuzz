//! Utilidades compartidas por los tests: un servidor HTTP de mentira en loopback.

use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Debug, Clone)]
pub struct Req {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Req {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

pub type Handler = Box<dyn Fn(&Req) -> (u16, String) + Send + Sync>;

/// Devuelve la URL base y las peticiones recibidas. Cada respuesta cierra la conexión.
pub async fn fake_http(handler: Handler) -> (String, Arc<Mutex<Vec<Req>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (log, handler) = (Arc::clone(&seen), Arc::new(handler));
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = listener.accept().await else { return };
            let (log, handler) = (Arc::clone(&log), Arc::clone(&handler));
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                let (head_end, len) = loop {
                    let n = s.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buf).to_string();
                    if let Some(i) = text.find("\r\n\r\n") {
                        let len = text[..i]
                            .lines()
                            .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                            .unwrap_or(0);
                        if buf.len() >= i + 4 + len {
                            break (i, len);
                        }
                    }
                };
                let text = String::from_utf8_lossy(&buf).to_string();
                let mut lines = text[..head_end].lines();
                let first = lines.next().unwrap_or("");
                let mut parts = first.split_whitespace();
                let req = Req {
                    method: parts.next().unwrap_or("").into(),
                    path: parts.next().unwrap_or("").into(),
                    headers: lines.filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))).collect(),
                    body: text[head_end + 4..head_end + 4 + len].to_string(),
                };
                let (status, body) = handler(&req);
                log.lock().unwrap().push(req);
                let out = format!("HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\ncontent-type: application/json\r\n\r\n{body}", body.len());
                let _ = s.write_all(out.as_bytes()).await;
            });
        }
    });
    (base, seen)
}

/// Reloj manipulable para probar caducidades.
#[derive(Default)]
pub struct TestClock(std::sync::atomic::AtomicUsize);

impl TestClock {
    pub fn advance(&self, ms: usize) {
        self.0.fetch_add(ms, std::sync::atomic::Ordering::SeqCst);
    }
}

impl crate::actions::clock::Clock for TestClock {
    fn now_ms(&self) -> i64 {
        i64::try_from(1_000_000 + self.0.load(std::sync::atomic::Ordering::SeqCst)).unwrap()
    }
}
