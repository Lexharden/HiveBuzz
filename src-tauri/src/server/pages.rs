//! Páginas de overlay embebidas en el ejecutable. Cada una es independiente (su propia URL);
//! el núcleo común (`common.js` / `common.css`) se inyecta donde la página lo marca.

use std::collections::HashMap;
use std::sync::LazyLock;

const COMMON_CSS: &str = include_str!("../../../overlays/common.css");
const COMMON_JS: &str = include_str!("../../../overlays/common.js");
const MARKER: &str = "<!--HB_COMMON-->";

/// `(id, html)`. El id coincide con el de `overlay_config::schema::registry()`.
const SOURCES: &[(&str, &str)] = &[
    ("alerts", include_str!("../../../overlays/alerts/index.html")),
    ("feed", include_str!("../../../overlays/feed/index.html")),
    ("chat", include_str!("../../../overlays/chat/index.html")),
    ("gifts", include_str!("../../../overlays/gifts/index.html")),
    ("leaderboard", include_str!("../../../overlays/leaderboard/index.html")),
    ("goals", include_str!("../../../overlays/goals/index.html")),
    ("timer", include_str!("../../../overlays/timer/index.html")),
    ("counters", include_str!("../../../overlays/counters/index.html")),
    ("wheel", include_str!("../../../overlays/wheel/index.html")),
    ("poll", include_str!("../../../overlays/poll/index.html")),
    ("nowplaying", include_str!("../../../overlays/nowplaying/index.html")),
];

static RENDERED: LazyLock<HashMap<&'static str, String>> = LazyLock::new(|| {
    let common = format!("<style>{COMMON_CSS}</style><script>{COMMON_JS}</script>");
    SOURCES.iter().map(|(id, html)| (*id, html.replacen(MARKER, &common, 1))).collect()
});

/// HTML listo de un overlay, o `None` si no existe.
pub fn render(name: &str) -> Option<&'static str> {
    RENDERED.get(name).map(String::as_str)
}

#[cfg(test)]
pub fn ids() -> impl Iterator<Item = &'static str> {
    SOURCES.iter().map(|(id, _)| *id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay_config::schema::registry;

    #[test]
    fn every_configurable_overlay_has_a_page_and_vice_versa() {
        let mut pages: Vec<_> = ids().collect();
        let mut defs: Vec<_> = registry().iter().map(|d| d.id).collect();
        pages.sort_unstable();
        defs.sort_unstable();
        assert_eq!(pages, defs);
    }

    #[test]
    fn the_common_core_is_injected_exactly_once_and_the_marker_is_gone() {
        for id in ids() {
            let html = render(id).expect(id);
            assert!(!html.contains(MARKER), "{id}: quedó el marcador");
            assert_eq!(html.matches("window.HB = (function").count(), 1, "{id}");
            assert!(html.contains("--hb-accent"), "{id}: falta el CSS común");
            assert!(html.contains("HB.start("), "{id}: no arranca el núcleo");
        }
    }

    #[test]
    fn each_page_declares_its_own_id_matching_its_config_channel() {
        for id in ids() {
            let html = render(id).expect(id);
            assert!(html.contains(&format!("id: \"{id}\"")), "{id}: HB.start debe usar su propio id");
        }
    }

    #[test]
    fn pages_never_insert_remote_text_as_html() {
        // Regla de seguridad: nada de innerHTML / insertAdjacentHTML / document.write en overlays.
        for id in ids() {
            let html = render(id).expect(id);
            for banned in ["innerHTML", "insertAdjacentHTML", "document.write", "outerHTML", "eval("] {
                assert!(!html.contains(banned), "{id} usa {banned}");
            }
        }
    }

    #[test]
    fn unknown_pages_do_not_exist() {
        assert!(render("nope").is_none());
        assert!(render("../etc/passwd").is_none());
    }
}
