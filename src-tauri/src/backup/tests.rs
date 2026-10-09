use std::io::Write;

use serde_json::json;

use super::*;
use crate::goals::{GoalKind, OnReach};
use crate::rules::model::{ActionPlan, ActionSpec, Conditions, PlanMode, Step, Trigger};
use crate::timers::{TimerConfig, TimerState};

fn rule(id: &str) -> Rule {
    Rule {
        id: id.into(),
        name: format!("regla {id}"),
        enabled: true,
        trigger: Trigger::Follow,
        conditions: Conditions::default(),
        plan: ActionPlan { mode: PlanMode::Sequence, steps: vec![Step { delay_ms: 0, action: ActionSpec::new("noop", json!({})) }] },
        priority: None,
        ttl_ms: 60_000,
        cost_points: None,
    }
}

struct Install {
    dir: tempfile::TempDir,
    db: Db,
}

impl Install {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("sounds")).unwrap();
        std::fs::create_dir_all(dir.path().join("media")).unwrap();
        Self { dir, db: Db::open_memory().await.unwrap() }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.dir.path().join(rel)
    }
}

/// Una instalación con de todo un poco.
async fn populated() -> Install {
    let i = Install::new().await;
    i.db.save_rule(&rule("a"), 1).await.unwrap();
    i.db.save_rule(&rule("b"), 2).await.unwrap();
    i.db
        .save_goal(&Goal {
            id: "g".into(),
            name: "Meta".into(),
            kind: GoalKind::Likes,
            target: 100,
            current: 7,
            on_reach: OnReach::Stop,
            reset_on_session: false,
            reached_count: 0,
        })
        .await
        .unwrap();
    i.db
        .save_timers(&[StoredTimer {
            config: TimerConfig { id: "t".into(), name: "Timer".into(), start_seconds: 60, max_seconds: None, extensions: vec![] },
            state: TimerState::idle(60_000),
        }])
        .await
        .unwrap();
    std::fs::write(i.path("sounds/s1.wav"), b"RIFFsound").unwrap();
    i.db.insert_sound(&Sound { id: "s1".into(), name: "Ding".into(), file: "s1.wav".into(), volume: 80, created_ms: 1 }).await.unwrap();
    std::fs::write(i.path("media/m1.png"), b"PNGimage").unwrap();
    i.db.insert_media(&Media { id: "m1".into(), name: "Logo".into(), file: "m1.png".into(), kind: MediaKind::Image, created_ms: 1 }).await.unwrap();
    i.db.set_overlay_config("alerts", r#"{"fontSize":40}"#).await.unwrap();
    i.db.set_setting("bot_config", r#"{"enabled":true}"#).await.unwrap();
    // Cosas que NO deben viajar.
    i.db.set_setting("server_port", "9999").await.unwrap();
    i.db.set_setting("last_username", "alguien").await.unwrap();
    i
}

fn zip_with(path: &Path, config: &str, files: &[(&str, &[u8])]) {
    let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
    let opts = zip::write::SimpleFileOptions::default();
    zip.start_file(CONFIG_ENTRY, opts).unwrap();
    zip.write_all(config.as_bytes()).unwrap();
    for (name, bytes) in files {
        zip.start_file(*name, opts).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
}

fn config(extra: Value) -> String {
    let mut base = json!({ "format": 1, "app": "hivebuzz", "version": "x", "exportedMs": 0 });
    base.as_object_mut().unwrap().extend(extra.as_object().cloned().unwrap());
    base.to_string()
}

#[tokio::test]
async fn export_then_import_restores_everything_on_a_fresh_install() {
    let src = populated().await;
    let file = src.path("backup.zip");
    let summary = export(&src.db, src.dir.path(), &file, "0.1.0", 123).await.unwrap();
    assert_eq!((summary.rules, summary.goals, summary.timers, summary.sounds, summary.media, summary.overlays), (2, 1, 1, 1, 1, 1));
    assert!(summary.skipped.is_empty());

    let dst = Install::new().await;
    // Un estado previo que la importación sustituye.
    dst.db.save_rule(&rule("vieja"), 1).await.unwrap();
    let staged = stage_import(&file, dst.dir.path()).unwrap();
    assert_eq!(staged.rules, 2);
    assert!(has_pending(dst.dir.path()));
    let applied = apply_pending(&dst.db, dst.dir.path(), 5).await.unwrap().unwrap();
    assert_eq!(applied.sounds, 1);
    assert!(!has_pending(dst.dir.path()), "el pendiente se consume");

    assert_eq!(dst.db.list_rules().await.unwrap().iter().map(|r| r.id.clone()).collect::<Vec<_>>(), ["a", "b"]);
    assert_eq!(dst.db.list_goals().await.unwrap()[0].current, 7);
    assert_eq!(dst.db.list_timers().await.unwrap().len(), 1);
    assert_eq!(dst.db.get_overlay_config("alerts").await.unwrap().as_deref(), Some(r#"{"fontSize":40}"#));
    assert_eq!(dst.db.get_setting("bot_config").await.unwrap().as_deref(), Some(r#"{"enabled":true}"#));
    assert_eq!(std::fs::read(dst.path("sounds/s1.wav")).unwrap(), b"RIFFsound");
    assert_eq!(std::fs::read(dst.path("media/m1.png")).unwrap(), b"PNGimage");
    assert_eq!(dst.db.list_sounds().await.unwrap()[0].volume, 80);
    assert_eq!(dst.db.get_setting("server_port").await.unwrap(), None, "el puerto es de cada instalación");
    assert_eq!(dst.db.get_setting("last_username").await.unwrap(), None);
}

#[tokio::test]
async fn the_export_never_contains_machine_settings_or_secrets() {
    let src = populated().await;
    let file = src.path("backup.zip");
    export(&src.db, src.dir.path(), &file, "0.1.0", 1).await.unwrap();
    let mut archive = zip::ZipArchive::new(File::open(&file).unwrap()).unwrap();
    let mut json = String::new();
    archive.by_name(CONFIG_ENTRY).unwrap().read_to_string(&mut json).unwrap();
    for forbidden in ["server_port", "last_username", "9999", "alguien", "euler", "overlay_token"] {
        assert!(!json.contains(forbidden), "{forbidden} no debe exportarse");
    }
}

#[tokio::test]
async fn missing_library_files_are_skipped_on_export() {
    let src = populated().await;
    std::fs::remove_file(src.path("sounds/s1.wav")).unwrap();
    let summary = export(&src.db, src.dir.path(), &src.path("b.zip"), "0.1.0", 1).await.unwrap();
    assert_eq!(summary.sounds, 0);
    assert_eq!(summary.skipped.len(), 1);
}

#[tokio::test]
async fn rejects_files_that_are_not_ours_or_too_new() {
    let i = Install::new().await;
    let p = i.path("x.zip");
    std::fs::write(&p, b"no es un zip").unwrap();
    assert!(inspect(&p).is_err());
    zip_with(&p, r#"{"format":1,"app":"otra-app","version":"x","exportedMs":0}"#, &[]);
    assert!(inspect(&p).unwrap_err().to_string().contains("no es un archivo de HiveBuzz"));
    zip_with(&p, r#"{"format":99,"app":"hivebuzz","version":"x","exportedMs":0}"#, &[]);
    assert!(inspect(&p).unwrap_err().to_string().contains("no compatible"));
    zip_with(&p, "{no json", &[]);
    assert!(inspect(&p).is_err());
    // Sin el JSON de configuración.
    let mut zip = zip::ZipWriter::new(File::create(&p).unwrap());
    zip.start_file("otra-cosa.txt", zip::write::SimpleFileOptions::default()).unwrap();
    zip.finish().unwrap();
    assert!(inspect(&p).unwrap_err().to_string().contains("hivebuzz-config.json"));
    assert!(stage_import(&p, i.dir.path()).is_err());
    assert!(!has_pending(i.dir.path()));
}

#[tokio::test]
async fn path_traversal_and_bad_extensions_in_the_library_are_dropped() {
    let i = Install::new().await;
    let p = i.path("evil.zip");
    let cfg = config(json!({
        "sounds": [
            { "id": "1", "name": "traversal", "file": "../escape.wav", "volume": 100, "createdMs": 0 },
            { "id": "2", "name": "subdir", "file": "a/b.wav", "volume": 100, "createdMs": 0 },
            { "id": "3", "name": "exe", "file": "virus.exe", "volume": 100, "createdMs": 0 },
            { "id": "4", "name": "oculto", "file": ".hidden.wav", "volume": 100, "createdMs": 0 },
            { "id": "5", "name": "bien", "file": "ok.wav", "volume": 100, "createdMs": 0 },
            { "id": "6", "name": "sin archivo", "file": "falta.wav", "volume": 100, "createdMs": 0 },
            { "id": "7", "name": "volumen", "file": "ok.wav", "volume": 250, "createdMs": 0 }
        ],
        "media": [
            { "id": "m1", "name": "svg", "file": "x.svg", "kind": "image", "createdMs": 0 },
            { "id": "m2", "name": "tipo cruzado", "file": "x.mp4", "kind": "image", "createdMs": 0 },
            { "id": "m3", "name": "ok", "file": "x.png", "kind": "image", "createdMs": 0 }
        ]
    }));
    zip_with(
        &p,
        &cfg,
        &[("sounds/ok.wav", b"RIFF"), ("sounds/virus.exe", b"x"), ("media/x.png", b"png"), ("media/x.svg", b"<svg/>"), ("media/x.mp4", b"mp4")],
    );
    let (bundle, skipped) = inspect(&p).unwrap();
    assert_eq!(bundle.sounds.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["5"]);
    assert_eq!(bundle.media.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["m3"]);
    assert_eq!(skipped.len(), 8);
    // Aplicar no escribe nada fuera de las carpetas de la biblioteca.
    stage_import(&p, i.dir.path()).unwrap();
    apply_pending(&i.db, i.dir.path(), 1).await.unwrap();
    assert!(!i.dir.path().join("escape.wav").exists());
    assert!(!i.dir.path().parent().unwrap().join("escape.wav").exists());
    assert!(i.path("sounds/ok.wav").is_file());
    assert!(!i.path("media/x.svg").exists());
}

#[tokio::test]
async fn invalid_rules_and_duplicate_ids_reject_the_whole_file() {
    let i = Install::new().await;
    let p = i.path("r.zip");
    let mut bad = rule("x");
    bad.plan.steps.clear();
    zip_with(&p, &config(json!({ "rules": [bad] })), &[]);
    assert!(inspect(&p).is_err(), "regla sin acciones");
    zip_with(&p, &config(json!({ "rules": [rule("a"), rule("a")] })), &[]);
    assert!(inspect(&p).is_err(), "regla repetida");
    let goal = json!({ "id": "g", "name": "M", "kind": {"type": "likes"}, "target": 0 });
    zip_with(&p, &config(json!({ "goals": [goal] })), &[]);
    assert!(inspect(&p).is_err(), "meta con objetivo 0");
}

#[tokio::test]
async fn only_whitelisted_valid_settings_and_known_overlays_are_applied() {
    let i = Install::new().await;
    let p = i.path("s.zip");
    let cfg = config(json!({
        "settings": { "bot_config": "{\"enabled\":true}", "server_port": "1", "euler_api_key": "secreto", "tts_config": "{roto" },
        "overlays": { "alerts": { "fontSize": 50, "campoFalso": 1 }, "fantasma": { "x": 1 }, "feed": { "fontSize": "mucho" } }
    }));
    zip_with(&p, &cfg, &[]);
    stage_import(&p, i.dir.path()).unwrap();
    let summary = apply_pending(&i.db, i.dir.path(), 1).await.unwrap().unwrap();
    assert_eq!(i.db.get_setting("bot_config").await.unwrap().as_deref(), Some("{\"enabled\":true}"));
    for k in ["server_port", "euler_api_key", "tts_config"] {
        assert_eq!(i.db.get_setting(k).await.unwrap(), None, "{k}");
    }
    assert_eq!(i.db.list_overlay_configs().await.unwrap().len(), 0, "alerts con un campo desconocido, fantasma y feed inválido se descartan");
    assert!(summary.skipped.len() >= 4);
}

#[tokio::test]
async fn a_failed_apply_keeps_the_previous_config_and_sets_the_file_aside() {
    let i = Install::new().await;
    i.db.save_rule(&rule("previa"), 1).await.unwrap();
    let p = i.path("pending-import.zip");
    std::fs::write(&p, b"basura").unwrap();
    let res = apply_pending(&i.db, i.dir.path(), 1).await;
    assert!(res.is_err());
    assert!(!has_pending(i.dir.path()));
    assert!(i.path("pending-import.failed.zip").is_file());
    assert_eq!(i.db.list_rules().await.unwrap().len(), 1, "no se tocó nada");
    assert!(apply_pending(&i.db, i.dir.path(), 1).await.unwrap().is_none());
}

#[tokio::test]
async fn cancel_pending_removes_the_staged_file() {
    let src = populated().await;
    let file = src.path("backup.zip");
    export(&src.db, src.dir.path(), &file, "0.1.0", 1).await.unwrap();
    let dst = Install::new().await;
    stage_import(&file, dst.dir.path()).unwrap();
    cancel_pending(dst.dir.path()).unwrap();
    assert!(!has_pending(dst.dir.path()));
    cancel_pending(dst.dir.path()).unwrap();
}
