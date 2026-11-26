//! Servicio del catálogo de regalos: carga la lista oficial, aprende de los regalos que llegan en directo, guarda lo
//! aprendido por lotes y mantiene la foto del catálogo que leen la interfaz y el motor de reglas.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, OnceLock, PoisonError, RwLock};
use std::time::Duration;

use serde::Deserialize;
use tokio::sync::broadcast::error::RecvError;

use super::catalog::{build, Catalog};
use super::reference::Reference;
use super::{looks_spanish, row_key, same_name, GiftView, LearnedGift};
use crate::actions::clock::Clock;
use crate::bus::EventBus;
use crate::db::Db;
use crate::error::{AppError, Result};
use crate::events::{EventType, LiveEvent, Platform, SIMULATED_ID_PREFIX};

/// Cada cuánto se guarda lo aprendido.
pub const FLUSH_EVERY: Duration = Duration::from_secs(5);
/// Tope de regalos aprendidos: un fallo de TikTok mandando ids raros no debe hacer crecer la tabla sin límite.
const MAX_LEARNED: usize = 5_000;
/// Largo máximo de un nombre (manual o recibido).
const MAX_NAME: usize = 80;
/// Precio máximo aceptado en un alta manual (el regalo más caro ronda los 45 000).
const MAX_DIAMONDS: u64 = 1_000_000;

/// Alta o edición manual desde la interfaz.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualGift {
    /// Clave de la fila que se edita (si cambia el id o el nombre, la fila se mueve).
    #[serde(default)]
    pub previous_key: Option<String>,
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default)]
    pub name_en: Option<String>,
    #[serde(default)]
    pub name_es: Option<String>,
    #[serde(default)]
    pub diamonds: Option<u64>,
}

#[derive(Default)]
struct State {
    learned: HashMap<String, LearnedGift>,
    /// Filas cambiadas desde el último guardado.
    dirty: HashSet<String>,
    /// Cambió algo que se ve en el catálogo y aún no se reconstruyó.
    stale: bool,
}

pub struct GiftService {
    db: Db,
    clock: Arc<dyn Clock>,
    state: Mutex<State>,
    reference: RwLock<Arc<Reference>>,
    catalog: RwLock<Arc<Catalog>>,
    /// Dónde sirve el servidor local las imágenes (`http://127.0.0.1:<puerto>/gifts`) y con qué token.
    local_images: OnceLock<(String, String)>,
}

impl GiftService {
    pub fn new(db: Db, clock: Arc<dyn Clock>) -> Arc<Self> {
        Arc::new(Self {
            db,
            clock,
            state: Mutex::new(State::default()),
            reference: RwLock::new(Arc::new(Reference::default())),
            catalog: RwLock::new(Arc::new(Catalog::default())),
            local_images: OnceLock::new(),
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Lee la lista oficial de `dir` (si hay) y lo aprendido de la base de datos.
    /// Sin lista oficial no es un error: el catálogo queda con lo aprendido.
    pub async fn load(&self, dir: Option<PathBuf>) -> Result<()> {
        if let Some(dir) = dir {
            let shown = dir.display().to_string();
            match tokio::task::spawn_blocking(move || Reference::load(&dir)).await {
                Ok((reference, problem)) => {
                    if let Some(p) = problem {
                        tracing::warn!(dir = %shown, problem = %p, "sin la lista oficial de regalos");
                    }
                    *self.reference.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(reference);
                }
                Err(e) => tracing::warn!(error = %e, "no se pudo leer la lista oficial de regalos"),
            }
        }
        let learned = self.db.list_learned_gifts().await?;
        {
            let mut st = self.lock();
            st.learned = learned.into_iter().map(|g| (g.key.clone(), g)).collect();
            st.dirty.clear();
        }
        self.rebuild();
        let cat = self.catalog();
        tracing::info!(gifts = cat.len(), official = self.reference_len(), learned = self.lock().learned.len(), "catálogo de regalos cargado");
        Ok(())
    }

    /// Foto actual del catálogo (barata: un `Arc`).
    pub fn catalog(&self) -> Arc<Catalog> {
        Arc::clone(&self.catalog.read().unwrap_or_else(PoisonError::into_inner))
    }

    /// Regalos de la lista oficial.
    pub fn reference_len(&self) -> usize {
        self.reference.read().unwrap_or_else(PoisonError::into_inner).len()
    }

    /// Carpeta de las imágenes de la lista oficial (para servirlas en `/gifts`).
    pub fn images_dir(&self) -> Option<PathBuf> {
        self.reference.read().unwrap_or_else(PoisonError::into_inner).images_dir.clone()
    }

    /// Dónde están las imágenes locales en el servidor de overlays, para el respaldo de `{giftimage}`.
    pub fn set_local_images(&self, base: String, token: String) {
        let _ = self.local_images.set((base, token));
    }

    /// Un regalo que llega sin imagen toma la del catálogo: el icono oficial de TikTok o, si no lo hay, la imagen
    /// incluida en la app servida por el servidor local. Así `{giftimage}` nunca queda vacío para un regalo conocido.
    pub fn fill_image(&self, ev: &mut LiveEvent) {
        let Some(gift) = ev.gift.as_mut().filter(|g| g.image.trim().is_empty()) else {
            return;
        };
        let catalog = self.catalog();
        let Some(view) = catalog.for_event(gift.id, &gift.name) else {
            return;
        };
        if !view.icon_url.is_empty() {
            gift.image.clone_from(&view.icon_url);
        } else if let (false, Some((base, token))) = (view.image.is_empty(), self.local_images.get()) {
            gift.image = format!("{base}/{}?token={token}", encode_path(&view.image));
        }
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<GiftView> {
        self.catalog().search(query, limit)
    }

    pub fn get(&self, key: &str) -> Option<GiftView> {
        self.catalog().by_key(key).cloned()
    }

    fn rebuild(&self) {
        let learned: Vec<LearnedGift> = {
            let mut st = self.lock();
            st.stale = false;
            st.learned.values().cloned().collect()
        };
        let reference = Arc::clone(&self.reference.read().unwrap_or_else(PoisonError::into_inner));
        let catalog = Arc::new(build(&reference, &learned));
        *self.catalog.write().unwrap_or_else(PoisonError::into_inner) = catalog;
    }

    /// Aprende de un regalo recibido en directo. Solo TikTok (los bits de Twitch también llegan como regalo) y
    /// nunca del simulador. Si cambia algo que afecta a cómo se reconoce un regalo (nuevo, otro nombre, otro
    /// precio), el catálogo se reconstruye ya; si solo suben las vistas, al guardar.
    pub fn observe(&self, ev: &LiveEvent) {
        if ev.kind != EventType::Gift || ev.platform != Platform::Tiktok || ev.id.starts_with(SIMULATED_ID_PREFIX) {
            return;
        }
        let Some(gift) = ev.gift.as_ref().filter(|g| g.id > 0) else {
            return;
        };
        let unit = if gift.count > 0 { gift.coins / u64::from(gift.count) } else { gift.coins };
        let key = gift.id.to_string();
        let name: String = gift.name.trim().chars().take(MAX_NAME).collect();
        let now = self.clock.now_ms();

        let structural = {
            let mut st = self.lock();
            if !st.learned.contains_key(&key) && st.learned.len() >= MAX_LEARNED {
                return;
            }
            let mut changed = false;
            let row = st.learned.entry(key.clone()).or_insert_with(|| {
                changed = true;
                LearnedGift { key: key.clone(), gift_id: Some(gift.id), ..LearnedGift::default() }
            });
            if !name.is_empty() {
                changed |= merge_name(row, &name);
            }
            if unit > 0 && row.diamonds != Some(unit) {
                row.diamonds = Some(unit);
                changed = true;
            }
            row.seen = row.seen.saturating_add(1);
            row.last_seen = Some(now);
            st.dirty.insert(key);
            st.stale = true;
            changed
        };
        if structural {
            self.rebuild();
        }
    }

    /// Guarda lo aprendido pendiente y, si hace falta, reconstruye el catálogo.
    pub async fn flush(&self) -> Result<()> {
        let (rows, stale) = {
            let mut st = self.lock();
            let keys: Vec<String> = st.dirty.drain().collect();
            let rows: Vec<LearnedGift> = keys.iter().filter_map(|k| st.learned.get(k).cloned()).collect();
            (rows, st.stale)
        };
        if stale {
            self.rebuild();
        }
        if let Err(e) = self.db.upsert_learned_gifts(&rows).await {
            // Se reintentará en el próximo guardado.
            self.lock().dirty.extend(rows.into_iter().map(|r| r.key));
            return Err(e);
        }
        Ok(())
    }

    /// Alta o edición manual. Hace falta un id o un nombre. Se guarda en el momento.
    pub async fn save_manual(&self, input: ManualGift) -> Result<GiftView> {
        let clean = |n: Option<String>| n.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let name_en = clean(input.name_en);
        let name_es = clean(input.name_es);
        if [&name_en, &name_es].into_iter().flatten().any(|n| n.chars().count() > MAX_NAME) {
            return Err(AppError::coded("gifts.nameTooLong", format!("el nombre no puede pasar de {MAX_NAME} caracteres")).with("max", MAX_NAME.to_string()));
        }
        if input.id.is_some_and(|id| id <= 0) {
            return Err(AppError::coded("gifts.badId", "el id del regalo debe ser un número positivo"));
        }
        if input.diamonds.is_some_and(|d| d > MAX_DIAMONDS) {
            return Err(AppError::coded("gifts.badDiamonds", "precio de diamantes no válido"));
        }
        let key = row_key(input.id, &[name_es.as_deref(), name_en.as_deref()])
            .ok_or_else(|| AppError::coded("gifts.needIdOrName", "pon el id o al menos un nombre del regalo"))?;
        let now = self.clock.now_ms();

        let (row, moved_from) = {
            let mut st = self.lock();
            let previous = input.previous_key.filter(|p| *p != key);
            let mut row = previous
                .as_ref()
                .and_then(|p| st.learned.remove(p))
                .or_else(|| st.learned.remove(&key))
                .unwrap_or_default();
            if let Some(p) = &previous {
                st.dirty.remove(p);
            }
            row.key.clone_from(&key);
            row.gift_id = input.id;
            row.name_en = name_en;
            row.name_es = name_es;
            row.also.retain(|a| !row.name_en.as_deref().is_some_and(|n| same_name(n, a)) && !row.name_es.as_deref().is_some_and(|n| same_name(n, a)));
            if input.diamonds.is_some() {
                row.diamonds = input.diamonds.filter(|d| *d > 0);
            }
            row.manual = true;
            row.last_seen = Some(now);
            st.learned.insert(key.clone(), row.clone());
            st.dirty.remove(&key);
            (row, previous)
        };
        if let Some(old) = moved_from {
            self.db.delete_learned_gift(&old).await?;
        }
        self.db.upsert_learned_gifts(std::slice::from_ref(&row)).await?;
        self.rebuild();
        self.catalog()
            .by_key(&key)
            .cloned()
            .ok_or_else(|| AppError::Invalid("el regalo guardado no aparece en el catálogo".into()))
    }

    /// Borra lo aprendido o puesto a mano de un regalo (uno de la lista oficial vuelve a su estado original).
    /// Devuelve si había algo que borrar.
    pub async fn delete(&self, key: &str) -> Result<bool> {
        let existed = {
            let mut st = self.lock();
            st.dirty.remove(key);
            st.learned.remove(key).is_some()
        };
        let deleted = self.db.delete_learned_gift(key).await?;
        if existed || deleted {
            self.rebuild();
        }
        Ok(existed || deleted)
    }

    /// Aprende de cada regalo del bus y guarda cada [`FLUSH_EVERY`].
    pub fn spawn(self: &Arc<Self>, bus: &EventBus, flush_every: Duration) {
        let this = Arc::clone(self);
        let mut events = bus.subscribe();
        tokio::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(ev) => this.observe(&ev),
                    Err(RecvError::Lagged(n)) => tracing::warn!(missed = n, "el catálogo de regalos se quedó atrás en el bus"),
                    Err(RecvError::Closed) => break,
                }
            }
        });
        let this = Arc::clone(self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(flush_every);
            loop {
                tick.tick().await;
                if let Err(e) = this.flush().await {
                    tracing::warn!(error = %e, "no se pudo guardar el catálogo de regalos");
                }
            }
        });
    }
}

/// Junta un nombre recibido con los que ya tenía la fila (como `MergeName` de HiveShock): el primero en español va
/// a `name_es`, el primero en inglés a `name_en`, y el resto a alias. Devuelve si cambió algo.
fn merge_name(row: &mut LearnedGift, name: &str) -> bool {
    let known = |row: &LearnedGift| {
        row.name_en.as_deref().is_some_and(|n| same_name(n, name))
            || row.name_es.as_deref().is_some_and(|n| same_name(n, name))
            || row.also.iter().any(|a| same_name(a, name))
    };
    if known(row) {
        return false;
    }
    if looks_spanish(name) {
        match &row.name_es {
            None => row.name_es = Some(name.to_string()),
            // Lo que había en `name_es` no era español: pasa a `name_en`.
            Some(prev) if row.name_en.is_none() && !looks_spanish(prev) => {
                row.name_en = row.name_es.take();
                row.name_es = Some(name.to_string());
            }
            Some(_) => row.also.push(name.to_string()),
        }
    } else if row.name_en.is_none() {
        row.name_en = Some(name.to_string());
    } else if row.name_es.is_none() {
        // Segundo nombre distinto: el del idioma del directo.
        row.name_es = Some(name.to_string());
    } else {
        row.also.push(name.to_string());
    }
    true
}

/// Conexión opcional al catálogo para los servicios que comparan regalos (reglas, metas, timers).
/// Sin conectar, compara con un catálogo vacío: id exacto y nombre sin mayúsculas ni acentos.
#[derive(Default)]
pub struct GiftLink(OnceLock<Arc<GiftService>>);

impl GiftLink {
    pub fn attach(&self, gifts: Arc<GiftService>, who: &str) {
        if self.0.set(gifts).is_err() {
            tracing::warn!(who, "el catálogo de regalos ya estaba conectado");
        }
    }

    /// Foto actual del catálogo (barata: un `Arc`).
    pub fn catalog(&self) -> Arc<Catalog> {
        static EMPTY: LazyLock<Arc<Catalog>> = LazyLock::new(|| Arc::new(Catalog::default()));
        self.0.get().map_or_else(|| Arc::clone(&EMPTY), |g| g.catalog())
    }

    /// Ver [`GiftService::fill_image`]. Sin conectar no hace nada.
    pub fn fill_image(&self, ev: &mut LiveEvent) {
        if let Some(g) = self.0.get() {
            g.fill_image(ev);
        }
    }
}

/// Codifica un nombre de archivo para una URL (todo lo que no sea letra, número, `.`, `-`, `_` o `~`).
fn encode_path(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Alias que se conservan como mucho por regalo al importar.
const MAX_ALIASES: usize = 20;

/// Revisa un regalo hecho a mano que viene de un archivo de configuración: nombres acotados, id y precio
/// razonables y la clave recalculada (no se fía de la del archivo). `Err` dice por qué se descarta.
pub fn sanitize_imported(mut g: LearnedGift) -> std::result::Result<LearnedGift, &'static str> {
    let clean = |n: Option<String>| n.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    g.name_en = clean(g.name_en);
    g.name_es = clean(g.name_es);
    g.also = g.also.into_iter().map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).take(MAX_ALIASES).collect();
    let names = [&g.name_en, &g.name_es].into_iter().flatten().chain(g.also.iter());
    if names.into_iter().any(|n| n.chars().count() > MAX_NAME) {
        return Err("nombre demasiado largo");
    }
    if g.gift_id.is_some_and(|id| id <= 0) || g.diamonds.is_some_and(|d| d > MAX_DIAMONDS) {
        return Err("id o precio no válidos");
    }
    g.key = row_key(g.gift_id, &[g.name_es.as_deref(), g.name_en.as_deref()]).ok_or("sin id ni nombre")?;
    g.manual = true;
    Ok(g)
}
