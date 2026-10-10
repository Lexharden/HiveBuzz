use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::body::Body;
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::get;
use axum::Router;

use super::*;

fn no_progress() -> impl Fn(Progress) + Send + Sync {
    |_| {}
}

/// Servidor HTTP local con respuestas fijas para probar la descarga sin red.
async fn serve(payload: Vec<u8>) -> String {
    let app = Router::new()
        .route("/ok", get(move || {
            let p = payload.clone();
            async move { Response::new(Body::from(p)) }
        }))
        .route("/empty", get(|| async { Response::new(Body::empty()) }))
        .route("/missing", get(|| async { StatusCode::NOT_FOUND }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}")
}

/// Servidor crudo que anuncia más bytes de los que envía y corta la conexión (hyper no lo permite).
async fn serve_truncated() -> String {
    use tokio::io::AsyncReadExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            let mut buf = [0u8; 1024];
            let _ = s.read(&mut buf).await;
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5000\r\nConnection: close\r\n\r\nsolo-unos-bytes").await;
            let _ = s.shutdown().await;
        }
    });
    format!("http://{addr}")
}

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn voice_urls_follow_the_hugging_face_layout_and_only_cover_the_catalog() {
    let (onnx, json) = voice_urls("es_MX-claude-high").expect("en catálogo");
    assert_eq!(
        onnx,
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/es/es_MX/claude/high/es_MX-claude-high.onnx"
    );
    assert_eq!(json, format!("{onnx}.json"));
    assert!(voice_urls("en_US-lessac-medium").expect("ok").0.contains("/en/en_US/lessac/medium/"));
    for evil in ["../../x", "es_MX-claude-high/../../z", "https://evil.com/x", "", "es_XX-nadie-low"] {
        assert!(voice_urls(evil).is_none(), "{evil:?}");
    }
}

#[test]
fn every_catalog_voice_resolves() {
    for v in VOICE_CATALOG {
        assert!(voice_urls(v.id).is_some(), "{}", v.id);
    }
}

#[test]
fn catalog_ids_are_unique_and_well_formed() {
    let mut seen = std::collections::HashSet::new();
    for v in VOICE_CATALOG {
        assert!(seen.insert(v.id), "repetida: {}", v.id);
        assert_eq!(v.id.split('-').count(), 3, "{}", v.id);
        assert!(v.id.is_ascii(), "{}", v.id);
    }
}

// ---- Voces personalizadas ----

const PIPER_CONFIG: &str = r#"{"audio": {"sample_rate": 22050}, "language": {"code": "es_MX"}}"#;

/// Un `.onnx` falso del tamaño mínimo aceptado, con su configuración al lado.
fn custom_model(dir: &Path, file: &str, config: Option<(&str, &str)>) -> PathBuf {
    let p = dir.join(file);
    std::fs::write(&p, vec![0u8; usize::try_from(MIN_CUSTOM_MODEL_BYTES).expect("cabe")]).expect("write");
    if let Some((name, text)) = config {
        std::fs::write(dir.join(name), text).expect("write");
    }
    p
}

#[test]
fn custom_voice_names_are_safe_file_names() {
    assert_eq!(custom_voice_name("Mi Voz (v2).final"), "Mi_Voz_v2_final");
    assert_eq!(custom_voice_name("../../etc/passwd"), "etc_passwd");
    assert_eq!(custom_voice_name("  __--  "), "voz");
    assert_eq!(custom_voice_name("ñandú"), "and");
    assert_eq!(custom_voice_name(&"x".repeat(200)).len(), 64);
    for raw in ["a/b", "a\\b", "C:x", "..", "con espacios"] {
        assert!(crate::tts::piper::safe_voice_name(&custom_voice_name(raw)), "{raw:?}");
    }
}

#[tokio::test]
async fn imports_a_custom_piper_voice_and_it_shows_up_in_the_scan() {
    let src = tempfile::tempdir().expect("tmp");
    let voices = tempfile::tempdir().expect("tmp");
    let onnx = custom_model(src.path(), "Mi voz.onnx", Some(("Mi voz.onnx.json", PIPER_CONFIG)));
    let name = import_custom_voice(voices.path(), &onnx, None).await.expect("importa");
    assert_eq!(name, "Mi_voz");
    let listed = crate::tts::piper::scan_voices(voices.path());
    assert_eq!(listed.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(), ["piper:Mi_voz"]);
    assert_eq!(listed[0].lang.as_deref(), Some("es-MX"));

    // Importarla otra vez no pisa la primera; con nombre propio se usa ese nombre.
    assert_eq!(import_custom_voice(voices.path(), &onnx, None).await.expect("otra"), "Mi_voz-2");
    assert_eq!(import_custom_voice(voices.path(), &onnx, Some("Locutor")).await.expect("nombre"), "Locutor");
    let leftovers = std::fs::read_dir(voices.path()).expect("dir").flatten().filter(|e| e.path().extension().is_some_and(|x| x == "part")).count();
    assert_eq!(leftovers, 0);
}

#[tokio::test]
async fn the_config_may_also_be_named_like_the_model_with_json() {
    let src = tempfile::tempdir().expect("tmp");
    let voices = tempfile::tempdir().expect("tmp");
    let onnx = custom_model(src.path(), "narrador.onnx", Some(("narrador.json", PIPER_CONFIG)));
    assert_eq!(import_custom_voice(voices.path(), &onnx, None).await.expect("importa"), "narrador");
    assert!(voices.path().join("narrador.onnx.json").is_file());
}

#[tokio::test]
async fn bad_custom_voices_are_rejected_without_leaving_files() {
    let src = tempfile::tempdir().expect("tmp");
    let voices = tempfile::tempdir().expect("tmp");
    let no_config = custom_model(src.path(), "a.onnx", None);
    let bad_json = custom_model(src.path(), "b.onnx", Some(("b.onnx.json", "no es json")));
    let not_piper = custom_model(src.path(), "c.onnx", Some(("c.onnx.json", r#"{"otra": "cosa"}"#)));
    let wrong_ext = custom_model(src.path(), "d.bin", Some(("d.bin.json", PIPER_CONFIG)));
    let tiny = src.path().join("e.onnx");
    std::fs::write(&tiny, b"x").expect("write");
    std::fs::write(src.path().join("e.onnx.json"), PIPER_CONFIG).expect("write");
    for (path, needle) in [
        (no_config, "falta la configuración"),
        (bad_json, "JSON"),
        (not_piper, "sample_rate"),
        (wrong_ext, ".onnx"),
        (tiny, "pequeño"),
        (src.path().join("no-existe.onnx"), ".onnx"),
    ] {
        let e = import_custom_voice(voices.path(), &path, None).await.expect_err(needle);
        assert!(e.to_string().contains(needle), "{}: {e}", path.display());
    }
    assert_eq!(std::fs::read_dir(voices.path()).expect("dir").count(), 0, "no debe quedar nada");
}

#[tokio::test]
async fn removes_installed_voices_and_refuses_paths() {
    let voices = tempfile::tempdir().expect("tmp");
    std::fs::write(voices.path().join("v.onnx"), b"x").expect("write");
    std::fs::write(voices.path().join("v.onnx.json"), PIPER_CONFIG).expect("write");
    remove_voice(voices.path(), "v").await.expect("borra");
    assert_eq!(std::fs::read_dir(voices.path()).expect("dir").count(), 0);
    assert!(remove_voice(voices.path(), "v").await.expect_err("ya no está").to_string().contains("no está instalada"));
    assert!(remove_voice(voices.path(), "../v").await.expect_err("ruta").to_string().contains("no válido"));
}

#[tokio::test]
async fn downloads_verifies_the_hash_and_reports_progress() {
    let data = vec![7u8; 600_000];
    let base = serve(data.clone()).await;
    let dir = tempfile::tempdir().expect("tmp");
    let dest = dir.path().join("sub").join("f.bin");
    let reports = Arc::new(AtomicU64::new(0));
    let r2 = reports.clone();
    let cb = move |p: Progress| {
        r2.fetch_max(p.done, Ordering::SeqCst);
    };
    download_inner(&format!("{base}/ok"), &dest, Some(&sha(&data)), false, "prueba", &cb).await.expect("descarga");
    assert_eq!(std::fs::read(&dest).expect("lee"), data);
    assert_eq!(reports.load(Ordering::SeqCst), 600_000, "el progreso llega al total");
    assert!(!dest.with_extension("part").exists());
}

#[tokio::test]
async fn a_wrong_hash_is_rejected_and_leaves_nothing_behind() {
    let base = serve(vec![1, 2, 3]).await;
    let dir = tempfile::tempdir().expect("tmp");
    let dest = dir.path().join("f.bin");
    let e = download_inner(&format!("{base}/ok"), &dest, Some(&"0".repeat(64)), false, "x", &no_progress())
        .await
        .expect_err("hash malo");
    assert!(e.to_string().contains("huella"));
    assert!(!dest.exists() && !dest.with_extension("part").exists());
}

#[tokio::test]
async fn truncated_empty_and_missing_downloads_fail_cleanly() {
    let base = serve(vec![9; 1000]).await;
    let truncated = serve_truncated().await;
    let dir = tempfile::tempdir().expect("tmp");
    for (url, name, needle) in [
        (format!("{truncated}/x"), "truncated", "interrumpi"),
        (format!("{base}/empty"), "empty", "vacía"),
        (format!("{base}/missing"), "missing", "404"),
    ] {
        let dest = dir.path().join(format!("{name}.bin"));
        let e = download_inner(&url, &dest, None, false, "x", &no_progress()).await.expect_err(name);
        let msg = e.to_string();
        // Una conexión cortada se reporta como «interrumpida» o «incompleta», según cómo la vea hyper.
        let ok = msg.contains(needle) || (name == "truncated" && msg.contains("incompleta"));
        assert!(ok, "{name}: {msg}");
        assert!(!dest.exists() && !dest.with_extension("part").exists(), "{name} dejó archivos");
    }
}

#[tokio::test]
async fn plain_http_is_refused_by_the_public_entry_point() {
    let dir = tempfile::tempdir().expect("tmp");
    let e = download("http://example.com/x", &dir.path().join("x"), None, "x", &no_progress())
        .await
        .expect_err("sin https");
    assert!(e.to_string().contains("HTTPS"));
}

fn make_zip(entries: &[(&str, &[u8])]) -> tempfile::NamedTempFile {
    let tmp = tempfile::NamedTempFile::new().expect("tmp");
    let mut w = zip::ZipWriter::new(tmp.reopen().expect("reopen"));
    let opts = zip::write::SimpleFileOptions::default();
    for (name, data) in entries {
        if name.ends_with('/') {
            w.add_directory(*name, opts).expect("dir");
        } else {
            w.start_file(*name, opts).expect("file");
            w.write_all(data).expect("write");
        }
    }
    w.finish().expect("finish");
    tmp
}

#[test]
fn extracts_nested_files() {
    let z = make_zip(&[("piper/", b""), ("piper/piper.exe", b"EXE"), ("piper/data/x.txt", b"X")]);
    let out = tempfile::tempdir().expect("tmp");
    extract_zip(z.path(), out.path()).expect("extrae");
    assert_eq!(std::fs::read(out.path().join("piper/piper.exe")).expect("lee"), b"EXE");
    assert_eq!(std::fs::read(out.path().join("piper/data/x.txt")).expect("lee"), b"X");
}

#[test]
fn zip_slip_paths_are_rejected_and_nothing_escapes() {
    for evil in ["../evil.txt", "a/../../evil.txt", "/abs/evil.txt", "..\\evil.txt"] {
        let z = make_zip(&[(evil, b"MAL")]);
        let parent = tempfile::tempdir().expect("tmp");
        let out = parent.path().join("dest");
        let r = extract_zip(z.path(), &out);
        assert!(r.is_err() || !parent.path().join("evil.txt").exists(), "{evil:?} escapó");
        assert!(!parent.path().join("evil.txt").exists(), "{evil:?} escribió fuera");
        assert!(!std::path::Path::new("/abs/evil.txt").exists());
    }
}

#[test]
fn garbage_is_not_a_zip() {
    let tmp = tempfile::NamedTempFile::new().expect("tmp");
    std::fs::write(tmp.path(), b"esto no es un zip").expect("write");
    assert!(extract_zip(tmp.path(), tempfile::tempdir().expect("tmp").path()).is_err());
}

#[test]
fn piper_exe_lives_under_the_piper_folder() {
    let p = piper_exe_path(Path::new("/datos/tts"));
    assert!(p.starts_with("/datos/tts/piper"));
}

/// Instala Piper y una voz reales y sintetiza: `cargo test piper_live -- --ignored` (≈85 MB de red).
#[cfg(windows)]
#[tokio::test]
#[ignore = "descarga ~85 MB"]
async fn piper_live_install_and_synthesize() {
    use crate::tts::engine::{SynthRequest, TtsEngine};
    use crate::tts::piper::{PiperEngine, PiperPaths};
    use std::sync::RwLock;

    let dir = tempfile::tempdir().expect("tmp");
    let tts_dir = dir.path().join("tts");
    let voices = tts_dir.join("voices");
    let on = |p: Progress| {
        if p.done == 0 {
            eprintln!("» {}", p.stage);
        }
    };
    let exe = install_piper(&tts_dir, &on).await.expect("instala Piper");
    install_voice(&voices, "es_MX-claude-high", &on).await.expect("instala la voz");

    let engine = PiperEngine::new(Arc::new(RwLock::new(PiperPaths { exe, voices_dir: voices })));
    let listed = engine.voices().await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].lang.as_deref(), Some("es-MX"));
    let out = engine
        .synthesize(&SynthRequest {
            text: "Hola, esto es una prueba de HiveBuzz con Piper.".into(),
            voice: "es_MX-claude-high".into(),
            rate: 1.0,
            out_dir: dir.path().to_path_buf(),
        })
        .await
        .expect("sintetiza");
    let len = std::fs::metadata(&out).expect("meta").len();
    assert!(len > 20_000, "audio demasiado corto: {len} bytes");
    assert!(rodio::Decoder::try_from(std::fs::File::open(&out).expect("abre")).is_ok());
}
