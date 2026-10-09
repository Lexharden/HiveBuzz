//! OBS Studio por obs-websocket v5 (incluido en OBS 28+).
//!
//! Acción `obs`, con `action`:
//! - `setScene` (`scene`)
//! - `setSourceVisible` (`scene`, `source`, `visible` = true) — con `durationMs` se revierte solo
//! - `setFilterEnabled` (`source`, `filter`, `enabled` = true) — con `durationMs` se revierte solo
//! - `startRecording` / `stopRecording`
//!
//! Cada acción abre su conexión, se autentica, pide lo suyo y cierra: sin estado que se pueda
//! quedar roto si OBS se reinicia. La contraseña vive en el llavero del sistema; el host y el
//! puerto, en los ajustes.

use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use super::{opt_bool, require_str};
use crate::actions::{ActionContext, ActionExecutor, Concurrency};
use crate::db::Db;
use crate::error::{AppError, Result};
use crate::secrets::SecretStore;

pub const KEY_OBS_CONFIG: &str = "obs_config";
pub const KEY_OBS_PASSWORD: &str = "obs_password";
const OP_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_REVERT_MS: u64 = 600_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ObsConfig {
    pub host: String,
    pub port: u16,
}

impl Default for ObsConfig {
    fn default() -> Self {
        Self { host: "127.0.0.1".into(), port: 4455 }
    }
}

impl ObsConfig {
    pub fn validate(&self) -> Result<()> {
        let h = self.host.trim();
        if h.is_empty() || h.len() > 253 || h.chars().any(|c| c.is_whitespace() || c == '/' || c == '@') {
            return Err(AppError::Invalid("el host de OBS no es válido".into()));
        }
        if self.port == 0 {
            return Err(AppError::Invalid("el puerto de OBS debe estar entre 1 y 65535".into()));
        }
        Ok(())
    }

    fn url(&self) -> String {
        let host = self.host.trim();
        // IPv6 literal sin corchetes → con corchetes.
        if host.contains(':') && !host.starts_with('[') {
            format!("ws://[{host}]:{}", self.port)
        } else {
            format!("ws://{host}:{}", self.port)
        }
    }
}

/// Versiones que informa OBS al probar la conexión.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObsInfo {
    pub obs_version: String,
    pub websocket_version: String,
}

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// `base64(sha256(base64(sha256(password + salt)) + challenge))` (obs-websocket v5).
pub fn auth_string(password: &str, salt: &str, challenge: &str) -> String {
    let secret = B64.encode(Sha256::digest(format!("{password}{salt}").as_bytes()));
    B64.encode(Sha256::digest(format!("{secret}{challenge}").as_bytes()))
}

/// Conexión ya identificada con OBS.
struct Connection {
    ws: Ws,
    next_id: u64,
}

fn bad(msg: impl std::fmt::Display) -> AppError {
    AppError::Invalid(format!("OBS: {msg}"))
}

impl Connection {
    async fn open(url: &str, password: Option<&str>) -> Result<Self> {
        let (mut ws, _) = tokio_tungstenite::connect_async(url).await.map_err(|e| bad(format!("no se pudo conectar ({e}). ¿Está abierto OBS con el servidor WebSocket activado?")))?;
        let hello = Self::read_op(&mut ws, 0).await?;
        let mut identify = json!({ "rpcVersion": 1, "eventSubscriptions": 0 });
        if let Some(auth) = hello["d"].get("authentication").filter(|a| !a.is_null()) {
            let pw = password.ok_or_else(|| bad("OBS pide contraseña y no hay ninguna guardada"))?;
            let (salt, challenge) = (auth["salt"].as_str().unwrap_or(""), auth["challenge"].as_str().unwrap_or(""));
            identify["authentication"] = Value::String(auth_string(pw, salt, challenge));
        }
        ws.send(Message::text(json!({ "op": 1, "d": identify }).to_string())).await.map_err(bad)?;
        Self::read_op(&mut ws, 2).await?;
        Ok(Self { ws, next_id: 0 })
    }

    /// Lee hasta recibir el `op` pedido. Un cierre con código 4009 es contraseña incorrecta.
    async fn read_op(ws: &mut Ws, want: u64) -> Result<Value> {
        loop {
            let msg = ws.next().await.ok_or_else(|| bad("OBS cerró la conexión"))?.map_err(bad)?;
            match msg {
                Message::Text(t) => {
                    let v: Value = serde_json::from_str(t.as_str()).map_err(|e| bad(format!("respuesta ilegible: {e}")))?;
                    if v["op"].as_u64() == Some(want) {
                        return Ok(v);
                    }
                }
                Message::Close(frame) => {
                    let code = frame.as_ref().map(|f| u16::from(f.code));
                    return Err(if code == Some(4009) { bad("contraseña incorrecta") } else { bad("OBS cerró la conexión") });
                }
                _ => {}
            }
        }
    }

    async fn request(&mut self, kind: &str, data: Value) -> Result<Value> {
        self.next_id += 1;
        let id = format!("hb-{}", self.next_id);
        let msg = json!({ "op": 6, "d": { "requestType": kind, "requestId": id, "requestData": data } });
        self.ws.send(Message::text(msg.to_string())).await.map_err(bad)?;
        loop {
            let v = Self::read_op(&mut self.ws, 7).await?;
            let d = &v["d"];
            if d["requestId"].as_str() != Some(id.as_str()) {
                continue;
            }
            if d["requestStatus"]["result"].as_bool() == Some(true) {
                return Ok(d["responseData"].clone());
            }
            let comment = d["requestStatus"]["comment"].as_str().unwrap_or("sin detalle");
            let code = d["requestStatus"]["code"].as_u64().unwrap_or(0);
            return Err(bad(format!("{kind} falló ({code}): {comment}")));
        }
    }

    async fn close(mut self) {
        let _ = self.ws.close(None).await;
    }
}

/// Lo que se le puede pedir a OBS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    SetScene { scene: String },
    SourceVisible { scene: String, source: String, visible: bool },
    FilterEnabled { source: String, filter: String, enabled: bool },
    StartRecording,
    StopRecording,
}

impl Op {
    /// La operación contraria, para `durationMs`. Cambiar de escena o grabar no se revierte.
    fn inverse(&self) -> Option<Op> {
        match self {
            Op::SourceVisible { scene, source, visible } => Some(Op::SourceVisible { scene: scene.clone(), source: source.clone(), visible: !visible }),
            Op::FilterEnabled { source, filter, enabled } => Some(Op::FilterEnabled { source: source.clone(), filter: filter.clone(), enabled: !enabled }),
            _ => None,
        }
    }

    async fn run(&self, c: &mut Connection) -> Result<()> {
        match self {
            Op::SetScene { scene } => {
                c.request("SetCurrentProgramScene", json!({ "sceneName": scene })).await?;
            }
            Op::SourceVisible { scene, source, visible } => {
                let item = c.request("GetSceneItemId", json!({ "sceneName": scene, "sourceName": source })).await?;
                let id = item["sceneItemId"].as_i64().ok_or_else(|| bad("OBS no devolvió el id del elemento"))?;
                c.request("SetSceneItemEnabled", json!({ "sceneName": scene, "sceneItemId": id, "sceneItemEnabled": visible })).await?;
            }
            Op::FilterEnabled { source, filter, enabled } => {
                c.request("SetSourceFilterEnabled", json!({ "sourceName": source, "filterName": filter, "filterEnabled": enabled })).await?;
            }
            Op::StartRecording => {
                c.request("StartRecord", json!({})).await?;
            }
            Op::StopRecording => {
                c.request("StopRecord", json!({})).await?;
            }
        }
        Ok(())
    }
}

/// Conexión con OBS y su configuración.
pub struct ObsService {
    db: Db,
    secrets: Arc<dyn SecretStore>,
    cfg: RwLock<ObsConfig>,
}

impl ObsService {
    pub fn new(db: Db, secrets: Arc<dyn SecretStore>) -> Arc<Self> {
        Arc::new(Self { db, secrets, cfg: RwLock::new(ObsConfig::default()) })
    }

    pub async fn load_config(&self) -> Result<()> {
        let cfg = match self.db.get_setting(KEY_OBS_CONFIG).await? {
            Some(json) => serde_json::from_str::<ObsConfig>(&json).ok().filter(|c| c.validate().is_ok()).unwrap_or_default(),
            None => ObsConfig::default(),
        };
        *self.cfg.write().unwrap_or_else(PoisonError::into_inner) = cfg;
        Ok(())
    }

    pub fn config(&self) -> ObsConfig {
        self.cfg.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    pub async fn set_config(&self, cfg: ObsConfig) -> Result<ObsConfig> {
        let cfg = ObsConfig { host: cfg.host.trim().to_string(), port: cfg.port };
        cfg.validate()?;
        self.db.set_setting(KEY_OBS_CONFIG, &serde_json::to_string(&cfg)?).await?;
        *self.cfg.write().unwrap_or_else(PoisonError::into_inner) = cfg.clone();
        Ok(cfg)
    }

    pub fn has_password(&self) -> bool {
        self.secrets.get(KEY_OBS_PASSWORD).ok().flatten().is_some_and(|p| !p.is_empty())
    }

    /// Guarda la contraseña en el llavero; vacía = borrarla.
    pub fn set_password(&self, password: &str) -> Result<()> {
        if password.is_empty() {
            self.secrets.delete(KEY_OBS_PASSWORD)
        } else if password.len() > 256 || password.chars().any(char::is_control) {
            Err(AppError::Invalid("la contraseña de OBS no es válida".into()))
        } else {
            self.secrets.set(KEY_OBS_PASSWORD, password)
        }
    }

    async fn connect(&self) -> Result<Connection> {
        let url = self.config().url();
        let password = self.secrets.get(KEY_OBS_PASSWORD)?;
        Connection::open(&url, password.as_deref()).await
    }

    /// Comprueba la conexión y la contraseña. Devuelve las versiones de OBS.
    pub async fn test(&self) -> Result<ObsInfo> {
        let url = self.config().url();
        let password = self.secrets.get(KEY_OBS_PASSWORD)?;
        let info = timeout(OP_TIMEOUT, async {
            let mut c = Connection::open(&url, password.as_deref()).await?;
            let v = c.request("GetVersion", json!({})).await;
            c.close().await;
            v
        })
        .await
        .map_err(|_| bad("tiempo agotado"))??;
        Ok(ObsInfo {
            obs_version: info["obsVersion"].as_str().unwrap_or("?").to_string(),
            websocket_version: info["obsWebSocketVersion"].as_str().unwrap_or("?").to_string(),
        })
    }

    /// Conecta, ejecuta la operación y cierra.
    pub async fn apply(&self, op: &Op) -> Result<()> {
        timeout(OP_TIMEOUT, async {
            let mut c = self.connect().await?;
            let r = op.run(&mut c).await;
            c.close().await;
            r
        })
        .await
        .map_err(|_| bad("tiempo agotado"))?
    }
}

pub struct ObsExecutor {
    obs: Arc<ObsService>,
}

impl ObsExecutor {
    pub fn new(obs: Arc<ObsService>) -> Self {
        Self { obs }
    }
}

fn name(ctx: Option<&ActionContext>, params: &Map<String, Value>, key: &str) -> Result<String> {
    let raw = require_str(params, key)?;
    Ok(ctx.map_or_else(|| raw.to_string(), |c| c.render(raw)))
}

fn parse_op(ctx: Option<&ActionContext>, params: &Map<String, Value>) -> Result<Op> {
    let action = require_str(params, "action")?;
    Ok(match action {
        "setScene" => Op::SetScene { scene: name(ctx, params, "scene")? },
        "setSourceVisible" => Op::SourceVisible {
            scene: name(ctx, params, "scene")?,
            source: name(ctx, params, "source")?,
            visible: opt_bool(params, "visible", true)?,
        },
        "setFilterEnabled" => Op::FilterEnabled {
            source: name(ctx, params, "source")?,
            filter: name(ctx, params, "filter")?,
            enabled: opt_bool(params, "enabled", true)?,
        },
        "startRecording" => Op::StartRecording,
        "stopRecording" => Op::StopRecording,
        other => return Err(AppError::Invalid(format!("acción de OBS desconocida: «{other}»"))),
    })
}

fn revert_after(params: &Map<String, Value>) -> Result<Option<Duration>> {
    match params.get("durationMs") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .filter(|n| (1..=MAX_REVERT_MS).contains(n))
            .map(|n| Some(Duration::from_millis(n)))
            .ok_or_else(|| AppError::Invalid(format!("«durationMs» debe estar entre 1 y {MAX_REVERT_MS}"))),
    }
}

#[async_trait]
impl ActionExecutor for ObsExecutor {
    fn kind(&self) -> &'static str {
        "obs"
    }

    fn concurrency(&self) -> Concurrency {
        Concurrency::Serial("obs")
    }

    fn validate(&self, params: &Map<String, Value>) -> Result<()> {
        let op = parse_op(None, params)?;
        let dur = revert_after(params)?;
        if dur.is_some() && op.inverse().is_none() {
            return Err(AppError::Invalid("«durationMs» solo sirve para mostrar/ocultar fuentes y activar filtros".into()));
        }
        Ok(())
    }

    async fn execute(&self, ctx: &ActionContext, params: &Map<String, Value>) -> Result<()> {
        let op = parse_op(Some(ctx), params)?;
        let revert = revert_after(params)?;
        self.obs.apply(&op).await?;
        if let (Some(d), Some(inv)) = (revert, op.inverse()) {
            tokio::time::sleep(d).await;
            self.obs.apply(&inv).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
