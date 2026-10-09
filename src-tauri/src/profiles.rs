//! Perfiles: instantáneas con nombre de las reglas y de la configuración de los overlays
//! («Minecraft», «Charla», «Sleep stream»…). Aplicar un perfil sustituye ambas cosas de un golpe.
//!
//! Un perfil NO incluye metas, timers, puntos, bot ni ruleta: son estado de la comunidad o del
//! canal, no de «cómo reacciono en este tipo de stream».

use std::sync::{Arc, PoisonError, RwLock};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::actions::clock::Clock;
use crate::db::{Db, ProfileRow};
use crate::error::{AppError, Result};
use crate::overlay_config::OverlayConfigService;
use crate::rules::engine::RuleEngine;
use crate::rules::model::Rule;

const MAX_PROFILES: usize = 50;
const MAX_NAME_CHARS: usize = 60;
const KEY_ACTIVE_PROFILE: &str = "active_profile";

/// Contenido de un perfil.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ProfileData {
    pub rules: Vec<Rule>,
    /// Lo que el usuario cambió en cada overlay, por id de overlay.
    pub overlays: Map<String, Value>,
}

/// Para listar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileInfo {
    pub id: String,
    pub name: String,
    pub updated_ms: i64,
    pub rule_count: usize,
    pub overlay_count: usize,
}

pub struct ProfileService {
    db: Db,
    rules: Arc<RuleEngine>,
    overlays: OverlayConfigService,
    clock: Arc<dyn Clock>,
    active: RwLock<Option<String>>,
}

fn clean_name(name: &str) -> Result<String> {
    let n: String = name.chars().filter(|c| !c.is_control()).collect::<String>().trim().chars().take(MAX_NAME_CHARS).collect();
    if n.is_empty() {
        Err(AppError::Invalid("el perfil necesita un nombre".into()))
    } else {
        Ok(n)
    }
}

fn info_of(row: &ProfileRow) -> ProfileInfo {
    let data = serde_json::from_str::<ProfileData>(&row.json).unwrap_or_default();
    ProfileInfo { id: row.id.clone(), name: row.name.clone(), updated_ms: row.updated_ms, rule_count: data.rules.len(), overlay_count: data.overlays.len() }
}

impl ProfileService {
    pub fn new(db: Db, rules: Arc<RuleEngine>, overlays: OverlayConfigService, clock: Arc<dyn Clock>) -> Arc<Self> {
        Arc::new(Self { db, rules, overlays, clock, active: RwLock::new(None) })
    }

    pub async fn load(&self) -> Result<()> {
        let id = self.db.get_setting(KEY_ACTIVE_PROFILE).await?.filter(|s| !s.is_empty());
        *self.active.write().unwrap_or_else(PoisonError::into_inner) = id;
        Ok(())
    }

    /// Perfil aplicado por última vez (si todavía existe y no se ha editado desde entonces es
    /// solo informativo: la UI lo muestra como «último aplicado»).
    pub fn active(&self) -> Option<String> {
        self.active.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    pub async fn list(&self) -> Result<Vec<ProfileInfo>> {
        Ok(self.db.list_profiles().await?.iter().map(info_of).collect())
    }

    /// Guarda el estado actual como perfil. Con `id` sobrescribe ese perfil (y puede renombrarlo);
    /// sin `id` crea uno nuevo.
    pub async fn save_current(&self, id: Option<String>, name: &str) -> Result<ProfileInfo> {
        let name = clean_name(name)?;
        let existing = self.db.list_profiles().await?;
        let id = match id {
            Some(id) => {
                if !existing.iter().any(|p| p.id == id) {
                    return Err(AppError::Invalid("ese perfil no existe".into()));
                }
                id
            }
            None => {
                if existing.len() >= MAX_PROFILES {
                    return Err(AppError::Invalid(format!("máximo {MAX_PROFILES} perfiles")));
                }
                uuid::Uuid::new_v4().to_string()
            }
        };
        if existing.iter().any(|p| p.id != id && p.name.eq_ignore_ascii_case(&name)) {
            return Err(AppError::Invalid(format!("ya hay un perfil llamado «{name}»")));
        }
        let data = ProfileData { rules: self.rules.list(), overlays: self.overlays.stored_all().await? };
        let row = ProfileRow { id, name, updated_ms: self.clock.now_ms(), json: serde_json::to_string(&data)? };
        self.db.save_profile(&row).await?;
        Ok(info_of(&row))
    }

    /// Sustituye las reglas y los overlays actuales por los del perfil.
    pub async fn apply(&self, id: &str) -> Result<()> {
        let row = self.db.get_profile(id).await?.ok_or_else(|| AppError::Invalid("ese perfil no existe".into()))?;
        let data: ProfileData = serde_json::from_str(&row.json).map_err(|e| AppError::Invalid(format!("el perfil está dañado: {e}")))?;
        // Primero lo que puede fallar por validación; si las reglas no valen, los overlays no se tocan.
        self.rules.replace_all(data.rules).await?;
        self.overlays.replace_all_stored(&data.overlays).await?;
        self.db.set_setting(KEY_ACTIVE_PROFILE, id).await?;
        *self.active.write().unwrap_or_else(PoisonError::into_inner) = Some(id.to_string());
        Ok(())
    }

    pub async fn rename(&self, id: &str, name: &str) -> Result<()> {
        let name = clean_name(name)?;
        let mut row = self.db.get_profile(id).await?.ok_or_else(|| AppError::Invalid("ese perfil no existe".into()))?;
        if self.db.list_profiles().await?.iter().any(|p| p.id != id && p.name.eq_ignore_ascii_case(&name)) {
            return Err(AppError::Invalid(format!("ya hay un perfil llamado «{name}»")));
        }
        row.name = name;
        self.db.save_profile(&row).await
    }

    pub async fn delete(&self, id: &str) -> Result<bool> {
        let existed = self.db.delete_profile(id).await?;
        if self.active().as_deref() == Some(id) {
            self.db.set_setting(KEY_ACTIVE_PROFILE, "").await?;
            *self.active.write().unwrap_or_else(PoisonError::into_inner) = None;
        }
        Ok(existed)
    }
}

#[cfg(test)]
mod tests;
