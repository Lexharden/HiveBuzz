//! Ejecutor `tts`: lee un texto en voz alta.
//!
//! Parámetros: `text` (plantilla con variables), `voice` (`motor:voz`, opcional: si falta se
//! elige por rol / aleatoria por usuario / predeterminada), `rate` 0.5–2.0 y `volume` 0–100.
//! Un TTS a la vez (grupo serial): lo que llega mientras habla espera su turno.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Map, Value};

use super::{opt_number, require_str};
use crate::actions::{ActionContext, ActionExecutor, Concurrency};
use crate::error::{AppError, Result};
use crate::tts::engine::split_voice_id;
use crate::tts::service::{SpeakOptions, TtsService};

pub struct TtsExecutor {
    service: Arc<TtsService>,
}

impl TtsExecutor {
    pub fn new(service: Arc<TtsService>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl ActionExecutor for TtsExecutor {
    fn kind(&self) -> &'static str {
        "tts"
    }

    fn concurrency(&self) -> Concurrency {
        Concurrency::Serial("tts")
    }

    fn validate(&self, params: &Map<String, Value>) -> Result<()> {
        require_str(params, "text")?;
        if let Some(v) = params.get("voice").and_then(Value::as_str).filter(|v| !v.trim().is_empty()) {
            if split_voice_id(v).is_none() {
                return Err(AppError::Invalid("la voz debe tener el formato «motor:voz»".into()));
            }
        }
        opt_number(params, "rate", 0.5, 2.0)?;
        opt_number(params, "volume", 0.0, 100.0)?;
        Ok(())
    }

    async fn execute(&self, ctx: &ActionContext, params: &Map<String, Value>) -> Result<()> {
        let text = ctx.render(require_str(params, "text")?);
        let voice = params.get("voice").and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty());
        let opts = SpeakOptions {
            rate: opt_number(params, "rate", 0.5, 2.0)?,
            volume: opt_number(params, "volume", 0.0, 100.0)?,
        };
        self.service.speak(&text, &ctx.vars, voice, opts).await
    }
}
