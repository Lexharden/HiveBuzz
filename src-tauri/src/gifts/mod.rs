//! Catálogo de regalos de TikTok (portado de HiveShock).
//!
//! Dos fuentes:
//! - la **lista oficial** (`resources/gifts/gifts.json` + `images/`), de solo lectura, que va con la app y se
//!   actualiza con `bun run gifts:sync` desde HiveShock: id, nombre, diamantes e imagen de cada regalo;
//! - lo **aprendido** en los directos y las **altas manuales** (tabla `gift_catalog`): nombres en el idioma del
//!   directo, alias, precio y veces vistos.
//!
//! [`catalog::build`] combina ambas: el nombre oficial manda y el del directo queda como alias, y las variantes del
//! mismo regalo (mismo nombre y precio, otro id) se juntan en una fila. Del resultado sale un [`catalog::Catalog`]
//! inmutable que usan la interfaz (selector de regalos) y el motor de reglas (un regalo elegido acepta también sus
//! variantes y alias).

pub mod catalog;
pub mod reference;
pub mod service;

#[cfg(test)]
mod tests;

/// Catálogo pequeño para los tests de reglas, metas y timers.
#[cfg(test)]
pub mod testing {
    use super::catalog::{build, Catalog};
    use super::reference::{Reference, ReferenceGift};
    use super::LearnedGift;

    fn r(id: i64, name: &str, diamonds: u64) -> ReferenceGift {
        ReferenceGift { id, name: name.into(), diamonds: Some(diamonds), icon_url: String::new(), image: String::new() }
    }

    /// Rose (5655, aprendida también como «Rosa»), Mishka Bear con dos ids (5566/5582) y dos Freestyle
    /// distintos (7000 de 1 💎 y 7001 de 1800 💎).
    pub fn catalog() -> Catalog {
        let reference = Reference::new(
            vec![r(5655, "Rose", 1), r(5566, "Mishka Bear", 100), r(5582, "Mishka Bear", 100), r(7000, "Freestyle", 1), r(7001, "Freestyle", 1800)],
            None,
        );
        let rosa = LearnedGift { key: "5655".into(), gift_id: Some(5655), name_es: Some("Rosa".into()), seen: 3, ..LearnedGift::default() };
        build(&reference, &[rosa])
    }
}

use serde::{Deserialize, Serialize};
use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

/// Minúsculas, sin acentos y sin espacios en los extremos («  Corazón » → «corazon»). Para buscar.
pub fn fold(text: &str) -> String {
    text.trim().to_lowercase().nfd().filter(|c| !is_combining_mark(*c)).nfc().collect()
}

/// Clave con la que se comparan nombres: [`fold`] y solo letras y dígitos («It’s Corn!» = «its corn»).
/// Un nombre sin letras ni dígitos (solo emoji) se compara entero, para que no coincida con cualquier otro.
pub fn name_key(name: &str) -> String {
    let folded = fold(name);
    let key: String = folded.chars().filter(|c| c.is_alphanumeric()).collect();
    if key.is_empty() {
        folded
    } else {
        key
    }
}

/// Clave de una fila del catálogo: el id, o `name:<clave>` para un regalo dado de alta sin id.
pub fn row_key(id: Option<i64>, names: &[Option<&str>]) -> Option<String> {
    if let Some(id) = id.filter(|i| *i > 0) {
        return Some(id.to_string());
    }
    names.iter().flatten().map(|n| name_key(n)).find(|k| !k.is_empty()).map(|k| format!("name:{k}"))
}

/// Lo que se guarda de un regalo: aprendido en un directo o dado de alta a mano.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LearnedGift {
    pub key: String,
    pub gift_id: Option<i64>,
    pub name_en: Option<String>,
    pub name_es: Option<String>,
    pub also: Vec<String>,
    /// Precio por unidad.
    pub diamonds: Option<u64>,
    pub seen: u64,
    /// Última vez visto o editado (ms desde epoch).
    pub last_seen: Option<i64>,
    pub manual: bool,
}

/// Un regalo tal como lo ve la interfaz: ya combinado con la lista oficial y con sus variantes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GiftView {
    /// Identifica la fila (para editarla o borrarla): el id en texto o `name:<clave>`.
    pub key: String,
    pub id: Option<i64>,
    /// Otros ids del mismo regalo (variantes de TikTok con el mismo nombre y precio).
    pub other_ids: Vec<i64>,
    pub name_en: Option<String>,
    pub name_es: Option<String>,
    pub also: Vec<String>,
    /// Nombre para mostrar: «ES / EN» si son distintos.
    pub display_name: String,
    pub diamonds: Option<u64>,
    pub seen: u64,
    pub last_seen: Option<i64>,
    /// Archivo dentro de `gifts/images` (servido en `/gifts/<archivo>`), o vacío si no hay.
    pub image: String,
    /// Icono oficial en el CDN de TikTok (respaldo si no hay imagen local).
    pub icon_url: String,
    /// Solo viene de la lista oficial: todavía no se ha visto en un directo.
    pub reference_only: bool,
    /// Tiene datos puestos a mano.
    pub manual: bool,
}

impl GiftView {
    /// Todos los ids del regalo (el principal y sus variantes).
    pub fn ids(&self) -> impl Iterator<Item = i64> + '_ {
        self.id.into_iter().chain(self.other_ids.iter().copied())
    }

    /// Todos sus nombres (oficial, en español y alias).
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.name_en.as_deref().into_iter().chain(self.name_es.as_deref()).chain(self.also.iter().map(String::as_str))
    }

    /// Para el buscador: nombre, alias o cualquier id, sin distinguir mayúsculas ni acentos. Vacío = coincide.
    pub fn matches_query(&self, folded_query: &str) -> bool {
        folded_query.is_empty()
            || self.ids().any(|id| id.to_string().contains(folded_query))
            || self.names().any(|n| fold(n).contains(folded_query))
    }
}

/// Nombre para mostrar, como en HiveShock: «ES / EN» si hay los dos y son distintos.
pub fn display_name(name_en: Option<&str>, name_es: Option<&str>, also: &[String], id: Option<i64>) -> String {
    match (name_es, name_en) {
        (Some(es), Some(en)) if !es.eq_ignore_ascii_case(en) => format!("{es} / {en}"),
        (Some(n), _) | (None, Some(n)) => n.to_string(),
        (None, None) => also.first().cloned().unwrap_or_else(|| id.map_or_else(String::new, |i| format!("#{i}"))),
    }
}

/// ¿Parece español? (TikTok envía el nombre en el idioma del directo.) Misma heurística que HiveShock.
pub fn looks_spanish(name: &str) -> bool {
    if name.chars().any(|c| "áéíóúüñÁÉÍÓÚÜÑ¿¡".contains(c)) {
        return true;
    }
    let lower = name.to_lowercase();
    (lower.contains("rosa") && lower != "rose") || lower.contains("para ti") || lower.contains("corazon")
}

/// Nombres iguales sin distinguir mayúsculas ni acentos.
pub fn same_name(a: &str, b: &str) -> bool {
    name_key(a) == name_key(b)
}
