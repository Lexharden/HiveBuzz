//! Buscador de sonidos de MyInstants (myinstants.com): listar, escuchar sin guardar y descargar a la biblioteca.
//!
//! MyInstants no tiene API pública, así que se lee su HTML (búsqueda y «en tendencia»). Solo se admiten
//! archivos de `/media/sounds/` en ese dominio, para que la app no pueda usarse para pedir otras URLs.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use serde::Serialize;

use crate::error::{AppError, Result};

const BASE: &str = "https://www.myinstants.com";
const MAX_BYTES: usize = 10 * 1024 * 1024;
const MAX_PAGE: u32 = 100;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Instant {
    /// Identificador de la página del sonido (`magic-fairy-3490`).
    pub slug: String,
    pub name: String,
    /// Ruta del audio dentro de MyInstants (`/media/sounds/magic-fairy.mp3`).
    pub file: String,
}

fn client() -> Result<&'static reqwest::Client> {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    if let Some(c) = CLIENT.get() {
        return Ok(c);
    }
    let c = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (HiveBuzz)")
        .timeout(Duration::from_secs(20))
        .https_only(true)
        .build()
        .map_err(|e| AppError::Invalid(format!("no se pudo preparar la conexión: {e}")))?;
    Ok(CLIENT.get_or_init(|| c))
}

fn net(e: &reqwest::Error) -> AppError {
    AppError::Invalid(format!("no se pudo contactar con MyInstants: {e}"))
}

/// Sonidos que coinciden con `query`; sin texto, los que están en tendencia. 36 por página.
pub async fn search(query: &str, page: u32) -> Result<Vec<Instant>> {
    let page = page.clamp(1, MAX_PAGE).to_string();
    let query = query.trim();
    let req = if query.is_empty() {
        client()?.get(format!("{BASE}/es/trending/")).query(&[("page", page.as_str())])
    } else {
        client()?.get(format!("{BASE}/es/search/")).query(&[("name", query), ("page", page.as_str())])
    };
    let resp = req.send().await.map_err(|e| net(&e))?;
    if !resp.status().is_success() {
        return Err(AppError::Invalid(format!("MyInstants respondió {}", resp.status())));
    }
    let html = resp.text().await.map_err(|e| net(&e))?;
    Ok(parse(&html))
}

/// Descarga el audio `file` (ruta de MyInstants) a `dir` y devuelve su ruta. Si ya estaba, no vuelve a bajarlo.
pub async fn fetch(file: &str, dir: &Path) -> Result<PathBuf> {
    let name = valid_file(file)?;
    tokio::fs::create_dir_all(dir).await?;
    let dest = dir.join(name);
    if tokio::fs::metadata(&dest).await.is_ok_and(|m| m.len() > 0) {
        return Ok(dest);
    }

    let mut resp = client()?.get(format!("{BASE}{file}")).send().await.map_err(|e| net(&e))?;
    if !resp.status().is_success() {
        return Err(AppError::Invalid(format!("MyInstants respondió {}", resp.status())));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| net(&e))? {
        if bytes.len() + chunk.len() > MAX_BYTES {
            return Err(AppError::Invalid("el sonido es demasiado grande (más de 10 MB)".into()));
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.is_empty() {
        return Err(AppError::Invalid("MyInstants devolvió un archivo vacío".into()));
    }
    // Se escribe con otro nombre y se renombra al final: una descarga cortada no se queda como «ya bajada».
    let partial = dir.join(format!("{name}.part"));
    tokio::fs::write(&partial, &bytes).await?;
    tokio::fs::rename(&partial, &dest).await?;
    Ok(dest)
}

/// Comprueba que `file` es `/media/sounds/<nombre>.mp3` con un nombre sin rarezas y devuelve ese nombre.
fn valid_file(file: &str) -> Result<&str> {
    let name = file.strip_prefix("/media/sounds/").unwrap_or("");
    let ok = !name.is_empty()
        && name.len() <= 200
        && std::path::Path::new(name).extension().is_some_and(|e| e.eq_ignore_ascii_case("mp3"))
        && !name.contains("..")
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '%'));
    if ok {
        Ok(name)
    } else {
        Err(AppError::Invalid("ese sonido no es de MyInstants".into()))
    }
}

/// Saca de la página los sonidos: cada uno es un bloque `class="instant"` con su botón `play('<ruta>', …)`
/// y su enlace `/es/instant/<slug>/` cuyo texto es el nombre.
fn parse(html: &str) -> Vec<Instant> {
    html.split("class=\"instant\"").skip(1).filter_map(parse_block).collect()
}

fn parse_block(block: &str) -> Option<Instant> {
    let file = between(block, "play('", "'")?;
    valid_file(file).ok()?;
    let slug = between(block, "href=\"/es/instant/", "/\"")?;
    let after_link = &block[block.find("class=\"instant-link")?..];
    let name = decode(between(after_link, ">", "</a>")?.trim());
    if name.is_empty() || slug.is_empty() || slug.contains(['/', '"', '<']) {
        return None;
    }
    Some(Instant { slug: slug.to_string(), name, file: file.to_string() })
}

fn between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let from = s.find(start)? + start.len();
    let len = s[from..].find(end)?;
    Some(&s[from..from + len])
}

fn decode(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"
        <div class="instant">
<button class="small-button" onclick="play('/media/sounds/magic-fairy.mp3', 'loader-237321', 'magic-fairy-3490')" type="button"></button>
<a href="/es/instant/magic-fairy-3490/" class="instant-link link-secondary">magic fairy</a>
        </div>
        <div class="instant">
<button class="small-button" onclick="play('/media/sounds/rock-roll.mp3', 'loader-1', 'rock-roll-1')" type="button"></button>
<a href="/es/instant/rock-roll-1/" class="instant-link link-secondary">Rock &amp; Roll &#39;99</a>
        </div>
        <div class="instant">
<button onclick="play('/media/sounds/../../etc.mp3', 'l', 's')"></button>
<a href="/es/instant/malo-1/" class="instant-link">malo</a>
        </div>
    "#;

    #[test]
    fn it_reads_names_and_files_from_the_page_and_skips_unsafe_ones() {
        let found = parse(PAGE);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0], Instant { slug: "magic-fairy-3490".into(), name: "magic fairy".into(), file: "/media/sounds/magic-fairy.mp3".into() });
        assert_eq!(found[1].name, "Rock & Roll '99");
    }

    #[test]
    fn only_mp3_files_under_media_sounds_are_accepted() {
        assert!(valid_file("/media/sounds/a-b_c.mp3").is_ok());
        for bad in ["", "/media/sounds/", "/media/sounds/a.wav", "/media/sounds/../a.mp3", "/otra/a.mp3", "/media/sounds/a/b.mp3", "https://evil.com/a.mp3", "/media/sounds/a b.mp3"] {
            assert!(valid_file(bad).is_err(), "{bad}");
        }
    }
}
