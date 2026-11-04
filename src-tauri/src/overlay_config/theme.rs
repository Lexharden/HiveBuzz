//! Temas de overlay: el estilo de un overlay (colores, fuente, tamaños…) en un archivo `.json` que se puede
//! compartir e importar en cualquier overlay.
//!
//! Un tema lleva **solo opciones de estilo** (grupo «Estilo»), nunca el comportamiento ni nada que lleve datos
//! del usuario. Al importar, cada valor se valida contra el esquema del overlay de destino: lo que no existe allí
//! o no es válido se ignora (un tema de otra persona no puede dañar la configuración). Una fuente que no esté
//! instalada no falla: el overlay usa la del sistema hasta que se instale.

use serde_json::{json, Map, Value};

use super::schema::{Group, OverlayDef};
use crate::error::{AppError, Result};

/// Versión del formato del archivo.
pub const FORMAT: u64 = 1;
/// Tamaño máximo de un archivo de tema.
pub const MAX_BYTES: u64 = 64 * 1024;

/// Archivo de tema con el estilo actual (`config` es la configuración completa del overlay).
pub fn export(def: &OverlayDef, config: &Map<String, Value>, name: &str) -> Value {
    let values: Map<String, Value> = def
        .fields
        .iter()
        .filter(|f| f.group == Group::Style)
        .filter_map(|f| config.get(f.key).map(|v| (f.key.to_string(), v.clone())))
        .collect();
    json!({
        "hivebuzzTheme": FORMAT,
        "name": name.chars().filter(|c| !c.is_control()).take(60).collect::<String>(),
        "overlay": def.id,
        "values": values,
    })
}

/// Resultado de leer un tema: el parche listo para aplicar y cuántos valores se descartaron.
#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    pub patch: Map<String, Value>,
    pub skipped: usize,
}

/// Lee un tema y lo adapta al overlay de destino.
pub fn import(def: &OverlayDef, text: &str) -> Result<Imported> {
    if u64::try_from(text.len()).unwrap_or(u64::MAX) > MAX_BYTES {
        return Err(AppError::Invalid("el archivo del tema es demasiado grande".into()));
    }
    let root: Value = serde_json::from_str(text).map_err(|_| AppError::Invalid("el archivo no es un tema de HiveBuzz".into()))?;
    if root.get("hivebuzzTheme").and_then(Value::as_u64) != Some(FORMAT) {
        return Err(AppError::Invalid("el archivo no es un tema de HiveBuzz (o es de una versión no compatible)".into()));
    }
    let values = root.get("values").and_then(Value::as_object).ok_or_else(|| AppError::Invalid("el tema no tiene valores".into()))?;
    let mut patch = Map::new();
    let mut skipped = 0;
    for (key, value) in values {
        // Solo estilo, y solo lo que este overlay tiene.
        let ok = def.fields.iter().find(|f| f.key == key && f.group == Group::Style).and_then(|f| f.normalize(value).ok());
        match ok {
            Some(v) => {
                patch.insert(key.clone(), v);
            }
            None => skipped += 1,
        }
    }
    if patch.is_empty() {
        return Err(AppError::Invalid("el tema no tiene ninguna opción aplicable a este overlay".into()));
    }
    Ok(Imported { patch, skipped })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay_config::schema::find;

    fn def(id: &str) -> OverlayDef {
        find(id).expect("overlay")
    }

    fn config_of(d: &OverlayDef) -> Map<String, Value> {
        d.merged(&Map::new())
    }

    #[test]
    fn an_exported_theme_has_only_style_options() {
        let d = def("chat");
        let theme = export(&d, &config_of(&d), "Mi tema");
        assert_eq!(theme["hivebuzzTheme"], 1);
        assert_eq!(theme["name"], "Mi tema");
        let values = theme["values"].as_object().expect("valores");
        assert!(values.contains_key("fontFamily") && values.contains_key("textColor"));
        for f in d.fields.iter().filter(|f| f.group == Group::Behavior) {
            assert!(!values.contains_key(f.key), "{} es de comportamiento", f.key);
        }
    }

    #[test]
    fn a_theme_round_trips_between_overlays_taking_only_what_both_have() {
        let from = def("chat");
        let mut cfg = config_of(&from);
        cfg.insert("textColor".into(), json!("#ff00aa"));
        cfg.insert("fontFamily".into(), json!("Press Start 2P"));
        let theme = export(&from, &cfg, "t").to_string();
        let to = def("alerts");
        let got = import(&to, &theme).expect("import");
        assert_eq!(got.patch["textColor"], "#ff00aa");
        assert_eq!(got.patch["fontFamily"], "Press Start 2P");
        // El patch resultante es válido para el destino.
        assert!(to.validate_patch(&got.patch).is_ok());
    }

    #[test]
    fn invalid_unknown_and_behavior_values_are_skipped_not_applied() {
        let d = def("chat");
        let behavior = d.fields.iter().find(|f| f.group == Group::Behavior).expect("comportamiento").key;
        let theme = json!({
            "hivebuzzTheme": 1,
            "values": {
                "textColor": "#112233",
                "accentColor": "no-es-un-color",
                "fontFamily": "x;y{}",
                "inventado": 5,
                behavior: 1,
            }
        })
        .to_string();
        let got = import(&d, &theme).expect("import");
        assert_eq!(got.patch.keys().collect::<Vec<_>>(), vec!["textColor"]);
        assert_eq!(got.skipped, 4);
    }

    #[test]
    fn foreign_or_broken_files_are_rejected() {
        let d = def("chat");
        assert!(import(&d, "no es json").is_err());
        assert!(import(&d, r##"{"values":{"textColor":"#112233"}}"##).is_err(), "sin marca de tema");
        assert!(import(&d, r##"{"hivebuzzTheme":2,"values":{"textColor":"#112233"}}"##).is_err(), "versión futura");
        assert!(import(&d, r#"{"hivebuzzTheme":1}"#).is_err(), "sin valores");
        assert!(import(&d, r#"{"hivebuzzTheme":1,"values":{"nada":1}}"#).is_err(), "nada aplicable");
        assert!(import(&d, &format!(r##"{{"hivebuzzTheme":1,"values":{{"textColor":"#112233"}},"x":"{}"}}"##, "a".repeat(70_000))).is_err(), "demasiado grande");
    }

    #[test]
    fn the_theme_name_is_cleaned() {
        let d = def("chat");
        let theme = export(&d, &config_of(&d), &format!("a\nb\u{7}{}", "x".repeat(100)));
        let name = theme["name"].as_str().expect("nombre");
        assert!(!name.chars().any(char::is_control));
        assert_eq!(name.chars().count(), 60);
    }
}
