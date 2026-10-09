//! Autenticación del servidor local: token + comprobación de `Host` y `Origin`.

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use super::ServerState;

pub async fn guard(State(state): State<ServerState>, req: Request<Body>, next: Next) -> Response {
    let headers = req.headers();
    if !host_allowed(headers) {
        return (StatusCode::FORBIDDEN, "host no permitido").into_response();
    }
    if !origin_allowed(headers) {
        return (StatusCode::FORBIDDEN, "origen no permitido").into_response();
    }
    let bearer = headers.get(header::AUTHORIZATION).and_then(|h| h.to_str().ok()).and_then(|h| h.strip_prefix("Bearer "));
    // La vuelta de Spotify llega desde el navegador sin nuestro token: se autentica con `state` (un solo uso).
    if req.uri().path() == crate::spotify::CALLBACK_PATH {
        return next.run(req).await;
    }
    let supplied = req.uri().query().and_then(|q| query_param(q, "token")).or(bearer);
    match supplied {
        Some(t) if constant_time_eq(t.as_bytes(), state.token.as_bytes()) => next.run(req).await,
        _ => (StatusCode::UNAUTHORIZED, "token inválido o ausente").into_response(),
    }
}

fn query_param<'a>(query: &'a str, name: &str) -> Option<&'a str> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == name).then_some(v)
    })
}

/// Compara sin cortar en el primer byte distinto, para no filtrar el token por tiempos.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Extrae el host (sin puerto ni esquema) de `Host` u `Origin`.
fn bare_host(value: &str) -> &str {
    let no_scheme = value.split_once("://").map_or(value, |(_, rest)| rest);
    let authority = no_scheme.split('/').next().unwrap_or("");
    if let Some(rest) = authority.strip_prefix('[') {
        // IPv6 entre corchetes: [::1]:1234
        return rest.split(']').next().unwrap_or("");
    }
    authority.split(':').next().unwrap_or("")
}

fn is_local(host: &str) -> bool {
    matches!(host.to_ascii_lowercase().as_str(), "127.0.0.1" | "localhost" | "::1")
}

/// Defensa contra DNS rebinding: la petición debe ir dirigida a un nombre local.
pub fn host_allowed(headers: &HeaderMap) -> bool {
    headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|h| is_local(bare_host(h)))
}

/// Si el navegador envía `Origin`, debe ser local (bloquea páginas web de terceros).
/// Sin `Origin` (scripts, Streamer.bot, mods) se acepta: ahí manda el token.
pub fn origin_allowed(headers: &HeaderMap) -> bool {
    match headers.get(header::ORIGIN) {
        None => true,
        Some(o) => o.to_str().ok().is_some_and(|o| is_local(bare_host(o))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, HeaderValue::from_static(v));
        }
        h
    }

    #[test]
    fn constant_time_eq_behaves_like_eq() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn host_must_be_local() {
        assert!(host_allowed(&headers(&[("host", "127.0.0.1:17890")])));
        assert!(host_allowed(&headers(&[("host", "localhost:1")])));
        assert!(host_allowed(&headers(&[("host", "[::1]:80")])));
        assert!(!host_allowed(&headers(&[("host", "evil.com")])));
        assert!(!host_allowed(&headers(&[("host", "127.0.0.1.evil.com:80")])));
        assert!(!host_allowed(&HeaderMap::new()));
    }

    #[test]
    fn origin_must_be_local_when_present() {
        assert!(origin_allowed(&HeaderMap::new()));
        assert!(origin_allowed(&headers(&[("origin", "http://127.0.0.1:17890")])));
        assert!(!origin_allowed(&headers(&[("origin", "https://evil.com")])));
        assert!(!origin_allowed(&headers(&[("origin", "null")])));
        assert!(!origin_allowed(&headers(&[("origin", "http://localhost.evil.com")])));
    }

    #[test]
    fn query_param_parsing() {
        assert_eq!(query_param("token=abc&x=1", "token"), Some("abc"));
        assert_eq!(query_param("x=1&token=abc", "token"), Some("abc"));
        assert_eq!(query_param("x=1", "token"), None);
        assert_eq!(query_param("tokenx=1", "token"), None);
    }
}
