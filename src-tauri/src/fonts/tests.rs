use serde_json::json;

use super::*;

fn woff2() -> Vec<u8> {
    let mut b = b"wOF2".to_vec();
    b.extend_from_slice(&[0; 64]);
    b
}

fn lib() -> (FontLibrary, OverlayHub, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tmp");
    let hub = OverlayHub::new(8);
    (FontLibrary::new(dir.path().join("fonts"), hub.clone()).expect("lib"), hub, dir)
}

#[test]
fn family_names_cannot_carry_css() {
    for ok in ["Roboto", "Press Start 2P", "Open Sans", "Anton-Regular", "Fuente_1", "Ñandú"] {
        assert!(is_font_name(ok), "{ok}");
    }
    for bad in ["", "   ", "Papyrus; background:url(x)", "a\"b", "a'b", "a(b)", "a{b}", "a/b", "a\\b", "a<b>", &"x".repeat(41)] {
        assert!(!is_font_name(bad), "{bad}");
    }
}

#[test]
fn fonts_are_recognised_by_their_header_not_their_extension() {
    assert_eq!(font_ext_from_magic(b"wOF2abcd"), Some("woff2"));
    assert_eq!(font_ext_from_magic(b"wOFFabcd"), Some("woff"));
    assert_eq!(font_ext_from_magic(&[0, 1, 0, 0, 9]), Some("ttf"));
    assert_eq!(font_ext_from_magic(b"OTTOabcd"), Some("otf"));
    assert_eq!(font_ext_from_magic(b"MZ\x90\x00 un exe"), None);
    assert_eq!(font_ext_from_magic(b"<svg"), None);
    assert_eq!(font_ext_from_magic(b"ab"), None);
}

const GOOGLE_CSS: &str = r#"
/* cyrillic */
@font-face {
  font-family: 'Roboto';
  font-style: normal;
  font-weight: 400;
  src: url(https://fonts.gstatic.com/s/roboto/v30/cyr.woff2) format('woff2');
}
/* latin */
@font-face {
  font-family: 'Roboto';
  font-style: normal;
  font-weight: 400;
  src: url(https://fonts.gstatic.com/s/roboto/v30/lat400.woff2) format('woff2');
}
/* latin */
@font-face {
  font-family: 'Roboto';
  font-style: normal;
  font-weight: 700;
  src: url(https://fonts.gstatic.com/s/roboto/v30/lat700.woff2) format('woff2');
}
"#;

#[test]
fn google_css_gives_one_latin_file_per_weight() {
    let faces = parse_google_css(GOOGLE_CSS);
    assert_eq!(
        faces,
        vec![
            (400, "https://fonts.gstatic.com/s/roboto/v30/lat400.woff2".to_string()),
            (700, "https://fonts.gstatic.com/s/roboto/v30/lat700.woff2".to_string()),
        ]
    );
}

#[test]
fn google_css_only_trusts_gstatic_https_woff2() {
    let evil = "/* latin */\n@font-face {\n  font-weight: 400;\n  src: url(https://evil.com/x.woff2) format('woff2');\n}\n";
    assert!(parse_google_css(evil).is_empty());
    let http = "/* latin */\n@font-face {\n  font-weight: 400;\n  src: url(http://fonts.gstatic.com/x.woff2) format('woff2');\n}\n";
    assert!(parse_google_css(http).is_empty(), "sin https no se acepta");
    let lookalike = "/* latin */\n@font-face {\n  font-weight: 400;\n  src: url(https://fonts.gstatic.com.evil.com/x.woff2) format('woff2');\n}\n";
    assert!(parse_google_css(lookalike).is_empty(), "el host debe ser exacto");
    let ttf = "/* latin */\n@font-face {\n  font-weight: 400;\n  src: url(https://fonts.gstatic.com/x.ttf) format('truetype');\n}\n";
    assert!(parse_google_css(ttf).is_empty());
}

#[test]
fn a_variable_font_range_uses_its_first_weight() {
    let css = "/* latin */\n@font-face {\n  font-weight: 100 900;\n  src: url(https://fonts.gstatic.com/v.woff2) format('woff2');\n}\n";
    assert_eq!(parse_google_css(css), vec![(100, "https://fonts.gstatic.com/v.woff2".to_string())]);
}

#[tokio::test]
async fn importing_a_font_stores_it_lists_it_and_publishes_it_to_the_overlays() {
    let (lib, hub, dir) = lib();
    let src = dir.path().join("Mi Fuente.woff2");
    std::fs::write(&src, woff2()).expect("write");
    let fam = lib.import(&src, None).await.expect("import");
    assert_eq!(fam.family, "Mi Fuente");
    assert_eq!(fam.weights, vec![400]);
    assert!(lib.dir().join("mi-fuente-400.woff2").is_file());
    assert_eq!(lib.list(), vec![fam]);
    let published = hub.retained().into_iter().find(|m| m.channel == CHANNEL).expect("publicada");
    assert_eq!(published.data, json!({ "faces": [{ "family": "Mi Fuente", "weight": 400, "file": "mi-fuente-400.woff2" }] }));
}

#[tokio::test]
async fn the_library_survives_a_restart_and_drops_missing_files() {
    let (lib, hub, dir) = lib();
    let src = dir.path().join("a.woff2");
    std::fs::write(&src, woff2()).expect("write");
    lib.import(&src, Some("Alpha".into())).await.expect("a");
    lib.import(&src, Some("Beta".into())).await.expect("b");
    std::fs::remove_file(lib.dir().join("beta-400.woff2")).expect("rm");
    let again = FontLibrary::new(lib.dir().to_path_buf(), hub).expect("reabrir");
    assert_eq!(again.list().into_iter().map(|f| f.family).collect::<Vec<_>>(), vec!["Alpha"]);
}

#[tokio::test]
async fn bad_fonts_are_rejected() {
    let (lib, _hub, dir) = lib();
    let exe = dir.path().join("virus.woff2");
    std::fs::write(&exe, b"MZ\x90\x00 no soy una fuente").expect("write");
    assert!(lib.import(&exe, None).await.is_err(), "cabecera falsa con extensión de fuente");
    let txt = dir.path().join("notes.txt");
    std::fs::write(&txt, woff2()).expect("write");
    assert!(lib.import(&txt, None).await.is_err(), "extensión no permitida");
    let ok = dir.path().join("ok.woff2");
    std::fs::write(&ok, woff2()).expect("write");
    assert!(lib.import(&ok, Some("a;b".into())).await.is_err(), "nombre con caracteres de CSS");
    assert!(lib.list().is_empty());
}

#[tokio::test]
async fn deleting_a_family_removes_its_files_and_the_overlay_entry() {
    let (lib, hub, dir) = lib();
    let src = dir.path().join("a.woff2");
    std::fs::write(&src, woff2()).expect("write");
    lib.import(&src, Some("Alpha".into())).await.expect("a");
    lib.delete("Alpha").await.expect("delete");
    assert!(lib.list().is_empty());
    assert!(!lib.dir().join("alpha-400.woff2").exists());
    let published = hub.retained().into_iter().find(|m| m.channel == CHANNEL).expect("publicada");
    assert_eq!(published.data, json!({ "faces": [] }));
}

#[test]
fn a_tampered_index_cannot_point_outside_the_folder() {
    let dir = tempfile::tempdir().expect("tmp");
    let fonts = dir.path().join("fonts");
    std::fs::create_dir_all(&fonts).expect("dir");
    std::fs::write(dir.path().join("secreto.woff2"), woff2()).expect("write");
    std::fs::write(
        fonts.join(INDEX_FILE),
        r#"[{"family":"Mala","weight":400,"file":"../secreto.woff2"},{"family":"x;y","weight":400,"file":"x.woff2"}]"#,
    )
    .expect("index");
    let lib = FontLibrary::new(fonts, OverlayHub::new(4)).expect("lib");
    assert!(lib.list().is_empty());
}
