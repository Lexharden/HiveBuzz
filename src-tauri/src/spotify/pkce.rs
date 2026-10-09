//! PKCE (RFC 7636): el flujo OAuth de Spotify sin secreto de cliente y sin servidor propio.

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;
use base64::Engine;
use sha2::{Digest, Sha256};

/// Caracteres sin reservar permitidos en el verificador (RFC 7636 §4.1).
const UNRESERVED: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";

/// Texto aleatorio de `len` caracteres sin reservar (43–128 para un verificador válido).
pub fn random_string(len: usize) -> String {
    (0..len).map(|_| char::from(UNRESERVED[rand::random_range(0..UNRESERVED.len())])).collect()
}

/// `base64url(sha256(verifier))` sin relleno.
pub fn challenge(verifier: &str) -> String {
    B64URL.encode(Sha256::digest(verifier.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_rfc_7636_test_vector() {
        assert_eq!(challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn random_strings_are_valid_and_different() {
        let (a, b) = (random_string(64), random_string(64));
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        assert!(a.bytes().all(|c| UNRESERVED.contains(&c)));
    }
}
