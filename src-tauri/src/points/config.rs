//! Configuración del sistema de puntos.

use serde::{Deserialize, Serialize};

/// Puntos que se ganan por una acción.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Earn {
    pub points: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PointsConfig {
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Nombre de la moneda en los mensajes («puntos», «abejas»…).
    #[serde(default = "default_currency")]
    pub currency_name: String,
    /// Puntos por cada intervalo que el espectador lleva presente.
    #[serde(default = "default_watch_points")]
    pub watch_points: u64,
    /// Cada cuántos minutos se reparten los puntos por ver.
    #[serde(default = "default_watch_minutes")]
    pub watch_interval_minutes: u64,
    /// Puntos por comentario y tiempo mínimo entre comentarios que puntúan (anti-spam).
    #[serde(default = "default_comment_points")]
    pub comment_points: u64,
    #[serde(default = "default_comment_cooldown")]
    pub comment_cooldown_ms: u64,
    /// Puntos por cada `like_every` likes de un espectador.
    #[serde(default = "default_like_points")]
    pub like_points: u64,
    #[serde(default = "default_like_every")]
    pub like_every: u64,
    #[serde(default = "default_share_points")]
    pub share_points: u64,
    #[serde(default = "default_follow_points")]
    pub follow_points: u64,
    #[serde(default = "default_subscribe_points")]
    pub subscribe_points: u64,
    /// Puntos por cada moneda regalada (admite decimales: 0.5 = un punto cada 2 monedas).
    #[serde(default = "default_per_coin")]
    pub points_per_coin: f64,
    /// Multiplicador para suscriptores (1.0 = sin bonus, 2.0 = el doble).
    #[serde(default = "default_sub_multiplier")]
    pub subscriber_multiplier: f64,
    /// Comando para consultar los puntos propios (con o sin `!`).
    #[serde(default = "default_points_command")]
    pub points_command: String,
    /// Comando para ver el top.
    #[serde(default = "default_top_command")]
    pub top_command: String,
    /// Cuántos puestos muestra el comando del top.
    #[serde(default = "default_top_size")]
    pub top_size: u32,
    /// Contar también los eventos del simulador (solo para probar; por defecto no, para que no
    /// llenen la base de datos de espectadores con usuarios falsos).
    #[serde(default)]
    pub count_simulated: bool,
}

fn yes() -> bool {
    true
}
fn default_currency() -> String {
    "puntos".into()
}
fn default_watch_points() -> u64 {
    5
}
fn default_watch_minutes() -> u64 {
    5
}
fn default_comment_points() -> u64 {
    2
}
fn default_comment_cooldown() -> u64 {
    30_000
}
fn default_like_points() -> u64 {
    1
}
fn default_like_every() -> u64 {
    50
}
fn default_share_points() -> u64 {
    20
}
fn default_follow_points() -> u64 {
    50
}
fn default_subscribe_points() -> u64 {
    500
}
fn default_per_coin() -> f64 {
    1.0
}
fn default_sub_multiplier() -> f64 {
    1.0
}
fn default_points_command() -> String {
    "puntos".into()
}
fn default_top_command() -> String {
    "top".into()
}
fn default_top_size() -> u32 {
    5
}

impl Default for PointsConfig {
    fn default() -> Self {
        // Una sola fuente de verdad para los valores por defecto: los de serde.
        serde_json::from_str("{}").unwrap_or_else(|_| unreachable!("todos los campos tienen valor por defecto"))
    }
}

impl PointsConfig {
    /// Corrige valores fuera de rango (ediciones manuales o importaciones).
    pub fn sanitized(mut self) -> Self {
        self.currency_name = self.currency_name.trim().chars().take(30).collect();
        if self.currency_name.is_empty() {
            self.currency_name = default_currency();
        }
        self.watch_interval_minutes = self.watch_interval_minutes.clamp(1, 120);
        self.like_every = self.like_every.max(1);
        self.points_per_coin = if self.points_per_coin.is_finite() { self.points_per_coin.clamp(0.0, 1000.0) } else { 1.0 };
        self.subscriber_multiplier = if self.subscriber_multiplier.is_finite() { self.subscriber_multiplier.clamp(1.0, 10.0) } else { 1.0 };
        self.points_command = clean_command(&self.points_command, "puntos");
        self.top_command = clean_command(&self.top_command, "top");
        self.top_size = self.top_size.clamp(1, 15);
        self
    }
}

/// Comando sin `!`, en minúsculas y sin espacios; si queda vacío, el valor por defecto.
fn clean_command(raw: &str, default: &str) -> String {
    let c: String = raw
        .trim()
        .trim_start_matches('!')
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if c.is_empty() {
        default.to_string()
    } else {
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_come_from_serde() {
        let c = PointsConfig::default();
        assert!(c.enabled);
        assert_eq!((c.watch_points, c.watch_interval_minutes, c.comment_points), (5, 5, 2));
        assert_eq!(c.points_command, "puntos");
        let partial: PointsConfig = serde_json::from_str(r#"{"commentPoints": 9}"#).expect("parsea");
        assert_eq!((partial.comment_points, partial.like_every), (9, 50));
    }

    #[test]
    fn sanitized_fixes_bad_values() {
        let c = PointsConfig {
            currency_name: "  ".into(),
            watch_interval_minutes: 0,
            like_every: 0,
            points_per_coin: f64::NAN,
            subscriber_multiplier: 0.2,
            points_command: " !PUNTOS ya ".into(),
            top_command: "!".into(),
            top_size: 99,
            ..Default::default()
        }
        .sanitized();
        assert_eq!(c.currency_name, "puntos");
        assert_eq!((c.watch_interval_minutes, c.like_every), (1, 1));
        assert_eq!((c.points_per_coin, c.subscriber_multiplier), (1.0, 1.0));
        assert_eq!(c.points_command, "puntosya");
        assert_eq!((c.top_command.as_str(), c.top_size), ("top", 15));
        assert_eq!(PointsConfig { currency_name: "x".repeat(99), ..Default::default() }.sanitized().currency_name.chars().count(), 30);
    }
}
