use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;

use super::*;
use crate::actions::clock::AppClock;
use crate::actions::queue::QueueConfig;
use crate::actions::store::MemoryJobStore;
use crate::actions::ExecutorRegistry;
use crate::audio::testing::FakeAudio;
use crate::events::testing::sample_event;
use crate::events::{Chat, EventType};
use crate::executors::tts::TtsExecutor;
use crate::tts::policy::VoiceMode;

#[derive(Debug, Clone, PartialEq)]
struct Call {
    voice: String,
    text: String,
    rate: f64,
}

struct FakeEngine {
    id: &'static str,
    voices: Vec<&'static str>,
    calls: Arc<Mutex<Vec<Call>>>,
    listings: Arc<AtomicUsize>,
    fail: bool,
}

impl FakeEngine {
    fn new(id: &'static str, voices: &[&'static str]) -> Self {
        Self { id, voices: voices.to_vec(), calls: Arc::default(), listings: Arc::default(), fail: false }
    }
}

#[async_trait]
impl TtsEngine for FakeEngine {
    fn id(&self) -> &'static str {
        self.id
    }
    async fn voices(&self) -> Vec<VoiceInfo> {
        self.listings.fetch_add(1, Ordering::SeqCst);
        self.voices
            .iter()
            .map(|v| VoiceInfo { id: format!("{}:{v}", self.id), engine: self.id.into(), name: (*v).into(), lang: None })
            .collect()
    }
    async fn synthesize(&self, req: &SynthRequest) -> Result<PathBuf> {
        self.calls.lock().expect("lock").push(Call { voice: req.voice.clone(), text: req.text.clone(), rate: req.rate });
        if self.fail {
            return Err(AppError::Invalid("el motor falló".into()));
        }
        let p = req.out_dir.join(format!("{}.wav", uuid::Uuid::new_v4()));
        std::fs::write(&p, b"audio").expect("write");
        Ok(p)
    }
}

struct Rig {
    svc: Arc<TtsService>,
    audio: Arc<FakeAudio>,
    piper: Arc<FakeEngine>,
    sapi: Arc<FakeEngine>,
    db: Db,
    cache: tempfile::TempDir,
    bus: EventBus,
    paths: Arc<RwLock<PiperPaths>>,
}

async fn rig_with(piper: FakeEngine, audio_fail: bool) -> Rig {
    let cache = tempfile::tempdir().expect("tmp");
    let db = Db::open_memory().await.expect("db");
    let audio = Arc::new(FakeAudio { fail: audio_fail, ..Default::default() });
    let piper = Arc::new(piper);
    let sapi = Arc::new(FakeEngine::new("sapi", &["z"]));
    let paths = Arc::new(RwLock::new(PiperPaths::default()));
    let clock: Arc<dyn Clock> = Arc::new(AppClock::new());
    let svc = TtsService::new(TtsDeps {
        engines: vec![piper.clone(), sapi.clone()],
        audio: audio.clone(),
        clock: clock.clone(),
        db: db.clone(),
        cache_dir: cache.path().join("tts"),
        piper_paths: paths.clone(),
        default_piper: PiperPaths { exe: "/app/piper".into(), voices_dir: "/app/voices".into() },
        mic: None,
    })
    .expect("svc");

    let mut registry = ExecutorRegistry::new();
    registry.register(Arc::new(TtsExecutor::new(svc.clone())));
    let queue = Arc::new(ActionQueue::start(registry, Arc::new(MemoryJobStore::default()), clock, QueueConfig::default()));
    svc.attach_queue(queue);
    let bus = EventBus::new(32);
    svc.spawn(&bus);
    Rig { svc, audio, piper, sapi, db, cache, bus, paths }
}

async fn rig() -> Rig {
    rig_with(FakeEngine::new("piper", &["a", "b"]), false).await
}

fn cache_files(r: &Rig) -> usize {
    std::fs::read_dir(r.cache.path().join("tts")).expect("dir").count()
}

fn chat(text: &str) -> crate::events::LiveEvent {
    let mut e = sample_event("c1");
    e.chat = Some(Chat { text: text.into(), emotes: None });
    e
}

async fn settle() {
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn speaks_with_the_first_available_voice_and_cleans_up_the_audio_file() {
    let r = rig().await;
    r.svc.speak("Hola 🔥 mundo", &Vars::new(), None, SpeakOptions::default()).await.expect("speak");
    assert_eq!(*r.piper.calls.lock().expect("lock"), [Call { voice: "a".into(), text: "Hola mundo".into(), rate: 1.0 }]);
    let tags = r.audio.tags.lock().expect("lock").clone();
    assert_eq!(tags, [TAG_TTS]);
    assert!((r.audio.played.lock().expect("lock")[0].1 - 1.0).abs() < 1e-6);
    assert_eq!(cache_files(&r), 0, "el audio temporal se borra");
}

#[tokio::test]
async fn rate_and_volume_combine_config_and_action() {
    let r = rig().await;
    r.svc.set_config(TtsConfig { volume: 50, rate: 1.5, ..Default::default() }).await.expect("cfg");
    r.svc.speak("hola", &Vars::new(), None, SpeakOptions { rate: None, volume: Some(50.0) }).await.expect("speak");
    r.svc.speak("hola", &Vars::new(), None, SpeakOptions { rate: Some(0.8), volume: None }).await.expect("speak");
    let calls = r.piper.calls.lock().expect("lock").clone();
    assert_eq!((calls[0].rate, calls[1].rate), (1.5, 0.8));
    let played = r.audio.played.lock().expect("lock").clone();
    assert!((played[0].1 - 0.25).abs() < 1e-6, "50% × 50%");
    assert!((played[1].1 - 0.5).abs() < 1e-6, "50% × 100%");
}

#[tokio::test]
async fn filtered_text_is_silently_dropped() {
    let r = rig().await;
    for text in ["https://spam.com", "🔥🔥", "eres un pendejo"] {
        r.svc.speak(text, &Vars::new(), None, SpeakOptions::default()).await.expect("no es un error");
    }
    assert!(r.piper.calls.lock().expect("lock").is_empty());
    assert!(r.audio.played.lock().expect("lock").is_empty());
}

#[tokio::test]
async fn an_explicit_voice_picks_another_engine() {
    let r = rig().await;
    r.svc.speak("hola", &Vars::new(), Some("sapi:z"), SpeakOptions::default()).await.expect("speak");
    assert_eq!(r.sapi.calls.lock().expect("lock").len(), 1);
    assert!(r.piper.calls.lock().expect("lock").is_empty());
}

#[tokio::test]
async fn role_based_voices_use_the_event_variables() {
    let r = rig().await;
    r.svc
        .set_config(TtsConfig {
            voice_mode: VoiceMode::ByRole,
            default_voice: Some("piper:a".into()),
            role_voices: crate::tts::policy::RoleVoices { moderator: Some("sapi:z".into()), ..Default::default() },
            ..Default::default()
        })
        .await
        .expect("cfg");
    let mod_vars: Vars = [("ismoderator".to_string(), "true".to_string())].into();
    r.svc.speak("hola", &mod_vars, None, SpeakOptions::default()).await.expect("speak");
    r.svc.speak("hola", &Vars::new(), None, SpeakOptions::default()).await.expect("speak");
    assert_eq!(r.sapi.calls.lock().expect("lock").len(), 1);
    assert_eq!(r.piper.calls.lock().expect("lock").len(), 1);
}

#[tokio::test]
async fn no_voices_is_a_clear_error() {
    let cache = tempfile::tempdir().expect("tmp");
    let svc = TtsService::new(TtsDeps {
        engines: vec![Arc::new(FakeEngine::new("piper", &[]))],
        audio: Arc::new(FakeAudio::default()),
        clock: Arc::new(AppClock::new()),
        db: Db::open_memory().await.expect("db"),
        cache_dir: cache.path().into(),
        piper_paths: Arc::default(),
        default_piper: PiperPaths::default(),
        mic: None,
    })
    .expect("svc");
    let e = svc.speak("hola", &Vars::new(), None, SpeakOptions::default()).await.expect_err("sin voces");
    assert!(e.to_string().contains("no hay voces"));
}

#[tokio::test]
async fn engine_and_audio_failures_do_not_leave_files_behind() {
    let mut bad = FakeEngine::new("piper", &["a"]);
    bad.fail = true;
    let r = rig_with(bad, false).await;
    assert!(r.svc.speak("hola", &Vars::new(), None, SpeakOptions::default()).await.is_err());
    assert_eq!(cache_files(&r), 0);

    let r = rig_with(FakeEngine::new("piper", &["a"]), true).await;
    assert!(r.svc.speak("hola", &Vars::new(), None, SpeakOptions::default()).await.is_err());
    assert_eq!(cache_files(&r), 0, "aunque falle la reproducción, se borra el audio");
}

#[tokio::test]
async fn skip_stops_only_the_tts_playback() {
    let r = rig().await;
    r.svc.skip();
    assert_eq!(*r.audio.stopped.lock().expect("lock"), ["tts"]);
}

#[tokio::test]
async fn config_is_saved_sanitized_applied_and_reloaded() {
    let r = rig().await;
    let saved = r
        .svc
        .set_config(TtsConfig { rate: 10.0, command: Some("!TTS".into()), piper_path: Some("C:/mi/piper.exe".into()), ..Default::default() })
        .await
        .expect("cfg");
    assert_eq!((saved.rate, saved.command.as_deref()), (2.0, Some("tts")));
    assert_eq!(r.paths.read().expect("lock").exe, PathBuf::from("C:/mi/piper.exe"));
    assert_eq!(r.paths.read().expect("lock").voices_dir, PathBuf::from("/app/voices"), "sin ruta propia usa la de la app");

    r.svc.set_config(TtsConfig::default()).await.expect("cfg");
    assert_eq!(r.paths.read().expect("lock").exe, PathBuf::from("/app/piper"));

    r.svc.set_config(saved.clone()).await.expect("cfg");
    r.svc.set_config(TtsConfig { enabled: true, ..Default::default() }).await.expect("cfg");
    r.svc.load_config().await.expect("load");
    assert!(r.svc.config().enabled);
}

#[tokio::test]
async fn corrupt_stored_config_falls_back_to_defaults() {
    let r = rig().await;
    r.db.set_setting(KEY_TTS_CONFIG, "{no es json").await.expect("set");
    r.svc.load_config().await.expect("load no falla");
    assert_eq!(r.svc.config(), TtsConfig::default());
}

#[tokio::test]
async fn voice_listing_is_cached_until_invalidated() {
    let r = rig().await;
    r.svc.voices().await;
    r.svc.voices().await;
    assert_eq!(r.piper.listings.load(Ordering::SeqCst), 1);
    r.svc.invalidate_voices();
    let v = r.svc.voices().await;
    assert_eq!(r.piper.listings.load(Ordering::SeqCst), 2);
    assert_eq!(v.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(), ["piper:a", "piper:b", "sapi:z"]);
}

#[tokio::test]
async fn stale_audio_from_a_previous_session_is_removed_on_startup() {
    let cache = tempfile::tempdir().expect("tmp");
    let dir = cache.path().join("tts");
    std::fs::create_dir_all(&dir).expect("dir");
    std::fs::write(dir.join("viejo.wav"), b"x").expect("write");
    let _svc = TtsService::new(TtsDeps {
        engines: vec![],
        audio: Arc::new(FakeAudio::default()),
        clock: Arc::new(AppClock::new()),
        db: Db::open_memory().await.expect("db"),
        cache_dir: dir.clone(),
        piper_paths: Arc::default(),
        default_piper: PiperPaths::default(),
        mic: None,
    })
    .expect("svc");
    assert_eq!(std::fs::read_dir(dir).expect("dir").count(), 0);
}

#[tokio::test]
async fn chat_is_read_through_the_queue_using_the_template() {
    let r = rig().await;
    r.svc.set_config(TtsConfig { enabled: true, user_cooldown_ms: 0, ..Default::default() }).await.expect("cfg");
    r.bus.publish(chat("hola a todos"));
    settle().await;
    let calls = r.piper.calls.lock().expect("lock").clone();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert_eq!(calls[0].text, "Ana dice: hola a todos");
}

#[tokio::test]
async fn chat_is_ignored_when_disabled_and_other_events_never_speak() {
    let r = rig().await;
    r.bus.publish(chat("hola"));
    let mut follow = sample_event("f1");
    follow.kind = EventType::Follow;
    follow.chat = None;
    r.svc.set_config(TtsConfig { enabled: true, ..Default::default() }).await.expect("cfg");
    r.bus.publish(follow);
    settle().await;
    assert!(r.piper.calls.lock().expect("lock").is_empty());
}

#[tokio::test]
async fn command_mode_reads_only_what_follows_the_command() {
    let r = rig().await;
    r.svc
        .set_config(TtsConfig { enabled: true, command: Some("tts".into()), user_cooldown_ms: 0, ..Default::default() })
        .await
        .expect("cfg");
    r.bus.publish(chat("charla normal"));
    let mut cmd = chat("!tts buenas noches");
    cmd.id = "c2".into();
    r.bus.publish(cmd);
    settle().await;
    let calls = r.piper.calls.lock().expect("lock").clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].text, "Ana dice: buenas noches");
}

// ---- No hablar encima del streamer (micrófono) ----

mod mic_guard {
    use super::*;
    use crate::mic::testing::FakeMic;
    use crate::tts::policy::{MicGuard, MicGuardMode};

    struct MicRig {
        svc: Arc<TtsService>,
        audio: Arc<FakeAudio>,
        mic: Arc<FakeMic>,
        _cache: tempfile::TempDir,
    }

    async fn rig(mode: MicGuardMode, enabled: bool) -> MicRig {
        let cache = tempfile::tempdir().expect("tmp");
        let audio = Arc::new(FakeAudio { duration: Duration::from_millis(400), ..Default::default() });
        let mic = FakeMic::new();
        let svc = TtsService::new(TtsDeps {
            engines: vec![Arc::new(FakeEngine::new("piper", &["a"]))],
            audio: audio.clone(),
            clock: Arc::new(AppClock::new()),
            db: Db::open_memory().await.expect("db"),
            cache_dir: cache.path().join("tts"),
            piper_paths: Arc::new(RwLock::new(PiperPaths::default())),
            default_piper: PiperPaths::default(),
            mic: Some(mic.clone()),
        })
        .expect("svc");
        let guard = MicGuard { enabled, mode, threshold_db: -35.0, ..MicGuard::default() };
        svc.set_config(TtsConfig { mic_guard: guard, ..TtsConfig::default() }).await.expect("cfg");
        MicRig { svc, audio, mic, _cache: cache }
    }

    fn speak(r: &MicRig) -> tokio::task::JoinHandle<Result<()>> {
        let svc = r.svc.clone();
        tokio::spawn(async move { svc.speak("hola", &Vars::new(), None, SpeakOptions::default()).await })
    }

    fn log(r: &MicRig) -> Vec<String> {
        r.audio.stopped.lock().expect("lock").clone()
    }

    async fn ms(n: u64) {
        tokio::time::sleep(Duration::from_millis(n)).await;
    }

    #[tokio::test]
    async fn enabling_it_starts_the_microphone_with_the_configured_sensitivity() {
        let r = rig(MicGuardMode::RepeatWord, true).await;
        let last = r.mic.configured.lock().expect("lock").last().cloned().flatten().expect("escuchando");
        assert_eq!((last.threshold_db, last.hold_ms, last.device), (-35.0, 800, None));
        r.svc.set_config(TtsConfig::default()).await.expect("cfg");
        assert_eq!(r.mic.configured.lock().expect("lock").last().cloned(), Some(None), "apagado deja de escuchar");
    }

    #[tokio::test]
    async fn talking_pauses_the_reading_and_it_repeats_the_cut_word() {
        let r = rig(MicGuardMode::RepeatWord, true).await;
        let h = speak(&r);
        ms(100).await;
        r.mic.set_speaking(true);
        ms(50).await;
        r.mic.set_speaking(false);
        h.await.expect("join").expect("lee");
        assert_eq!(log(&r), ["pause:tts", "resume:tts:1200"]);
    }

    #[tokio::test]
    async fn repeat_message_mode_starts_over() {
        let r = rig(MicGuardMode::RepeatMessage, true).await;
        let h = speak(&r);
        ms(100).await;
        r.mic.set_speaking(true);
        ms(50).await;
        r.mic.set_speaking(false);
        h.await.expect("join").expect("lee");
        assert_eq!(log(&r), ["pause:tts", "resume:tts:start"]);
    }

    #[tokio::test]
    async fn skip_mode_cuts_the_message() {
        let r = rig(MicGuardMode::Skip, true).await;
        let h = speak(&r);
        ms(100).await;
        r.mic.set_speaking(true);
        h.await.expect("join").expect("lee");
        assert_eq!(log(&r), ["tts"]);
    }

    #[tokio::test]
    async fn a_new_reading_waits_until_the_streamer_stops_talking() {
        let r = rig(MicGuardMode::RepeatWord, true).await;
        r.mic.set_speaking(true);
        let h = speak(&r);
        ms(150).await;
        assert!(r.audio.played.lock().expect("lock").is_empty(), "no empieza encima");
        r.mic.set_speaking(false);
        h.await.expect("join").expect("lee");
        assert_eq!(r.audio.played.lock().expect("lock").len(), 1);
        assert!(log(&r).is_empty(), "no hizo falta pausar");
    }

    #[tokio::test]
    async fn disabled_it_ignores_the_microphone() {
        let r = rig(MicGuardMode::RepeatWord, false).await;
        r.mic.set_speaking(true);
        speak(&r).await.expect("join").expect("lee");
        assert_eq!(r.audio.played.lock().expect("lock").len(), 1);
        assert!(log(&r).is_empty());
    }
}
