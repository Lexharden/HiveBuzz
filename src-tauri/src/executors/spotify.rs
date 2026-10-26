//! Ejecutor `spotify`: controla el reproductor desde una regla.
//!
//! Parámetros: `op` (`next`, `previous`, `pause`, `resume`, `volumeSet`, `volumeChange`) y, para los dos
//! de volumen, `value` (0 a 100 para fijar; -100 a 100 para subir o bajar).

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Map, Value};

use super::require_str;
use crate::actions::{ActionContext, ActionExecutor, Concurrency};
use crate::error::Result;
use crate::spotify::service::SongService;
use crate::spotify::ControlOp;

pub struct SpotifyExecutor {
    spotify: Arc<SongService>,
}

impl SpotifyExecutor {
    pub fn new(spotify: Arc<SongService>) -> Self {
        Self { spotify }
    }
}

fn parse(params: &Map<String, Value>) -> Result<ControlOp> {
    #[allow(clippy::cast_possible_truncation)]
    let value = params.get("value").and_then(Value::as_f64).filter(|v| v.is_finite()).map(|v| v.round() as i64);
    ControlOp::parse(require_str(params, "op")?, value)
}

#[async_trait]
impl ActionExecutor for SpotifyExecutor {
    fn kind(&self) -> &'static str {
        "spotify"
    }

    /// Las órdenes llegan al reproductor una a una y en el orden en que se dispararon.
    fn concurrency(&self) -> Concurrency {
        Concurrency::Serial("spotify")
    }

    fn validate(&self, params: &Map<String, Value>) -> Result<()> {
        parse(params).map(|_| ())
    }

    async fn execute(&self, _ctx: &ActionContext, params: &Map<String, Value>) -> Result<()> {
        self.spotify.control(parse(params)?).await
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn p(v: Value) -> Result<ControlOp> {
        parse(v.as_object().expect("objeto"))
    }

    #[test]
    fn every_operation_parses_and_bad_ones_are_rejected() {
        assert_eq!(p(json!({"op": "next"})).expect("next"), ControlOp::Next);
        assert_eq!(p(json!({"op": "pause"})).expect("pause"), ControlOp::Pause);
        assert_eq!(p(json!({"op": "volumeSet", "value": 40})).expect("set"), ControlOp::VolumeSet(40));
        assert_eq!(p(json!({"op": "volumeSet", "value": 400})).expect("set"), ControlOp::VolumeSet(100), "se acota");
        assert_eq!(p(json!({"op": "volumeChange", "value": -15})).expect("change"), ControlOp::VolumeChange(-15));
        assert!(p(json!({"op": "volumeSet"})).is_err(), "falta el valor");
        assert!(p(json!({"op": "bailar"})).is_err());
        assert!(p(json!({})).is_err());
    }
}
