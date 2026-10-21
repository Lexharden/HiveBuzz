//! Grupos de espectadores («VIPs», «Amigos»…): listas con nombre de usuarios que una regla o un
//! comando puede exigir o excluir. Se guardan juntos en un único ajuste y el motor lee una
//! copia inmutable (`GroupIndex`), así comprobar si alguien pertenece a un grupo no bloquea nada.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, PoisonError, RwLock};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::db::Db;
use crate::error::{AppError, Result};
use crate::events::{Platform, User};

const KEY_GROUPS: &str = "viewer.groups";
pub const MAX_GROUPS: usize = 50;
pub const MAX_MEMBERS: usize = 500;
const MAX_NAME_CHARS: usize = 40;
const MAX_HANDLE_CHARS: usize = 60;

/// Un usuario del grupo. Sin `platform` vale en cualquier plataforma.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub handle: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<Platform>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewerGroup {
    /// Vacío al crear: el servicio asigna uno.
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub members: Vec<Member>,
}

/// Forma canónica de un @: sin espacios, sin `@` inicial y en minúsculas.
pub fn normalize_handle(raw: &str) -> String {
    raw.trim().trim_start_matches('@').trim().to_lowercase()
}

/// Copia de solo lectura de los grupos, pensada para consultar desde el motor de reglas.
#[derive(Debug, Default)]
pub struct GroupIndex {
    sets: HashMap<String, HashSet<(Option<Platform>, String)>>,
}

impl GroupIndex {
    pub fn new(groups: &[ViewerGroup]) -> Self {
        let sets = groups
            .iter()
            .map(|g| (g.id.clone(), g.members.iter().map(|m| (m.platform, m.handle.clone())).collect()))
            .collect();
        Self { sets }
    }

    /// ¿Está `user` (de `platform`) en el grupo `group_id`? Falla cerrado: un grupo que ya no existe o un
    /// usuario sin @ nunca pertenecen.
    pub fn contains(&self, group_id: &str, user: &User, platform: Platform) -> bool {
        let handle = normalize_handle(&user.unique_id);
        if handle.is_empty() {
            return false;
        }
        self.sets
            .get(group_id)
            .is_some_and(|set| set.contains(&(None, handle.clone())) || set.contains(&(Some(platform), handle)))
    }
}

pub struct ViewerGroups {
    db: Db,
    groups: Mutex<Vec<ViewerGroup>>,
    index: RwLock<Arc<GroupIndex>>,
}

impl ViewerGroups {
    pub fn new(db: Db) -> Self {
        Self { db, groups: Mutex::new(Vec::new()), index: RwLock::new(Arc::new(GroupIndex::default())) }
    }

    /// Carga lo guardado (un valor ilegible se ignora y se empieza sin grupos).
    pub async fn load(&self) -> Result<usize> {
        let mut list: Vec<ViewerGroup> = match self.db.get_setting(KEY_GROUPS).await? {
            Some(raw) => serde_json::from_str(&raw).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "los grupos de espectadores guardados son ilegibles; se empieza sin grupos");
                Vec::new()
            }),
            None => Vec::new(),
        };
        for g in &mut list {
            g.members = clean_members(std::mem::take(&mut g.members));
        }
        let n = list.len();
        self.publish(&list);
        *self.groups.lock().await = list;
        Ok(n)
    }

    /// Copia actual para consultar (barata: solo clona un `Arc`).
    pub fn index(&self) -> Arc<GroupIndex> {
        Arc::clone(&self.index.read().unwrap_or_else(PoisonError::into_inner))
    }

    pub async fn list(&self) -> Vec<ViewerGroup> {
        self.groups.lock().await.clone()
    }

    /// Crea o sustituye un grupo (por `id`; vacío = nuevo). Devuelve el grupo ya normalizado.
    pub async fn save(&self, group: ViewerGroup) -> Result<ViewerGroup> {
        let name: String = group.name.trim().chars().take(MAX_NAME_CHARS).collect();
        if name.is_empty() {
            return Err(AppError::Invalid("el grupo necesita un nombre".into()));
        }
        let members = clean_members(group.members);
        if members.len() > MAX_MEMBERS {
            return Err(AppError::Invalid(format!("un grupo admite como máximo {MAX_MEMBERS} usuarios")));
        }

        let mut list = self.groups.lock().await;
        let existing = list.iter().position(|g| !group.id.is_empty() && g.id == group.id);
        if list.iter().enumerate().any(|(i, g)| Some(i) != existing && g.name.eq_ignore_ascii_case(&name)) {
            return Err(AppError::Invalid(format!("ya hay un grupo llamado «{name}»")));
        }
        let id = if existing.is_some() { group.id } else { uuid::Uuid::new_v4().to_string() };
        let saved = ViewerGroup { id, name, members };
        let mut next = list.clone();
        match existing {
            Some(i) => next[i] = saved.clone(),
            None if next.len() >= MAX_GROUPS => return Err(AppError::Invalid(format!("máximo {MAX_GROUPS} grupos"))),
            None => next.push(saved.clone()),
        }
        self.persist(&next).await?;
        self.publish(&next);
        *list = next;
        Ok(saved)
    }

    /// Borra un grupo. Las reglas que lo usaban dejan de dispararse para todos (fallan cerradas).
    pub async fn delete(&self, id: &str) -> Result<()> {
        let mut list = self.groups.lock().await;
        let next: Vec<ViewerGroup> = list.iter().filter(|g| g.id != id).cloned().collect();
        if next.len() == list.len() {
            return Ok(());
        }
        self.persist(&next).await?;
        self.publish(&next);
        *list = next;
        Ok(())
    }

    async fn persist(&self, list: &[ViewerGroup]) -> Result<()> {
        self.db.set_setting(KEY_GROUPS, &serde_json::to_string(list)?).await
    }

    fn publish(&self, list: &[ViewerGroup]) {
        *self.index.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(GroupIndex::new(list));
    }
}

/// Normaliza, descarta vacíos y repetidos (misma plataforma y @) y recorta nombres absurdos.
fn clean_members(raw: Vec<Member>) -> Vec<Member> {
    let mut seen = HashSet::new();
    raw.into_iter()
        .filter_map(|m| {
            let handle: String = normalize_handle(&m.handle).chars().take(MAX_HANDLE_CHARS).collect();
            (!handle.is_empty() && seen.insert((m.platform, handle.clone()))).then_some(Member { handle, platform: m.platform })
        })
        .collect()
}

#[cfg(test)]
mod tests;
