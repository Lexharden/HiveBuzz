//! Preferencias de la aplicación (bandeja, arranque minimizado, idioma de la interfaz).

use std::sync::{PoisonError, RwLock};

use serde::{Deserialize, Serialize};

use crate::db::Db;
use crate::error::{AppError, Result};

pub const KEY_APP_PREFS: &str = "app_prefs";
pub const LANGUAGES: [&str; 2] = ["es", "en"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppPrefs {
    /// Al cerrar la ventana la app se queda en la bandeja del sistema.
    pub close_to_tray: bool,
    /// Si la app arranca con el sistema (`--minimized`), no muestra la ventana.
    pub start_minimized: bool,
    /// Idioma de la interfaz y de los overlays: `es` o `en`.
    pub language: String,
    /// Repositorio de GitHub (`usuario/repo`) del que se descargan las actualizaciones. Vacío = ninguno.
    pub update_repo: String,
    /// Comprobar si hay actualización al abrir la app (solo si hay repositorio).
    pub auto_update_check: bool,
}

impl Default for AppPrefs {
    fn default() -> Self {
        Self { close_to_tray: false, start_minimized: true, language: "es".into(), update_repo: String::new(), auto_update_check: true }
    }
}

impl AppPrefs {
    pub fn validate(&self) -> Result<()> {
        if !LANGUAGES.contains(&self.language.as_str()) {
            return Err(AppError::Invalid(format!("idioma no admitido: «{}»", self.language)));
        }
        if !self.update_repo.is_empty() && !crate::updater::valid_repo(&self.update_repo) {
            return Err(AppError::Invalid("el repositorio de actualizaciones debe tener la forma usuario/repo".into()));
        }
        Ok(())
    }
}

pub struct PrefsService {
    db: Db,
    cur: RwLock<AppPrefs>,
}

impl PrefsService {
    pub fn new(db: Db) -> Self {
        Self { db, cur: RwLock::new(AppPrefs::default()) }
    }

    pub async fn load(&self) -> Result<()> {
        let prefs = match self.db.get_setting(KEY_APP_PREFS).await? {
            Some(json) => serde_json::from_str::<AppPrefs>(&json).ok().filter(|p| p.validate().is_ok()).unwrap_or_default(),
            None => AppPrefs::default(),
        };
        *self.cur.write().unwrap_or_else(PoisonError::into_inner) = prefs;
        Ok(())
    }

    pub fn get(&self) -> AppPrefs {
        self.cur.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    pub async fn set(&self, prefs: AppPrefs) -> Result<AppPrefs> {
        prefs.validate()?;
        self.db.set_setting(KEY_APP_PREFS, &serde_json::to_string(&prefs)?).await?;
        *self.cur.write().unwrap_or_else(PoisonError::into_inner) = prefs.clone();
        Ok(prefs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn defaults_roundtrip_and_validation() {
        let db = Db::open_memory().await.unwrap();
        let svc = PrefsService::new(db.clone());
        svc.load().await.unwrap();
        assert_eq!(svc.get(), AppPrefs::default());
        let saved = svc.set(AppPrefs { close_to_tray: true, start_minimized: false, language: "en".into(), update_repo: "yafel/hive-buzz".into(), auto_update_check: false }).await.unwrap();
        let again = PrefsService::new(db.clone());
        again.load().await.unwrap();
        assert_eq!(again.get(), saved);
        assert!(svc.set(AppPrefs { language: "xx".into(), ..AppPrefs::default() }).await.is_err());
        assert_eq!(svc.get().language, "en", "un valor inválido no cambia nada");
        assert!(svc.set(AppPrefs { update_repo: "no valido".into(), ..AppPrefs::default() }).await.is_err());
    }

    #[tokio::test]
    async fn corrupt_stored_prefs_fall_back_to_defaults() {
        let db = Db::open_memory().await.unwrap();
        db.set_setting(KEY_APP_PREFS, "{no es json").await.unwrap();
        let svc = PrefsService::new(db.clone());
        svc.load().await.unwrap();
        assert_eq!(svc.get(), AppPrefs::default());
        db.set_setting(KEY_APP_PREFS, r#"{"language":"zz"}"#).await.unwrap();
        svc.load().await.unwrap();
        assert_eq!(svc.get().language, "es");
    }
}
