//! Auto-actualización con el updater de Tauri desde GitHub Releases.
//!
//! La clave pública con la que se verifican las actualizaciones va en `tauri.conf.json`
//! (`plugins.updater.pubkey`) y la genera el dueño del repositorio con `tauri signer generate`;
//! sin ella no se actualiza nada (una actualización sin verificar no se instala jamás). El
//! repositorio (`usuario/repo`) se elige en Ajustes: no hay nada fijo en el código.

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::error::{AppError, Result};

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

/// La actualización encontrada, a la espera de que el usuario la instale.
#[derive(Default)]
pub struct PendingUpdate(Mutex<Option<Update>>);

/// `usuario/repo` válido de GitHub (solo los caracteres que GitHub admite).
pub fn valid_repo(repo: &str) -> bool {
    let Some((owner, name)) = repo.split_once('/') else { return false };
    let part = |s: &str| (1..=100).contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) && s != "." && s != "..";
    part(owner) && part(name) && !name.contains('/')
}

/// Dirección del `latest.json` que publican los releases de GitHub.
pub fn manifest_url(repo: &str) -> Result<reqwest::Url> {
    if !valid_repo(repo) {
        return Err(AppError::Invalid("el repositorio debe tener la forma usuario/repo".into()));
    }
    reqwest::Url::parse(&format!("https://github.com/{repo}/releases/latest/download/latest.json")).map_err(|e| AppError::Invalid(format!("URL de actualizaciones: {e}")))
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

pub async fn check<R: Runtime>(app: &AppHandle<R>, repo: &str, pending: &PendingUpdate) -> Result<UpdateInfo> {
    let current = app.package_info().version.to_string();
    if repo.trim().is_empty() {
        return Err(AppError::Invalid("indica el repositorio de GitHub de las actualizaciones en Ajustes".into()));
    }
    if !configured_pubkey(app) {
        return Err(AppError::Invalid(
            "esta compilación no tiene clave pública de actualizaciones (plugins.updater.pubkey en tauri.conf.json)".into(),
        ));
    }
    let endpoint = manifest_url(repo.trim())?;
    let updater = app
        .updater_builder()
        .endpoints(vec![endpoint])
        .map_err(|e| AppError::Invalid(format!("actualizaciones: {e}")))?
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
    fn repo_names_are_validated() {
        for ok in ["yafel/hive-buzz", "a/b", "Org_1/repo.name"] {
            assert!(valid_repo(ok), "{ok}");
        }
        for bad in ["", "solo", "a/b/c", "/b", "a/", "a b/c", "a/../b", "../x", "a/b?x=1", "a/b#frag", "https://x/y", "a@b/c", "./x", "x/.."] {
            assert!(!valid_repo(bad), "{bad}");
        }
    }

    #[test]
    fn manifest_url_points_at_the_latest_release_over_https() {
        let u = manifest_url("yafel/hive-buzz").unwrap();
        assert_eq!(u.as_str(), "https://github.com/yafel/hive-buzz/releases/latest/download/latest.json");
        assert!(manifest_url("no valido").is_err());
        assert!(manifest_url("a/b/../../etc").is_err());
    }
}
