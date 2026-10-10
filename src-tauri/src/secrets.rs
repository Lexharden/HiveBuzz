//! Secretos (API key de Euler, token de los overlays, y en el futuro Spotify/TikTok)
//! en el llavero del sistema operativo. Nunca en SQLite.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::error::{AppError, Result};
use crate::source::protocol::SessionPayload;

pub const KEY_EULER_API: &str = "euler_api_key";
pub const KEY_OVERLAY_TOKEN: &str = "overlay_token";

const SERVICE: &str = "com.yafel.hivebuzz";

pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>>;
    fn set(&self, key: &str, value: &str) -> Result<()>;
    fn delete(&self, key: &str) -> Result<()>;
}

/// Llavero real: Credential Manager (Windows), Keychain (macOS), Secret Service (Linux).
pub struct KeyringStore;

impl KeyringStore {
    fn entry(key: &str) -> Result<keyring::Entry> {
        Ok(keyring::Entry::new(SERVICE, key)?)
    }
}

impl SecretStore for KeyringStore {
    fn get(&self, key: &str) -> Result<Option<String>> {
        match Self::entry(key)?.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn set(&self, key: &str, value: &str) -> Result<()> {
        Ok(Self::entry(key)?.set_password(value)?)
    }

    fn delete(&self, key: &str) -> Result<()> {
        match Self::entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

/// Almacén en memoria para tests.
#[derive(Default)]
pub struct MemoryStore(Mutex<HashMap<String, String>>);

impl SecretStore for MemoryStore {
    fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.0.lock().map_or(None, |m| m.get(key).cloned()))
    }

    fn set(&self, key: &str, value: &str) -> Result<()> {
        if let Ok(mut m) = self.0.lock() {
            m.insert(key.to_string(), value.to_string());
        }
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<()> {
        if let Ok(mut m) = self.0.lock() {
            m.remove(key);
        }
        Ok(())
    }
}

pub const KEY_TIKTOK_SESSION_ID: &str = "tiktok_session_id";
pub const KEY_TIKTOK_TT_TARGET_IDC: &str = "tiktok_tt_target_idc";

/// Sesión de TikTok guardada (las dos cookies), si la hay. Una a medias cuenta como «no hay».
pub fn load_tiktok_session(store: &dyn SecretStore) -> Result<Option<SessionPayload>> {
    let (id, idc) = (store.get(KEY_TIKTOK_SESSION_ID)?, store.get(KEY_TIKTOK_TT_TARGET_IDC)?);
    Ok(match (id, idc) {
        (Some(session_id), Some(tt_target_idc)) if !session_id.is_empty() && !tt_target_idc.is_empty() => {
            Some(SessionPayload { session_id, tt_target_idc })
        }
        _ => None,
    })
}

/// Valor de cookie razonable: sin espacios ni caracteres de control, de longitud acotada.
fn valid_cookie_value(v: &str) -> bool {
    !v.is_empty() && v.len() <= 512 && v.chars().all(|c| c.is_ascii_graphic())
}

pub fn save_tiktok_session(store: &dyn SecretStore, session: &SessionPayload) -> Result<()> {
    if !valid_cookie_value(&session.session_id) || !valid_cookie_value(&session.tt_target_idc) {
        return Err(AppError::Invalid("la sesión de TikTok no es válida".into()));
    }
    store.set(KEY_TIKTOK_SESSION_ID, &session.session_id)?;
    store.set(KEY_TIKTOK_TT_TARGET_IDC, &session.tt_target_idc)
}

pub fn clear_tiktok_session(store: &dyn SecretStore) -> Result<()> {
    store.delete(KEY_TIKTOK_SESSION_ID)?;
    store.delete(KEY_TIKTOK_TT_TARGET_IDC)
}

/// Devuelve el token de overlays/API; lo genera (256 bits) la primera vez.
pub fn ensure_overlay_token(store: &dyn SecretStore) -> Result<String> {
    if let Some(t) = store.get(KEY_OVERLAY_TOKEN)? {
        if t.len() >= 32 {
            return Ok(t);
        }
    }
    let token = generate_token();
    store.set(KEY_OVERLAY_TOKEN, &token)?;
    Ok(token)
}

/// Como `ensure_overlay_token`, pero si el llavero no está disponible (p. ej. Linux sin Secret
/// Service) el token se guarda en `fallback` en vez de impedir que la app arranque. El token solo
/// protege el servidor local y ya viaja en las URLs de los overlays, así que un archivo basta.
pub fn overlay_token_with_fallback(store: &dyn SecretStore, fallback: &std::path::Path) -> Result<String> {
    match ensure_overlay_token(store) {
        Ok(t) => Ok(t),
        Err(e) => {
            tracing::warn!(error = %e, "llavero no disponible; el token de los overlays se guarda en un archivo");
            if let Ok(t) = std::fs::read_to_string(fallback) {
                let t = t.trim();
                if t.len() >= 32 && t.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Ok(t.to_string());
                }
            }
            let token = generate_token();
            std::fs::write(fallback, &token)?;
            Ok(token)
        }
    }
}

/// 32 bytes aleatorios en hexadecimal.
pub fn generate_token() -> String {
    let bytes: [u8; 32] = rand::random();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_64_hex_chars_and_random() {
        let a = generate_token();
        let b = generate_token();
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    struct BrokenStore;
    impl SecretStore for BrokenStore {
        fn get(&self, _: &str) -> Result<Option<String>> {
            Err(AppError::Invalid("sin llavero".into()))
        }
        fn set(&self, _: &str, _: &str) -> Result<()> {
            Err(AppError::Invalid("sin llavero".into()))
        }
        fn delete(&self, _: &str) -> Result<()> {
            Err(AppError::Invalid("sin llavero".into()))
        }
    }

    #[test]
    fn without_a_keyring_the_overlay_token_lives_in_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("overlay-token");
        let a = overlay_token_with_fallback(&BrokenStore, &file).unwrap();
        let b = overlay_token_with_fallback(&BrokenStore, &file).unwrap();
        assert_eq!(a, b, "se reutiliza entre arranques");
        assert_eq!(a.len(), 64);
        // Con llavero funcionando, el archivo no se usa.
        let store = MemoryStore::default();
        assert_ne!(overlay_token_with_fallback(&store, &file).unwrap(), a);
    }

    #[test]
    fn overlay_token_is_created_once_and_reused() {
        let store = MemoryStore::default();
        let first = ensure_overlay_token(&store).expect("token");
        assert_eq!(ensure_overlay_token(&store).expect("token"), first);
    }

    #[test]
    fn weak_stored_token_is_replaced() {
        let store = MemoryStore::default();
        store.set(KEY_OVERLAY_TOKEN, "corto").expect("set");
        let t = ensure_overlay_token(&store).expect("token");
        assert_eq!(t.len(), 64);
    }

    fn session(id: &str, idc: &str) -> SessionPayload {
        SessionPayload { session_id: id.into(), tt_target_idc: idc.into() }
    }

    #[test]
    fn tiktok_session_roundtrip_and_clear() {
        let s = MemoryStore::default();
        assert_eq!(load_tiktok_session(&s).expect("load"), None);
        save_tiktok_session(&s, &session("abc123", "useast1a")).expect("save");
        assert_eq!(load_tiktok_session(&s).expect("load"), Some(session("abc123", "useast1a")));
        clear_tiktok_session(&s).expect("clear");
        assert_eq!(load_tiktok_session(&s).expect("load"), None);
    }

    #[test]
    fn a_half_stored_session_counts_as_no_session() {
        let s = MemoryStore::default();
        s.set(KEY_TIKTOK_SESSION_ID, "solo-una").expect("set");
        assert_eq!(load_tiktok_session(&s).expect("load"), None);
        s.set(KEY_TIKTOK_TT_TARGET_IDC, "").expect("set");
        assert_eq!(load_tiktok_session(&s).expect("load"), None);
    }

    #[test]
    fn cookie_values_with_spaces_control_characters_or_excess_length_are_rejected() {
        let s = MemoryStore::default();
        for bad in ["", "con espacio", "salto\nlinea", "tab\t", "ñandú", &"x".repeat(513)] {
            assert!(save_tiktok_session(&s, &session(bad, "idc")).is_err(), "{bad:?}");
            assert!(save_tiktok_session(&s, &session("ok", bad)).is_err(), "{bad:?}");
        }
        assert_eq!(load_tiktok_session(&s).expect("load"), None, "un intento fallido no guarda nada");
    }

    #[test]
    fn memory_store_roundtrip() {
        let s = MemoryStore::default();
        assert_eq!(s.get("k").expect("get"), None);
        s.set("k", "v").expect("set");
        assert_eq!(s.get("k").expect("get").as_deref(), Some("v"));
        s.delete("k").expect("delete");
        assert_eq!(s.get("k").expect("get"), None);
    }
}
