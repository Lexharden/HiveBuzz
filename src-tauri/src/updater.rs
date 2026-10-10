//! Auto-actualización con el updater de Tauri desde GitHub Releases.
//!
//! La clave pública con la que se verifican las actualizaciones va en `tauri.conf.json`
//! (`plugins.updater.pubkey`) y la genera el dueño del repositorio con `tauri signer generate`;
//! sin ella no se actualiza nada (una actualización sin verificar no se instala jamás). Las versiones
//! salen siempre del repositorio oficial ([`REPO`]); se puede instalar la última o elegir cualquier
//! otra de la lista de releases (cada release lleva su propio `latest.json` firmado).

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::error::{AppError, Result};

/// Repositorio oficial de HiveBuzz en GitHub: de aquí salen todas las versiones.
pub const REPO: &str = "Lexharden/HiveBuzz";
/// Página de releases (para abrir en el navegador).
pub const RELEASES_PAGE: &str = "https://github.com/Lexharden/HiveBuzz/releases";

/// Evento de Tauri con el progreso de la descarga (`UpdateProgress`).
pub const EVT_UPDATE_PROGRESS: &str = "update-progress";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub available: bool,
    pub version: Option<String>,
    pub current: String,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
}

/// Una versión publicada en GitHub.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseInfo {
    pub tag: String,
    /// La etiqueta sin la `v` (`0.2.0`).
    pub version: String,
    pub name: String,
    pub notes: String,
    /// Fecha de publicación (ISO 8601), si GitHub la da.
    pub published_at: Option<String>,
    pub prerelease: bool,
    /// Trae `latest.json` firmado: se puede instalar desde la app (si no, solo descargar a mano).
    pub installable: bool,
    pub url: String,
    /// Frente a la versión instalada.
    pub relation: Relation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Relation {
    Newer,
    Current,
    Older,
    /// La etiqueta no es una versión semver.
    Unknown,
}

fn relation(version: &str, current: &str) -> Relation {
    match (semver::Version::parse(version), semver::Version::parse(current)) {
        (Ok(v), Ok(c)) => match v.cmp(&c) {
            std::cmp::Ordering::Greater => Relation::Newer,
            std::cmp::Ordering::Equal => Relation::Current,
            std::cmp::Ordering::Less => Relation::Older,
        },
        _ => Relation::Unknown,
    }
}

#[derive(serde::Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(serde::Deserialize)]
struct GhAsset {
    name: String,
}

/// Convierte la respuesta de `GET /repos/{repo}/releases` (sin borradores, en el orden de GitHub: lo más nuevo primero).
pub fn parse_releases(json: &str, current: &str) -> Result<Vec<ReleaseInfo>> {
    let list: Vec<GhRelease> = serde_json::from_str(json).map_err(|e| AppError::Invalid(format!("respuesta de GitHub inesperada: {e}")))?;
    Ok(list
        .into_iter()
        .filter(|r| !r.draft && valid_tag(&r.tag_name))
        .map(|r| ReleaseInfo {
            relation: relation(r.tag_name.trim_start_matches('v'), current),
            version: r.tag_name.trim_start_matches('v').to_string(),
            name: r.name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| r.tag_name.clone()),
            notes: r.body.unwrap_or_default(),
            published_at: r.published_at,
            prerelease: r.prerelease,
            installable: r.assets.iter().any(|a| a.name == "latest.json"),
            url: r.html_url,
            tag: r.tag_name,
        })
        .collect())
}

/// Versiones publicadas en el repositorio oficial.
pub async fn list_releases(current: &str) -> Result<Vec<ReleaseInfo>> {
    let http = reqwest::Client::builder()
        .user_agent(concat!("HiveBuzz/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| AppError::Invalid(format!("actualizaciones: {e}")))?;
    let res = http
        .get(format!("https://api.github.com/repos/{REPO}/releases?per_page=30"))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| AppError::Invalid(format!("no se pudo consultar GitHub: {e}")))?;
    match res.status().as_u16() {
        200 => {}
        403 | 429 => return Err(AppError::Invalid("GitHub limita las consultas por hora: prueba de nuevo en un rato".into())),
        code => return Err(AppError::Invalid(format!("GitHub respondió {code} al pedir las versiones"))),
    }
    let body = res.text().await.map_err(|e| AppError::Invalid(format!("no se pudo leer la respuesta de GitHub: {e}")))?;
    parse_releases(&body, current)
}

/// Etiqueta de release segura para meterla en una URL (`v0.2.0`, `v1.0.0-beta.1`).
pub fn valid_tag(tag: &str) -> bool {
    (1..=64).contains(&tag.len()) && tag.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+')) && !tag.contains("..")
}

/// La actualización encontrada, a la espera de que el usuario la instale.
#[derive(Default)]
pub struct PendingUpdate(Mutex<Option<Update>>);

/// `latest.json` del último release, o del de una versión concreta.
pub fn manifest_url(tag: Option<&str>) -> Result<reqwest::Url> {
    let url = match tag {
        None => format!("https://github.com/{REPO}/releases/latest/download/latest.json"),
        Some(t) if valid_tag(t) => format!("https://github.com/{REPO}/releases/download/{t}/latest.json"),
        Some(_) => return Err(AppError::Invalid("versión no válida".into())),
    };
    reqwest::Url::parse(&url).map_err(|e| AppError::Invalid(format!("URL de actualizaciones: {e}")))
}

fn configured_pubkey<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.config()
        .plugins
        .0
        .get("updater")
        .and_then(|u| u.get("pubkey"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|k| !k.trim().is_empty())
}

/// Busca la última versión (`tag: None`) o prepara una concreta, aunque sea anterior a la instalada.
pub async fn check<R: Runtime>(app: &AppHandle<R>, tag: Option<&str>, pending: &PendingUpdate) -> Result<UpdateInfo> {
    let current = app.package_info().version.to_string();
    if !configured_pubkey(app) {
        return Err(AppError::Invalid(
            "esta compilación no tiene clave pública de actualizaciones (plugins.updater.pubkey en tauri.conf.json)".into(),
        ));
    }
    let endpoint = manifest_url(tag)?;
    let mut builder = app.updater_builder().endpoints(vec![endpoint]).map_err(|e| AppError::Invalid(format!("actualizaciones: {e}")))?;
    if tag.is_some() {
        // Elegida a mano: vale cualquier versión distinta de la instalada (también volver a una anterior).
        builder = builder.version_comparator(|current, remote| remote.version != current);
    }
    let updater = builder
        .build()
        .map_err(|e| AppError::Invalid(format!("actualizaciones: {e}")))?;
    let found = updater.check().await.map_err(|e| AppError::Invalid(format!("no se pudo comprobar: {e}")))?;
    let info = match &found {
        Some(u) => UpdateInfo { available: true, version: Some(u.version.clone()), current, notes: u.body.clone() },
        None => UpdateInfo { available: false, version: None, current, notes: None },
    };
    *pending.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = found;
    Ok(info)
}

/// Descarga e instala la actualización encontrada con `check` y reinicia la app.
/// `before_install` se espera tras la descarga y justo antes de instalar (guardar el estado).
pub async fn install<R: Runtime>(app: &AppHandle<R>, pending: &PendingUpdate, before_install: impl std::future::Future<Output = ()>) -> Result<()> {
    let update = pending
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
        .ok_or_else(|| AppError::Invalid("no hay ninguna actualización pendiente: comprueba primero".into()))?;
    let mut downloaded = 0u64;
    let handle = app.clone();
    let bytes = update
        .download(
            move |chunk, total| {
                downloaded += chunk as u64;
                let _ = handle.emit(EVT_UPDATE_PROGRESS, UpdateProgress { downloaded, total });
            },
            || {},
        )
        .await
        .map_err(|e| AppError::Invalid(format!("no se pudo descargar la actualización: {e}")))?;
    before_install.await;
    update.install(bytes).map_err(|e| AppError::Invalid(format!("no se pudo instalar la actualización: {e}")))?;
    app.restart();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_validated() {
        for ok in ["v0.2.0", "v1.0.0-beta.1", "0.3.0", "v1.0.0+build.5"] {
            assert!(valid_tag(ok), "{ok}");
        }
        for bad in ["", "v1/../x", "a b", "v1?x=1", "v1#f", "..", "v1/2", &"v".repeat(65)] {
            assert!(!valid_tag(bad), "{bad}");
        }
    }

    #[test]
    fn manifest_urls_point_at_the_official_releases() {
        assert_eq!(manifest_url(None).unwrap().as_str(), "https://github.com/Lexharden/HiveBuzz/releases/latest/download/latest.json");
        assert_eq!(manifest_url(Some("v0.2.0")).unwrap().as_str(), "https://github.com/Lexharden/HiveBuzz/releases/download/v0.2.0/latest.json");
        assert!(manifest_url(Some("../../x")).is_err());
    }

    #[test]
    fn releases_skip_drafts_and_know_which_can_be_installed() {
        let json = r#"[
          {"tag_name":"v0.3.0","name":"","body":"nuevo","draft":true,"prerelease":false,"html_url":"u3","assets":[{"name":"latest.json"}]},
          {"tag_name":"v0.2.0","name":"HiveBuzz v0.2.0","body":"- cosas","draft":false,"prerelease":false,"published_at":"2026-10-01T10:00:00Z","html_url":"u2","assets":[{"name":"HiveBuzz_0.2.0_x64-setup.exe"},{"name":"latest.json"}]},
          {"tag_name":"v0.1.0","name":null,"body":null,"draft":false,"prerelease":true,"html_url":"u1","assets":[]},
          {"tag_name":"bad tag","draft":false,"html_url":"x","assets":[]}
        ]"#;
        let r = parse_releases(json, "0.1.0").unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].tag, "v0.2.0");
        assert_eq!(r[0].version, "0.2.0");
        assert_eq!(r[0].name, "HiveBuzz v0.2.0");
        assert!(r[0].installable && !r[0].prerelease);
        assert_eq!(r[0].published_at.as_deref(), Some("2026-10-01T10:00:00Z"));
        assert_eq!(r[1].name, "v0.1.0", "sin nombre, la etiqueta");
        assert!(!r[1].installable && r[1].prerelease);
        assert_eq!(r[0].relation, Relation::Newer);
        assert_eq!(r[1].relation, Relation::Current);
        assert!(parse_releases("{}", "0.1.0").is_err());
        assert_eq!(relation("0.1.0-beta.1", "0.1.0"), Relation::Older);
        assert_eq!(relation("nightly", "0.1.0"), Relation::Unknown);
    }
}
