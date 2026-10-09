//! Pruebas de integración del servidor contra un puerto real en loopback.

use futures_util::StreamExt;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use super::*;
use crate::events::testing::sample_event;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

async fn http_get(addr: SocketAddr, path: &str, host: &str) -> (u16, String) {
    let mut s = TcpStream::connect(addr).await.expect("connect");
    let req = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    s.write_all(req.as_bytes()).await.expect("write");
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).await.expect("read");
    let raw = String::from_utf8_lossy(&raw).into_owned();
    let status = raw
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .expect("status");
    (status, raw)
}

struct Rig {
    handle: ServerHandle,
    bus: EventBus,
    hub: OverlayHub,
    recent: RecentEvents,
    media: tempfile::TempDir,
}

impl Rig {
    fn deps(&self) -> ServerDeps {
        ServerDeps {
            bus: self.bus.clone(),
            hub: self.hub.clone(),
            recent: self.recent.clone(),
            media_dir: self.media.path().to_path_buf(),
            token: TOKEN.into(),
            api: None,
            oauth: None,
        }
    }
}

async fn started() -> Rig {
    let bus = EventBus::new(16);
    let hub = OverlayHub::new(16);
    let recent = RecentEvents::new(10);
    let media = tempfile::tempdir().expect("tmp");
    let deps = ServerDeps {
        bus: bus.clone(),
        hub: hub.clone(),
        recent: recent.clone(),
        media_dir: media.path().to_path_buf(),
        token: TOKEN.into(),
        api: None,
        oauth: None,
    };
    let handle = start(&deps, 0).await.expect("start");
    Rig { handle, bus, hub, recent, media }
}

#[tokio::test]
async fn only_binds_to_loopback() {
    let r = started().await;
    assert!(r.handle.addr.ip().is_loopback());
}

#[tokio::test]
async fn pages_require_the_token() {
    let r = started().await;
    let host = r.handle.addr.to_string();
    for path in ["/overlay/feed", "/overlay/alerts", "/overlay/goals", "/media/x.png", "/nada"] {
        assert_eq!(http_get(r.handle.addr, path, &host).await.0, 401, "{path} sin token");
        assert_eq!(http_get(r.handle.addr, &format!("{path}?token=mal"), &host).await.0, 401, "{path} token malo");
    }
}

#[tokio::test]
async fn every_overlay_page_is_served_with_security_headers() {
    let r = started().await;
    let host = r.handle.addr.to_string();
    let expected = [
        ("feed", "Feed de eventos"),
        ("alerts", "Alertas"),
        ("chat", "Chat"),
        ("gifts", "Regalos recientes"),
        ("leaderboard", "Top donadores"),
        ("goals", "Metas"),
        ("timer", "Timer"),
        ("counters", "Contadores"),
        ("wheel", "Ruleta"),
        ("poll", "Encuesta"),
        ("nowplaying", "Sonando ahora"),
    ];
    for (name, marker) in expected {
        let (status, raw) = http_get(r.handle.addr, &format!("/overlay/{name}?token={TOKEN}"), &host).await;
        assert_eq!(status, 200, "{name}");
        let lower = raw.to_ascii_lowercase();
        assert!(lower.contains("content-security-policy"), "{name}");
        assert!(lower.contains("x-content-type-options: nosniff"), "{name}");
        assert!(lower.contains("cache-control: no-store"), "{name}");
        assert!(raw.contains(marker), "{name} debe contener {marker}");
        assert!(raw.contains("window.HB = (function"), "{name}: falta el núcleo común");
    }
}

#[tokio::test]
async fn unknown_overlays_are_404_not_a_path_lookup() {
    let r = started().await;
    let host = r.handle.addr.to_string();
    for name in ["nope", "..%2f..%2fCargo.toml", "feed.html"] {
        let (status, _) = http_get(r.handle.addr, &format!("/overlay/{name}?token={TOKEN}"), &host).await;
        assert_eq!(status, 404, "{name}");
    }
}

#[tokio::test]
async fn foreign_host_is_rejected_even_with_a_valid_token() {
    let r = started().await;
    let path = format!("/overlay/feed?token={TOKEN}");
    assert_eq!(http_get(r.handle.addr, &path, "evil.example.com").await.0, 403);
}

#[tokio::test]
async fn media_files_are_served_with_the_token_and_cannot_escape_the_folder() {
    let r = started().await;
    let host = r.handle.addr.to_string();
    std::fs::write(r.media.path().join("a.png"), b"PNGDATA").expect("write");
    // Un archivo secreto fuera de la carpeta de medios.
    let secret = r.media.path().parent().expect("padre").join("secreto-hivebuzz-test.txt");
    std::fs::write(&secret, b"SECRETO").expect("write");

    let (status, raw) = http_get(r.handle.addr, &format!("/media/a.png?token={TOKEN}"), &host).await;
    assert_eq!(status, 200);
    assert!(raw.contains("PNGDATA"));

    for evil in [
        "/media/../secreto-hivebuzz-test.txt",
        "/media/%2e%2e/secreto-hivebuzz-test.txt",
        "/media/..%2fsecreto-hivebuzz-test.txt",
    ] {
        let (status, raw) = http_get(r.handle.addr, &format!("{evil}?token={TOKEN}"), &host).await;
        assert!(!raw.contains("SECRETO"), "{evil} filtró el archivo (status {status})");
    }
    let _ = std::fs::remove_file(secret);
    assert_eq!(http_get(r.handle.addr, &format!("/media/noexiste.png?token={TOKEN}"), &host).await.0, 404);
}

#[tokio::test]
async fn websocket_streams_bus_events() {
    let r = started().await;
    let url = format!("ws://{}/ws?token={TOKEN}", r.handle.addr);
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.expect("ws");

    let hello = ws.next().await.expect("msg").expect("ok");
    assert!(matches!(&hello, WsMessage::Text(t) if t.contains("\"hello\"")));

    r.bus.publish(sample_event("ws-1"));
    let msg = ws.next().await.expect("msg").expect("ok");
    let WsMessage::Text(text) = msg else {
        panic!("se esperaba texto");
    };
    let v: serde_json::Value = serde_json::from_str(&text).expect("json");
    assert_eq!(v["type"], "event");
    assert_eq!(v["event"]["id"], "ws-1");
    assert_eq!(v["event"]["type"], "chat");
}

#[tokio::test]
async fn websocket_forwards_overlay_messages_by_channel() {
    let r = started().await;
    let url = format!("ws://{}/ws?token={TOKEN}", r.handle.addr);
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.expect("ws");
    let _hello = ws.next().await;

    r.hub.publish("alerts", json!({"text": "hola"}));
    let WsMessage::Text(text) = ws.next().await.expect("msg").expect("ok") else {
        panic!("se esperaba texto");
    };
    let v: serde_json::Value = serde_json::from_str(&text).expect("json");
    assert_eq!(v["type"], "overlay");
    assert_eq!(v["channel"], "alerts");
    assert_eq!(v["data"]["text"], "hola");
}

async fn next_json(ws: &mut (impl StreamExt<Item = std::result::Result<WsMessage, tokio_tungstenite::tungstenite::Error>> + Unpin)) -> serde_json::Value {
    let WsMessage::Text(text) = ws.next().await.expect("msg").expect("ok") else {
        panic!("se esperaba texto");
    };
    serde_json::from_str(&text).expect("json")
}

#[tokio::test]
async fn new_connections_get_retained_state_then_history_before_live_traffic() {
    let r = started().await;
    r.hub.publish_retained("config:chat", json!({"fontSize": 22}));
    r.hub.publish_retained("goals", json!({"goals": []}));
    r.hub.publish("alerts", json!({"text": "efímero"})); // no es retenido: no se repite
    r.recent.push(Arc::new(sample_event("old-1")));
    r.recent.push(Arc::new(sample_event("old-2")));

    let url = format!("ws://{}/ws?token={TOKEN}", r.handle.addr);
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.expect("ws");

    assert_eq!(next_json(&mut ws).await["type"], "hello");
    let a = next_json(&mut ws).await;
    let b = next_json(&mut ws).await;
    assert_eq!((a["type"].clone(), a["channel"].clone(), a["data"]["fontSize"].clone()), (json!("overlay"), json!("config:chat"), json!(22)));
    assert_eq!((b["type"].clone(), b["channel"].clone()), (json!("overlay"), json!("goals")));
    let history = next_json(&mut ws).await;
    assert_eq!(history["type"], "history");
    let ids: Vec<_> = history["events"].as_array().expect("arr").iter().map(|e| e["id"].as_str().expect("id").to_string()).collect();
    assert_eq!(ids, ["old-1", "old-2"]);

    // Y después, el tráfico en vivo.
    r.bus.publish(sample_event("live-1"));
    let live = next_json(&mut ws).await;
    assert_eq!((live["type"].clone(), live["event"]["id"].clone()), (json!("event"), json!("live-1")));
}

#[tokio::test]
async fn a_connection_without_history_or_state_goes_straight_to_live_traffic() {
    let r = started().await;
    let url = format!("ws://{}/ws?token={TOKEN}", r.handle.addr);
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.expect("ws");
    assert_eq!(next_json(&mut ws).await["type"], "hello");
    r.hub.publish_retained("timer", json!({"timers": []}));
    assert_eq!(next_json(&mut ws).await["channel"], "timer");
}

#[tokio::test]
async fn websocket_rejects_bad_token_and_foreign_origin() {
    let r = started().await;
    let bad = format!("ws://{}/ws?token=mal", r.handle.addr);
    assert!(tokio_tungstenite::connect_async(bad).await.is_err());

    let mut req = format!("ws://{}/ws?token={TOKEN}", r.handle.addr)
        .into_client_request()
        .expect("request");
    req.headers_mut()
        .insert("origin", "https://evil.example.com".parse().expect("header"));
    assert!(tokio_tungstenite::connect_async(req).await.is_err());
}

#[tokio::test]
async fn busy_port_is_an_error_not_a_panic() {
    let r = started().await;
    let err = start(&r.deps(), r.handle.addr.port()).await.err().expect("debe fallar");
    assert!(err.to_string().contains("no se pudo abrir"));
}

// ---- API local ----

use std::sync::Mutex as StdMutex;

use crate::rules::template::Vars;

struct FakeApi {
    calls: StdMutex<Vec<(String, Vars)>>,
    queued: usize,
}

#[async_trait::async_trait]
impl api::ApiBackend for FakeApi {
    fn names(&self) -> Vec<String> {
        vec!["gracias".into()]
    }

    async fn trigger(&self, name: &str, vars: Vars) -> usize {
        self.calls.lock().expect("lock").push((name.to_string(), vars));
        if name == "gracias" {
            self.queued
        } else {
            0
        }
    }
}

async fn api_rig(queued: usize) -> (ServerHandle, Arc<FakeApi>, tempfile::TempDir) {
    let media = tempfile::tempdir().expect("tmp");
    let fake = Arc::new(FakeApi { calls: StdMutex::new(Vec::new()), queued });
    let deps = ServerDeps {
        bus: EventBus::new(16),
        hub: OverlayHub::new(16),
        recent: RecentEvents::new(10),
        media_dir: media.path().to_path_buf(),
        token: TOKEN.into(),
        api: Some(Arc::clone(&fake) as api::SharedApi),
        oauth: None,
    };
    (start(&deps, 0).await.expect("start"), fake, media)
}

async fn http_post(addr: SocketAddr, path: &str, extra_headers: &str, body: &str) -> (u16, String) {
    let mut s = TcpStream::connect(addr).await.expect("connect");
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra_headers}Connection: close\r\n\r\n{body}",
        body.len()
    );
    s.write_all(req.as_bytes()).await.expect("write");
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).await.expect("read");
    let raw = String::from_utf8_lossy(&raw).into_owned();
    let status = raw.split_whitespace().nth(1).and_then(|c| c.parse().ok()).expect("status");
    (status, raw)
}

#[tokio::test]
async fn api_requires_the_token_and_accepts_bearer() {
    let (h, fake, _m) = api_rig(1).await;
    let body = r#"{"name":"gracias"}"#;
    assert_eq!(http_post(h.addr, "/api/trigger", "", body).await.0, 401);
    assert_eq!(http_post(h.addr, "/api/trigger?token=mal", "", body).await.0, 401);
    assert_eq!(http_post(h.addr, "/api/trigger", "Authorization: Bearer mal\r\n", body).await.0, 401);
    assert!(fake.calls.lock().expect("lock").is_empty(), "sin token no se dispara nada");
    assert_eq!(http_post(h.addr, &format!("/api/trigger?token={TOKEN}"), "", body).await.0, 202);
    assert_eq!(http_post(h.addr, "/api/trigger", &format!("Authorization: Bearer {TOKEN}\r\n"), body).await.0, 202);
    assert_eq!(fake.calls.lock().expect("lock").len(), 2);
}

#[tokio::test]
async fn api_passes_clean_variables_and_reports_unknown_names() {
    let (h, fake, _m) = api_rig(2).await;
    let url = format!("/api/trigger?token={TOKEN}");
    let (s, raw) = http_post(h.addr, &url, "", r#"{"name":"gracias","vars":{"User":"ana","n":3,"ok":true}}"#).await;
    assert_eq!(s, 202);
    assert!(raw.contains(r#""queued":2"#), "{raw}");
    let calls = fake.calls.lock().expect("lock").clone();
    assert_eq!(calls[0].1.get("user").map(String::as_str), Some("ana"), "las claves van en minúsculas");
    assert_eq!(calls[0].1.get("n").map(String::as_str), Some("3"));
    assert_eq!(calls[0].1.get("ok").map(String::as_str), Some("true"));
    assert_eq!(http_post(h.addr, &url, "", r#"{"name":"nada"}"#).await.0, 404);
}

#[tokio::test]
async fn api_rejects_bad_input() {
    let (h, fake, _m) = api_rig(1).await;
    let url = format!("/api/trigger?token={TOKEN}");
    for body in [
        r#"{"name":"../x"}"#,
        r#"{"name":""}"#,
        r#"{"name":"gracias","vars":{"a b":"x"}}"#,
        r#"{"name":"gracias","vars":{"a":{"x":1}}}"#,
        r#"{"name":"gracias","vars":{"a":[1]}}"#,
    ] {
        assert_eq!(http_post(h.addr, &url, "", body).await.0, 422, "{body}");
    }
    assert_eq!(http_post(h.addr, &url, "", "no es json").await.0, 400);
    assert_eq!(http_post(h.addr, &url, "", r#"{"name":"gracias","extra":1}"#).await.0, 422, "campos desconocidos");
    let big = format!(r#"{{"name":"gracias","vars":{{"a":"{}"}}}}"#, "x".repeat(600));
    assert_eq!(http_post(h.addr, &url, "", &big).await.0, 422);
    let huge = format!(r#"{{"name":"gracias","vars":{{"a":"{}"}}}}"#, "x".repeat(20_000));
    assert_eq!(http_post(h.addr, &url, "", &huge).await.0, 413, "cuerpo demasiado grande");
    assert!(fake.calls.lock().expect("lock").is_empty());
}

#[tokio::test]
async fn api_full_queue_is_429_and_list_and_status_work() {
    let (h, _fake, _m) = api_rig(0).await;
    let url = format!("/api/trigger?token={TOKEN}");
    assert_eq!(http_post(h.addr, &url, "", r#"{"name":"gracias"}"#).await.0, 429);
    let host = h.addr.to_string();
    let (s, raw) = http_get(h.addr, &format!("/api/triggers?token={TOKEN}"), &host).await;
    assert_eq!(s, 200);
    assert!(raw.contains(r#""triggers":["gracias"]"#), "{raw}");
    assert_eq!(http_get(h.addr, &format!("/api/status?token={TOKEN}"), &host).await.0, 200);
    assert_eq!(http_get(h.addr, "/api/status", &host).await.0, 401);
}

#[tokio::test]
async fn api_refuses_foreign_hosts_and_origins() {
    let (h, fake, _m) = api_rig(1).await;
    let mut s = TcpStream::connect(h.addr).await.expect("connect");
    let body = r#"{"name":"gracias"}"#;
    let req = format!(
        "POST /api/trigger?token={TOKEN} HTTP/1.1\r\nHost: {}\r\nOrigin: https://evil.example\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        h.addr,
        body.len()
    );
    s.write_all(req.as_bytes()).await.expect("write");
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).await.expect("read");
    assert!(String::from_utf8_lossy(&raw).contains(" 403 "));
    assert!(fake.calls.lock().expect("lock").is_empty());
}

// ---- Vuelta de OAuth (Spotify) ----

struct FakeOAuth(StdMutex<Vec<(String, String)>>);

#[async_trait::async_trait]
impl oauth::OAuthCallback for FakeOAuth {
    async fn spotify(&self, code: &str, state: &str) -> crate::error::Result<()> {
        self.0.lock().expect("lock").push((code.into(), state.into()));
        if state == "bueno" {
            Ok(())
        } else {
            Err(crate::error::AppError::Invalid("estado incorrecto".into()))
        }
    }
}

async fn oauth_rig() -> (ServerHandle, Arc<FakeOAuth>, tempfile::TempDir) {
    let media = tempfile::tempdir().expect("tmp");
    let fake = Arc::new(FakeOAuth(StdMutex::new(Vec::new())));
    let deps = ServerDeps {
        bus: EventBus::new(16),
        hub: OverlayHub::new(16),
        recent: RecentEvents::new(10),
        media_dir: media.path().to_path_buf(),
        token: TOKEN.into(),
        api: None,
        oauth: Some(Arc::clone(&fake) as oauth::SharedOAuth),
    };
    (start(&deps, 0).await.expect("start"), fake, media)
}

#[tokio::test]
async fn the_oauth_callback_works_without_the_overlay_token_but_only_with_a_valid_state() {
    let (h, fake, _m) = oauth_rig().await;
    let host = h.addr.to_string();
    let (s, raw) = http_get(h.addr, "/spotify/callback?code=abc&state=bueno", &host).await;
    assert_eq!(s, 200);
    assert!(raw.contains("Spotify conectado"));
    assert!(raw.to_lowercase().contains("content-security-policy: default-src 'none'"));
    let (s, _) = http_get(h.addr, "/spotify/callback?code=abc&state=malo", &host).await;
    assert_eq!(s, 400);
    assert_eq!(http_get(h.addr, "/spotify/callback?error=access_denied", &host).await.0, 400);
    assert_eq!(http_get(h.addr, "/spotify/callback", &host).await.0, 400);
    assert_eq!(fake.0.lock().expect("lock").len(), 2, "solo las peticiones completas llegan al servicio");
}

#[tokio::test]
async fn the_oauth_callback_never_reflects_its_input_and_keeps_the_host_check() {
    let (h, _fake, _m) = oauth_rig().await;
    let host = h.addr.to_string();
    let (_, raw) = http_get(h.addr, "/spotify/callback?code=%3Cscript%3Ealert(1)%3C%2Fscript%3E&state=%3Cb%3Ex", &host).await;
    assert!(!raw.contains("<script>alert"), "nada de la URL aparece en la página");
    assert_eq!(http_get(h.addr, "/spotify/callback?code=a&state=bueno", "evil.example").await.0, 403);
    // El resto del servidor sigue pidiendo token.
    assert_eq!(http_get(h.addr, "/overlay/feed", &host).await.0, 401);
}

#[tokio::test]
async fn without_an_oauth_handler_the_callback_is_404() {
    let r = started().await;
    let host = r.handle.addr.to_string();
    assert_eq!(http_get(r.handle.addr, "/spotify/callback?code=a&state=b", &host).await.0, 404);
}
