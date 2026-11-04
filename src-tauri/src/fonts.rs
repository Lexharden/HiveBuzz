//! Biblioteca local de fuentes para los overlays.
//!
//! Las fuentes se guardan en disco (las propias del usuario o las descargadas de Google Fonts) y el servidor
//! local las sirve en `/fonts/<archivo>` (con token). Así los overlays no dependen de Internet ni de Google
//! cuando están en directo, y no se contacta con terceros desde OBS. La lista se publica a los overlays por el
//! canal retenido `fonts` para que declaren sus `@font-face`.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::{AppError, Result};
use crate::overlay::OverlayHub;

/// Canal del overlay con la lista de fuentes instaladas.
pub const CHANNEL: &str = "fonts";
pub(crate) const INDEX_FILE: &str = "fonts.json";
pub(crate) const MAX_FONT_BYTES: u64 = 5 * 1024 * 1024;
const MAX_FAMILIES: usize = 60;
const MAX_NAME_CHARS: usize = 40;
pub(crate) const FONT_EXT: &[&str] = &["woff2", "woff", "ttf", "otf"];
/// Pesos que se piden a Google Fonts (normal y negrita).
const GOOGLE_WEIGHTS: &str = "400;700";
const GOOGLE_CSS_HOST: &str = "https://fonts.googleapis.com/css2";
const GOOGLE_FILE_HOST: &str = "fonts.gstatic.com";
/// Google solo entrega `woff2` a navegadores modernos.
const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36";

/// Un archivo de fuente (una familia puede tener varios pesos).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FontFace {
    pub family: String,
    pub weight: u16,
    pub file: String,
}

/// Familia instalada, tal como la ve la interfaz.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FontFamily {
    pub family: String,
    pub weights: Vec<u16>,
}

/// ¿Es un nombre de familia aceptable? Solo letras, números, espacios, guiones y guion bajo: así nunca puede
/// colarse CSS (`;`, `:`, comillas, paréntesis…) al usarlo en una hoja de estilos.
pub fn is_font_name(name: &str) -> bool {
    let n = name.trim();
    !n.is_empty()
        && n.chars().count() <= MAX_NAME_CHARS
        && n.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
}

fn slug(family: &str) -> String {
    family.trim().chars().map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect()
}

/// Reconoce los formatos de fuente por su cabecera (no se fía de la extensión).
pub(crate) fn font_ext_from_magic(bytes: &[u8]) -> Option<&'static str> {
    match bytes.get(..4)? {
        b"wOF2" => Some("woff2"),
        b"wOFF" => Some("woff"),
        [0x00, 0x01, 0x00, 0x00] | b"true" => Some("ttf"),
        b"OTTO" => Some("otf"),
        _ => None,
    }
}

pub struct FontLibrary {
    dir: PathBuf,
    faces: Mutex<Vec<FontFace>>,
    hub: OverlayHub,
}

impl FontLibrary {
    /// Abre (y crea si falta) la carpeta y carga el índice. Un índice ilegible se ignora, no impide arrancar.
    pub fn new(dir: PathBuf, hub: OverlayHub) -> Result<Self> {
        std::fs::create_dir_all(&dir)?;
        let faces = read_index(&dir);
        let lib = Self { dir, faces: Mutex::new(faces), hub };
        lib.publish();
        Ok(lib)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn lock(&self) -> MutexGuard<'_, Vec<FontFace>> {
        self.faces.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn list(&self) -> Vec<FontFamily> {
        let mut out: Vec<FontFamily> = Vec::new();
        for f in self.lock().iter() {
            match out.iter_mut().find(|o| o.family == f.family) {
                Some(o) => o.weights.push(f.weight),
                None => out.push(FontFamily { family: f.family.clone(), weights: vec![f.weight] }),
            }
        }
        out.iter_mut().for_each(|f| f.weights.sort_unstable());
        out.sort_by_key(|f| f.family.to_lowercase());
        out
    }

    /// Añade un archivo de fuente propio. `name` es la familia; por defecto, el nombre del archivo.
    pub async fn import(&self, path: &Path, name: Option<String>) -> Result<FontFamily> {
        let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
        if !FONT_EXT.contains(&ext.as_str()) {
            return Err(AppError::Invalid(format!("la fuente debe terminar en .{}", FONT_EXT.join(" o ."))));
        }
        let meta = tokio::fs::metadata(path).await?;
        if !meta.is_file() || meta.len() > MAX_FONT_BYTES {
            return Err(AppError::Invalid("la fuente no es un archivo válido o pesa más de 5 MB".into()));
        }
        let bytes = tokio::fs::read(path).await?;
        let family = match name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()) {
            Some(n) => n,
            None => path.file_stem().and_then(|s| s.to_str()).unwrap_or("").replace(['.', '(', ')'], " ").trim().to_string(),
        };
        self.store(&family, 400, &bytes).await?;
        self.family(&family)
    }

    /// Descarga una familia de Google Fonts (letra normal y negrita, alfabeto latino) y la guarda en disco.
    pub async fn download_google(&self, family: &str) -> Result<FontFamily> {
        let family = family.trim();
        if !is_font_name(family) {
            return Err(AppError::Invalid("nombre de fuente no válido (solo letras, números, espacios y guiones)".into()));
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| AppError::Invalid(format!("cliente HTTP: {e}")))?;
        let encoded = family.replace(' ', "+");
        let css = match fetch_css(&client, &format!("{GOOGLE_CSS_HOST}?family={encoded}:wght@{GOOGLE_WEIGHTS}&display=swap")).await {
            Ok(css) => css,
            // Hay familias sin negrita (p. ej. Pacifico): se reintenta con el peso por defecto.
            Err(_) => fetch_css(&client, &format!("{GOOGLE_CSS_HOST}?family={encoded}&display=swap")).await?,
        };
        let faces = parse_google_css(&css);
        if faces.is_empty() {
            return Err(AppError::Invalid(format!("Google Fonts no devolvió «{family}»")));
        }
        for (weight, url) in faces {
            let res = client.get(&url).send().await.map_err(|e| AppError::Invalid(format!("descarga: {e}")))?;
            if !res.status().is_success() {
                return Err(AppError::Invalid(format!("Google Fonts respondió {}", res.status())));
            }
            let bytes = res.bytes().await.map_err(|e| AppError::Invalid(format!("descarga: {e}")))?;
            self.store(family, weight, &bytes).await?;
        }
        self.family(family)
    }

    /// Quita una familia y sus archivos.
    pub async fn delete(&self, family: &str) -> Result<()> {
        let removed: Vec<FontFace> = {
            let mut faces = self.lock();
            let (gone, keep): (Vec<_>, Vec<_>) = faces.drain(..).partition(|f| f.family == family);
            *faces = keep;
            gone
        };
        for f in &removed {
            // Que el archivo ya no esté no es un error.
            let _ = tokio::fs::remove_file(self.dir.join(&f.file)).await;
        }
        self.save_index().await?;
        self.publish();
        Ok(())
    }

    fn family(&self, family: &str) -> Result<FontFamily> {
        self.list().into_iter().find(|f| f.family == family).ok_or_else(|| AppError::Invalid("la fuente no se guardó".into()))
    }

    async fn store(&self, family: &str, weight: u16, bytes: &[u8]) -> Result<()> {
        if !is_font_name(family) {
            return Err(AppError::Invalid("nombre de fuente no válido (solo letras, números, espacios y guiones)".into()));
        }
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_FONT_BYTES {
            return Err(AppError::Invalid("la fuente pesa más de 5 MB".into()));
        }
        let ext = font_ext_from_magic(bytes).ok_or_else(|| AppError::Invalid("el archivo no es una fuente válida".into()))?;
        let family = family.trim().to_string();
        {
            let faces = self.lock();
            let known = faces.iter().any(|f| f.family == family);
            if !known && faces.iter().map(|f| f.family.as_str()).collect::<std::collections::HashSet<_>>().len() >= MAX_FAMILIES {
                return Err(AppError::Invalid(format!("máximo {MAX_FAMILIES} fuentes: borra alguna")));
            }
        }
        let file = format!("{}-{weight}.{ext}", slug(&family));
        tokio::fs::write(self.dir.join(&file), bytes).await?;
        {
            let mut faces = self.lock();
            // Mismo nombre y peso: se sustituye (si cambió de formato, el archivo viejo se retira).
            if let Some(old) = faces.iter().position(|f| f.family == family && f.weight == weight) {
                let old = faces.remove(old);
                if old.file != file {
                    let _ = std::fs::remove_file(self.dir.join(old.file));
                }
            }
            faces.push(FontFace { family, weight, file });
        }
        self.save_index().await?;
        self.publish();
        Ok(())
    }

    /// Escritura atómica (archivo temporal + renombrado): un corte a mitad no deja el índice a medias.
    async fn save_index(&self) -> Result<()> {
        let json = serde_json::to_string_pretty(&*self.lock())?;
        let tmp = self.dir.join(format!("{INDEX_FILE}.tmp"));
        tokio::fs::write(&tmp, json).await?;
        tokio::fs::rename(&tmp, self.dir.join(INDEX_FILE)).await?;
        Ok(())
    }

    fn publish(&self) {
        let faces: Vec<_> = self.lock().iter().map(|f| json!({ "family": f.family, "weight": f.weight, "file": f.file })).collect();
        self.hub.publish_retained(CHANNEL, json!({ "faces": faces }));
    }
}

/// Fuentes anotadas en el índice de `dir` que siguen siendo válidas: nombre sin CSS, archivo simple y existente.
/// Un índice ilegible o editado a mano no puede apuntar fuera de la carpeta ni romper nada: se ignora lo dudoso.
pub fn read_index(dir: &Path) -> Vec<FontFace> {
    std::fs::read_to_string(dir.join(INDEX_FILE))
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<FontFace>>(&s).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|f| is_font_name(&f.family) && is_plain_file_name(&f.file) && dir.join(&f.file).is_file())
        .collect()
}

/// Añade `faces` al índice de `dir` (cuyos archivos ya están copiados). Si una familia y peso ya existían, mandan las
/// nuevas; el resto de fuentes del equipo se conserva. Escritura atómica (temporal + renombrado).
pub fn merge_into_index(dir: &Path, faces: &[FontFace]) -> Result<()> {
    let mut all: Vec<FontFace> = read_index(dir)
        .into_iter()
        .filter(|old| !faces.iter().any(|n| n.family == old.family && n.weight == old.weight))
        .collect();
    all.extend(faces.iter().filter(|f| dir.join(&f.file).is_file()).cloned());
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!("{INDEX_FILE}.tmp"));
    std::fs::write(&tmp, serde_json::to_string_pretty(&all)?)?;
    std::fs::rename(&tmp, dir.join(INDEX_FILE))?;
    Ok(())
}

pub(crate) fn is_plain_file_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 100 && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) && !name.starts_with('.')
}

async fn fetch_css(client: &reqwest::Client, url: &str) -> Result<String> {
    let res = client
        .get(url)
        .header(reqwest::header::USER_AGENT, BROWSER_UA)
        .send()
        .await
        .map_err(|e| AppError::Invalid(format!("no se pudo contactar con Google Fonts: {e}")))?;
    if !res.status().is_success() {
        return Err(AppError::Invalid(format!("Google Fonts respondió {} (¿existe esa fuente?)", res.status())));
    }
    res.text().await.map_err(|e| AppError::Invalid(format!("respuesta ilegible: {e}")))
}

/// Saca de la hoja de estilos de Google los archivos del alfabeto latino: `(peso, url)`. Solo se aceptan
/// direcciones `https://fonts.gstatic.com/…`; cualquier otra se ignora.
pub fn parse_google_css(css: &str) -> Vec<(u16, String)> {
    let mut out: Vec<(u16, String)> = Vec::new();
    let mut subset = String::new();
    let mut weight: Option<u16> = None;
    for line in css.lines() {
        let l = line.trim();
        if let Some(c) = l.strip_prefix("/*").and_then(|s| s.strip_suffix("*/")) {
            subset = c.trim().to_string();
            weight = None;
        } else if let Some(w) = l.strip_prefix("font-weight:") {
            // «400» o un intervalo «100 900» (fuente variable): se toma el primer valor.
            weight = w.trim().trim_end_matches(';').split_whitespace().next().and_then(|n| n.parse().ok());
        } else if l.starts_with("src:") && subset == "latin" {
            let url = l.split("url(").nth(1).and_then(|r| r.split(')').next()).map(|u| u.trim_matches(['"', '\'']));
            if let (Some(w), Some(u)) = (weight, url) {
                let ok = u.strip_prefix("https://").and_then(|r| r.split('/').next()) == Some(GOOGLE_FILE_HOST) && u.ends_with(".woff2");
                if ok && !out.iter().any(|(ow, _)| *ow == w) {
                    out.push((w, u.to_string()));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests;
