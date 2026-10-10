//! Esquema de configuración de cada overlay: campos, valores por defecto, rangos y validación.
//!
//! Es la única fuente de verdad: la UI genera el formulario a partir de esto y los overlays reciben
//! la configuración ya completa y validada. Las etiquetas son claves de i18n, no texto.

use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::error::{AppError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Group {
    Style,
    Behavior,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FieldKind {
    /// `#RRGGBB`
    Color { default: &'static str },
    Number { default: f64, min: f64, max: f64, step: f64 },
    /// Lista cerrada: `(valor, clave de i18n)`.
    Select { default: &'static str, options: Vec<(&'static str, &'static str)> },
    Bool { default: bool },
    Text { default: &'static str, max_len: usize },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldDef {
    pub key: &'static str,
    /// Clave de i18n.
    pub label: &'static str,
    pub group: Group,
    #[serde(flatten)]
    pub kind: FieldKind,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayDef {
    pub id: &'static str,
    /// Clave de i18n.
    pub name: &'static str,
    pub fields: Vec<FieldDef>,
}

impl FieldDef {
    pub fn default_value(&self) -> Value {
        match &self.kind {
            FieldKind::Color { default } | FieldKind::Select { default, .. } | FieldKind::Text { default, .. } => json!(default),
            FieldKind::Number { default, min, max, step } => number_value(*default, *min, *max, *step),
            FieldKind::Bool { default } => json!(default),
        }
    }

    /// Comprueba (y normaliza) un valor propuesto. Los números se acotan al rango del campo.
    pub fn normalize(&self, v: &Value) -> Result<Value> {
        let bad = |why: &str| Err(AppError::Invalid(format!("«{}»: {why}", self.key)));
        match &self.kind {
            FieldKind::Color { .. } => match v.as_str() {
                Some(s) if is_hex_color(s) => Ok(json!(s.to_ascii_lowercase())),
                _ => bad("debe ser un color #RRGGBB"),
            },
            FieldKind::Number { min, max, step, .. } => match v.as_f64().filter(|n| n.is_finite()) {
                Some(n) => Ok(number_value(n, *min, *max, *step)),
                None => bad("debe ser un número"),
            },
            FieldKind::Select { options, .. } => match v.as_str() {
                Some(s) if options.iter().any(|(val, _)| *val == s) => Ok(json!(s)),
                _ => bad("valor no permitido"),
            },
            FieldKind::Bool { .. } => v.as_bool().map(|b| json!(b)).map_or_else(|| bad("debe ser verdadero o falso"), Ok),
            FieldKind::Text { max_len, .. } => match v.as_str() {
                Some(s) => Ok(json!(s.chars().filter(|c| !c.is_control()).take(*max_len).collect::<String>())),
                None => bad("debe ser texto"),
            },
        }
    }
}

/// Ajusta `n` a la rejilla `min + k·step`, lo acota a `[min, max]` y lo guarda como entero si el
/// campo es entero (así la UI no ve «14.000000001» ni «16.0»).
fn number_value(n: f64, min: f64, max: f64, step: f64) -> Value {
    let snapped = if step > 0.0 { min + ((n - min) / step).round() * step } else { n };
    let clamped = snapped.clamp(min, max);
    if step.fract() == 0.0 && min.fract() == 0.0 {
        #[allow(clippy::cast_possible_truncation)]
        let i = clamped.round() as i64;
        json!(i)
    } else {
        json!((clamped * 1e6).round() / 1e6)
    }
}

fn is_hex_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

impl OverlayDef {
    pub fn field(&self, key: &str) -> Option<&FieldDef> {
        self.fields.iter().find(|f| f.key == key)
    }

    pub fn defaults(&self) -> Map<String, Value> {
        self.fields.iter().map(|f| (f.key.to_string(), f.default_value())).collect()
    }

    /// Valida un parche: claves desconocidas o valores inválidos son un error (nada se aplica).
    pub fn validate_patch(&self, patch: &Map<String, Value>) -> Result<Map<String, Value>> {
        let mut out = Map::new();
        for (k, v) in patch {
            let field = self
                .field(k)
                .ok_or_else(|| AppError::Invalid(format!("opción desconocida para este overlay: «{k}»")))?;
            out.insert(k.clone(), field.normalize(v)?);
        }
        Ok(out)
    }

    /// Valores por defecto + lo guardado. Lo guardado que ya no sea válido (campo eliminado o
    /// fuera de rango tras una actualización) se ignora en vez de romper el overlay.
    pub fn merged(&self, stored: &Map<String, Value>) -> Map<String, Value> {
        let mut cfg = self.defaults();
        for (k, v) in stored {
            if let Some(Ok(ok)) = self.field(k).map(|f| f.normalize(v)) {
                cfg.insert(k.clone(), ok);
            }
        }
        cfg
    }
}

// ---- Constructores de campos ----------------------------------------------------------------------

const fn color(key: &'static str, label: &'static str, group: Group, default: &'static str) -> FieldDef {
    FieldDef { key, label, group, kind: FieldKind::Color { default } }
}

const fn num(key: &'static str, label: &'static str, group: Group, default: f64, min: f64, max: f64, step: f64) -> FieldDef {
    FieldDef { key, label, group, kind: FieldKind::Number { default, min, max, step } }
}

const fn boolean(key: &'static str, label: &'static str, group: Group, default: bool) -> FieldDef {
    FieldDef { key, label, group, kind: FieldKind::Bool { default } }
}

fn select(key: &'static str, label: &'static str, group: Group, default: &'static str, options: &[(&'static str, &'static str)]) -> FieldDef {
    FieldDef { key, label, group, kind: FieldKind::Select { default, options: options.to_vec() } }
}

const fn text(key: &'static str, label: &'static str, group: Group, default: &'static str, max_len: usize) -> FieldDef {
    FieldDef { key, label, group, kind: FieldKind::Text { default, max_len } }
}

const FONTS: &[(&str, &str)] = &[
    ("Segoe UI", "font.segoe"),
    ("Arial", "font.arial"),
    ("Verdana", "font.verdana"),
    ("Georgia", "font.georgia"),
    ("Impact", "font.impact"),
    ("Trebuchet MS", "font.trebuchet"),
    ("Comic Sans MS", "font.comic"),
    ("Courier New", "font.courier"),
];

const ANCHORS: &[(&str, &str)] = &[
    ("top-left", "anchor.topLeft"),
    ("top-right", "anchor.topRight"),
    ("bottom-left", "anchor.bottomLeft"),
    ("bottom-right", "anchor.bottomRight"),
    ("center", "anchor.center"),
];

/// Campos de estilo comunes a todos los overlays.
fn common_style(anchor: &'static str, with_background: bool) -> Vec<FieldDef> {
    use Group::Style;
    let mut f = vec![
        select("fontFamily", "field.fontFamily", Style, "Segoe UI", FONTS),
        num("fontSize", "field.fontSize", Style, 16.0, 8.0, 120.0, 1.0),
        color("textColor", "field.textColor", Style, "#ffffff"),
        color("accentColor", "field.accentColor", Style, "#ffc113"),
    ];
    if with_background {
        f.push(color("backgroundColor", "field.backgroundColor", Style, "#141418"));
        f.push(num("backgroundOpacity", "field.backgroundOpacity", Style, 82.0, 0.0, 100.0, 1.0));
        f.push(num("borderRadius", "field.borderRadius", Style, 10.0, 0.0, 40.0, 1.0));
    }
    f.push(select("anchor", "field.anchor", Style, anchor, ANCHORS));
    f.push(num("margin", "field.margin", Style, 16.0, 0.0, 200.0, 1.0));
    f.push(num("scale", "field.scale", Style, 100.0, 40.0, 250.0, 5.0));
    f
}

/// Todos los overlays y sus campos.
pub fn registry() -> Vec<OverlayDef> {
    use Group::Behavior as B;
    vec![
        OverlayDef {
            id: "alerts",
            name: "overlay.alerts",
            fields: {
                let mut f = common_style("center", false);
                f.push(num("titleSize", "field.titleSize", Group::Style, 40.0, 12.0, 160.0, 1.0));
                f.push(num("textSize", "field.textSize", Group::Style, 30.0, 10.0, 120.0, 1.0));
                f.push(num("mediaMaxWidth", "field.mediaMaxWidth", Group::Style, 60.0, 10.0, 100.0, 1.0));
                f.push(select("animation", "field.animation", B, "pop", &[("pop", "animation.pop"), ("slide", "animation.slide"), ("fade", "animation.fade")]));
                // Alertas sin reglas: el overlay reacciona solo a estos eventos. Las reglas con la
                // acción «Mostrar alerta» siguen funcionando aparte.
                f.push(boolean("autoGift", "field.autoGift", B, true));
                f.push(num("minCoins", "field.autoMinCoins", B, 1.0, 0.0, 100_000.0, 1.0));
                f.push(boolean("autoFollow", "field.autoFollow", B, true));
                f.push(boolean("autoSubscribe", "field.autoSubscribe", B, true));
                f.push(boolean("autoShare", "field.autoShare", B, false));
                f.push(num("autoDurationSec", "field.autoDurationSec", B, 5.0, 1.0, 60.0, 1.0));
                f
            },
        },
        OverlayDef {
            id: "feed",
            name: "overlay.feed",
            fields: {
                let mut f = common_style("bottom-left", true);
                f.push(num("maxItems", "field.maxItems", B, 12.0, 1.0, 40.0, 1.0));
                f.push(num("lifetimeSec", "field.lifetimeSec", B, 20.0, 0.0, 600.0, 1.0));
                f.push(boolean("showAvatar", "field.showAvatar", B, true));
                f.push(boolean("showChat", "field.showChat", B, true));
                f.push(boolean("showGift", "field.showGift", B, true));
                f.push(boolean("showFollow", "field.showFollow", B, true));
                f.push(boolean("showShare", "field.showShare", B, true));
                f.push(boolean("showSubscribe", "field.showSubscribe", B, true));
                f.push(boolean("showEmote", "field.showEmote", B, false));
                f.push(boolean("showPlatform", "field.showPlatform", B, false));
                f
            },
        },
        OverlayDef {
            id: "chat",
            name: "overlay.chat",
            fields: {
                let mut f = common_style("bottom-left", true);
                f.push(num("width", "field.width", Group::Style, 420.0, 200.0, 1200.0, 10.0));
                f.push(select("nameColor", "field.nameColor", Group::Style, "accent", &[("accent", "nameColor.accent"), ("perUser", "nameColor.perUser")]));
                f.push(num("maxMessages", "field.maxMessages", B, 10.0, 1.0, 40.0, 1.0));
                f.push(num("lifetimeSec", "field.lifetimeSec", B, 0.0, 0.0, 600.0, 1.0));
                f.push(boolean("showAvatar", "field.showAvatar", B, true));
                f.push(boolean("showBadges", "field.showBadges", B, true));
                f.push(boolean("hideCommands", "field.hideCommands", B, true));
                f.push(boolean("showEmotes", "field.showEmotes", B, true));
                f.push(text("hideUsers", "field.hideUsers", B, "", 300));
                f.push(boolean("showPlatform", "field.showPlatform", B, false));
                f
            },
        },
        OverlayDef {
            id: "gifts",
            name: "overlay.gifts",
            fields: {
                let mut f = common_style("top-right", true);
                f.push(num("width", "field.width", Group::Style, 340.0, 200.0, 900.0, 10.0));
                f.push(num("maxItems", "field.maxItems", B, 6.0, 1.0, 30.0, 1.0));
                f.push(num("lifetimeSec", "field.lifetimeSec", B, 30.0, 0.0, 600.0, 1.0));
                f.push(num("minCoins", "field.minCoins", B, 0.0, 0.0, 1_000_000.0, 1.0));
                f.push(boolean("showImage", "field.showGiftImage", B, true));
                f.push(boolean("showAvatar", "field.showAvatar", B, false));
                f.push(boolean("showCoins", "field.showCoins", B, true));
                f.push(boolean("showPlatform", "field.showPlatform", B, false));
                f
            },
        },
        OverlayDef {
            id: "leaderboard",
            name: "overlay.leaderboard",
            fields: {
                let mut f = common_style("top-left", true);
                f.push(num("width", "field.width", Group::Style, 320.0, 200.0, 900.0, 10.0));
                f.push(select("scope", "field.scope", B, "session", &[("session", "scope.session"), ("day", "scope.day"), ("all", "scope.all")]));
                f.push(num("size", "field.boardSize", B, 5.0, 1.0, 15.0, 1.0));
                f.push(text("title", "field.title", B, "Top donadores", 60));
                f.push(boolean("showAvatar", "field.showAvatar", B, true));
                f.push(boolean("showCoins", "field.showCoins", B, true));
                f.push(boolean("medals", "field.medals", B, true));
                f
            },
        },
        OverlayDef {
            id: "goals",
            name: "overlay.goals",
            fields: {
                let mut f = common_style("bottom-right", true);
                f.push(num("width", "field.width", Group::Style, 420.0, 200.0, 1400.0, 10.0));
                f.push(color("trackColor", "field.trackColor", Group::Style, "#3f3f46"));
                f.push(num("barHeight", "field.barHeight", Group::Style, 22.0, 6.0, 80.0, 1.0));
                f.push(text("goalId", "field.goalId", B, "", 80));
                f.push(boolean("showLabel", "field.showLabel", B, true));
                f.push(boolean("showNumbers", "field.showNumbers", B, true));
                f.push(boolean("showPercent", "field.showPercent", B, true));
                f
            },
        },
        OverlayDef {
            id: "timer",
            name: "overlay.timer",
            fields: {
                let mut f = common_style("top-right", true);
                f.push(num("digitSize", "field.digitSize", Group::Style, 64.0, 16.0, 300.0, 1.0));
                f.push(color("lowColor", "field.lowColor", Group::Style, "#ef4444"));
                f.push(text("timerId", "field.timerId", B, "", 80));
                f.push(boolean("showLabel", "field.showLabel", B, true));
                f.push(select("format", "field.timeFormat", B, "auto", &[("auto", "format.auto"), ("hms", "format.hms"), ("ms", "format.ms")]));
                f.push(num("lowSeconds", "field.lowSeconds", B, 60.0, 0.0, 3600.0, 1.0));
                f.push(text("endText", "field.endText", B, "¡Tiempo!", 60));
                f
            },
        },
        OverlayDef {
            id: "counters",
            name: "overlay.counters",
            fields: {
                let mut f = common_style("top-left", true);
                f.push(boolean("showLikes", "field.showLikes", B, true));
                f.push(boolean("showViewers", "field.showViewers", B, true));
                f.push(text("likesLabel", "field.likesLabel", B, "Likes", 30));
                f.push(text("viewersLabel", "field.viewersLabel", B, "Espectadores", 30));
                f.push(select("layout", "field.layout", B, "row", &[("row", "layout.row"), ("column", "layout.column")]));
                f
            },
        },
        OverlayDef {
            id: "wheel",
            name: "overlay.wheel",
            fields: {
                let mut f = common_style("center", false);
                f.push(num("size", "field.wheelSize", Group::Style, 420.0, 200.0, 1000.0, 10.0));
                f.push(color("pointerColor", "field.pointerColor", Group::Style, "#ffffff"));
                f.push(boolean("showWhenIdle", "field.showWhenIdle", B, false));
                f.push(num("resultSec", "field.resultSec", B, 6.0, 1.0, 60.0, 1.0));
                f
            },
        },
        OverlayDef {
            id: "poll",
            name: "overlay.poll",
            fields: {
                let mut f = common_style("top-left", true);
                f.push(num("width", "field.width", Group::Style, 380.0, 220.0, 1000.0, 10.0));
                f.push(color("trackColor", "field.trackColor", Group::Style, "#3f3f46"));
                f.push(num("barHeight", "field.barHeight", Group::Style, 26.0, 10.0, 80.0, 1.0));
                f.push(boolean("showVotes", "field.showVotes", B, true));
                f.push(boolean("showPercent", "field.showPercent", B, true));
                f.push(boolean("showTimer", "field.showTimer", B, true));
                f.push(num("hideAfterSec", "field.hideAfterSec", B, 15.0, 0.0, 600.0, 1.0));
                f
            },
        },
        OverlayDef {
            id: "nowplaying",
            name: "overlay.nowplaying",
            fields: {
                let mut f = common_style("bottom-left", true);
                f.push(num("width", "field.width", Group::Style, 420.0, 220.0, 1000.0, 10.0));
                f.push(num("coverSize", "field.coverSize", Group::Style, 72.0, 32.0, 200.0, 2.0));
                f.push(boolean("showCover", "field.showCover", B, true));
                f.push(boolean("showProgress", "field.showProgress", B, true));
                f.push(boolean("showRequester", "field.showRequester", B, true));
                f.push(boolean("hideWhenPaused", "field.hideWhenPaused", B, false));
                f
            },
        },
    ]
}

pub fn find(id: &str) -> Option<OverlayDef> {
    registry().into_iter().find(|d| d.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn registry_ids_are_unique_and_known() {
        let ids: Vec<_> = registry().iter().map(|d| d.id).collect();
        assert_eq!(ids, ["alerts", "feed", "chat", "gifts", "leaderboard", "goals", "timer", "counters", "wheel", "poll", "nowplaying"]);
        assert_eq!(ids.iter().collect::<HashSet<_>>().len(), ids.len());
    }

    #[test]
    fn field_keys_are_unique_within_each_overlay() {
        for d in registry() {
            let keys: Vec<_> = d.fields.iter().map(|f| f.key).collect();
            assert_eq!(keys.iter().collect::<HashSet<_>>().len(), keys.len(), "{}", d.id);
        }
    }

    #[test]
    fn every_default_passes_its_own_validation_unchanged() {
        for d in registry() {
            for f in &d.fields {
                let dv = f.default_value();
                assert_eq!(f.normalize(&dv).expect(f.key), dv, "{}.{}", d.id, f.key);
            }
        }
    }

    #[test]
    fn defaults_cover_every_field() {
        for d in registry() {
            assert_eq!(d.defaults().len(), d.fields.len(), "{}", d.id);
        }
    }

    #[test]
    fn numbers_are_clamped_and_integers_stay_integers() {
        let d = find("chat").expect("chat");
        let patch = d.validate_patch(json!({"fontSize": 9999, "maxMessages": -5, "scale": 101}).as_object().expect("obj")).expect("ok");
        assert_eq!(patch["fontSize"], json!(120));
        assert_eq!(patch["maxMessages"], json!(1));
        assert_eq!(patch["scale"], json!(100), "se redondea al entero más cercano para campos enteros");
    }

    #[test]
    fn colors_must_be_hex_and_are_lowercased() {
        let d = find("chat").expect("chat");
        let ok = d.validate_patch(json!({"textColor": "#FFAA00"}).as_object().expect("obj")).expect("ok");
        assert_eq!(ok["textColor"], json!("#ffaa00"));
        for bad in [json!("red"), json!("#fff"), json!("#gggggg"), json!("url(javascript:x)"), json!(5), json!("#ffaa00;}body{display:none")] {
            assert!(d.validate_patch(json!({"textColor": bad}).as_object().expect("obj")).is_err(), "{bad}");
        }
    }

    #[test]
    fn selects_only_accept_listed_values() {
        let d = find("chat").expect("chat");
        assert!(d.validate_patch(json!({"anchor": "center"}).as_object().expect("obj")).is_ok());
        assert!(d.validate_patch(json!({"anchor": "diagonal"}).as_object().expect("obj")).is_err());
        assert!(d.validate_patch(json!({"fontFamily": "Papyrus; background:url(x)"}).as_object().expect("obj")).is_err());
    }

    #[test]
    fn unknown_keys_and_wrong_types_are_rejected_without_partial_application() {
        let d = find("chat").expect("chat");
        assert!(d.validate_patch(json!({"nope": 1}).as_object().expect("obj")).is_err());
        assert!(d.validate_patch(json!({"showAvatar": "si"}).as_object().expect("obj")).is_err());
        assert!(d.validate_patch(json!({"fontSize": "grande"}).as_object().expect("obj")).is_err());
        assert!(d.validate_patch(json!({"fontSize": 20, "textColor": "malo"}).as_object().expect("obj")).is_err());
    }

    #[test]
    fn text_is_trimmed_to_length_and_stripped_of_control_characters() {
        let d = find("leaderboard").expect("lb");
        let long = "x".repeat(200);
        let p = d.validate_patch(json!({"title": format!("Top\n{long}")}).as_object().expect("obj")).expect("ok");
        let t = p["title"].as_str().expect("str");
        assert_eq!(t.chars().count(), 60);
        assert!(!t.contains('\n'));
    }

    #[test]
    fn merged_ignores_stale_or_invalid_stored_values() {
        let d = find("chat").expect("chat");
        let stored = json!({"fontSize": 30, "removedField": 1, "textColor": "no-es-color", "anchor": "mars"});
        let m = d.merged(stored.as_object().expect("obj"));
        assert_eq!(m["fontSize"], json!(30));
        assert_eq!(m["textColor"], json!("#ffffff"), "color inválido → por defecto");
        assert_eq!(m["anchor"], json!("bottom-left"));
        assert!(!m.contains_key("removedField"));
        assert_eq!(m.len(), d.fields.len());
    }

    #[test]
    fn schema_serializes_with_flat_kind_for_the_ui() {
        let d = find("chat").expect("chat");
        let v = serde_json::to_value(d.field("fontSize").expect("f")).expect("json");
        assert_eq!(v["kind"], "number");
        assert_eq!(v["min"], 8.0);
        assert_eq!(v["group"], "style");
        assert_eq!(v["label"], "field.fontSize");
        let sel = serde_json::to_value(d.field("anchor").expect("f")).expect("json");
        assert_eq!(sel["kind"], "select");
        assert!(sel["options"].as_array().expect("arr").len() >= 5);
    }
}
