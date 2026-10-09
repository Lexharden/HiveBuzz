//! Configuración y estado de Twitch para la UI (Client ID propio opcional, sesión iniciada).

use std::sync::{Arc, PoisonError, RwLock};

use serde::{Deserialize, Serialize};

use super::auth::{Account, LoginState, TwitchAuth};
use super::{default_client_id, effective_client_id, KEY_TWITCH_CONFIG};
use crate::db::Db;
use crate::error::{AppError, Result};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TwitchConfig {
    /// Client ID de una app de Twitch propia (opción avanzada). Vacío = el integrado en HiveBuzz.
    pub client_id: String,
}

impl TwitchConfig {
    pub fn validate(&self) -> Result<()> {
        let id = self.client_id.trim();
        if !id.is_empty() && (id.len() > 64 || !id.chars().all(|c| c.is_ascii_alphanumeric())) {
            return Err(AppError::Invalid("el Client ID de Twitch solo lleva letras y números".into()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TwitchStatus {
    /// Hay un Client ID con el que iniciar sesión (el integrado o uno propio).
    pub login_available: bool,
    pub uses_builtin_app: bool,
    pub logged_in: bool,
    pub login: LoginState,
    /// Cuenta que inició sesión (solo si se pudo validar).
    pub account: Option<Account>,
}

pub struct TwitchService {
    db: Db,
    auth: Arc<TwitchAuth>,
    cfg: RwLock<TwitchConfig>,
}

impl TwitchService {
    pub fn new(db: Db, auth: Arc<TwitchAuth>) -> Arc<Self> {
        Arc::new(Self { db, auth, cfg: RwLock::new(TwitchConfig::default()) })
    }

    pub fn auth(&self) -> &Arc<TwitchAuth> {
        &self.auth
    }

    pub async fn load_config(&self) -> Result<()> {
        let cfg = match self.db.get_setting(KEY_TWITCH_CONFIG).await? {
            Some(json) => serde_json::from_str::<TwitchConfig>(&json).ok().filter(|c| c.validate().is_ok()).unwrap_or_default(),
            None => TwitchConfig::default(),
        };
        self.auth.set_client_id(effective_client_id(&cfg.client_id, default_client_id()));
        *self.cfg.write().unwrap_or_else(PoisonError::into_inner) = cfg;
        Ok(())
    }

    pub fn config(&self) -> TwitchConfig {
        self.cfg.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    pub async fn set_config(&self, cfg: TwitchConfig) -> Result<TwitchConfig> {
        let cfg = TwitchConfig { client_id: cfg.client_id.trim().to_string() };
        cfg.validate()?;
        self.db.set_setting(KEY_TWITCH_CONFIG, &serde_json::to_string(&cfg)?).await?;
        self.auth.set_client_id(effective_client_id(&cfg.client_id, default_client_id()));
        *self.cfg.write().unwrap_or_else(PoisonError::into_inner) = cfg.clone();
        Ok(cfg)
    }

    pub async fn status(&self) -> TwitchStatus {
        let cfg = self.config();
        let effective = effective_client_id(&cfg.client_id, default_client_id()).to_string();
        let logged_in = self.auth.is_logged_in();
        // Validar toca la red: solo si hay sesión, y un fallo no rompe el estado.
        let account = if logged_in && !effective.is_empty() { self.auth.validate().await.ok() } else { None };
        TwitchStatus {
            login_available: !effective.is_empty(),
            uses_builtin_app: cfg.client_id.trim().is_empty() && !default_client_id().is_empty(),
            logged_in: logged_in && self.auth.is_logged_in(),
            login: self.auth.login_state(),
            account,
        }
    }
}
