//! Configuración de los overlays: se guarda en SQLite, se valida contra el esquema y se
//! publica (retenida) en `config:<id>` para que el overlay —y la vista previa del editor— la
//! apliquen al instante.

pub mod schema;

use serde_json::{Map, Value};

use crate::db::Db;
use crate::error::{AppError, Result};
use crate::overlay::OverlayHub;
use schema::{alert_screen_id, find, registry, OverlayDef, DEFAULT_ALERT_SCREENS, MAX_ALERT_SCREENS};

/// Ajuste con cuántas pantallas de alertas hay creadas.
const SCREENS_KEY: &str = "alert_screens";

/// Canal retenido con la configuración de un overlay.
pub fn channel(id: &str) -> String {
    format!("config:{id}")
}

#[derive(Clone)]
pub struct OverlayConfigService {
    db: Db,
    hub: OverlayHub,
}

impl OverlayConfigService {
    pub fn new(db: Db, hub: OverlayHub) -> Self {
        Self { db, hub }
    }

    pub fn defs(&self) -> Vec<OverlayDef> {
        registry()
    }

    fn def(id: &str) -> Result<OverlayDef> {
        find(id).ok_or_else(|| AppError::Invalid(format!("overlay desconocido: «{id}»")))
    }

    /// Configuración completa (valores por defecto + lo que el usuario cambió).
    pub async fn get(&self, id: &str) -> Result<Map<String, Value>> {
        let def = Self::def(id)?;
        Ok(def.merged(&self.stored(id).await?))
    }

    /// Aplica un parche validado y lo publica. Si algo del parche es inválido no se aplica nada.
    pub async fn set(&self, id: &str, patch: &Map<String, Value>) -> Result<Map<String, Value>> {
        let def = Self::def(id)?;
        let valid = def.validate_patch(patch)?;
        let mut stored = self.stored(id).await?;
        stored.extend(valid);
        self.db.set_overlay_config(id, &serde_json::to_string(&stored)?).await?;
        let merged = def.merged(&stored);
        self.publish(id, &merged);
        Ok(merged)
    }

    /// Cuántas pantallas de alertas hay creadas (de fábrica, `DEFAULT_ALERT_SCREENS`).
    pub async fn screens(&self) -> Result<usize> {
        let stored = self.db.get_setting(SCREENS_KEY).await?.and_then(|v| v.trim().parse::<usize>().ok());
        Ok(stored.unwrap_or(DEFAULT_ALERT_SCREENS).clamp(1, MAX_ALERT_SCREENS))
    }

    /// Fija cuántas pantallas de alertas hay. Las que se quitan pierden su estilo: si se vuelven a
    /// crear, empiezan de cero.
    pub async fn set_screens(&self, n: usize) -> Result<usize> {
        let n = n.clamp(1, MAX_ALERT_SCREENS);
        let before = self.screens().await?;
        for gone in (n + 1)..=before {
            if let Some(id) = alert_screen_id(gone) {
                self.reset(id).await?;
            }
        }
        self.db.set_setting(SCREENS_KEY, &n.to_string()).await?;
        Ok(n)
    }

    /// Pantalla de alertas que debe usarse para una petición: la pedida si existe; si no (se borró),
    /// la 1.
    pub async fn resolve_screen(&self, wanted: usize) -> Result<&'static str> {
        let count = self.screens().await?;
        let n = if (1..=count).contains(&wanted) { wanted } else { 1 };
        Ok(alert_screen_id(n).unwrap_or("alerts"))
    }

    /// Vuelve a los valores por defecto.
    pub async fn reset(&self, id: &str) -> Result<Map<String, Value>> {
        let def = Self::def(id)?;
        self.db.delete_overlay_config(id).await?;
        let merged = def.defaults();
        self.publish(id, &merged);
        Ok(merged)
    }

    /// Lo que el usuario cambió en cada overlay (sin los valores por defecto), por id.
    pub async fn stored_all(&self) -> Result<Map<String, Value>> {
        let mut out = Map::new();
        for def in registry() {
            let stored = self.stored(def.id).await?;
            if !stored.is_empty() {
                out.insert(def.id.to_string(), Value::Object(stored));
            }
        }
        Ok(out)
    }

    /// Sustituye la configuración de todos los overlays por la dada (aplicar un perfil). Los que no
    /// aparecen vuelven a sus valores por defecto. Si algo es inválido no se cambia nada.
    pub async fn replace_all_stored(&self, all: &Map<String, Value>) -> Result<()> {
        let mut valid: Vec<(&str, Map<String, Value>)> = Vec::new();
        for def in registry() {
            let patch = match all.get(def.id) {
                Some(Value::Object(m)) => def.validate_patch(m)?,
                Some(_) => return Err(AppError::Invalid(format!("configuración inválida para «{}»", def.id))),
                None => Map::new(),
            };
            valid.push((def.id, patch));
        }
        if let Some(unknown) = all.keys().find(|k| find(k).is_none()) {
            return Err(AppError::Invalid(format!("overlay desconocido: «{unknown}»")));
        }
        for (id, patch) in valid {
            if patch.is_empty() {
                self.db.delete_overlay_config(id).await?;
            } else {
                self.db.set_overlay_config(id, &serde_json::to_string(&patch)?).await?;
            }
        }
        self.publish_all().await
    }

    /// Publica la configuración de todos los overlays (al arrancar), para que cualquiera que se
    /// conecte la reciba de inmediato.
    pub async fn publish_all(&self) -> Result<()> {
        for def in registry() {
            let merged = def.merged(&self.stored(def.id).await?);
            self.publish(def.id, &merged);
        }
        Ok(())
    }

    fn publish(&self, id: &str, cfg: &Map<String, Value>) {
        self.hub.publish_retained(&channel(id), Value::Object(cfg.clone()));
    }

    /// Lo guardado; si el JSON está dañado se ignora (se vuelve a los valores por defecto).
    async fn stored(&self, id: &str) -> Result<Map<String, Value>> {
        Ok(match self.db.get_overlay_config(id).await? {
            Some(json) => match serde_json::from_str::<Value>(&json) {
                Ok(Value::Object(m)) => m,
                _ => {
                    tracing::warn!(overlay = id, "configuración de overlay ilegible; se usan los valores por defecto");
                    Map::new()
                }
            },
            None => Map::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    async fn svc() -> (OverlayConfigService, OverlayHub, Db) {
        let db = Db::open_memory().await.expect("db");
        let hub = OverlayHub::new(16);
        (OverlayConfigService::new(db.clone(), hub.clone()), hub, db)
    }

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().cloned().expect("objeto")
    }

    #[tokio::test]
    async fn unset_overlays_return_their_defaults() {
        let (s, _, _) = svc().await;
        let cfg = s.get("chat").await.expect("get");
        assert_eq!(cfg["fontSize"], json!(16));
        assert_eq!(cfg["maxMessages"], json!(10));
    }

    #[tokio::test]
    async fn set_merges_patches_persists_and_publishes_retained() {
        let (s, hub, _) = svc().await;
        s.set("chat", &obj(json!({"fontSize": 22}))).await.expect("set");
        let merged = s.set("chat", &obj(json!({"textColor": "#ff0000"}))).await.expect("set");
        assert_eq!((merged["fontSize"].clone(), merged["textColor"].clone()), (json!(22), json!("#ff0000")));
        // Sobrevive a un «reinicio» (nuevo servicio sobre la misma base).
        let again = OverlayConfigService::new(s.db.clone(), hub.clone()).get("chat").await.expect("get");
        assert_eq!(again["fontSize"], json!(22));
        let retained = hub.retained_for("config:chat").expect("retenida");
        assert_eq!(retained.data["textColor"], json!("#ff0000"));
    }

    #[tokio::test]
    async fn an_invalid_patch_changes_nothing() {
        let (s, hub, _) = svc().await;
        s.set("chat", &obj(json!({"fontSize": 22}))).await.expect("set");
        assert!(s.set("chat", &obj(json!({"fontSize": 30, "textColor": "malo"}))).await.is_err());
        assert!(s.set("chat", &obj(json!({"inexistente": 1}))).await.is_err());
        assert_eq!(s.get("chat").await.expect("get")["fontSize"], json!(22));
        assert_eq!(hub.retained_for("config:chat").expect("ret").data["fontSize"], json!(22));
    }

    #[tokio::test]
    async fn reset_returns_to_defaults() {
        let (s, hub, _) = svc().await;
        s.set("chat", &obj(json!({"fontSize": 40}))).await.expect("set");
        let d = s.reset("chat").await.expect("reset");
        assert_eq!(d["fontSize"], json!(16));
        assert_eq!(s.get("chat").await.expect("get")["fontSize"], json!(16));
        assert_eq!(hub.retained_for("config:chat").expect("ret").data["fontSize"], json!(16));
    }

    #[tokio::test]
    async fn unknown_overlay_is_an_error() {
        let (s, _, _) = svc().await;
        assert!(s.get("nada").await.is_err());
        assert!(s.set("nada", &Map::new()).await.is_err());
        assert!(s.reset("nada").await.is_err());
    }

    #[tokio::test]
    async fn corrupt_stored_json_falls_back_to_defaults() {
        let (s, _, db) = svc().await;
        db.set_overlay_config("chat", "{roto").await.expect("raw");
        assert_eq!(s.get("chat").await.expect("get")["fontSize"], json!(16));
        // Y se puede volver a configurar encima.
        assert_eq!(s.set("chat", &obj(json!({"fontSize": 18}))).await.expect("set")["fontSize"], json!(18));
    }

    #[tokio::test]
    async fn publish_all_retains_every_overlay_config() {
        let (s, _, db) = svc().await;
        s.set("goals", &obj(json!({"showPercent": false}))).await.expect("set");
        // Un hub nuevo (como tras reiniciar la app) debe quedar poblado con todas las configuraciones.
        let fresh_hub = OverlayHub::new(8);
        OverlayConfigService::new(db, fresh_hub.clone()).publish_all().await.expect("publish");
        let channels: Vec<_> = fresh_hub.retained().iter().map(|m| m.channel.clone()).collect();
        assert_eq!(channels.len(), registry().len());
        assert!(channels.contains(&"config:alerts".to_string()));
        assert_eq!(fresh_hub.retained_for("config:goals").expect("ret").data["showPercent"], json!(false));
    }

    #[tokio::test]
    async fn alert_screens_default_to_five_and_can_grow_and_shrink() {
        let (s, hub, _) = svc().await;
        assert_eq!(s.screens().await.expect("n"), 5);
        assert_eq!(s.set_screens(8).await.expect("n"), 8);
        s.set("alerts7", &obj(json!({"fontSize": 50}))).await.expect("set");
        assert_eq!(s.set_screens(6).await.expect("n"), 6);
        assert_eq!(s.screens().await.expect("n"), 6);
        // La pantalla quitada olvida su estilo.
        assert_eq!(s.get("alerts7").await.expect("get")["fontSize"], json!(16));
        assert_eq!(hub.retained_for("config:alerts7").expect("ret").data["fontSize"], json!(16));
        assert_eq!(s.set_screens(0).await.expect("n"), 1, "siempre queda una");
        assert_eq!(s.set_screens(500).await.expect("n"), MAX_ALERT_SCREENS);
    }

    #[tokio::test]
    async fn a_removed_screen_falls_back_to_the_first() {
        let (s, _, _) = svc().await;
        assert_eq!(s.resolve_screen(3).await.expect("s"), "alerts3");
        assert_eq!(s.resolve_screen(9).await.expect("s"), "alerts");
        assert_eq!(s.resolve_screen(0).await.expect("s"), "alerts");
        s.set_screens(2).await.expect("n");
        assert_eq!(s.resolve_screen(3).await.expect("s"), "alerts");
    }
}
