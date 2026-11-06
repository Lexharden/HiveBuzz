//! Autenticación del servidor local: token + comprobación de `Host` y `Origin`.

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderMap, Request, StatusCode, Uri};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use super::ServerState;

pub async fn guard(State(state): State<ServerState>, req: Request<Body>, next: Next) -> Response {
    let headers = req.headers();
    if !request_host_allowed(headers, req.uri()) {
        // Sin la URL (lleva el token): solo el `Host` recibido, acortado, para poder diagnosticar clientes raros.
        let host = request_host(headers, req.uri()).unwrap_or("<sin Host>");
        tracing::warn!(host = %host.chars().take(80).collect::<String>().escape_debug(), "petición rechazada: host no permitido");
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
    let host = host.to_ascii_lowercase();
    // El dominio de los overlays resuelve a 127.0.0.1 (ver `tls::OVERLAY_HOST`); solo ese nombre exacto.
    matches!(host.as_str(), "127.0.0.1" | "localhost" | "::1") || host == super::tls::OVERLAY_HOST
}

/// Destino de la petición. En HTTP/1.x es la cabecera `Host`; en HTTP/2 (que Chrome y LIVE Studio negocian por
/// HTTPS) no hay `Host`, el destino va en `:authority` y llega en la URI.
fn request_host<'a>(headers: &'a HeaderMap, uri: &'a Uri) -> Option<&'a str> {
    headers.get(header::HOST).and_then(|h| h.to_str().ok()).or_else(|| uri.authority().map(|a| a.as_str()))
}

/// Como [`host_allowed`], pero también acepta el destino de HTTP/2 cuando no hay cabecera `Host`.
pub fn request_host_allowed(headers: &HeaderMap, uri: &Uri) -> bool {
    request_host(headers, uri).is_some_and(|h| is_local(bare_host(h)))
}

/// Defensa contra DNS rebinding: la petición debe ir dirigida a un nombre local.
#[cfg(test)]
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
        assert!(host_allowed(&headers(&[("host", "hivebuzz.yafel.dev:17891")])));
        assert!(host_allowed(&headers(&[("host", "HiveBuzz.Yafel.Dev")])));
        assert!(!host_allowed(&headers(&[("host", "evil.hivebuzz.yafel.dev")])));
        assert!(!host_allowed(&headers(&[("host", "hivebuzz.yafel.dev.evil.com")])));
        assert!(!host_allowed(&headers(&[("host", "yafel.dev")])));
        assert!(!host_allowed(&headers(&[("host", "evil.com")])));
        assert!(!host_allowed(&headers(&[("host", "127.0.0.1.evil.com:80")])));
        assert!(!host_allowed(&HeaderMap::new()));
    }

    #[test]
    fn http2_requests_carry_the_destination_in_the_uri_not_in_a_host_header() {
        let none = HeaderMap::new();
        let uri = |s: &str| s.parse::<Uri>().expect("uri");
        assert!(request_host_allowed(&none, &uri("https://hivebuzz.yafel.dev:17891/overlay/alerts")));
        assert!(request_host_allowed(&none, &uri("https://127.0.0.1:17891/x")));
        assert!(!request_host_allowed(&none, &uri("https://evil.com/x")));
        assert!(!request_host_allowed(&none, &uri("/overlay/alerts")), "sin Host ni autoridad");
        // Si hay `Host`, manda él: una autoridad local en la URI no lo compensa.
        assert!(!request_host_allowed(&headers(&[("host", "evil.com")]), &uri("https://hivebuzz.yafel.dev/x")));
        assert!(request_host_allowed(&headers(&[("host", "localhost:1")]), &uri("/x")));
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
