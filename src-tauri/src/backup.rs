//! Exportar e importar la configuración completa en un único archivo `.zip` (con sus sonidos e
//! imágenes). Nunca incluye secretos (llavero) ni datos de la comunidad (espectadores, puntos,
//! donaciones, log de eventos).
//!
//! Importar es un proceso en dos tiempos para no pelearse con el estado en memoria de metas, timers
//! y demás (que se vuelca a SQLite al cerrar): el archivo se **valida y se deja pendiente**; al
//! siguiente arranque, antes de que ningún servicio cargue nada, se aplica de una vez.

use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::db::Db;
use crate::error::{AppError, Result};
use crate::goals::Goal;
use crate::media::{Media, MediaKind, IMAGE_EXT, VIDEO_EXT};
use crate::overlay_config::schema::find as find_overlay;
use crate::profiles::ProfileData;
use crate::rules::model::Rule;
use crate::rules::validate_rule;
use crate::sounds::{Sound, EXTENSIONS as SOUND_EXT};
use crate::timers::service::StoredTimer;

pub const FORMAT: u32 = 1;
const APP: &str = "hivebuzz";
const CONFIG_ENTRY: &str = "hivebuzz-config.json";
const SOUNDS_DIR: &str = "sounds";
const MEDIA_DIR: &str = "media";
pub const PENDING_FILE: &str = "pending-import.zip";
const MAX_ENTRIES: usize = 5_000;
const MAX_CONFIG_BYTES: u64 = 32 * 1024 * 1024;
const MAX_SOUND_BYTES: u64 = 25 * 1024 * 1024;
const MAX_MEDIA_BYTES: u64 = 60 * 1024 * 1024;
const MAX_SETTING_BYTES: usize = 256 * 1024;

/// Ajustes que viajan en el archivo. Todo lo demás (puerto, último usuario…) es de esta instalación.
pub const SETTINGS_KEYS: [&str; 6] = ["bot_config", "points_config", "wheel_config", "tts_config", "obs_config", "app_prefs"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileBundle {
    pub id: String,
    pub name: String,
    pub updated_ms: i64,
    pub data: ProfileData,
}

/// Contenido de `hivebuzz-config.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bundle {
    pub format: u32,
    pub app: String,
    pub version: String,
    pub exported_ms: i64,
    #[serde(default)]
    pub settings: BTreeMap<String, String>,
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub goals: Vec<Goal>,
    #[serde(default)]
    pub timers: Vec<StoredTimer>,
    #[serde(default)]
    pub overlays: Map<String, Value>,
    #[serde(default)]
    pub sounds: Vec<Sound>,
    #[serde(default)]
    pub media: Vec<Media>,
    #[serde(default)]
    pub profiles: Vec<ProfileBundle>,
}

/// Lo que contiene (o contenía) un archivo, para mostrarlo al usuario.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub rules: usize,
    pub goals: usize,
    pub timers: usize,
    pub overlays: usize,
    pub sounds: usize,
    pub media: usize,
    pub profiles: usize,
    /// Cosas del archivo que se descartaron y por qué.
    pub skipped: Vec<String>,
}

impl Summary {
    fn of(b: &Bundle, skipped: Vec<String>) -> Self {
        Self {
            rules: b.rules.len(),
            goals: b.goals.len(),
            timers: b.timers.len(),
            overlays: b.overlays.len(),
            sounds: b.sounds.len(),
            media: b.media.len(),
            profiles: b.profiles.len(),
            skipped,
        }
    }
}

fn bad(msg: impl std::fmt::Display) -> AppError {
    AppError::Invalid(format!("archivo de configuración: {msg}"))
}

/// Nombre de archivo simple: nada de rutas, `..` ni caracteres raros.
fn safe_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

fn ext_of(name: &str) -> String {
    Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

fn short_text(s: &str, max: usize) -> bool {
    !s.trim().is_empty() && s.chars().count() <= max && !s.chars().any(char::is_control)
}

// ---------------------------------------------------------------- exportar

pub async fn export(db: &Db, data_dir: &Path, dest: &Path, version: &str, now_ms: i64) -> Result<Summary> {
    let (sounds_dir, media_dir) = (data_dir.join(SOUNDS_DIR), data_dir.join(MEDIA_DIR));
    let mut settings = BTreeMap::new();
    for key in SETTINGS_KEYS {
        if let Some(v) = db.get_setting(key).await? {
            settings.insert(key.to_string(), v);
        }
    }
    let mut overlays = Map::new();
    for (id, json) in db.list_overlay_configs().await? {
        if let Ok(v) = serde_json::from_str::<Value>(&json) {
            overlays.insert(id, v);
        }
    }
    let mut skipped = Vec::new();
    // Solo viajan las entradas cuyo archivo existe: un paquete no debe nacer con huecos.
    let sounds: Vec<Sound> = db
        .list_sounds()
        .await?
        .into_iter()
        .filter(|s| {
            let ok = sounds_dir.join(&s.file).is_file();
            if !ok {
                skipped.push(format!("sonido «{}»: falta el archivo", s.name));
            }
            ok
        })
        .collect();
    let media: Vec<Media> = db
        .list_media()
        .await?
        .into_iter()
        .filter(|m| {
            let ok = media_dir.join(&m.file).is_file();
            if !ok {
                skipped.push(format!("medio «{}»: falta el archivo", m.name));
            }
            ok
        })
        .collect();
    let profiles = db
        .list_profiles()
        .await?
        .into_iter()
        .filter_map(|p| serde_json::from_str::<ProfileData>(&p.json).ok().map(|data| ProfileBundle { id: p.id, name: p.name, updated_ms: p.updated_ms, data }))
        .collect();
    let bundle = Bundle {
        format: FORMAT,
        app: APP.into(),
        version: version.into(),
        exported_ms: now_ms,
        settings,
        rules: db.list_rules().await?,
        goals: db.list_goals().await?,
        timers: db.list_timers().await?,
        overlays,
        sounds,
        media,
        profiles,
    };
    let summary = Summary::of(&bundle, skipped);
    let dest = dest.to_path_buf();
    tokio::task::spawn_blocking(move || write_zip(&bundle, &sounds_dir, &media_dir, &dest))
        .await
        .map_err(|e| bad(format!("la exportación falló: {e}")))??;
    Ok(summary)
}

fn write_zip(bundle: &Bundle, sounds_dir: &Path, media_dir: &Path, dest: &Path) -> Result<()> {
    // Se escribe a un temporal y se renombra: un fallo a medias no deja un archivo roto con el nombre bueno.
    let mut tmp = dest.as_os_str().to_owned();
    tmp.push(".part");
    let tmp = PathBuf::from(tmp);
    let result = (|| -> Result<()> {
        let file = File::create(&tmp)?;
        let mut zip = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        zip.start_file(CONFIG_ENTRY, opts).map_err(bad)?;
        zip.write_all(&serde_json::to_vec_pretty(bundle)?)?;
        // Audio e imagen ya vienen comprimidos: se guardan tal cual.
        let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for s in &bundle.sounds {
            zip.start_file(format!("{SOUNDS_DIR}/{}", s.file), stored).map_err(bad)?;
            zip.write_all(&std::fs::read(sounds_dir.join(&s.file))?)?;
        }
        for m in &bundle.media {
            zip.start_file(format!("{MEDIA_DIR}/{}", m.file), stored).map_err(bad)?;
            zip.write_all(&std::fs::read(media_dir.join(&m.file))?)?;
        }
        zip.finish().map_err(bad)?;
        Ok(())
    })();
    match result {
        Ok(()) => {
            std::fs::rename(&tmp, dest)?;
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

// ---------------------------------------------------------------- validar

/// Lee y valida un archivo sin tocar nada. Devuelve el paquete ya depurado y lo que se descartó.
pub fn inspect(path: &Path) -> Result<(Bundle, Vec<String>)> {
    let file = File::open(path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| bad(format!("no es un .zip válido ({e})")))?;
    if archive.len() > MAX_ENTRIES {
        return Err(bad("demasiados archivos dentro"));
    }
    let mut raw = Vec::new();
    {
        let entry = archive.by_name(CONFIG_ENTRY).map_err(|_| bad("no contiene hivebuzz-config.json"))?;
        if entry.size() > MAX_CONFIG_BYTES {
            return Err(bad("la configuración es demasiado grande"));
        }
        entry.take(MAX_CONFIG_BYTES + 1).read_to_end(&mut raw)?;
        if raw.len() as u64 > MAX_CONFIG_BYTES {
            return Err(bad("la configuración es demasiado grande"));
        }
    }
    let mut bundle: Bundle = serde_json::from_slice(&raw).map_err(|e| bad(format!("JSON no válido ({e})")))?;
    if bundle.app != APP {
        return Err(bad("no es un archivo de HiveBuzz"));
    }
    if bundle.format != FORMAT {
        return Err(bad(format!("formato {} no compatible con esta versión (usa {FORMAT})", bundle.format)));
    }
    let mut skipped = Vec::new();
    sanitize(&mut bundle, &mut archive, &mut skipped)?;
    Ok((bundle, skipped))
}

fn sanitize(b: &mut Bundle, archive: &mut zip::ZipArchive<File>, skipped: &mut Vec<String>) -> Result<()> {
    // Ajustes: lista blanca, JSON válido y tamaño razonable.
    b.settings.retain(|k, v| {
        let ok = SETTINGS_KEYS.contains(&k.as_str()) && v.len() <= MAX_SETTING_BYTES && serde_json::from_str::<Value>(v).is_ok();
        if !ok {
            skipped.push(format!("ajuste «{k}» descartado"));
        }
        ok
    });

    let mut seen = HashSet::new();
    for r in &b.rules {
        validate_rule(r)?;
        if !seen.insert(r.id.clone()) {
            return Err(bad(format!("la regla «{}» está repetida", r.name)));
        }
    }
    for g in &b.goals {
        g.validate()?;
    }
    for t in &b.timers {
        t.config.validate()?;
    }
    check_unique("meta", b.goals.iter().map(|g| g.id.as_str()))?;
    check_unique("timer", b.timers.iter().map(|t| t.config.id.as_str()))?;
    check_unique("perfil", b.profiles.iter().map(|p| p.id.as_str()))?;
    for p in &b.profiles {
        if !short_text(&p.name, 60) {
            return Err(bad("un perfil tiene un nombre no válido"));
        }
        for r in &p.data.rules {
            validate_rule(r)?;
        }
    }

    // Overlays: solo los que existen, con valores válidos.
    let mut overlays = Map::new();
    for (id, cfg) in std::mem::take(&mut b.overlays) {
        match (find_overlay(&id), cfg) {
            (Some(def), Value::Object(m)) => match def.validate_patch(&m) {
                Ok(valid) if !valid.is_empty() => {
                    overlays.insert(id, Value::Object(valid));
                }
                Ok(_) => {}
                Err(e) => skipped.push(format!("overlay «{id}» descartado: {e}")),
            },
            _ => skipped.push(format!("overlay «{id}» descartado: desconocido o mal formado")),
        }
    }
    b.overlays = overlays;

    // Bibliotecas: nombres de archivo seguros, extensiones permitidas, tamaños acotados y el archivo dentro del zip.
    let mut ids = HashSet::new();
    b.sounds.retain(|s| {
        let why = if !short_text(&s.id, 64) || !ids.insert(s.id.clone()) {
            Some("id no válido o repetido")
        } else if !short_text(&s.name, 80) || s.volume > 100 {
            Some("datos no válidos")
        } else if !safe_file_name(&s.file) || !SOUND_EXT.contains(&ext_of(&s.file).as_str()) {
            Some("nombre de archivo no permitido")
        } else {
            check_entry(archive, &format!("{SOUNDS_DIR}/{}", s.file), MAX_SOUND_BYTES)
        };
        if let Some(why) = why {
            skipped.push(format!("sonido «{}» descartado: {why}", s.name));
        }
        why.is_none()
    });
    let mut ids = HashSet::new();
    b.media.retain(|m| {
        let ext = ext_of(&m.file);
        let kind_ok = match m.kind {
            MediaKind::Image => IMAGE_EXT.contains(&ext.as_str()),
            MediaKind::Video => VIDEO_EXT.contains(&ext.as_str()),
        };
        let why = if !short_text(&m.id, 64) || !ids.insert(m.id.clone()) {
            Some("id no válido o repetido")
        } else if !short_text(&m.name, 80) {
            Some("datos no válidos")
        } else if !safe_file_name(&m.file) || !kind_ok {
            Some("nombre de archivo no permitido")
        } else {
            check_entry(archive, &format!("{MEDIA_DIR}/{}", m.file), MAX_MEDIA_BYTES)
        };
        if let Some(why) = why {
            skipped.push(format!("medio «{}» descartado: {why}", m.name));
        }
        why.is_none()
    });
    Ok(())
}

fn check_unique<'a>(what: &str, ids: impl Iterator<Item = &'a str>) -> Result<()> {
    let mut seen = HashSet::new();
    for id in ids {
        if !short_text(id, 64) || !seen.insert(id) {
            return Err(bad(format!("{what} con id no válido o repetido")));
        }
    }
    Ok(())
}

/// `None` si la entrada existe y su tamaño declarado es aceptable.
fn check_entry(archive: &mut zip::ZipArchive<File>, name: &str, max: u64) -> Option<&'static str> {
    match archive.by_name(name) {
        Ok(e) if e.is_file() && e.size() <= max => None,
        Ok(_) => Some("archivo demasiado grande"),
        Err(_) => Some("falta el archivo en el paquete"),
    }
}

// ---------------------------------------------------------------- importar

/// Valida el archivo y lo deja pendiente para el próximo arranque.
pub fn stage_import(path: &Path, data_dir: &Path) -> Result<Summary> {
    let (bundle, skipped) = inspect(path)?;
    let pending = data_dir.join(PENDING_FILE);
    let mut tmp = pending.as_os_str().to_owned();
    tmp.push(".part");
    let tmp = PathBuf::from(tmp);
    std::fs::copy(path, &tmp)?;
    std::fs::rename(&tmp, &pending)?;
    Ok(Summary::of(&bundle, skipped))
}

pub fn has_pending(data_dir: &Path) -> bool {
    data_dir.join(PENDING_FILE).is_file()
}

pub fn cancel_pending(data_dir: &Path) -> Result<()> {
    match std::fs::remove_file(data_dir.join(PENDING_FILE)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Aplica la importación pendiente (si hay). Debe llamarse antes de que se cargue ningún servicio.
/// Si falla, el archivo se aparta como `.failed` y la app arranca con lo que tenía.
pub async fn apply_pending(db: &Db, data_dir: &Path, now_ms: i64) -> Result<Option<Summary>> {
    let pending = data_dir.join(PENDING_FILE);
    if !pending.is_file() {
        return Ok(None);
    }
    let result = apply_file(db, data_dir, &pending, now_ms).await;
    match &result {
        Ok(_) => {
            let _ = std::fs::remove_file(&pending);
        }
        Err(_) => {
            let failed = data_dir.join("pending-import.failed.zip");
            let _ = std::fs::rename(&pending, failed);
        }
    }
    result.map(Some)
}

async fn apply_file(db: &Db, data_dir: &Path, path: &Path, now_ms: i64) -> Result<Summary> {
    let path_owned = path.to_path_buf();
    let data_dir = data_dir.to_path_buf();
    let dirs = (data_dir.join(SOUNDS_DIR), data_dir.join(MEDIA_DIR));
    let (bundle, skipped) = tokio::task::spawn_blocking(move || -> Result<(Bundle, Vec<String>)> {
        let (bundle, skipped) = inspect(&path_owned)?;
        extract_files(&path_owned, &bundle, &dirs.0, &dirs.1)?;
        Ok((bundle, skipped))
    })
    .await
    .map_err(|e| bad(format!("la importación falló: {e}")))??;
    db.replace_config(&bundle, now_ms).await?;
    Ok(Summary::of(&bundle, skipped))
}

fn extract_files(path: &Path, b: &Bundle, sounds_dir: &Path, media_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(sounds_dir)?;
    std::fs::create_dir_all(media_dir)?;
    let mut archive = zip::ZipArchive::new(File::open(path)?).map_err(bad)?;
    let items = b
        .sounds
        .iter()
        .map(|s| (format!("{SOUNDS_DIR}/{}", s.file), sounds_dir.join(&s.file), MAX_SOUND_BYTES))
        .chain(b.media.iter().map(|m| (format!("{MEDIA_DIR}/{}", m.file), media_dir.join(&m.file), MAX_MEDIA_BYTES)));
    for (entry_name, target, max) in items {
        let entry = archive.by_name(&entry_name).map_err(|e| bad(format!("{entry_name}: {e}")))?;
        let mut tmp = target.as_os_str().to_owned();
        tmp.push(".part");
        let tmp = PathBuf::from(tmp);
        let written = {
            let mut out = File::create(&tmp)?;
            // El tamaño declarado en la cabecera puede mentir: se corta al límite al copiar.
            std::io::copy(&mut entry.take(max + 1), &mut out)?
        };
        if written > max {
            let _ = std::fs::remove_file(&tmp);
            return Err(bad(format!("{entry_name} supera el tamaño permitido")));
        }
        std::fs::rename(&tmp, &target)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
