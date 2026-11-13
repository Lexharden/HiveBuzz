//! Notificación del sistema cuando se cae una conexión (y cuando vuelve) mientras la ventana está oculta en la
//! bandeja: el streamer no la está mirando y, si no, no se entera hasta que el chat deja de sonar.
//!
//! Solo cuenta lo que no pidió el usuario: perder una conexión que estaba activa. Desconectar a mano o que el
//! LIVE termine (pasa a «esperando el directo») no avisan.

use crate::events::Platform;
use crate::source::protocol::ConnectionState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    /// Se perdió una conexión activa (se está reintentando).
    Lost,
    /// Volvió una conexión que se había perdido.
    Back,
}

/// Recuerda, por plataforma, el estado anterior y si ya se avisó de la caída.
#[derive(Debug, Clone, Copy)]
pub struct Tracker {
    prev: ConnectionState,
    dropped: bool,
}

impl Default for Tracker {
    fn default() -> Self {
        Self { prev: ConnectionState::Disconnected, dropped: false }
    }
}

impl Tracker {
    /// Anota el nuevo estado y devuelve el aviso que corresponde, si alguno.
    pub fn update(&mut self, next: ConnectionState) -> Option<Notice> {
        use ConnectionState::{Connected, Disconnected, Reconnecting, SignatureError};
        let prev = std::mem::replace(&mut self.prev, next);
        match next {
            Reconnecting | SignatureError if prev == Connected && !self.dropped => {
                self.dropped = true;
                Some(Notice::Lost)
            }
            Connected if self.dropped => {
                self.dropped = false;
                Some(Notice::Back)
            }
            // Desconectar a mano cierra el episodio sin avisar.
            Disconnected => {
                self.dropped = false;
                None
            }
            _ => None,
        }
    }
}

fn platform_name(platform: Platform) -> &'static str {
    match platform {
        Platform::Tiktok => "TikTok LIVE",
        Platform::Twitch => "Twitch",
    }
}

/// Título y texto de la notificación en el idioma de la app (el español es el de fábrica).
pub fn text(notice: Notice, platform: Platform, language: &str) -> (String, String) {
    let p = platform_name(platform);
    match (notice, language) {
        (Notice::Lost, "en") => ("HiveBuzz: connection lost".into(), format!("Lost the connection to {p}. HiveBuzz is trying to reconnect.")),
        (Notice::Back, "en") => ("HiveBuzz: connected again".into(), format!("The connection to {p} is back.")),
        (Notice::Lost, _) => ("HiveBuzz: se perdió la conexión".into(), format!("Se perdió la conexión con {p}. HiveBuzz está intentando reconectar.")),
        (Notice::Back, _) => ("HiveBuzz: conexión recuperada".into(), format!("Se recuperó la conexión con {p}.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ConnectionState::*;

    fn run(states: &[ConnectionState]) -> Vec<Option<Notice>> {
        let mut t = Tracker::default();
        states.iter().map(|s| t.update(*s)).collect()
    }

    #[test]
    fn a_drop_and_its_recovery_are_announced_once() {
        assert_eq!(
            run(&[Connected, Reconnecting, Reconnecting, SignatureError, Reconnecting, Connected, Connected]),
            vec![None, Some(Notice::Lost), None, None, None, Some(Notice::Back), None]
        );
    }

    #[test]
    fn manual_disconnects_and_live_ending_are_silent() {
        assert_eq!(run(&[Connected, Disconnected, Connected]), vec![None, None, None]);
        assert_eq!(run(&[Connected, WaitingLive, Connected]), vec![None, None, None]);
        // Reintentos al conectar por primera vez (nunca estuvo conectado): no es una caída.
        assert_eq!(run(&[Reconnecting, Reconnecting, Connected]), vec![None, None, None]);
        // Si tras caerse el usuario desconecta, no queda un «recuperada» pendiente.
        assert_eq!(run(&[Connected, Reconnecting, Disconnected, Connected]), vec![None, Some(Notice::Lost), None, None]);
    }

    #[test]
    fn texts_follow_the_language() {
        assert_eq!(text(Notice::Lost, Platform::Twitch, "en").0, "HiveBuzz: connection lost");
        assert!(text(Notice::Lost, Platform::Tiktok, "es").1.contains("TikTok LIVE"));
        assert_eq!(text(Notice::Back, Platform::Tiktok, "fr").0, "HiveBuzz: conexión recuperada");
    }
}
