use std::sync::Mutex;

use serde_json::json;
use tokio::net::TcpListener;

use super::*;
use crate::rules::template::Vars;
use crate::secrets::MemoryStore;

fn obj(v: Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap_or_default()
}

fn ctx(pairs: &[(&str, &str)]) -> ActionContext {
    let vars: Vars = pairs.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect();
    ActionContext { rule_id: "r".into(), vars }
}

/// OBS de mentira: habla el protocolo v5 y guarda las peticiones recibidas.
struct FakeObs {
    port: u16,
    requests: Arc<Mutex<Vec<(String, Value)>>>,
}

/// `password`: si existe, exige autenticación. `fail`: tipo de petición que responde con error.
async fn fake_obs(password: Option<&'static str>, fail: Option<&'static str>) -> FakeObs {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&requests);
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else { return };
            let log = Arc::clone(&log);
            tokio::spawn(async move {
                let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else { return };
                let (salt, challenge) = ("c2FsdA==", "Y2hhbGxlbmdl");
                let mut hello = json!({ "obsWebSocketVersion": "5.5.0", "rpcVersion": 1 });
                if password.is_some() {
                    hello["authentication"] = json!({ "salt": salt, "challenge": challenge });
                }
                ws.send(Message::text(json!({ "op": 0, "d": hello }).to_string())).await.unwrap();
                // Identify
                let Some(Ok(Message::Text(t))) = ws.next().await else { return };
                let ident: Value = serde_json::from_str(t.as_str()).unwrap();
                if let Some(pw) = password {
                    if ident["d"]["authentication"].as_str() != Some(auth_string(pw, salt, challenge).as_str()) {
                        let _ = ws
                            .close(Some(tokio_tungstenite::tungstenite::protocol::CloseFrame {
                                code: 4009.into(),
                                reason: "auth".into(),
                            }))
                            .await;
                        return;
                    }
                }
                ws.send(Message::text(json!({ "op": 2, "d": { "negotiatedRpcVersion": 1 } }).to_string())).await.unwrap();
                while let Some(Ok(msg)) = ws.next().await {
                    let Message::Text(t) = msg else { continue };
                    let v: Value = serde_json::from_str(t.as_str()).unwrap();
                    if v["op"] != 6 {
                        continue;
                    }
                    let kind = v["d"]["requestType"].as_str().unwrap().to_string();
                    log.lock().unwrap().push((kind.clone(), v["d"]["requestData"].clone()));
                    let (ok, data) = match kind.as_str() {
                        _ if fail == Some(kind.as_str()) => (false, Value::Null),
                        "GetSceneItemId" => (true, json!({ "sceneItemId": 7 })),
                        "GetVersion" => (true, json!({ "obsVersion": "31.0.0", "obsWebSocketVersion": "5.5.0" })),
                        _ => (true, json!({})),
                    };
                    let reply = json!({ "op": 7, "d": {
                        "requestType": kind, "requestId": v["d"]["requestId"],
                        "requestStatus": { "result": ok, "code": if ok { 100 } else { 600 }, "comment": if ok { Value::Null } else { json!("No existe") } },
                        "responseData": data
                    }});
                    ws.send(Message::text(reply.to_string())).await.unwrap();
                }
            });
        }
    });
    FakeObs { port, requests }
}

use futures_util::{SinkExt, StreamExt};

async fn service(port: u16, password: Option<&str>) -> Arc<ObsService> {
    let db = Db::open_memory().await.unwrap();
    let store = Arc::new(MemoryStore::default());
    let svc = ObsService::new(db, store);
    svc.set_config(ObsConfig { host: "127.0.0.1".into(), port }).await.unwrap();
    if let Some(p) = password {
        svc.set_password(p).unwrap();
    }
    svc
}

#[test]
fn auth_string_is_deterministic_and_depends_on_every_input() {
    let a = auth_string("pw", "salt", "chal");
    assert_eq!(a, auth_string("pw", "salt", "chal"));
    assert_ne!(a, auth_string("pw2", "salt", "chal"));
    assert_ne!(a, auth_string("pw", "salt2", "chal"));
    assert_ne!(a, auth_string("pw", "salt", "chal2"));
    // sha256 en base64 = 44 caracteres.
    assert_eq!(a.len(), 44);
}

#[tokio::test]
async fn validates_actions() {
    let e = ObsExecutor::new(service(1, None).await);
    assert!(e.validate(&obj(json!({"action": "setScene", "scene": "Juego"}))).is_ok());
    assert!(e.validate(&obj(json!({"action": "setScene"}))).is_err());
    assert!(e.validate(&obj(json!({"action": "setSourceVisible", "scene": "A", "source": "B", "durationMs": 5000}))).is_ok());
    assert!(e.validate(&obj(json!({"action": "setScene", "scene": "A", "durationMs": 5000}))).is_err());
    assert!(e.validate(&obj(json!({"action": "startRecording"}))).is_ok());
    assert!(e.validate(&obj(json!({"action": "reboot"}))).is_err());
    assert!(e.validate(&obj(json!({"action": "setFilterEnabled", "source": "Cam", "filter": "Blur", "durationMs": 0}))).is_err());
}

#[test]
fn config_validation() {
    assert!(ObsConfig::default().validate().is_ok());
    assert!(ObsConfig { host: " ".into(), port: 4455 }.validate().is_err());
    assert!(ObsConfig { host: "a b".into(), port: 4455 }.validate().is_err());
    assert!(ObsConfig { host: "x".into(), port: 0 }.validate().is_err());
    assert_eq!(ObsConfig { host: "::1".into(), port: 4455 }.url(), "ws://[::1]:4455");
}

#[tokio::test]
async fn switches_scene_without_password() {
    let obs = fake_obs(None, None).await;
    let svc = service(obs.port, None).await;
    ObsExecutor::new(svc).execute(&ctx(&[]), &obj(json!({"action": "setScene", "scene": "Juego"}))).await.unwrap();
    let reqs = obs.requests.lock().unwrap().clone();
    assert_eq!(reqs, vec![("SetCurrentProgramScene".to_string(), json!({"sceneName": "Juego"}))]);
}

#[tokio::test]
async fn authenticates_with_the_stored_password() {
    let obs = fake_obs(Some("secreto"), None).await;
    let svc = service(obs.port, Some("secreto")).await;
    let info = svc.test().await.unwrap();
    assert_eq!(info, ObsInfo { obs_version: "31.0.0".into(), websocket_version: "5.5.0".into() });
}

#[tokio::test]
async fn wrong_or_missing_password_is_reported() {
    let obs = fake_obs(Some("secreto"), None).await;
    let wrong = service(obs.port, Some("otra")).await;
    let err = wrong.test().await.unwrap_err();
    assert!(err.to_string().contains("contraseña"), "{err}");
    let none = service(obs.port, None).await;
    let err = none.test().await.unwrap_err();
    assert!(err.to_string().contains("contraseña"), "{err}");
}

#[tokio::test]
async fn source_visibility_looks_up_the_scene_item_id() {
    let obs = fake_obs(None, None).await;
    let svc = service(obs.port, None).await;
    let p = obj(json!({"action": "setSourceVisible", "scene": "Juego", "source": "Alerta {user}", "visible": true}));
    ObsExecutor::new(svc).execute(&ctx(&[("user", "ana")]), &p).await.unwrap();
    let reqs = obs.requests.lock().unwrap().clone();
    assert_eq!(reqs[0], ("GetSceneItemId".to_string(), json!({"sceneName": "Juego", "sourceName": "Alerta ana"})));
    assert_eq!(reqs[1], ("SetSceneItemEnabled".to_string(), json!({"sceneName": "Juego", "sceneItemId": 7, "sceneItemEnabled": true})));
}

#[tokio::test]
async fn duration_reverts_the_change() {
    let obs = fake_obs(None, None).await;
    let svc = service(obs.port, None).await;
    let p = obj(json!({"action": "setFilterEnabled", "source": "Cam", "filter": "Blur", "durationMs": 30}));
    ObsExecutor::new(svc).execute(&ctx(&[]), &p).await.unwrap();
    let reqs = obs.requests.lock().unwrap().clone();
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[0].1["filterEnabled"], true);
    assert_eq!(reqs[1].1["filterEnabled"], false);
}

#[tokio::test]
async fn obs_errors_carry_the_reason() {
    let obs = fake_obs(None, Some("SetCurrentProgramScene")).await;
    let svc = service(obs.port, None).await;
    let err = ObsExecutor::new(svc).execute(&ctx(&[]), &obj(json!({"action": "setScene", "scene": "Nada"}))).await.unwrap_err();
    assert!(err.to_string().contains("No existe"), "{err}");
}

#[tokio::test]
async fn recording_requests() {
    let obs = fake_obs(None, None).await;
    let svc = service(obs.port, None).await;
    let e = ObsExecutor::new(svc);
    e.execute(&ctx(&[]), &obj(json!({"action": "startRecording"}))).await.unwrap();
    e.execute(&ctx(&[]), &obj(json!({"action": "stopRecording"}))).await.unwrap();
    let kinds: Vec<String> = obs.requests.lock().unwrap().iter().map(|r| r.0.clone()).collect();
    assert_eq!(kinds, ["StartRecord", "StopRecord"]);
}

#[tokio::test]
async fn closed_port_gives_a_helpful_error() {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    drop(l);
    let svc = service(port, None).await;
    let err = svc.test().await.unwrap_err();
    assert!(err.to_string().contains("OBS"), "{err}");
}

#[tokio::test]
async fn password_goes_to_the_secret_store_and_can_be_cleared() {
    let svc = service(1, None).await;
    assert!(!svc.has_password());
    svc.set_password("abc").unwrap();
    assert!(svc.has_password());
    assert!(svc.set_password("a\nb").is_err());
    svc.set_password("").unwrap();
    assert!(!svc.has_password());
}
