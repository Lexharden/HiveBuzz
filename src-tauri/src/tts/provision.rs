//! Instalación de Piper y de sus voces desde las fuentes oficiales, para que la app sea
//! autosuficiente sin incluir cientos de MB en el instalador.
//!
//! Solo se descargan URLs de un catálogo cerrado (nunca una dada por la UI), solo por HTTPS,
//! y el ejecutable de Piper se verifica contra un SHA-256 fijado en el código.

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::error::{AppError, Result};

pub const PIPER_URL: &str = "https://github.com/rhasspy/piper/releases/download/2023.11.14-2/piper_windows_amd64.zip";
/// SHA-256 del zip de la versión 2023.11.14-2 para Windows (calculado al integrarlo).
pub const PIPER_SHA256: &str = "f3c58906402b24f3a96d92145f58acba6d86c9b5db896d207f78dc80811efcea";

const MAX_EXTRACTED_BYTES: u64 = 400 * 1024 * 1024;
const MAX_ENTRIES: usize = 2_000;
const PROGRESS_STEP: u64 = 256 * 1024;

/// Voz del catálogo de Piper (https://huggingface.co/rhasspy/piper-voices).
pub struct VoiceSpec {
    /// `idioma_PAÍS-nombre-calidad`
    pub id: &'static str,
    pub label: &'static str,
    pub approx_mb: u32,
}

/// Ids y tamaños comprobados contra `voices.json` del repositorio oficial (octubre de 2026).
pub const VOICE_CATALOG: &[VoiceSpec] = &[
    // Español
    VoiceSpec { id: "es_MX-claude-high", label: "Claude · español (México) · alta calidad", approx_mb: 60 },
    VoiceSpec { id: "es_MX-ald-medium", label: "Ald · español (México)", approx_mb: 60 },
    VoiceSpec { id: "es_MX-ald-x_low", label: "Ald · español (México) · ligera", approx_mb: 20 },
    VoiceSpec { id: "es_AR-daniela-high", label: "Daniela · español (Argentina) · alta calidad", approx_mb: 109 },
    VoiceSpec { id: "es_ES-davefx-medium", label: "Davefx · español (España)", approx_mb: 60 },
    VoiceSpec { id: "es_ES-sharvard-medium", label: "Sharvard · español (España)", approx_mb: 73 },
    VoiceSpec { id: "es_ES-mls_10246-low", label: "MLS 10246 · español (España)", approx_mb: 60 },
    VoiceSpec { id: "es_ES-mls_9972-low", label: "MLS 9972 · español (España)", approx_mb: 60 },
    VoiceSpec { id: "es_ES-carlfm-x_low", label: "Carlfm · español (España) · ligera", approx_mb: 27 },
    // Inglés
    VoiceSpec { id: "en_US-lessac-medium", label: "Lessac · inglés (EE. UU.)", approx_mb: 60 },
    VoiceSpec { id: "en_US-lessac-high", label: "Lessac · inglés (EE. UU.) · alta calidad", approx_mb: 109 },
    VoiceSpec { id: "en_US-amy-medium", label: "Amy · inglés (EE. UU.)", approx_mb: 60 },
    VoiceSpec { id: "en_US-ryan-high", label: "Ryan · inglés (EE. UU.) · alta calidad", approx_mb: 115 },
    VoiceSpec { id: "en_US-joe-medium", label: "Joe · inglés (EE. UU.)", approx_mb: 60 },
    VoiceSpec { id: "en_US-john-medium", label: "John · inglés (EE. UU.)", approx_mb: 61 },
    VoiceSpec { id: "en_US-kristin-medium", label: "Kristin · inglés (EE. UU.)", approx_mb: 61 },
    VoiceSpec { id: "en_US-hfc_female-medium", label: "HFC mujer · inglés (EE. UU.)", approx_mb: 60 },
    VoiceSpec { id: "en_US-hfc_male-medium", label: "HFC hombre · inglés (EE. UU.)", approx_mb: 60 },
    VoiceSpec { id: "en_US-norman-medium", label: "Norman · inglés (EE. UU.)", approx_mb: 61 },
    VoiceSpec { id: "en_US-bryce-medium", label: "Bryce · inglés (EE. UU.)", approx_mb: 61 },
    VoiceSpec { id: "en_US-ljspeech-high", label: "LJSpeech · inglés (EE. UU.) · alta calidad", approx_mb: 109 },
    VoiceSpec { id: "en_GB-alan-medium", label: "Alan · inglés (Reino Unido)", approx_mb: 60 },
    VoiceSpec { id: "en_GB-alba-medium", label: "Alba · inglés (Reino Unido)", approx_mb: 60 },
    VoiceSpec { id: "en_GB-cori-high", label: "Cori · inglés (Reino Unido) · alta calidad", approx_mb: 109 },
    VoiceSpec { id: "en_GB-jenny_dioco-medium", label: "Jenny · inglés (Reino Unido)", approx_mb: 60 },
    VoiceSpec { id: "en_GB-northern_english_male-medium", label: "Hombre del norte · inglés (Reino Unido)", approx_mb: 60 },
    // Portugués
    VoiceSpec { id: "pt_BR-faber-medium", label: "Faber · portugués (Brasil)", approx_mb: 60 },
    VoiceSpec { id: "pt_BR-cadu-medium", label: "Cadu · portugués (Brasil)", approx_mb: 60 },
    VoiceSpec { id: "pt_BR-jeff-medium", label: "Jeff · portugués (Brasil)", approx_mb: 60 },
    // Francés
    VoiceSpec { id: "fr_FR-siwis-medium", label: "Siwis · francés (Francia)", approx_mb: 60 },
    VoiceSpec { id: "fr_FR-tom-medium", label: "Tom · francés (Francia)", approx_mb: 61 },
    VoiceSpec { id: "fr_FR-gilles-low", label: "Gilles · francés (Francia)", approx_mb: 60 },
    // Italiano
    VoiceSpec { id: "it_IT-paola-medium", label: "Paola · italiano", approx_mb: 61 },
    VoiceSpec { id: "it_IT-serena-high", label: "Serena · italiano · alta calidad", approx_mb: 109 },
    VoiceSpec { id: "it_IT-riccardo-x_low", label: "Riccardo · italiano · ligera", approx_mb: 27 },
    // Alemán
    VoiceSpec { id: "de_DE-thorsten-high", label: "Thorsten · alemán · alta calidad", approx_mb: 109 },
    VoiceSpec { id: "de_DE-thorsten-medium", label: "Thorsten · alemán", approx_mb: 60 },
    VoiceSpec { id: "de_DE-kerstin-low", label: "Kerstin · alemán", approx_mb: 60 },
    VoiceSpec { id: "de_DE-ramona-low", label: "Ramona · alemán", approx_mb: 60 },
];

/// Tamaño máximo de un modelo de voz importado por el usuario.
const MAX_CUSTOM_MODEL_BYTES: u64 = 1024 * 1024 * 1024;
/// Un `.onnx` de Piper real pesa decenas de MB; algo diminuto no es un modelo.
const MIN_CUSTOM_MODEL_BYTES: u64 = 64 * 1024;
const MAX_VOICE_CONFIG_BYTES: u64 = 2 * 1024 * 1024;

/// URLs del modelo y su configuración. Solo para voces del catálogo.
pub fn voice_urls(id: &str) -> Option<(String, String)> {
    VOICE_CATALOG.iter().find(|v| v.id == id)?;
    let mut parts = id.splitn(3, '-');
    let (locale, name, quality) = (parts.next()?, parts.next()?, parts.next()?);
    let family = locale.split('_').next()?;
    let base = format!("https://huggingface.co/rhasspy/piper-voices/resolve/main/{family}/{locale}/{name}/{quality}/{id}");
    Some((format!("{base}.onnx"), format!("{base}.onnx.json")))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub stage: String,
    pub done: u64,
    pub total: Option<u64>,
}

pub type OnProgress<'a> = &'a (dyn Fn(Progress) + Send + Sync);

// ---- Descarga ------------------------------------------------------------------------------------

fn client(https_only: bool) -> Result<reqwest::Client> {
    let policy = reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= 8 {
            attempt.error("demasiadas redirecciones")
        } else if https_only && attempt.url().scheme() != "https" {
            attempt.error("redirección a un destino sin HTTPS")
        } else {
            attempt.follow()
        }
    });
    reqwest::Client::builder()
        .redirect(policy)
        .connect_timeout(Duration::from_secs(20))
        .read_timeout(Duration::from_secs(45))
        .user_agent(concat!("HiveBuzz/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| AppError::Invalid(format!("no se pudo preparar la descarga: {e}")))
}

/// Descarga `url` a `dest` (primero a `.part`), verificando tamaño y, si se pide, SHA-256.
pub async fn download(url: &str, dest: &Path, sha256: Option<&str>, stage: &str, on: OnProgress<'_>) -> Result<()> {
    download_inner(url, dest, sha256, true, stage, on).await
}

async fn download_inner(url: &str, dest: &Path, sha256: Option<&str>, https_only: bool, stage: &str, on: OnProgress<'_>) -> Result<()> {
    if https_only && !url.starts_with("https://") {
        return Err(AppError::Invalid("solo se admiten descargas por HTTPS".into()));
    }
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let part = dest.with_extension("part");
    let result = fetch_to(&part, url, sha256, https_only, stage, on).await;
    match result {
        Ok(()) => {
            tokio::fs::rename(&part, dest).await?;
            Ok(())
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&part).await;
            Err(e)
        }
    }
}

async fn fetch_to(part: &Path, url: &str, sha256: Option<&str>, https_only: bool, stage: &str, on: OnProgress<'_>) -> Result<()> {
    let resp = client(https_only)?
        .get(url)
        .send()
        .await
        .map_err(|e| AppError::Invalid(format!("no se pudo descargar ({stage}): {e}")))?;
    if !resp.status().is_success() {
        return Err(AppError::Invalid(format!("la descarga de {stage} falló: HTTP {}", resp.status())));
    }
    let total = resp.content_length();
    let mut file = tokio::fs::File::create(part).await?;
    let mut hasher = Sha256::new();
    let (mut done, mut last_report) = (0u64, 0u64);
    let mut stream = resp.bytes_stream();
    on(Progress { stage: stage.into(), done: 0, total });
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| AppError::Invalid(format!("la descarga de {stage} se interrumpió: {e}")))?;
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
        done += chunk.len() as u64;
        if done - last_report >= PROGRESS_STEP {
            last_report = done;
            on(Progress { stage: stage.into(), done, total });
        }
    }
    file.flush().await?;
    drop(file);
    on(Progress { stage: stage.into(), done, total });

    if let Some(expected) = total {
        if expected != done {
            return Err(AppError::Invalid(format!("la descarga de {stage} quedó incompleta ({done} de {expected} bytes)")));
        }
    }
    if done == 0 {
        return Err(AppError::Invalid(format!("la descarga de {stage} llegó vacía")));
    }
    if let Some(expected) = sha256 {
        let actual: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(AppError::Invalid(format!("la descarga de {stage} no coincide con la huella esperada; no se instala")));
        }
    }
    Ok(())
}

// ---- Extracción --------------------------------------------------------------------------------------

/// Extrae un zip a `dest` sin permitir salir de esa carpeta (zip-slip) ni bombas de descompresión.
pub fn extract_zip(zip_path: &Path, dest: &Path) -> Result<()> {
    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| AppError::Invalid(format!("zip no válido: {e}")))?;
    if archive.len() > MAX_ENTRIES {
        return Err(AppError::Invalid("el zip tiene demasiados archivos".into()));
    }
    std::fs::create_dir_all(dest)?;
    let mut written: u64 = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| AppError::Invalid(format!("zip dañado: {e}")))?;
        let rel = entry
            .enclosed_name()
            .ok_or_else(|| {
                AppError::Invalid(format!("ruta peligrosa dentro del zip: {}", String::from_utf8_lossy(entry.name_raw())))
            })?;
        let out = dest.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // El tamaño declarado puede mentir: se cuenta lo realmente escrito.
        let mut limited = std::io::Read::take(&mut entry, MAX_EXTRACTED_BYTES.saturating_sub(written) + 1);
        let mut dst = std::fs::File::create(&out)?;
        written += std::io::copy(&mut limited, &mut dst)?;
        if written > MAX_EXTRACTED_BYTES {
            return Err(AppError::Invalid("el zip descomprime a demasiado tamaño".into()));
        }
    }
    Ok(())
}

// ---- Instalación ---------------------------------------------------------------------------------------

/// Ruta del ejecutable de Piper dentro de la carpeta de TTS de la app.
pub fn piper_exe_path(tts_dir: &Path) -> PathBuf {
    tts_dir.join("piper").join(if cfg!(windows) { "piper.exe" } else { "piper" })
}

/// Descarga e instala Piper en `<tts_dir>/piper/`. Solo automático en Windows.
pub async fn install_piper(tts_dir: &Path, on: OnProgress<'_>) -> Result<PathBuf> {
    if !cfg!(windows) {
        return Err(AppError::Invalid(
            "la instalación automática de Piper solo está disponible en Windows; instálalo a mano y configura su ruta".into(),
        ));
    }
    let zip = tts_dir.join("downloads").join("piper.zip");
    download(PIPER_URL, &zip, Some(PIPER_SHA256), "Piper", on).await?;
    on(Progress { stage: "Extrayendo Piper".into(), done: 0, total: None });

    let (zip_path, dest) = (zip.clone(), tts_dir.to_path_buf());
    tokio::task::spawn_blocking(move || extract_zip(&zip_path, &dest))
        .await
        .map_err(|e| AppError::Invalid(e.to_string()))??;
    let _ = tokio::fs::remove_file(&zip).await;

    let exe = piper_exe_path(tts_dir);
    if !exe.is_file() {
        return Err(AppError::Invalid("el paquete de Piper no contenía el ejecutable esperado".into()));
    }
    Ok(exe)
}

/// Descarga una voz del catálogo (modelo + configuración) en `voices_dir`.
pub async fn install_voice(voices_dir: &Path, id: &str, on: OnProgress<'_>) -> Result<()> {
    let (onnx_url, json_url) = voice_urls(id).ok_or_else(|| AppError::Invalid("esa voz no está en el catálogo".into()))?;
    // Primero la configuración (pequeña): si falla, no se baja el modelo grande.
    let json_path = voices_dir.join(format!("{id}.onnx.json"));
    download(&json_url, &json_path, None, "configuración de la voz", on).await?;
    if serde_json::from_str::<serde_json::Value>(&tokio::fs::read_to_string(&json_path).await?).is_err() {
        let _ = tokio::fs::remove_file(&json_path).await;
        return Err(AppError::Invalid("la configuración de la voz no es válida".into()));
    }
    let onnx_path = voices_dir.join(format!("{id}.onnx"));
    if let Err(e) = download(&onnx_url, &onnx_path, None, id, on).await {
        let _ = tokio::fs::remove_file(&json_path).await;
        return Err(e);
    }
    Ok(())
}

// ---- Voces personalizadas ----------------------------------------------------------------------------

/// Nombre de archivo seguro a partir de lo que diga el usuario: letras, dígitos, `_` y `-`.
pub fn custom_voice_name(raw: &str) -> String {
    let mut out = String::new();
    for c in raw.trim().chars() {
        let c = if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' };
        if !(c == '_' && out.ends_with('_')) {
            out.push(c);
        }
    }
    let out: String = out.trim_matches(['_', '-']).chars().take(64).collect();
    if out.is_empty() {
        "voz".into()
    } else {
        out
    }
}

/// Configuración que acompaña a un modelo: `voz.onnx.json` o, si no, `voz.json`.
fn sibling_config(onnx: &Path) -> Option<PathBuf> {
    let with_json = PathBuf::from(format!("{}.json", onnx.display()));
    if with_json.is_file() {
        return Some(with_json);
    }
    let plain = onnx.with_extension("json");
    plain.is_file().then_some(plain)
}

/// Importa un modelo de Piper propio (`.onnx` + su `.onnx.json`) a la carpeta de voces y devuelve
/// el nombre con el que quedó (sin pisar ninguna voz existente).
pub async fn import_custom_voice(voices_dir: &Path, onnx: &Path, name: Option<&str>) -> Result<String> {
    let bad = |m: &str| AppError::Invalid(m.into());
    let is_onnx = onnx.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("onnx"));
    if !is_onnx || !onnx.is_file() {
        return Err(bad("elige el archivo .onnx del modelo de voz"));
    }
    let size = tokio::fs::metadata(onnx).await?.len();
    if size < MIN_CUSTOM_MODEL_BYTES {
        return Err(bad("el archivo .onnx es demasiado pequeño para ser un modelo de voz"));
    }
    if size > MAX_CUSTOM_MODEL_BYTES {
        return Err(bad("el modelo pesa más de 1 GB; no se importa"));
    }
    let config = sibling_config(onnx).ok_or_else(|| {
        let file = onnx.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        AppError::Invalid(format!("falta la configuración «{file}.json» junto al modelo (Piper la necesita)"))
    })?;
    if tokio::fs::metadata(&config).await?.len() > MAX_VOICE_CONFIG_BYTES {
        return Err(bad("el archivo de configuración de la voz es demasiado grande"));
    }
    let text = tokio::fs::read_to_string(&config).await.map_err(|_| bad("la configuración de la voz no es texto válido"))?;
    let json: serde_json::Value = serde_json::from_str(&text).map_err(|_| bad("la configuración de la voz no es JSON válido"))?;
    if json.pointer("/audio/sample_rate").and_then(serde_json::Value::as_u64).is_none() {
        return Err(bad("la configuración no es de una voz de Piper (falta audio.sample_rate)"));
    }

    tokio::fs::create_dir_all(voices_dir).await?;
    let stem = onnx.file_stem().and_then(|s| s.to_str()).unwrap_or("voz");
    let base = custom_voice_name(name.filter(|n| !n.trim().is_empty()).unwrap_or(stem));
    let mut voice = base.clone();
    let mut n = 2;
    while voices_dir.join(format!("{voice}.onnx")).exists() || voices_dir.join(format!("{voice}.onnx.json")).exists() {
        voice = format!("{base}-{n}");
        n += 1;
    }
    let model_dst = voices_dir.join(format!("{voice}.onnx"));
    let config_dst = voices_dir.join(format!("{voice}.onnx.json"));
    let part = voices_dir.join(format!("{voice}.onnx.part"));

    // El modelo se copia a `.part` y solo se renombra con la configuración ya escrita: una voz a
    // medias nunca aparece en la lista.
    let copied = async {
        tokio::fs::copy(onnx, &part).await?;
        tokio::fs::write(&config_dst, text.as_bytes()).await?;
        tokio::fs::rename(&part, &model_dst).await?;
        Ok::<(), std::io::Error>(())
    }
    .await;
    if let Err(e) = copied {
        let _ = tokio::fs::remove_file(&part).await;
        let _ = tokio::fs::remove_file(&config_dst).await;
        return Err(AppError::Invalid(format!("no se pudo copiar la voz: {e}")));
    }
    Ok(voice)
}

/// Borra una voz de Piper instalada (modelo y configuración).
pub async fn remove_voice(voices_dir: &Path, name: &str) -> Result<()> {
    if !super::piper::safe_voice_name(name) {
        return Err(AppError::Invalid("nombre de voz no válido".into()));
    }
    let model = voices_dir.join(format!("{name}.onnx"));
    if !model.is_file() {
        return Err(AppError::Invalid(format!("la voz «{name}» no está instalada")));
    }
    tokio::fs::remove_file(&model).await?;
    let _ = tokio::fs::remove_file(voices_dir.join(format!("{name}.onnx.json"))).await;
    Ok(())
}

#[cfg(test)]
mod tests;
