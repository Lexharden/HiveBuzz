//! Ejecutores de acciones. Cada uno implementa `ActionExecutor` y se registra en
//! `ExecutorRegistry`; el núcleo (reglas y cola) no conoce ninguno en concreto.

pub mod alert;
pub mod bot;
pub mod interact;
pub mod internal;
pub mod keys;
pub mod netcmd;
pub mod obs;
pub mod sound;
pub mod tts;
pub mod webhook;

use serde_json::{Map, Value};

use crate::actions::ActionContext;
use crate::error::{AppError, Result};

/// Codifica un texto para ir dentro de una URL (todo lo que no sea sin reservar se escapa).
pub(crate) fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Como `ctx.render`, pero cada variable se codifica para URL (un apodo con `&` o `#` no puede
/// alterar la consulta).
pub(crate) fn render_url(ctx: &ActionContext, template: &str) -> String {
    let encoded = ctx.vars.iter().map(|(k, v)| (k.clone(), percent_encode(v))).collect();
    crate::rules::template::render(template, &encoded)
}

/// Aplica las plantillas a todos los textos de un JSON (valores, no claves).
pub(crate) fn render_json(ctx: &ActionContext, v: &Value) -> Value {
    match v {
        Value::String(s) => Value::String(ctx.render(s)),
        Value::Array(a) => Value::Array(a.iter().map(|x| render_json(ctx, x)).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, x)| (k.clone(), render_json(ctx, x))).collect()),
        other => other.clone(),
    }
}

/// Lee un texto obligatorio de los parámetros.
pub(crate) fn require_str<'a>(params: &'a Map<String, Value>, key: &str) -> Result<&'a str> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::Invalid(format!("falta el parámetro «{key}»")))
}

/// Lee un número opcional dentro de un rango; error si existe y no es válido.
pub(crate) fn opt_number(params: &Map<String, Value>, key: &str, min: f64, max: f64) -> Result<Option<f64>> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => match v.as_f64() {
            Some(n) if (min..=max).contains(&n) => Ok(Some(n)),
            _ => Err(AppError::Invalid(format!("«{key}» debe ser un número entre {min} y {max}"))),
        },
    }
}

pub(crate) fn opt_bool(params: &Map<String, Value>, key: &str, default: bool) -> Result<bool> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(AppError::Invalid(format!("«{key}» debe ser verdadero o falso"))),
    }
}
