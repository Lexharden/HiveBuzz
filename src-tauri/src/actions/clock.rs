//! Reloj inyectable. En producción es hora Unix + reloj monotónico; en tests sigue al tiempo
//! pausado de Tokio, de modo que `sleep` y la caducidad avanzan juntos.

use std::time::{SystemTime, UNIX_EPOCH};

use tokio::time::Instant;

pub trait Clock: Send + Sync {
    /// Milisegundos desde epoch.
    fn now_ms(&self) -> i64;
}

/// Hora Unix al arrancar + tiempo monotónico transcurrido (inmune a saltos del reloj del sistema).
pub struct AppClock {
    base_ms: i64,
    start: Instant,
}

impl AppClock {
    pub fn new() -> Self {
        let base_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
        Self {
            base_ms,
            start: Instant::now(),
        }
    }

    /// Reloj con una base fija (tests).
    pub fn with_base(base_ms: i64) -> Self {
        Self {
            base_ms,
            start: Instant::now(),
        }
    }
}

impl Default for AppClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for AppClock {
    fn now_ms(&self) -> i64 {
        let elapsed = i64::try_from(self.start.elapsed().as_millis()).unwrap_or(i64::MAX);
        self.base_ms.saturating_add(elapsed)
    }
}
