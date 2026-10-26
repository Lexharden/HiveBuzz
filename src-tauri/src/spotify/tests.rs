use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use async_trait::async_trait;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::api::{parse_track, track_id_from_ref, HttpSpotify, PlayerCmd, Playing, SpotifyApi, Track};
use super::auth::SpotifyAuth;
use super::service::SongService;
use super::*;
use crate::actions::clock::{AppClock, Clock};
use crate::db::Db;
use crate::error::Result;
use crate::events::testing::sample_event;
use crate::events::{Chat, EventType, LiveEvent, Platform};
use crate::overlay::OverlayHub;
use crate::rules::engine::PointsGate;
use crate::secrets::{MemoryStore, SecretStore};

// ------------------------------------------------------------ servidor HTTP de mentira

#[derive(Debug, Clone)]
struct Req {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl Req {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

type Handler = Box<dyn Fn(&Req) -> (u16, Vec<(&'static str, String)>, String) + Send + Sync>;

/// Devuelve la URL base y las peticiones recibidas.
async fn fake_server(handler: Handler) -> (String, Arc<StdMutex<Vec<Req>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let seen = Arc::new(StdMutex::new(Vec::new()));
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
                let (status, headers, body) = handler(&req);
                log.lock().unwrap().push(req);
                let mut out = format!("HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\ncontent-type: application/json\r\n", body.len());
                for (k, v) in headers {
                    out.push_str(&format!("{k}: {v}\r\n"));
                }
                out.push_str("\r\n");
                out.push_str(&body);
                let _ = s.write_all(out.as_bytes()).await;
            });
        }
    });
    (base, seen)
}

fn track_json(id: &str, name: &str, artist: &str, ms: u64) -> serde_json::Value {
    json!({
        "id": id, "uri": format!("spotify:track:{id}"), "name": name, "duration_ms": ms,
        "artists": [{ "name": artist }],
        "album": { "images": [
            { "url": "https://i.scdn.co/image/big", "width": 640 },
            { "url": "https://i.scdn.co/image/mid", "width": 300 },
            { "url": "https://i.scdn.co/image/small", "width": 64 }
        ]}
    })
}

const ID1: &str = "4uLU6hMCjMI75M1A2tKUQC";
const ID2: &str = "7ouMYWpwJ422jRcDASZB7P";

// ------------------------------------------------------------ configuración y utilidades

#[test]
fn config_is_sanitized_and_validated() {
    let mut c = SpotifyConfig { client_id: "  abc123  ".into(), ..SpotifyConfig::default() };
    c.song.commands = vec!["!SR".into(), " sr ".into(), "".into(), "Pedir".into()];
    c.song.blocked_users = vec!["@Troll".into(), "troll".into()];
    c.song.per_user_limit = 0;
    c.song.max_duration_s = 5;
    c.song.cost_points = u64::MAX;
    let c = c.sanitized();
    assert_eq!(c.client_id, "abc123");
    assert_eq!(c.song.commands, ["sr", "pedir"]);
    assert_eq!(c.song.blocked_users, ["troll"]);
    assert_eq!((c.song.per_user_limit, c.song.max_duration_s, c.song.cost_points), (1, 30, 1_000_000_000));
    assert!(c.validate().is_ok());

    let mut bad = c.clone();
    bad.client_id = "no válido!".into();
    assert!(bad.validate().is_err());
    let mut none = c.clone();
    none.song.enabled = true;
    none.song.commands.clear();
    assert!(none.validate().is_err());
}

#[test]
fn track_references_are_recognized() {
    for ok in [
        format!("https://open.spotify.com/track/{ID1}?si=abc"),
        format!("https://open.spotify.com/intl-es/track/{ID1}"),
        format!("spotify:track:{ID1}"),
        format!("  https://open.spotify.com/track/{ID1}#x "),
    ] {
        assert_eq!(track_id_from_ref(&ok).as_deref(), Some(ID1), "{ok}");
    }
    for no in ["la bamba", "https://open.spotify.com/album/4uLU6hMCjMI75M1A2tKUQC", "https://evil.example/track/4uLU6hMCjMI75M1A2tKUQC", "spotify:track:corto", ""] {
        assert_eq!(track_id_from_ref(no), None, "{no}");
    }
}

#[test]
fn parse_track_picks_a_reasonable_cover_and_requires_https() {
    let t = parse_track(&track_json(ID1, "Canción", "Artista", 1000)).unwrap();
    assert_eq!(t.image.as_deref(), Some("https://i.scdn.co/image/mid"));
    assert_eq!(t.uri, format!("spotify:track:{ID1}"));
    let mut http = track_json(ID1, "x", "y", 1);
    http["album"]["images"] = json!([{ "url": "http://insegura/x.jpg", "width": 300 }]);
    assert_eq!(parse_track(&http).unwrap().image, None);
    assert!(parse_track(&json!({ "name": "sin id" })).is_none());
}

// ------------------------------------------------------------ OAuth

fn auth_with(token_url: &str) -> (Arc<SpotifyAuth>, Arc<MemoryStore>, Arc<AppClockAdjustable>) {
    let store = Arc::new(MemoryStore::default());
    let clock = Arc::new(AppClockAdjustable::default());
    let auth = SpotifyAuth::with_endpoints(store.clone(), clock.clone(), "https://accounts.example/authorize", token_url).unwrap();
    auth.set_client_id("cid123");
    (auth, store, clock)
}

/// Reloj manipulable para probar caducidades.
#[derive(Default)]
struct AppClockAdjustable(AtomicUsize);

impl AppClockAdjustable {
    fn advance(&self, ms: usize) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for AppClockAdjustable {
    fn now_ms(&self) -> i64 {
        i64::try_from(1_000_000 + self.0.load(Ordering::SeqCst)).unwrap()
    }
}

fn form_value(body: &str, key: &str) -> Option<String> {
    body.split('&').find_map(|p| p.split_once('=').filter(|(k, _)| *k == key).map(|(_, v)| v.to_string()))
}

#[tokio::test]
async fn begin_builds_a_pkce_url_and_complete_exchanges_the_code() {
    let (base, seen) = fake_server(Box::new(|r| {
        assert_eq!(r.method, "POST");
        (200, vec![], json!({ "access_token": "AT1", "refresh_token": "RT1", "expires_in": 3600 }).to_string())
    }))
    .await;
    let (auth, store, _) = auth_with(&format!("{base}/api/token"));
    let url = auth.begin("http://127.0.0.1:17890/spotify/callback").unwrap();
    assert!(url.starts_with("https://accounts.example/authorize?"));
    let query = url.split_once('?').unwrap().1;
    assert_eq!(form_value(query, "client_id").as_deref(), Some("cid123"));
    assert_eq!(form_value(query, "response_type").as_deref(), Some("code"));
    assert_eq!(form_value(query, "code_challenge_method").as_deref(), Some("S256"));
    assert_eq!(form_value(query, "redirect_uri").as_deref(), Some("http%3A%2F%2F127.0.0.1%3A17890%2Fspotify%2Fcallback"));
    let state = form_value(query, "state").unwrap();
    let challenge = form_value(query, "code_challenge").unwrap();

    auth.complete("CODE", &state).await.unwrap();
    assert!(auth.is_connected());
    assert_eq!(store.get(KEY_SPOTIFY_REFRESH).unwrap().as_deref(), Some("RT1"));
    let req = seen.lock().unwrap()[0].clone();
    assert_eq!(form_value(&req.body, "grant_type").as_deref(), Some("authorization_code"));
    assert_eq!(form_value(&req.body, "code").as_deref(), Some("CODE"));
    assert_eq!(form_value(&req.body, "client_id").as_deref(), Some("cid123"));
    // El verificador enviado corresponde al desafío de la URL.
    let verifier = form_value(&req.body, "code_verifier").unwrap();
    assert_eq!(pkce::challenge(&verifier), challenge);
    assert!(verifier.len() >= 43, "RFC 7636: mínimo 43 caracteres");
    // Sin secreto de cliente.
    assert!(!req.body.contains("client_secret"));
    assert_eq!(auth.access_token().await.unwrap(), "AT1", "el primer access token queda en caché");
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_wrong_or_reused_state_is_rejected_and_nothing_is_stored() {
    let (base, seen) = fake_server(Box::new(|_| (200, vec![], json!({ "access_token": "A", "refresh_token": "R" }).to_string()))).await;
    let (auth, store, _) = auth_with(&format!("{base}/t"));
    assert!(auth.complete("c", "x").await.is_err(), "sin autorización en curso");
    let url = auth.begin("http://127.0.0.1:1/cb").unwrap();
    let state = form_value(url.split_once('?').unwrap().1, "state").unwrap();
    assert!(auth.complete("c", "estado-falso").await.is_err());
    assert!(seen.lock().unwrap().is_empty(), "no se contacta con Spotify");
    assert!(store.get(KEY_SPOTIFY_REFRESH).unwrap().is_none());
    // El intento falso no gasta el `state` verdadero.
    auth.complete("c", &state).await.unwrap();
    assert!(auth.complete("c", &state).await.is_err(), "un solo uso");
}

#[tokio::test]
async fn a_stale_authorization_expires() {
    let (base, _) = fake_server(Box::new(|_| (200, vec![], json!({ "access_token": "A", "refresh_token": "R" }).to_string()))).await;
    let (auth, _, clock) = auth_with(&format!("{base}/t"));
    let url = auth.begin("http://127.0.0.1:1/cb").unwrap();
    let state = form_value(url.split_once('?').unwrap().1, "state").unwrap();
    clock.advance(11 * 60 * 1000);
    assert!(auth.complete("c", &state).await.is_err());
}

#[tokio::test]
async fn begin_requires_a_client_id() {
    let (auth, _, _) = auth_with("http://127.0.0.1:1/t");
    auth.set_client_id("");
    assert!(auth.begin("http://127.0.0.1:1/cb").is_err());
}

#[tokio::test]
async fn expired_tokens_are_refreshed_and_rotated_refresh_tokens_stored() {
    let n = Arc::new(AtomicUsize::new(0));
    let n2 = Arc::clone(&n);
    let (base, seen) = fake_server(Box::new(move |_| {
        let i = n2.fetch_add(1, Ordering::SeqCst);
        (200, vec![], json!({ "access_token": format!("AT{i}"), "refresh_token": format!("RT{i}"), "expires_in": 3600 }).to_string())
    }))
    .await;
    let (auth, store, clock) = auth_with(&format!("{base}/t"));
    store.set(KEY_SPOTIFY_REFRESH, "RT-inicial").unwrap();
    assert_eq!(auth.access_token().await.unwrap(), "AT0");
    assert_eq!(auth.access_token().await.unwrap(), "AT0", "en caché");
    let first = seen.lock().unwrap()[0].clone();
    assert_eq!(form_value(&first.body, "grant_type").as_deref(), Some("refresh_token"));
    assert_eq!(form_value(&first.body, "refresh_token").as_deref(), Some("RT-inicial"));
    assert_eq!(store.get(KEY_SPOTIFY_REFRESH).unwrap().as_deref(), Some("RT0"), "Spotify rotó el refresh token");
    clock.advance(3_600_000);
    assert_eq!(auth.access_token().await.unwrap(), "AT1");
    assert_eq!(n.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn concurrent_callers_share_one_refresh() {
    let n = Arc::new(AtomicUsize::new(0));
    let n2 = Arc::clone(&n);
    let (base, _) = fake_server(Box::new(move |_| {
        n2.fetch_add(1, Ordering::SeqCst);
        (200, vec![], json!({ "access_token": "AT", "expires_in": 3600 }).to_string())
    }))
    .await;
    let (auth, store, _) = auth_with(&format!("{base}/t"));
    store.set(KEY_SPOTIFY_REFRESH, "RT").unwrap();
    let results = futures_util::future::join_all((0..5).map(|_| {
        let a = Arc::clone(&auth);
        async move { a.access_token().await }
    }))
    .await;
    assert!(results.iter().all(|r| r.as_deref() == Ok("AT")));
    assert_eq!(n.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_revoked_grant_forgets_the_session() {
    let (base, _) = fake_server(Box::new(|_| (400, vec![], json!({ "error": "invalid_grant", "error_description": "Refresh token revoked" }).to_string()))).await;
    let (auth, store, _) = auth_with(&format!("{base}/t"));
    store.set(KEY_SPOTIFY_REFRESH, "RT").unwrap();
    assert_eq!(auth.access_token().await, Err(ApiError::NotConnected));
    assert!(!auth.is_connected());
    // Sin sesión ni siquiera se intenta.
    assert_eq!(auth.access_token().await, Err(ApiError::NotConnected));
}

#[tokio::test]
async fn an_invalid_refresh_token_also_forgets_the_session() {
    // Spotify no dice «revoked» aquí: el código `invalid_grant` es lo que cuenta.
    let (base, _) = fake_server(Box::new(|_| (400, vec![], json!({ "error": "invalid_grant", "error_description": "Invalid refresh token" }).to_string()))).await;
    let (auth, store, _) = auth_with(&format!("{base}/t"));
    store.set(KEY_SPOTIFY_REFRESH, "RT").unwrap();
    assert_eq!(auth.access_token().await, Err(ApiError::NotConnected));
    assert!(!auth.is_connected());
}

#[tokio::test]
async fn changing_the_client_id_forgets_the_old_session() {
    let (auth, store, _) = auth_with("http://127.0.0.1:9/t");
    store.set(KEY_SPOTIFY_REFRESH, "RT").unwrap();
    auth.set_client_id("cid123"); // mismo valor que ya tenía: nada cambia
    assert!(auth.is_connected());
    auth.set_client_id("otra-app");
    assert!(!auth.is_connected());
}

// ------------------------------------------------------------ API HTTP

async fn http_api(handler: Handler) -> (HttpSpotify, Arc<StdMutex<Vec<Req>>>) {
    let (base, seen) = fake_server(handler).await;
    let store = Arc::new(MemoryStore::default());
    store.set(KEY_SPOTIFY_REFRESH, "RT").unwrap();
    let auth = SpotifyAuth::with_endpoints(store, Arc::new(AppClock::new()), "https://a/authorize", &format!("{base}/token")).unwrap();
    auth.set_client_id("cid");
    (HttpSpotify::with_base(auth, &format!("{base}/v1")).unwrap(), seen)
}

fn token_ok() -> (u16, Vec<(&'static str, String)>, String) {
    (200, vec![], json!({ "access_token": "AT", "expires_in": 3600 }).to_string())
}

#[tokio::test]
async fn search_encodes_the_query_and_sends_the_bearer_token() {
    let (api, seen) = http_api(Box::new(|r| {
        if r.path.starts_with("/token") {
            token_ok()
        } else {
            (200, vec![], json!({ "tracks": { "items": [track_json(ID1, "La Bamba", "Ritchie", 200_000)] } }).to_string())
        }
    }))
    .await;
    let t = api.find_track("la bamba & más").await.unwrap().unwrap();
    assert_eq!((t.name.as_str(), t.artist_line().as_str()), ("La Bamba", "Ritchie"));
    let search = seen.lock().unwrap().iter().find(|r| r.path.starts_with("/v1/search")).cloned().unwrap();
    assert_eq!(search.path, "/v1/search?q=la%20bamba%20%26%20m%C3%A1s&type=track&limit=1");
    assert_eq!(search.header("authorization"), Some("Bearer AT"));
}

#[tokio::test]
async fn spotify_links_resolve_a_track_directly() {
    let (api, seen) = http_api(Box::new(|r| {
        if r.path.starts_with("/token") {
            token_ok()
        } else {
            (200, vec![], track_json(ID1, "Directa", "Alguien", 1000).to_string())
        }
    }))
    .await;
    let t = api.find_track(&format!("https://open.spotify.com/track/{ID1}?si=zzz")).await.unwrap().unwrap();
    assert_eq!(t.name, "Directa");
    assert!(seen.lock().unwrap().iter().any(|r| r.path == format!("/v1/tracks/{ID1}")));
}

#[tokio::test]
async fn queue_errors_are_translated() {
    let cases = [
        (404, json!({ "error": { "status": 404, "message": "Player command failed: No active device found", "reason": "NO_ACTIVE_DEVICE" } }), ApiError::NoDevice),
        (403, json!({ "error": { "status": 403, "message": "Player command failed: Premium required", "reason": "PREMIUM_REQUIRED" } }), ApiError::PremiumRequired),
    ];
    for (status, body, expected) in cases {
        let b = body.to_string();
        let (api, _) = http_api(Box::new(move |r| if r.path.starts_with("/token") { token_ok() } else { (status, vec![], b.clone()) })).await;
        let t = parse_track(&track_json(ID1, "x", "y", 1)).unwrap();
        assert_eq!(api.queue(&t).await, Err(expected));
    }
    let (api, _) = http_api(Box::new(|r| if r.path.starts_with("/token") { token_ok() } else { (429, vec![("retry-after", "7".into())], "{}".into()) })).await;
    let t = parse_track(&track_json(ID1, "x", "y", 1)).unwrap();
    assert_eq!(api.queue(&t).await, Err(ApiError::RateLimited(7)));
}

#[tokio::test]
async fn queue_posts_the_encoded_uri() {
    let (api, seen) = http_api(Box::new(|r| if r.path.starts_with("/token") { token_ok() } else { (204, vec![], String::new()) })).await;
    let t = parse_track(&track_json(ID1, "x", "y", 1)).unwrap();
    api.queue(&t).await.unwrap();
    let q = seen.lock().unwrap().iter().find(|r| r.path.starts_with("/v1/me/player/queue")).cloned().unwrap();
    assert_eq!(q.method, "POST");
    assert_eq!(q.path, format!("/v1/me/player/queue?uri=spotify%3Atrack%3A{ID1}"));
}

#[tokio::test]
async fn a_401_refreshes_the_token_once_and_retries() {
    let calls = Arc::new(AtomicUsize::new(0));
    let c2 = Arc::clone(&calls);
    let (api, seen) = http_api(Box::new(move |r| {
        if r.path.starts_with("/token") {
            token_ok()
        } else if c2.fetch_add(1, Ordering::SeqCst) == 0 {
            (401, vec![], json!({ "error": { "status": 401, "message": "The access token expired" } }).to_string())
        } else {
            (204, vec![], String::new())
        }
    }))
    .await;
    assert_eq!(api.now_playing().await, Ok(None));
    assert_eq!(seen.lock().unwrap().iter().filter(|r| r.path.starts_with("/token")).count(), 2);
}

#[tokio::test]
async fn now_playing_parses_tracks_and_ignores_ads_and_idle() {
    let body = json!({ "is_playing": true, "progress_ms": 1234, "item": track_json(ID1, "Suena", "Banda", 180_000) });
    let (api, _) = http_api(Box::new(move |r| if r.path.starts_with("/token") { token_ok() } else { (200, vec![], body.to_string()) })).await;
    let p = api.now_playing().await.unwrap().unwrap();
    assert_eq!((p.track.name.as_str(), p.progress_ms, p.is_playing), ("Suena", 1234, true));
    let (api, _) = http_api(Box::new(|r| if r.path.starts_with("/token") { token_ok() } else { (200, vec![], json!({ "is_playing": true, "item": null }).to_string()) })).await;
    assert_eq!(api.now_playing().await, Ok(None), "un anuncio no tiene item");
}

// ------------------------------------------------------------ servicio de peticiones

struct FakeApi {
    queue_result: StdMutex<std::result::Result<(), ApiError>>,
    queued: StdMutex<Vec<String>>,
    find: StdMutex<Option<Track>>,
    playing: StdMutex<Option<Playing>>,
    /// Órdenes de control recibidas, volumen del dispositivo y cola que devuelve Spotify.
    commands: StdMutex<Vec<PlayerCmd>>,
    volume: StdMutex<Option<u8>>,
    upcoming: StdMutex<Vec<Track>>,
}

impl FakeApi {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            queue_result: StdMutex::new(Ok(())),
            queued: StdMutex::new(Vec::new()),
            find: StdMutex::new(parse_track(&track_json(ID1, "La Bamba", "Ritchie Valens", 120_000))),
            playing: StdMutex::new(None),
            commands: StdMutex::new(Vec::new()),
            volume: StdMutex::new(Some(50)),
            upcoming: StdMutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl SpotifyApi for FakeApi {
    async fn find_track(&self, _q: &str) -> std::result::Result<Option<Track>, ApiError> {
        Ok(self.find.lock().unwrap().clone())
    }
    async fn queue(&self, t: &Track) -> std::result::Result<(), ApiError> {
        let r = self.queue_result.lock().unwrap().clone();
        if r.is_ok() {
            self.queued.lock().unwrap().push(t.uri.clone());
        }
        r
    }
    async fn now_playing(&self) -> std::result::Result<Option<Playing>, ApiError> {
        Ok(self.playing.lock().unwrap().clone())
    }
    async fn control(&self, op: PlayerCmd) -> std::result::Result<(), ApiError> {
        self.commands.lock().unwrap().push(op);
        Ok(())
    }
    async fn volume(&self) -> std::result::Result<Option<u8>, ApiError> {
        Ok(*self.volume.lock().unwrap())
    }
    async fn queue_list(&self) -> std::result::Result<Vec<Track>, ApiError> {
        Ok(self.upcoming.lock().unwrap().clone())
    }
}

#[derive(Default)]
struct FakeWallet {
    balance: StdMutex<u64>,
    refunds: StdMutex<u64>,
}

#[async_trait]
impl PointsGate for FakeWallet {
    async fn balance(&self, _u: &str) -> Result<u64> {
        Ok(*self.balance.lock().unwrap())
    }
    async fn spend(&self, _u: &str, cost: u64, _r: &str) -> Result<Option<u64>> {
        let mut b = self.balance.lock().unwrap();
        if *b >= cost {
            *b -= cost;
            Ok(Some(*b))
        } else {
            Ok(None)
        }
    }
    async fn refund(&self, _u: &str, amount: u64, _r: &str) -> Result<()> {
        *self.balance.lock().unwrap() += amount;
        *self.refunds.lock().unwrap() += amount;
        Ok(())
    }
}

struct Rig {
    svc: Arc<SongService>,
    api: Arc<FakeApi>,
    wallet: Arc<FakeWallet>,
    said: StdMutex<Vec<String>>,
    clock: Arc<AppClockAdjustable>,
    hub: OverlayHub,
}

async fn rig(tweak: impl FnOnce(&mut SongConfig)) -> Rig {
    let db = Db::open_memory().await.unwrap();
    let store = Arc::new(MemoryStore::default());
    store.set(KEY_SPOTIFY_REFRESH, "RT").unwrap();
    let clock = Arc::new(AppClockAdjustable::default());
    let auth = SpotifyAuth::with_endpoints(store, clock.clone(), "https://a/authorize", "http://127.0.0.1:1/t").unwrap();
    let (api, wallet, hub) = (FakeApi::new(), Arc::new(FakeWallet::default()), OverlayHub::new(16));
    let svc = SongService::new(db, auth, api.clone(), wallet.clone(), hub.clone(), clock.clone());
    let mut cfg = SpotifyConfig { client_id: "cid".into(), ..SpotifyConfig::default() };
    cfg.song.enabled = true;
    cfg.song.user_cooldown_s = 0;
    // La mayoría de pruebas piden siempre la misma canción: las de «no repetir» lo activan a propósito.
    cfg.song.no_repeat = false;
    tweak(&mut cfg.song);
    svc.set_config(cfg).await.unwrap();
    Rig { svc, api, wallet, said: StdMutex::default(), clock, hub }
}

fn chat_ev(id: &str, user: &str, text: &str) -> LiveEvent {
    let mut ev = sample_event(id);
    ev.kind = EventType::Chat;
    ev.user.id = format!("id-{user}");
    ev.user.unique_id = user.into();
    ev.user.nickname = user.to_uppercase();
    ev.user.is_follower = false;
    ev.user.is_subscriber = false;
    ev.user.is_moderator = false;
    ev.chat = Some(Chat { text: text.into(), emotes: None });
    ev
}

impl Rig {
    /// Lo que respondió el servicio a cada comando (lo que antes salía por el chat).
    fn said(&self) -> Vec<String> {
        self.said.lock().unwrap().clone()
    }

    /// Entrega un mensaje de chat al servicio; `true` si era un comando de canciones.
    async fn on_event(&self, ev: &LiveEvent) -> bool {
        let outcome = self.svc.on_event(ev).await;
        let handled = outcome.is_some();
        self.said.lock().unwrap().extend(outcome);
        handled
    }
}

#[tokio::test]
async fn a_request_is_queued_and_announced() {
    let r = rig(|_| {}).await;
    assert!(r.on_event(&chat_ev("1", "ana", "!sr la bamba")).await);
    assert_eq!(r.api.queued.lock().unwrap().len(), 1);
    let said = r.said();
    assert_eq!(said.len(), 1);
    assert!(said[0].contains("@ana") && said[0].contains("La Bamba") && said[0].contains("Ritchie Valens"), "{said:?}");
}

#[tokio::test]
async fn disabled_service_and_simulated_events_do_nothing() {
    let r = rig(|c| c.enabled = false).await;
    assert!(!r.on_event(&chat_ev("1", "ana", "!sr x")).await);
    let r = rig(|_| {}).await;
    assert!(!r.on_event(&chat_ev("sim-1", "ana", "!sr x")).await);
    assert!(r.api.queued.lock().unwrap().is_empty());
    assert!(!r.on_event(&chat_ev("2", "ana", "hola !sr")).await, "el comando va al principio");
    assert!(!r.on_event(&chat_ev("3", "ana", "!otra cosa")).await);
}

#[tokio::test]
async fn empty_queries_get_usage_help() {
    let r = rig(|_| {}).await;
    r.on_event(&chat_ev("1", "ana", "!sr")).await;
    assert!(r.said()[0].contains("uso: !sr"));
    assert!(r.api.queued.lock().unwrap().is_empty());
}

#[tokio::test]
async fn roles_blocked_users_and_terms_are_enforced() {
    let r = rig(|c| {
        c.min_role = MinRole::Subscriber;
        c.blocked_users = vec!["troll".into()];
    })
    .await;
    r.on_event(&chat_ev("1", "ana", "!sr x")).await;
    assert!(r.said()[0].contains("permiso"));
    let mut sub = chat_ev("2", "troll", "!sr x");
    sub.user.is_subscriber = true;
    r.on_event(&sub).await;
    assert!(r.said()[1].contains("no puedes"));
    let mut ok = chat_ev("3", "bea", "!sr x");
    ok.user.is_moderator = true;
    r.on_event(&ok).await;
    assert_eq!(r.api.queued.lock().unwrap().len(), 1, "un moderador cumple «suscriptor»");

    let r = rig(|c| c.blocked_terms = vec!["ritchie".into()]).await;
    r.on_event(&chat_ev("4", "ana", "!sr x")).await;
    assert!(r.said()[0].contains("bloqueada"));
    assert!(r.api.queued.lock().unwrap().is_empty());
}

#[tokio::test]
async fn long_tracks_and_unknown_tracks_are_rejected() {
    let r = rig(|c| c.max_duration_s = 60).await;
    r.on_event(&chat_ev("1", "ana", "!sr x")).await;
    assert!(r.said()[0].contains("dura demasiado"));
    *r.api.find.lock().unwrap() = None;
    r.on_event(&chat_ev("2", "ana", "!sr nada")).await;
    assert!(r.said()[1].contains("no encontré"));
    assert!(r.api.queued.lock().unwrap().is_empty());
}

#[tokio::test]
async fn per_user_limit_and_cooldown_apply() {
    let r = rig(|c| c.per_user_limit = 2).await;
    for i in 0..3 {
        r.on_event(&chat_ev(&format!("a{i}"), "ana", "!sr x")).await;
    }
    assert_eq!(r.api.queued.lock().unwrap().len(), 2);
    assert!(r.said()[2].contains("ya tienes 2"));
    // Otro espectador no se ve afectado.
    r.on_event(&chat_ev("b", "bea", "!sr x")).await;
    assert_eq!(r.api.queued.lock().unwrap().len(), 3);

    let r = rig(|c| c.user_cooldown_s = 30).await;
    r.on_event(&chat_ev("1", "ana", "!sr x")).await;
    r.on_event(&chat_ev("2", "ana", "!sr x")).await;
    assert!(r.said()[1].contains("espera"));
    r.clock.advance(31_000);
    r.on_event(&chat_ev("3", "ana", "!sr x")).await;
    assert_eq!(r.api.queued.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn points_are_charged_and_refunded_when_queueing_fails() {
    let r = rig(|c| c.cost_points = 50).await;
    *r.wallet.balance.lock().unwrap() = 80;
    r.on_event(&chat_ev("1", "ana", "!sr x")).await;
    assert_eq!(*r.wallet.balance.lock().unwrap(), 30);
    r.on_event(&chat_ev("2", "bea", "!sr x")).await;
    assert!(r.said()[1].contains("necesitas 50 puntos"));
    assert_eq!(r.api.queued.lock().unwrap().len(), 1);

    *r.wallet.balance.lock().unwrap() = 100;
    *r.api.queue_result.lock().unwrap() = Err(ApiError::NoDevice);
    r.on_event(&chat_ev("3", "cris", "!sr x")).await;
    assert_eq!(*r.wallet.balance.lock().unwrap(), 100, "se devolvieron los puntos");
    assert_eq!(*r.wallet.refunds.lock().unwrap(), 50);
    assert!(r.said()[2].contains("no hay Spotify activo"));
}

#[tokio::test]
async fn a_failed_attempt_does_not_burn_the_cooldown() {
    let r = rig(|c| c.user_cooldown_s = 60).await;
    *r.api.queue_result.lock().unwrap() = Err(ApiError::PremiumRequired);
    r.on_event(&chat_ev("1", "ana", "!sr x")).await;
    *r.api.queue_result.lock().unwrap() = Ok(());
    r.on_event(&chat_ev("2", "ana", "!sr x")).await;
    assert_eq!(r.api.queued.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn now_playing_updates_pending_requests_and_the_overlay() {
    let r = rig(|c| c.per_user_limit = 1).await;
    r.on_event(&chat_ev("1", "ana", "!sr x")).await;
    r.on_event(&chat_ev("2", "ana", "!sr x")).await;
    assert!(r.said()[1].contains("ya tienes 1"));

    let track = parse_track(&track_json(ID1, "La Bamba", "Ritchie Valens", 120_000)).unwrap();
    *r.api.playing.lock().unwrap() = Some(Playing { track: track.clone(), progress_ms: 5000, is_playing: true });
    r.svc.poll_once().await;
    let msg = r.hub.retained().into_iter().find(|m| m.channel == CHANNEL).unwrap();
    assert_eq!(msg.data["title"], "La Bamba");
    assert_eq!(msg.data["requestedBy"], "ana");
    assert_eq!(msg.data["playing"], true);

    // Suena otra canción: la pedida terminó y ana puede pedir de nuevo.
    let other = parse_track(&track_json(ID2, "Otra", "Alguien", 1000)).unwrap();
    *r.api.playing.lock().unwrap() = Some(Playing { track: other, progress_ms: 0, is_playing: true });
    r.svc.poll_once().await;
    r.on_event(&chat_ev("3", "ana", "!sr x")).await;
    assert_eq!(r.api.queued.lock().unwrap().len(), 2);

    *r.api.playing.lock().unwrap() = None;
    r.svc.poll_once().await;
    let msg = r.hub.retained().into_iter().find(|m| m.channel == CHANNEL).unwrap();
    assert_eq!(msg.data["kind"], "none");
}

#[tokio::test]
async fn manual_queue_skips_roles_and_costs() {
    let r = rig(|c| {
        c.min_role = MinRole::Moderator;
        c.cost_points = 999;
    })
    .await;
    let t = r.svc.manual_queue("lo que sea").await.unwrap();
    assert_eq!(t.name, "La Bamba");
    assert_eq!(r.api.queued.lock().unwrap().len(), 1);
    *r.api.find.lock().unwrap() = None;
    assert!(r.svc.manual_queue("nada").await.is_err());
}

#[tokio::test]
async fn config_persists_and_survives_garbage() {
    let r = rig(|c| c.cost_points = 10).await;
    let db = r.svc.config();
    assert_eq!(db.song.cost_points, 10);
    assert!(r.svc.set_config(SpotifyConfig { client_id: "inválido!".into(), ..SpotifyConfig::default() }).await.is_err());
    assert_eq!(r.svc.config().client_id, "cid", "lo inválido no se aplica");
}

#[test]
fn the_builtin_client_id_is_used_unless_the_user_sets_their_own() {
    assert_eq!(effective_client_id("", "integrado"), "integrado");
    assert_eq!(effective_client_id("   ", "integrado"), "integrado");
    assert_eq!(effective_client_id(" propio ", "integrado"), "propio");
    assert_eq!(effective_client_id("", ""), "");
}

#[tokio::test]
async fn song_requests_from_twitch_are_queued_too() {
    let r = rig(|_| {}).await;
    let mut ev = chat_ev("t1", "ana", "!sr la bamba");
    ev.platform = Platform::Twitch;
    assert!(r.on_event(&ev).await);
    assert_eq!(r.api.queued.lock().unwrap().len(), 1, "la canción entra a la cola");
}

// ------------------------------------------------------------ control del reproductor y cola

#[tokio::test]
async fn player_commands_use_the_right_endpoints() {
    let (api, seen) = http_api(Box::new(|r| if r.path.starts_with("/token") { token_ok() } else { (204, vec![], String::new()) })).await;
    for op in [PlayerCmd::Next, PlayerCmd::Previous, PlayerCmd::Pause, PlayerCmd::Resume, PlayerCmd::Volume(40), PlayerCmd::Volume(250)] {
        api.control(op).await.unwrap();
    }
    let got: Vec<(String, String)> = seen.lock().unwrap().iter().filter(|r| r.path.starts_with("/v1/")).map(|r| (r.method.clone(), r.path.clone())).collect();
    let expected = [
        ("POST", "/v1/me/player/next"),
        ("POST", "/v1/me/player/previous"),
        ("PUT", "/v1/me/player/pause"),
        ("PUT", "/v1/me/player/play"),
        ("PUT", "/v1/me/player/volume?volume_percent=40"),
        ("PUT", "/v1/me/player/volume?volume_percent=100"),
    ];
    assert_eq!(got, expected.map(|(m, p)| (m.to_string(), p.to_string())));
}

#[tokio::test]
async fn player_command_errors_are_translated() {
    let body = json!({ "error": { "status": 403, "message": "Player command failed: Premium required", "reason": "PREMIUM_REQUIRED" } }).to_string();
    let (api, _) = http_api(Box::new(move |r| if r.path.starts_with("/token") { token_ok() } else { (403, vec![], body.clone()) })).await;
    assert_eq!(api.control(PlayerCmd::Next).await, Err(ApiError::PremiumRequired));
}

#[tokio::test]
async fn volume_and_queue_are_parsed() {
    let player = json!({ "is_playing": true, "device": { "volume_percent": 35 } }).to_string();
    let queue = json!({ "currently_playing": track_json(ID1, "Suena", "A", 1), "queue": [
        track_json(ID2, "Siguiente", "B", 1000),
        { "id": "ep", "name": "Un episodio", "type": "episode" }
    ]})
    .to_string();
    let (api, _) = http_api(Box::new(move |r| match r.path.as_str() {
        p if p.starts_with("/token") => token_ok(),
        "/v1/me/player/queue" => (200, vec![], queue.clone()),
        _ => (200, vec![], player.clone()),
    }))
    .await;
    assert_eq!(api.volume().await, Ok(Some(35)));
    let q = api.queue_list().await.unwrap();
    assert_eq!(q.len(), 1, "los episodios no cuentan");
    assert_eq!(q[0].name, "Siguiente");
    let (api, _) = http_api(Box::new(|r| if r.path.starts_with("/token") { token_ok() } else { (204, vec![], String::new()) })).await;
    assert_eq!(api.volume().await, Ok(None), "sin dispositivo activo");
    assert_eq!(api.queue_list().await, Ok(Vec::new()));
}

async fn control_rig(tweak: impl FnOnce(&mut ControlConfig)) -> Rig {
    let r = rig(|_| {}).await;
    let mut cfg = r.svc.config();
    cfg.control.enabled = true;
    tweak(&mut cfg.control);
    r.svc.set_config(cfg).await.unwrap();
    r
}

fn mod_ev(id: &str, user: &str, text: &str) -> LiveEvent {
    let mut ev = chat_ev(id, user, text);
    ev.user.is_moderator = true;
    ev
}

#[tokio::test]
async fn moderators_skip_pause_resume_and_change_the_volume() {
    let r = control_rig(|_| {}).await;
    for (i, text) in ["!skip", "!pausa", "!play", "!volumen 30", "!volumen +", "!volumen -", "!volumen -5", "!volumen +5"].into_iter().enumerate() {
        assert!(r.on_event(&mod_ev(&i.to_string(), "mod", text)).await, "{text}");
    }
    assert_eq!(
        *r.api.commands.lock().unwrap(),
        [
            PlayerCmd::Next,
            PlayerCmd::Pause,
            PlayerCmd::Resume,
            PlayerCmd::Volume(30),
            PlayerCmd::Volume(60),
            PlayerCmd::Volume(40),
            PlayerCmd::Volume(45),
            PlayerCmd::Volume(55),
        ]
    );
    assert!(r.said()[0].contains("saltó"), "{:?}", r.said());
}

#[tokio::test]
async fn viewers_cannot_control_the_player_and_a_bad_volume_explains_the_usage() {
    let r = control_rig(|_| {}).await;
    for text in ["!skip", "!pausa", "!play", "!volumen 10"] {
        assert!(r.on_event(&chat_ev("x", "ana", text)).await);
    }
    assert!(r.api.commands.lock().unwrap().is_empty());
    assert!(r.said().iter().all(|s| s.contains("no tienes permiso")), "{:?}", r.said());
    assert!(r.on_event(&mod_ev("y", "mod", "!volumen mucho")).await);
    assert!(r.said().last().unwrap().contains("uso: !volumen"));
    assert!(r.api.commands.lock().unwrap().is_empty());
}

#[tokio::test]
async fn control_commands_do_nothing_while_disabled() {
    let r = rig(|_| {}).await;
    assert!(!r.on_event(&mod_ev("1", "mod", "!skip")).await);
    assert!(r.api.commands.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_minimum_role_for_control_can_be_relaxed() {
    let r = control_rig(|c| c.min_role = MinRole::Everyone).await;
    assert!(r.on_event(&chat_ev("1", "ana", "!pausa")).await);
    assert_eq!(*r.api.commands.lock().unwrap(), [PlayerCmd::Pause]);
}

#[tokio::test]
async fn enough_votes_skip_the_song_and_each_viewer_counts_once() {
    let r = control_rig(|c| c.skip_votes = 3).await;
    let track = parse_track(&track_json(ID1, "La Bamba", "Ritchie Valens", 120_000)).unwrap();
    *r.api.playing.lock().unwrap() = Some(Playing { track, progress_ms: 0, is_playing: true });
    r.svc.poll_once().await;
    r.on_event(&chat_ev("1", "ana", "!skip")).await;
    r.on_event(&chat_ev("2", "ana", "!skip")).await;
    r.on_event(&chat_ev("3", "bob", "!skip")).await;
    assert!(r.api.commands.lock().unwrap().is_empty(), "dos votantes distintos no bastan");
    assert!(r.said()[1].contains("1/3"), "el voto repetido no suma: {:?}", r.said());
    r.on_event(&chat_ev("4", "eva", "!skip")).await;
    assert_eq!(*r.api.commands.lock().unwrap(), [PlayerCmd::Next]);
    // Los votos se vacían: hacen falta otros tres.
    r.on_event(&chat_ev("5", "ana", "!skip")).await;
    assert_eq!(r.api.commands.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn votes_do_not_carry_over_to_the_next_song() {
    let r = control_rig(|c| c.skip_votes = 2).await;
    let set = |id: &str| *r.api.playing.lock().unwrap() = Some(Playing { track: parse_track(&track_json(id, "x", "y", 1000)).unwrap(), progress_ms: 0, is_playing: true });
    set(ID1);
    r.svc.poll_once().await;
    r.on_event(&chat_ev("1", "ana", "!skip")).await;
    set(ID2);
    r.svc.poll_once().await;
    r.on_event(&chat_ev("2", "bob", "!skip")).await;
    assert!(r.api.commands.lock().unwrap().is_empty(), "el voto de ana era de la otra canción");
}

#[tokio::test]
async fn the_queue_command_asks_the_overlay_to_show_the_list() {
    let r = control_rig(|c| c.queue_show_s = 20).await;
    *r.api.upcoming.lock().unwrap() = vec![parse_track(&track_json(ID2, "Siguiente", "B", 1000)).unwrap()];
    let track = parse_track(&track_json(ID1, "Suena", "A", 1000)).unwrap();
    *r.api.playing.lock().unwrap() = Some(Playing { track, progress_ms: 0, is_playing: true });
    r.svc.poll_once().await;
    let mut rx = r.hub.subscribe();
    assert!(r.on_event(&chat_ev("1", "ana", "!cola")).await, "cualquiera puede pedir ver la cola");
    let m = rx.recv().await.unwrap();
    assert_eq!((m.channel.as_str(), m.data["kind"].clone(), m.data["seconds"].clone()), (QUEUE_CHANNEL, json!("show"), json!(20)));
    assert!(r.said()[0].contains("Siguiente"), "{:?}", r.said());
}

#[tokio::test]
async fn the_queue_is_published_with_who_asked_for_each_song() {
    let r = rig(|_| {}).await;
    r.on_event(&chat_ev("1", "ana", "!sr x")).await;
    *r.api.upcoming.lock().unwrap() = vec![
        parse_track(&track_json(ID1, "La Bamba", "Ritchie Valens", 120_000)).unwrap(),
        parse_track(&track_json(ID2, "De Spotify", "Alguien", 1000)).unwrap(),
    ];
    let other = parse_track(&track_json("1111111111111111111111", "Suena", "A", 1000)).unwrap();
    *r.api.playing.lock().unwrap() = Some(Playing { track: other, progress_ms: 0, is_playing: true });
    r.svc.poll_once().await;
    let msg = r.hub.retained().into_iter().find(|m| m.channel == QUEUE_CHANNEL).unwrap();
    assert_eq!(msg.data["kind"], "queue");
    let items = msg.data["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!((items[0]["title"].clone(), items[0]["requestedBy"].clone()), (json!("La Bamba"), json!("ana")));
    assert_eq!(items[1]["requestedBy"], json!(null), "la añadida desde Spotify no tiene quien la pidió");
    assert_eq!(r.svc.overview().queue.len(), 2);

    // Sin canción sonando, la cola se vacía.
    *r.api.playing.lock().unwrap() = None;
    r.svc.poll_once().await;
    let msg = r.hub.retained().into_iter().find(|m| m.channel == QUEUE_CHANNEL).unwrap();
    assert!(msg.data["items"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn the_queue_is_only_reread_when_the_song_changes_or_after_a_while() {
    let r = rig(|_| {}).await;
    let playing = parse_track(&track_json(ID1, "Suena", "A", 1000)).unwrap();
    *r.api.playing.lock().unwrap() = Some(Playing { track: playing, progress_ms: 0, is_playing: true });
    r.svc.poll_once().await;
    *r.api.upcoming.lock().unwrap() = vec![parse_track(&track_json(ID2, "Nueva", "B", 1000)).unwrap()];
    r.svc.poll_once().await;
    assert!(r.svc.overview().queue.is_empty(), "no se vuelve a leer a los pocos segundos");
    r.clock.advance(20_000);
    r.svc.poll_once().await;
    assert_eq!(r.svc.overview().queue.len(), 1);
}

#[tokio::test]
async fn songs_already_requested_playing_or_recently_played_are_rejected_when_asked() {
    let r = rig(|c| c.no_repeat = true).await;
    r.on_event(&chat_ev("1", "ana", "!sr x")).await;
    r.on_event(&chat_ev("2", "bob", "!sr x")).await;
    assert_eq!(r.api.queued.lock().unwrap().len(), 1);
    assert!(r.said()[1].contains("ya está en la cola"), "{:?}", r.said());

    // Suena otra y la pedida termina: sigue contando como «sonó hace poco».
    let set = |id: &str| *r.api.playing.lock().unwrap() = Some(Playing { track: parse_track(&track_json(id, "x", "y", 1000)).unwrap(), progress_ms: 0, is_playing: true });
    set(ID1);
    r.svc.poll_once().await;
    set(ID2);
    r.svc.poll_once().await;
    r.on_event(&chat_ev("3", "eva", "!sr x")).await;
    assert_eq!(r.api.queued.lock().unwrap().len(), 1, "{:?}", r.said());

    // Y un rechazo no deja al espectador esperando el cooldown.
    let r = rig(|c| {
        c.no_repeat = true;
        c.user_cooldown_s = 600;
    })
    .await;
    r.on_event(&chat_ev("1", "ana", "!sr x")).await;
    r.on_event(&chat_ev("2", "bob", "!sr x")).await;
    *r.api.find.lock().unwrap() = parse_track(&track_json(ID2, "Otra", "B", 1000));
    r.on_event(&chat_ev("3", "bob", "!sr otra")).await;
    assert_eq!(r.api.queued.lock().unwrap().len(), 2, "{:?}", r.said());
}

#[tokio::test]
async fn control_settings_are_sanitized() {
    let r = rig(|_| {}).await;
    let mut cfg = r.svc.config();
    cfg.control.skip_commands = vec!["!Skip".into(), " ".into(), "skip".into()];
    cfg.control.volume_step = 0;
    cfg.control.queue_show_s = 9_999;
    cfg.control.skip_votes = 5_000_000;
    let got = r.svc.set_config(cfg).await.unwrap().control;
    assert_eq!(got.skip_commands, ["skip"]);
    assert_eq!((got.volume_step, got.queue_show_s, got.skip_votes), (1, 300, 1_000));
}

#[test]
fn control_operations_parse_from_the_ui_and_rules() {
    assert_eq!(ControlOp::parse("volumeChange", Some(-300)).unwrap(), ControlOp::VolumeChange(-100));
    assert_eq!(ControlOp::parse("previous", None).unwrap(), ControlOp::Previous);
    assert!(ControlOp::parse("volumeSet", None).is_err());
    assert!(ControlOp::parse("nope", None).is_err());
}

#[tokio::test]
async fn manual_control_adjusts_the_volume_relative_to_the_device() {
    let r = rig(|_| {}).await;
    r.svc.control(ControlOp::VolumeChange(-70)).await.unwrap();
    r.svc.control(ControlOp::Next).await.unwrap();
    assert_eq!(*r.api.commands.lock().unwrap(), [PlayerCmd::Volume(0), PlayerCmd::Next]);
    *r.api.volume.lock().unwrap() = None;
    assert!(r.svc.control(ControlOp::VolumeChange(10)).await.is_err(), "sin dispositivo no se puede subir el volumen");
}
