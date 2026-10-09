use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use serde_json::json;

use super::*;
use crate::secrets::MemoryStore;
use crate::testutil::{fake_http, Handler, Req, TestClock};

fn form_value(body: &str, key: &str) -> Option<String> {
    body.split('&').find_map(|p| p.split_once('=').filter(|(k, _)| *k == key).map(|(_, v)| v.to_string()))
}

async fn auth_with(handler: Handler) -> (Arc<TwitchAuth>, Arc<MemoryStore>, Arc<TestClock>, Arc<std::sync::Mutex<Vec<Req>>>) {
    let (base, seen) = fake_http(handler).await;
    let store = Arc::new(MemoryStore::default());
    let clock = Arc::new(TestClock::default());
    let auth = TwitchAuth::with_endpoint(store.clone(), clock.clone(), &base, Some(Duration::from_millis(10))).unwrap();
    auth.set_client_id("cid123");
    (auth, store, clock, seen)
}

fn device_ok() -> (u16, String) {
    (200, json!({ "device_code": "DEV", "user_code": "ABCDEFGH", "verification_uri": "https://www.twitch.tv/activate?device-code=ABCDEFGH", "expires_in": 1800, "interval": 1 }).to_string())
}

async fn wait_for(auth: &TwitchAuth, want: impl Fn(&LoginState) -> bool) -> LoginState {
    for _ in 0..200 {
        let s = auth.login_state();
        if want(&s) {
            return s;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("el estado no llegó: {:?}", auth.login_state());
}

#[tokio::test]
async fn login_requires_a_client_id() {
    let (auth, _, _, seen) = auth_with(Box::new(|_| device_ok())).await;
    auth.set_client_id("");
    assert_eq!(auth.begin_login().await, Err(AuthError::NoClientId));
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn device_flow_waits_while_pending_then_stores_the_session() {
    let polls = Arc::new(AtomicUsize::new(0));
    let p2 = Arc::clone(&polls);
    let (auth, store, _, seen) = auth_with(Box::new(move |r| {
        if r.path == "/oauth2/device" {
            device_ok()
        } else if p2.fetch_add(1, Ordering::SeqCst) < 2 {
            (400, json!({ "status": 400, "message": "authorization_pending" }).to_string())
        } else {
            (200, json!({ "access_token": "AT", "refresh_token": "RT", "expires_in": 3600 }).to_string())
        }
    }))
    .await;
    let info = auth.begin_login().await.unwrap();
    assert_eq!(info.user_code, "ABCDEFGH");
    assert!(info.verification_uri.starts_with("https://www.twitch.tv/activate"));
    assert!(matches!(auth.login_state(), LoginState::Pending { .. }));
    wait_for(&auth, |s| *s == LoginState::Done).await;
    assert!(auth.is_logged_in());
    assert_eq!(store.get(KEY_TWITCH_REFRESH).unwrap().as_deref(), Some("RT"));
    let reqs = seen.lock().unwrap().clone();
    let first = &reqs[0];
    assert_eq!(first.path, "/oauth2/device");
    assert_eq!(form_value(&first.body, "client_id").as_deref(), Some("cid123"));
    assert_eq!(form_value(&first.body, "scopes").as_deref(), Some("moderator%3Aread%3Afollowers"));
    let poll = reqs.iter().find(|r| r.path == "/oauth2/token").unwrap();
    assert_eq!(form_value(&poll.body, "device_code").as_deref(), Some("DEV"));
    assert_eq!(form_value(&poll.body, "grant_type").as_deref(), Some("urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code"));
    assert!(!first.body.contains("client_secret"));
    assert_eq!(auth.access_token().await.unwrap(), "AT", "el primer token queda en caché");
}

#[tokio::test]
async fn a_rejected_or_expired_code_fails_with_a_reason() {
    let (auth, store, _, _) = auth_with(Box::new(|r| {
        if r.path == "/oauth2/device" {
            device_ok()
        } else {
            (400, json!({ "status": 400, "message": "invalid device code" }).to_string())
        }
    }))
    .await;
    auth.begin_login().await.unwrap();
    let s = wait_for(&auth, |s| matches!(s, LoginState::Failed { .. })).await;
    assert!(matches!(s, LoginState::Failed { ref reason } if reason.contains("invalid device code")));
    assert!(!auth.is_logged_in());
    assert!(store.get(KEY_TWITCH_REFRESH).unwrap().is_none());
}

#[tokio::test]
async fn the_code_expires_with_the_clock() {
    let (auth, _, clock, _) = auth_with(Box::new(|r| {
        if r.path == "/oauth2/device" {
            (200, json!({ "device_code": "D", "user_code": "U", "verification_uri": "https://www.twitch.tv/activate", "expires_in": 60, "interval": 1 }).to_string())
        } else {
            (400, json!({ "message": "authorization_pending" }).to_string())
        }
    }))
    .await;
    auth.begin_login().await.unwrap();
    clock.advance(61_000);
    let s = wait_for(&auth, |s| matches!(s, LoginState::Failed { .. })).await;
    assert!(matches!(s, LoginState::Failed { ref reason } if reason.contains("caducó")));
}

#[tokio::test]
async fn cancelling_or_restarting_stops_the_previous_attempt() {
    let tokens = Arc::new(AtomicUsize::new(0));
    let t2 = Arc::clone(&tokens);
    let (auth, store, _, _) = auth_with(Box::new(move |r| {
        if r.path == "/oauth2/device" {
            device_ok()
        } else {
            t2.fetch_add(1, Ordering::SeqCst);
            (400, json!({ "message": "authorization_pending" }).to_string())
        }
    }))
    .await;
    auth.begin_login().await.unwrap();
    auth.cancel_login();
    assert_eq!(auth.login_state(), LoginState::Idle);
    tokio::time::sleep(Duration::from_millis(80)).await;
    let polled = tokens.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert!(tokens.load(Ordering::SeqCst) <= polled + 1, "el sondeo cancelado se detiene");
    assert!(store.get(KEY_TWITCH_REFRESH).unwrap().is_none());
}

#[tokio::test]
async fn expired_tokens_are_refreshed_and_rotated_refresh_tokens_stored() {
    let n = Arc::new(AtomicUsize::new(0));
    let n2 = Arc::clone(&n);
    let (auth, store, clock, seen) = auth_with(Box::new(move |_| {
        let i = n2.fetch_add(1, Ordering::SeqCst);
        (200, json!({ "access_token": format!("AT{i}"), "refresh_token": format!("RT{i}"), "expires_in": 3600 }).to_string())
    }))
    .await;
    store.set(KEY_TWITCH_REFRESH, "RT-inicial").unwrap();
    assert_eq!(auth.access_token().await.unwrap(), "AT0");
    assert_eq!(auth.access_token().await.unwrap(), "AT0");
    let first = seen.lock().unwrap()[0].clone();
    assert_eq!(form_value(&first.body, "grant_type").as_deref(), Some("refresh_token"));
    assert_eq!(form_value(&first.body, "refresh_token").as_deref(), Some("RT-inicial"));
    assert_eq!(store.get(KEY_TWITCH_REFRESH).unwrap().as_deref(), Some("RT0"));
    clock.advance(3_600_000);
    assert_eq!(auth.access_token().await.unwrap(), "AT1");
}

#[tokio::test]
async fn concurrent_callers_share_one_refresh() {
    let n = Arc::new(AtomicUsize::new(0));
    let n2 = Arc::clone(&n);
    let (auth, store, _, _) = auth_with(Box::new(move |_| {
        n2.fetch_add(1, Ordering::SeqCst);
        (200, json!({ "access_token": "AT", "expires_in": 3600 }).to_string())
    }))
    .await;
    store.set(KEY_TWITCH_REFRESH, "RT").unwrap();
    let results = futures_util::future::join_all((0..5).map(|_| {
        let a = Arc::clone(&auth);
        async move { a.access_token().await }
    }))
    .await;
    assert!(results.iter().all(|r| r.as_deref() == Ok("AT")));
    assert_eq!(n.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_revoked_session_is_forgotten() {
    let (auth, store, _, _) = auth_with(Box::new(|_| (400, json!({ "status": 400, "message": "Invalid refresh token" }).to_string()))).await;
    store.set(KEY_TWITCH_REFRESH, "RT").unwrap();
    assert_eq!(auth.access_token().await, Err(AuthError::NotLoggedIn));
    assert!(!auth.is_logged_in());
    assert_eq!(auth.access_token().await, Err(AuthError::NotLoggedIn), "sin sesión ni se intenta");
}

#[tokio::test]
async fn validate_tells_whose_session_it_is() {
    let (auth, store, _, seen) = auth_with(Box::new(|r| {
        if r.path == "/oauth2/validate" {
            (200, json!({ "client_id": "cid123", "login": "MiCanal", "user_id": "4242", "expires_in": 3000, "scopes": [] }).to_string())
        } else {
            (200, json!({ "access_token": "AT", "expires_in": 3600 }).to_string())
        }
    }))
    .await;
    store.set(KEY_TWITCH_REFRESH, "RT").unwrap();
    assert_eq!(auth.validate().await.unwrap(), Account { login: "micanal".into(), user_id: "4242".into() });
    let v = seen.lock().unwrap().iter().find(|r| r.path == "/oauth2/validate").cloned().unwrap();
    assert_eq!(v.header("authorization"), Some("OAuth AT"));
}

#[tokio::test]
async fn logout_forgets_everything() {
    let (auth, store, _, _) = auth_with(Box::new(|_| (200, json!({ "access_token": "AT", "expires_in": 3600 }).to_string()))).await;
    store.set(KEY_TWITCH_REFRESH, "RT").unwrap();
    auth.access_token().await.unwrap();
    auth.logout().unwrap();
    assert!(!auth.is_logged_in());
    assert_eq!(auth.login_state(), LoginState::Idle);
    assert_eq!(auth.access_token().await, Err(AuthError::NotLoggedIn));
}
