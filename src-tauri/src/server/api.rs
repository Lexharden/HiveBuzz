//! API local para integraciones: dispara reglas desde fuera (scripts, Streamer.bot, mods).
//!
//! - `GET  /api/status`   → `{"ok":true,"version":"…"}`
//! - `GET  /api/triggers` → nombres de llamada definidos en las reglas (disparador «API local»)
//! - `POST /api/trigger`  → `{"name":"mi-accion","vars":{"user":"ana"}}`
//!
//! Se autentica como el resto del servidor (`?token=` o `Authorization: Bearer`). Solo se pueden
//! disparar reglas guardadas por el streamer; no se aceptan acciones arbitrarias en la petición.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

use super::ServerState;
use crate::rules::template::Vars;
use crate::rules::valid_api_name;

const MAX_BODY_BYTES: usize = 16 * 1024;
const MAX_VARS: usize = 30;
const MAX_VAR_VALUE_CHARS: usize = 500;

/// Lo que la API necesita del resto de la app (así el servidor no conoce el motor de reglas).
#[async_trait]
pub trait ApiBackend: Send + Sync {
    /// Nombres de llamada con al menos una regla activa.
    fn names(&self) -> Vec<String>;
    /// Encola las reglas con ese nombre. Devuelve cuántas.
    async fn trigger(&self, name: &str, vars: Vars) -> usize;
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TriggerRequest {
    name: String,
    #[serde(default)]
    vars: HashMap<String, serde_json::Value>,
}

fn error(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "ok": false, "error": msg }))).into_response()
}

/// Variables de la petición: nombres simples y valores de texto cortos. Los números y booleanos se
/// convierten a texto; el resto se rechaza.
fn clean_vars(raw: HashMap<String, serde_json::Value>) -> Result<Vars, String> {
    if raw.len() > MAX_VARS {
        return Err(format!("demasiadas variables (máximo {MAX_VARS})"));
    }
    let mut out = Vars::new();
    for (k, v) in raw {
        if k.is_empty() || k.len() > 40 || !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(format!("nombre de variable inválido: «{k}»"));
        }
        let text = match v {
            serde_json::Value::String(s) => s,
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::Bool(b) => b.to_string(),
            _ => return Err(format!("la variable «{k}» debe ser texto, número o booleano")),
        };
        if text.chars().count() > MAX_VAR_VALUE_CHARS {
            return Err(format!("la variable «{k}» supera {MAX_VAR_VALUE_CHARS} caracteres"));
        }
        out.insert(k.to_ascii_lowercase(), text.chars().filter(|c| !c.is_control() || *c == '\n').collect());
    }
    Ok(out)
}

async fn trigger(State(state): State<ServerState>, Json(req): Json<TriggerRequest>) -> Response {
    let Some(api) = state.api.as_ref() else { return error(StatusCode::SERVICE_UNAVAILABLE, "la API no está disponible") };
    if !valid_api_name(&req.name) {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "nombre inválido (1–64 letras, números, «-» o «_»)");
    }
    let vars = match clean_vars(req.vars) {
        Ok(v) => v,
        Err(e) => return error(StatusCode::UNPROCESSABLE_ENTITY, &e),
    };
    match api.trigger(&req.name, vars).await {
        0 if !api.names().iter().any(|n| n.eq_ignore_ascii_case(&req.name)) => error(StatusCode::NOT_FOUND, "ninguna regla activa usa esa llamada"),
        0 => error(StatusCode::TOO_MANY_REQUESTS, "la cola de acciones está llena"),
        n => (StatusCode::ACCEPTED, Json(json!({ "ok": true, "queued": n }))).into_response(),
    }
}

async fn triggers(State(state): State<ServerState>) -> Response {
    let names = state.api.as_ref().map(|a| a.names()).unwrap_or_default();
    Json(json!({ "triggers": names })).into_response()
}

async fn status() -> Response {
    Json(json!({ "ok": true, "app": "hivebuzz", "version": env!("CARGO_PKG_VERSION") })).into_response()
}

pub fn routes() -> Router<ServerState> {
    Router::new()
        .route("/api/status", get(status))
        .route("/api/triggers", get(triggers))
        .route("/api/trigger", post(trigger))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
}

pub type SharedApi = Arc<dyn ApiBackend>;

#[async_trait]
impl ApiBackend for crate::rules::engine::RuleEngine {
    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .list()
            .into_iter()
            .filter(|r| r.enabled)
            .filter_map(|r| match r.trigger {
                crate::rules::model::Trigger::Api { name } => Some(name),
                _ => None,
            })
            .collect();
        names.sort_unstable();
        names.dedup();
        names
    }

    async fn trigger(&self, name: &str, vars: Vars) -> usize {
        self.fire_api(name, vars).await
    }
}
