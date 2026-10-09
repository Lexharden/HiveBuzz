use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

use super::*;
use crate::events::{EventType, LiveEvent, Platform};
use crate::secrets::{MemoryStore, SecretStore};
use crate::testutil::{fake_http, TestClock};
use crate::twitch::KEY_TWITCH_REFRESH;

// ------------------------------------------------------------ servidor IRC de mentira

#[derive(Clone)]
enum Then {
    Hold,
    Close,
}

#[derive(Clone)]
struct Script {
    /// Líneas que se envían tras recibir el JOIN.
    lines: Vec<String>,
    then: Then,
}

struct Irc {
    url: String,
    connections: Arc<AtomicUsize>,
    received: Arc<Mutex<Vec<String>>>,
}

/// La conexión número N usa el guion N (o el último si hay menos).
async fn fake_irc(scripts: Vec<Script>) -> Irc {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let (connections, received) = (Arc::new(AtomicUsize::new(0)), Arc::new(Mutex::new(Vec::new())));
    let (c2, r2) = (Arc::clone(&connections), Arc::clone(&received));
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else { return };
            let index = c2.fetch_add(1, Ordering::SeqCst);
            let script = scripts[index.min(scripts.len() - 1)].clone();
            let received = Arc::clone(&r2);
            tokio::spawn(async move {
                let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else { return };
                while let Some(Ok(msg)) = ws.next().await {
                    let Message::Text(t) = msg else { continue };
                    received.lock().unwrap().push(t.to_string());
                    if t.starts_with("JOIN ") {
                        for line in &script.lines {
                            if ws.send(Message::text(format!("{line}\r\n"))).await.is_err() {
                                return;
                            }
                        }
                        if matches!(script.then, Then::Close) {
                            let _ = ws.close(None).await;
                            return;
                        }
                    }
                }
            });
        }
    });
    Irc { url, connections, received }
}

fn chat_line(id: &str, user_id: &str, login: &str, text: &str) -> String {
    format!("@badges=;display-name={login};id={id};user-id={user_id};tmi-sent-ts=1700000000000 :{login}!{login}@{login}.tmi.twitch.tv PRIVMSG #canal :{text}")
}

const ROOMSTATE: &str = "@room-id=1 :tmi.twitch.tv ROOMSTATE #canal";

// ------------------------------------------------------------ servidor EventSub de mentira

/// Envía un welcome y, tras `delay_ms`, un aviso de nuevo seguidor. Cuenta las conexiones.
async fn fake_eventsub(follower: bool) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let count = Arc::new(AtomicUsize::new(0));
    let c2 = Arc::clone(&count);
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else { return };
            c2.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else { return };
                let welcome = json!({
                    "metadata": { "message_id": "w1", "message_type": "session_welcome" },
                    "payload": { "session": { "id": "SESSION1", "keepalive_timeout_seconds": 30 } }
                });
                let _ = ws.send(Message::text(welcome.to_string())).await;
                if follower {
                    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                    let note = json!({
                        "metadata": { "message_id": "n1", "message_type": "notification" },
                        "payload": {
                            "subscription": { "type": "channel.follow" },
                            "event": { "user_id": "555", "user_login": "NuevoFan", "user_name": "NuevoFan" }
                        }
                    });
                    let _ = ws.send(Message::text(note.to_string())).await;
                }
                while ws.next().await.is_some() {}
            });
        }
    });
    (url, count)
}

// ------------------------------------------------------------ plataforma

struct Rig {
    source: TwitchSource,
    rx: mpsc::Receiver<SourceMessage>,
    auth: Arc<TwitchAuth>,
    store: Arc<MemoryStore>,
    live: Arc<AtomicBool>,
    viewers: Arc<AtomicU64>,
    http_seen: Arc<Mutex<Vec<crate::testutil::Req>>>,
}

fn fast() -> Tuning {
    Tuning {
        backoff_base: std::time::Duration::from_millis(10),
        backoff_max: std::time::Duration::from_millis(40),
        healthy_after: std::time::Duration::from_secs(60),
        idle_timeout: std::time::Duration::from_secs(5),
        viewers_every: std::time::Duration::from_secs(30),
    }
}

/// `login`: cuenta con la que hay sesión iniciada (None = sin sesión).
async fn rig(irc_url: &str, eventsub_url: &str, login: Option<&str>, tuning: Tuning) -> Rig {
    let live = Arc::new(AtomicBool::new(true));
    let viewers = Arc::new(AtomicU64::new(42));
    let (l2, v2) = (Arc::clone(&live), Arc::clone(&viewers));
    let login_owned = login.map(str::to_string);
    let (base, http_seen) = fake_http(Box::new(move |r| {
        let p = r.path.split('?').next().unwrap_or("");
        match p {
            "/oauth2/token" => (200, json!({ "access_token": "AT", "expires_in": 3600 }).to_string()),
            "/oauth2/validate" => (200, json!({ "login": login_owned.clone().unwrap_or_default(), "user_id": "999" }).to_string()),
            "/helix/users" => (200, json!({ "data": [{ "id": "999" }] }).to_string()),
            "/helix/streams" => {
                if l2.load(Ordering::SeqCst) {
                    (200, json!({ "data": [{ "viewer_count": v2.load(Ordering::SeqCst) }] }).to_string())
                } else {
                    (200, json!({ "data": [] }).to_string())
                }
            }
            "/helix/eventsub/subscriptions" => (202, "{}".into()),
            _ => (404, "{}".into()),
        }
    }))
    .await;
    let store = Arc::new(MemoryStore::default());
    if login.is_some() {
        store.set(KEY_TWITCH_REFRESH, "RT").unwrap();
    }
    let clock = Arc::new(TestClock::default());
    let auth = TwitchAuth::with_endpoint(store.clone(), clock.clone(), &base, None).unwrap();
    auth.set_client_id("cid");
    let helix = Arc::new(Helix::with_base(Arc::clone(&auth), &format!("{base}/helix")).unwrap());
    let (tx, rx) = mpsc::channel(256);
    let source = TwitchSource::with_options(tx, Arc::clone(&auth), helix, clock, Endpoints { irc_url: irc_url.into(), eventsub_url: eventsub_url.into() }, tuning);
    Rig { source, rx, auth, store, live, viewers, http_seen }
}

async fn recv(rx: &mut mpsc::Receiver<SourceMessage>, pred: impl Fn(&SourceMessage) -> bool) -> SourceMessage {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(left, rx.recv()).await {
            Ok(Some(m)) if pred(&m) => return m,
            Ok(Some(_)) => {}
            _ => panic!("no llegó el mensaje esperado a tiempo"),
        }
    }
}

/// Recoge todo lo que llegue durante `ms` milisegundos (para comprobar el conjunto, sin depender del orden).
async fn collect(rx: &mut mpsc::Receiver<SourceMessage>, ms: u64) -> Vec<SourceMessage> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(ms);
    let mut out = Vec::new();
    while let Ok(Some(m)) = tokio::time::timeout(deadline.saturating_duration_since(tokio::time::Instant::now()), rx.recv()).await {
        out.push(m);
    }
    out
}

fn is_state(s: ConnectionState) -> impl Fn(&SourceMessage) -> bool {
    move |m| matches!(m, SourceMessage::Status(u) if u.state == s)
}

fn chat_event(m: &SourceMessage) -> Option<&LiveEvent> {
    match m {
        SourceMessage::Event(e) if e.kind == EventType::Chat => Some(e),
        _ => None,
    }
}

async fn connect(r: &Rig, channel: &str) {
    r.source.connect(ConnectTarget::new(channel)).await.unwrap();
}

// ------------------------------------------------------------ pruebas

#[tokio::test]
async fn anonymous_chat_connects_without_an_account_and_delivers_events() {
    let irc = fake_irc(vec![Script { lines: vec![ROOMSTATE.into(), chat_line("m1", "1", "ana", "hola")], then: Then::Hold }]).await;
    let mut r = rig(&irc.url, "ws://127.0.0.1:1", None, fast()).await;
    connect(&r, "canal").await;
    let st = recv(&mut r.rx, is_state(ConnectionState::Connected)).await;
    assert!(matches!(&st, SourceMessage::Status(s) if s.detail.as_deref() == Some(DETAIL_CHAT)));
    let m = recv(&mut r.rx, |m| chat_event(m).is_some()).await;
    let ev = chat_event(&m).unwrap();
    assert_eq!((ev.user.id.as_str(), ev.chat.as_ref().unwrap().text.as_str()), ("tw:1", "hola"));
    let sent = irc.received.lock().unwrap().clone();
    assert!(sent.iter().any(|l| l.starts_with("NICK justinfan")), "entra como anónimo: {sent:?}");
    assert!(sent.iter().any(|l| l == "JOIN #canal"));
    assert!(sent.iter().any(|l| l.contains("twitch.tv/tags")), "pide las etiquetas");
    assert!(!sent.iter().any(|l| l.contains("oauth:")), "nunca se envía un token por IRC");
}

#[tokio::test]
async fn ping_gets_a_pong_and_duplicates_are_dropped() {
    let irc = fake_irc(vec![Script {
        lines: vec![ROOMSTATE.into(), "PING :tmi.twitch.tv".into(), chat_line("same", "1", "ana", "uno"), chat_line("same", "1", "ana", "uno"), chat_line("otro", "2", "bea", "dos")],
        then: Then::Hold,
    }])
    .await;
    let mut r = rig(&irc.url, "ws://127.0.0.1:1", None, fast()).await;
    connect(&r, "canal").await;
    let a = recv(&mut r.rx, |m| chat_event(m).is_some()).await;
    let b = recv(&mut r.rx, |m| chat_event(m).is_some()).await;
    assert_eq!(chat_event(&a).unwrap().id, "same");
    assert_eq!(chat_event(&b).unwrap().id, "otro", "el repetido no pasó");
    for _ in 0..50 {
        if irc.received.lock().unwrap().iter().any(|l| l.starts_with("PONG")) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("no respondió al PING");
}

#[tokio::test]
async fn a_dropped_connection_reconnects_and_replayed_messages_are_not_repeated() {
    let irc = fake_irc(vec![
        Script { lines: vec![ROOMSTATE.into(), chat_line("m1", "1", "ana", "antes")], then: Then::Close },
        // Tras reconectar, Twitch reenvía el mensaje anterior y añade uno nuevo.
        Script { lines: vec![ROOMSTATE.into(), chat_line("m1", "1", "ana", "antes"), chat_line("m2", "1", "ana", "despues")], then: Then::Hold },
    ])
    .await;
    let mut r = rig(&irc.url, "ws://127.0.0.1:1", None, fast()).await;
    connect(&r, "canal").await;
    let first = recv(&mut r.rx, |m| chat_event(m).is_some()).await;
    assert_eq!(chat_event(&first).unwrap().id, "m1");
    let re = recv(&mut r.rx, is_state(ConnectionState::Reconnecting)).await;
    assert!(matches!(&re, SourceMessage::Status(s) if s.attempt == Some(1) && s.retry_in_ms.is_some()));
    let next = recv(&mut r.rx, |m| chat_event(m).is_some()).await;
    assert_eq!(chat_event(&next).unwrap().id, "m2", "m1 ya se había entregado");
    assert_eq!(irc.connections.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn twitch_asking_to_reconnect_is_honoured() {
    let irc = fake_irc(vec![
        Script { lines: vec![ROOMSTATE.into(), ":tmi.twitch.tv RECONNECT".into()], then: Then::Hold },
        Script { lines: vec![ROOMSTATE.into(), chat_line("m9", "1", "ana", "ya")], then: Then::Hold },
    ])
    .await;
    let mut r = rig(&irc.url, "ws://127.0.0.1:1", None, fast()).await;
    connect(&r, "canal").await;
    recv(&mut r.rx, is_state(ConnectionState::Reconnecting)).await;
    recv(&mut r.rx, |m| chat_event(m).is_some()).await;
    assert_eq!(irc.connections.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_silent_connection_is_replaced() {
    let irc = fake_irc(vec![Script { lines: vec![ROOMSTATE.into()], then: Then::Hold }]).await;
    let tuning = Tuning { idle_timeout: std::time::Duration::from_millis(150), ..fast() };
    let mut r = rig(&irc.url, "ws://127.0.0.1:1", None, tuning).await;
    connect(&r, "canal").await;
    let st = recv(&mut r.rx, is_state(ConnectionState::Reconnecting)).await;
    assert!(matches!(&st, SourceMessage::Status(s) if s.detail.as_deref().is_some_and(|d| d.contains("dejó de responder"))));
}

#[tokio::test]
async fn a_rejected_connection_is_reported() {
    let irc = fake_irc(vec![Script { lines: vec![":tmi.twitch.tv NOTICE * :Login unsuccessful".into()], then: Then::Hold }]).await;
    let mut r = rig(&irc.url, "ws://127.0.0.1:1", None, fast()).await;
    connect(&r, "canal").await;
    let st = recv(&mut r.rx, is_state(ConnectionState::Reconnecting)).await;
    assert!(matches!(&st, SourceMessage::Status(s) if s.detail.as_deref().is_some_and(|d| d.contains("rechazó"))));
}

#[tokio::test]
async fn an_unreachable_server_keeps_retrying_with_growing_attempts() {
    let mut r = rig("ws://127.0.0.1:1", "ws://127.0.0.1:1", None, fast()).await;
    connect(&r, "canal").await;
    let a = recv(&mut r.rx, |m| matches!(m, SourceMessage::Status(s) if s.attempt == Some(1))).await;
    let b = recv(&mut r.rx, |m| matches!(m, SourceMessage::Status(s) if s.attempt == Some(3))).await;
    assert!(matches!(a, SourceMessage::Status(_)) && matches!(b, SourceMessage::Status(_)));
}

#[tokio::test]
async fn disconnect_stops_everything() {
    let irc = fake_irc(vec![Script { lines: vec![ROOMSTATE.into()], then: Then::Close }]).await;
    let mut r = rig(&irc.url, "ws://127.0.0.1:1", None, fast()).await;
    connect(&r, "canal").await;
    recv(&mut r.rx, is_state(ConnectionState::Reconnecting)).await;
    r.source.disconnect().await.unwrap();
    recv(&mut r.rx, is_state(ConnectionState::Disconnected)).await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let n = irc.connections.load(Ordering::SeqCst);
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(irc.connections.load(Ordering::SeqCst), n, "ya no reconecta");
}

#[tokio::test]
async fn reconnecting_to_another_channel_replaces_the_old_session() {
    let irc = fake_irc(vec![Script { lines: vec![ROOMSTATE.into()], then: Then::Hold }]).await;
    let mut r = rig(&irc.url, "ws://127.0.0.1:1", None, fast()).await;
    connect(&r, "canal").await;
    recv(&mut r.rx, is_state(ConnectionState::Connected)).await;
    connect(&r, "otro_canal").await;
    for _ in 0..100 {
        if irc.received.lock().unwrap().iter().any(|l| l == "JOIN #otro_canal") {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("no entró al canal nuevo");
}

#[tokio::test]
async fn the_channel_owner_gets_followers_live_state_and_viewers() {
    let irc = fake_irc(vec![Script { lines: vec![ROOMSTATE.into()], then: Then::Hold }]).await;
    let (es_url, es_count) = fake_eventsub(true).await;
    let mut r = rig(&irc.url, &es_url, Some("canal"), fast()).await;
    connect(&r, "canal").await;
    let st = recv(&mut r.rx, is_state(ConnectionState::Connected)).await;
    assert!(matches!(&st, SourceMessage::Status(s) if s.detail.as_deref() == Some(DETAIL_FULL)));
    let v = recv(&mut r.rx, |m| matches!(m, SourceMessage::Viewers(_))).await;
    assert_eq!(v, SourceMessage::Viewers(42));
    let f = recv(&mut r.rx, |m| matches!(m, SourceMessage::Event(e) if e.kind == EventType::Follow)).await;
    let SourceMessage::Event(ev) = f else { unreachable!() };
    assert_eq!((ev.user.id.as_str(), ev.user.unique_id.as_str(), ev.user.is_follower), ("tw:555", "nuevofan", true));
    assert_eq!(es_count.load(Ordering::SeqCst), 1);
    // Pidió seguidores y estado del directo con la sesión de EventSub, usando el token del streamer.
    let subs: Vec<_> = r.http_seen.lock().unwrap().iter().filter(|q| q.path == "/helix/eventsub/subscriptions").cloned().collect();
    let kinds: Vec<String> = subs.iter().map(|q| serde_json::from_str::<serde_json::Value>(&q.body).unwrap()["type"].as_str().unwrap().to_string()).collect();
    for want in ["stream.online", "stream.offline", "channel.follow"] {
        assert!(kinds.contains(&want.to_string()), "{want} en {kinds:?}");
    }
    assert!(subs.iter().all(|q| q.header("authorization") == Some("Bearer AT") && q.header("client-id") == Some("cid")));
    let follow = subs.iter().find(|q| q.body.contains("channel.follow")).unwrap();
    assert!(follow.body.contains("\"SESSION1\"") && follow.body.contains("moderator_user_id"));
}

#[tokio::test]
async fn an_offline_channel_waits_for_the_live_but_still_reads_chat() {
    let irc = fake_irc(vec![Script { lines: vec![ROOMSTATE.into(), chat_line("m1", "1", "ana", "hola")], then: Then::Hold }]).await;
    let (es_url, _) = fake_eventsub(false).await;
    let mut r = rig(&irc.url, &es_url, Some("canal"), fast()).await;
    r.live.store(false, Ordering::SeqCst);
    r.viewers.store(0, Ordering::SeqCst);
    connect(&r, "canal").await;
    let got = collect(&mut r.rx, 600).await;
    assert!(got.iter().any(|m| matches!(m, SourceMessage::Status(s) if s.state == ConnectionState::WaitingLive && s.detail.as_deref() == Some(DETAIL_FULL))), "{got:?}");
    assert!(got.iter().any(|m| chat_event(m).is_some()), "el chat se lee aunque no haya directo");
    assert!(!got.iter().any(|m| matches!(m, SourceMessage::Status(s) if s.state == ConnectionState::Connected)));
}

#[tokio::test]
async fn a_session_for_another_account_stays_chat_only() {
    let irc = fake_irc(vec![Script { lines: vec![ROOMSTATE.into()], then: Then::Hold }]).await;
    let (es_url, es_count) = fake_eventsub(true).await;
    let mut r = rig(&irc.url, &es_url, Some("otra_cuenta"), fast()).await;
    connect(&r, "canal").await;
    let st = recv(&mut r.rx, is_state(ConnectionState::Connected)).await;
    assert!(matches!(&st, SourceMessage::Status(s) if s.detail.as_deref() == Some(DETAIL_NOT_OWNER)));
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(es_count.load(Ordering::SeqCst), 0, "sin ser el dueño no se abre EventSub");
    assert!(!r.http_seen.lock().unwrap().iter().any(|q| q.path.starts_with("/helix/")), "ni se consulta Helix");
}

#[tokio::test]
async fn a_revoked_login_degrades_to_chat_only_and_forgets_the_session() {
    let irc = fake_irc(vec![Script { lines: vec![ROOMSTATE.into(), chat_line("m1", "1", "ana", "hola")], then: Then::Hold }]).await;
    let mut r = rig(&irc.url, "ws://127.0.0.1:1", Some("canal"), fast()).await;
    // Twitch rechaza el refresh token guardado.
    let (base, _) = fake_http(Box::new(|_| (400, json!({ "message": "Invalid refresh token" }).to_string()))).await;
    r.auth = TwitchAuth::with_endpoint(r.store.clone(), Arc::new(TestClock::default()), &base, None).unwrap();
    r.auth.set_client_id("cid");
    let helix = Arc::new(Helix::with_base(Arc::clone(&r.auth), &format!("{base}/helix")).unwrap());
    let (tx, rx) = mpsc::channel(64);
    r.rx = rx;
    r.source = TwitchSource::with_options(tx, Arc::clone(&r.auth), helix, Arc::new(TestClock::default()), Endpoints { irc_url: irc.url.clone(), eventsub_url: "ws://127.0.0.1:1".into() }, fast());
    connect(&r, "canal").await;
    let got = collect(&mut r.rx, 600).await;
    assert!(got.iter().any(|m| matches!(m, SourceMessage::Status(s) if s.state == ConnectionState::Connected && s.detail.as_deref() == Some(DETAIL_CHAT))), "{got:?}");
    assert!(got.iter().any(|m| chat_event(m).is_some()));
    assert!(!r.auth.is_logged_in(), "la sesión revocada se olvida");
}

#[tokio::test]
async fn eventsub_follow_mapping_rejects_bad_ids() {
    let ok = eventsub::map_notification("channel.follow", &json!({ "user_id": "7", "user_login": "Zed", "user_name": "" }), "mid", 5).unwrap();
    let Notice::Follow(ev) = ok else { panic!() };
    assert_eq!((ev.user.nickname.as_str(), ev.id.as_str(), ev.ts, ev.platform), ("Zed", "es-mid", 5, Platform::Twitch));
    for bad in [json!({ "user_id": "abc", "user_login": "z" }), json!({ "user_login": "z" }), json!({ "user_id": "1" }), json!({ "user_id": "1", "user_login": "" })] {
        assert!(eventsub::map_notification("channel.follow", &bad, "m", 1).is_none(), "{bad}");
    }
    assert_eq!(eventsub::map_notification("stream.online", &json!({}), "m", 1), Some(Notice::Online));
    assert_eq!(eventsub::map_notification("stream.offline", &json!({}), "m", 1), Some(Notice::Offline));
    assert!(eventsub::map_notification("channel.raid", &json!({}), "m", 1).is_none());
}
