//! Lista oficial de regalos (`gifts.json`), de solo lectura.
//!
//! Tolerante con el formato, como la de HiveShock: un array o `{ "gifts": [...] }`, claves en inglés o en español,
//! id como número o texto. Las entradas incompletas se saltan en lugar de invalidar el archivo.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::name_key;

/// Archivo con la lista dentro de la carpeta de regalos.
pub const LIST_FILE: &str = "gifts.json";
/// Subcarpeta con las imágenes.
pub const IMAGES_DIR: &str = "images";
const IMAGE_EXTENSIONS: [&str; 4] = ["webp", "png", "jpg", "jpeg"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceGift {
    pub id: i64,
    pub name: String,
    pub diamonds: Option<u64>,
    pub icon_url: String,
    /// Archivo dentro de `images/` (vacío si no hay o no existe).
    pub image: String,
}

/// La lista oficial con índices por id y por nombre.
#[derive(Debug, Default)]
pub struct Reference {
    pub gifts: Vec<ReferenceGift>,
    by_id: HashMap<i64, usize>,
    by_name: HashMap<String, usize>,
    /// Carpeta de las imágenes, si la lista se leyó de disco.
    pub images_dir: Option<PathBuf>,
}

impl Reference {
    pub fn new(gifts: Vec<ReferenceGift>, images_dir: Option<PathBuf>) -> Self {
        let mut by_id = HashMap::new();
        let mut by_name = HashMap::new();
        for (i, g) in gifts.iter().enumerate() {
            by_id.entry(g.id).or_insert(i);
            // Hay nombres repetidos con otro id (versiones del mismo regalo): gana el primero.
            let key = name_key(&g.name);
            if !key.is_empty() {
                by_name.entry(key).or_insert(i);
            }
        }
        Self { gifts, by_id, by_name, images_dir }
    }

    pub fn len(&self) -> usize {
        self.gifts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.gifts.is_empty()
    }

    pub fn by_id(&self, id: i64) -> Option<&ReferenceGift> {
        self.by_id.get(&id).map(|&i| &self.gifts[i])
    }

    pub fn by_name(&self, name: &str) -> Option<&ReferenceGift> {
        self.by_name.get(&name_key(name)).map(|&i| &self.gifts[i])
    }

    /// Lee `<dir>/gifts.json` y comprueba qué imágenes de `<dir>/images` existen. Sin la carpeta o con la lista
    /// dañada devuelve una lista vacía (la app funciona igual, sin catálogo oficial) y el motivo.
    pub fn load(dir: &Path) -> (Self, Option<String>) {
        let text = match std::fs::read_to_string(dir.join(LIST_FILE)) {
            Ok(t) => t,
            Err(e) => return (Self::default(), Some(format!("no se pudo leer {LIST_FILE}: {e}"))),
        };
        let mut gifts = match parse(&text) {
            Ok(g) => g,
            Err(e) => return (Self::default(), Some(format!("{LIST_FILE} no es válido: {e}"))),
        };
        let images_dir = dir.join(IMAGES_DIR);
        let files = image_files(&images_dir);
        for g in &mut gifts {
            if !files.contains(&g.image) {
                g.image.clear();
            }
        }
        (Self::new(gifts, Some(images_dir)), None)
    }
}

/// Nombres de las imágenes de la carpeta (solo archivos con extensión de imagen).
fn image_files(dir: &Path) -> HashSet<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return HashSet::new();
    };
    entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| safe_image_name(n))
        .collect()
}

/// Un nombre de archivo de imagen sin rutas («9427_Pegasus.webp»).
pub fn safe_image_name(name: &str) -> bool {
    let ext = Path::new(name).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    !name.is_empty()
        && name.len() <= 200
        && !name.contains(['/', '\\', ':'])
        && !name.starts_with('.')
        && ext.is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.as_str()))
}

fn pick<'a>(obj: &'a serde_json::Map<String, Value>, names: &[&str]) -> Option<&'a Value> {
    names.iter().find_map(|n| obj.get(*n).filter(|v| !v.is_null()))
}

fn text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn number(v: Option<&Value>) -> Option<u64> {
    match v {
        Some(Value::Number(n)) => n.as_u64(),
        Some(Value::String(s)) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Convierte el JSON de la lista. Si un id se repite, gana el primero.
pub fn parse(json: &str) -> serde_json::Result<Vec<ReferenceGift>> {
    let mut root: Value = serde_json::from_str(json)?;
    if let Value::Object(obj) = &mut root {
        root = ["gifts", "regalos"].iter().find_map(|k| obj.remove(*k)).unwrap_or(Value::Null);
    }
    let Value::Array(items) = root else {
        return Ok(Vec::new());
    };
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(items.len());
    for item in &items {
        let Value::Object(obj) = item else { continue };
        let Some(id) = text(pick(obj, &["id", "gift_id", "giftId"])).parse::<i64>().ok().filter(|i| *i > 0) else {
            continue;
        };
        if !seen.insert(id) {
            continue;
        }
        // La imagen puede venir con una ruta de otra carpeta («gift_images\9427_Pegasus.webp»): vale el nombre.
        let image = text(pick(obj, &["image", "imagen_local", "imageLocal", "imagen"]));
        let image = image.rsplit(['/', '\\']).next().unwrap_or("").to_string();
        // Solo iconos por HTTPS: terminan en un `<img src>` de la interfaz y de los overlays.
        let icon_url = text(pick(obj, &["iconUrl", "icon_url", "icon"]));
        out.push(ReferenceGift {
            id,
            name: text(pick(obj, &["name", "nombre_en", "nameEn", "nombre"])),
            diamonds: number(pick(obj, &["diamonds", "diamantes", "diamond_count", "diamondCount"])),
            icon_url: if icon_url.starts_with("https://") { icon_url } else { String::new() },
            image: if safe_image_name(&image) { image } else { String::new() },
        });
    }
    Ok(out)
}
