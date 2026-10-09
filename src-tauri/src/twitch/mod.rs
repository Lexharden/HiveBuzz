//! Twitch (solo lectura): chat y suscripciones por IRC anónimo —no hace falta cuenta—, y, si el
//! streamer inicia sesión, seguidores, estado del directo y espectadores por EventSub/Helix.
//!
//! Los ids de usuario de Twitch llevan el prefijo `tw:` para no chocar nunca con los de TikTok.

pub mod auth;
pub mod eventsub;
pub mod helix;
pub mod irc;
pub mod service;
pub mod source;

use std::collections::{HashSet, VecDeque};

use crate::error::{AppError, Result};

pub const KEY_TWITCH_REFRESH: &str = "twitch_refresh_token";
pub const KEY_TWITCH_CONFIG: &str = "twitch_config";
/// Prefijo de los ids de usuario de Twitch.
pub const USER_ID_PREFIX: &str = "tw:";

/// Client ID de la app de Twitch de HiveBuzz, fijado al compilar con `HIVEBUZZ_TWITCH_CLIENT_ID`.
/// Con el flujo de código de dispositivo el Client ID es público (no hay secreto en el cliente).
pub fn default_client_id() -> &'static str {
    option_env!("HIVEBUZZ_TWITCH_CLIENT_ID").unwrap_or("")
}

/// El Client ID que se usa: el propio del usuario (opción avanzada) o, si no hay, el integrado.
pub fn effective_client_id<'a>(configured: &'a str, builtin: &'a str) -> &'a str {
    if configured.trim().is_empty() {
        builtin
    } else {
        configured.trim()
    }
}

/// Acepta `canal`, `#canal`, `@canal` o una URL de `twitch.tv/canal` y devuelve el login en minúsculas.
/// Los logins de Twitch son letras, números y `_`, de 3 a 25 caracteres.
pub fn normalize_channel(raw: &str) -> Result<String> {
    let t = raw.trim();
    let candidate = match t.to_ascii_lowercase().find("twitch.tv/") {
        Some(i) => t[i + "twitch.tv/".len()..].split(['/', '?', '#']).next().unwrap_or(""),
        None => t.trim_start_matches(['#', '@']),
    };
    let c = candidate.to_ascii_lowercase();
    if (3..=25).contains(&c.len()) && c.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
        Ok(c)
    } else {
        Err(AppError::Invalid("canal de Twitch no válido (3 a 25 letras, números o «_»)".into()))
    }
}

/// Ids ya vistos (Twitch reenvía mensajes al reconectar). Acotado: olvida los más antiguos.
pub struct Seen {
    set: HashSet<String>,
    order: VecDeque<String>,
    cap: usize,
}

impl Seen {
    pub fn new(cap: usize) -> Self {
        Self { set: HashSet::new(), order: VecDeque::new(), cap: cap.max(1) }
    }

    /// `true` si es la primera vez que se ve.
    pub fn insert(&mut self, id: &str) -> bool {
        if self.set.contains(id) {
            return false;
        }
        if self.order.len() >= self.cap {
            if let Some(old) = self.order.pop_front() {
                self.set.remove(&old);
            }
        }
        self.set.insert(id.to_string());
        self.order.push_back(id.to_string());
        true
    }
}

#[cfg(test)]
mod tests;
