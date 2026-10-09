//! Ejecutor `playSound`: reproduce un sonido de la biblioteca.
//!
//! Parámetros: `soundId` (o `soundIds`: se elige uno al azar), `volume` 0–100 (multiplica el
//! volumen propio del sonido, por defecto 100) y `wait` (esperar a que termine antes de seguir
//! con la siguiente acción del plan; por defecto `true`).

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Map, Value};

use super::{opt_bool, opt_number};
use crate::actions::{ActionContext, ActionExecutor, Concurrency};
use crate::audio::{AudioBackend, TAG_SOUND};
use crate::error::{AppError, Result};
use crate::sounds::SoundLibrary;

pub struct PlaySoundExecutor {
    library: Arc<SoundLibrary>,
    audio: Arc<dyn AudioBackend>,
}

impl PlaySoundExecutor {
    pub fn new(library: Arc<SoundLibrary>, audio: Arc<dyn AudioBackend>) -> Self {
        Self { library, audio }
    }
}

fn sound_ids(params: &Map<String, Value>) -> Vec<String> {
    let mut ids: Vec<String> = params
        .get("soundIds")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    if let Some(one) = params.get("soundId").and_then(Value::as_str) {
        ids.push(one.to_string());
    }
    ids.retain(|s| !s.trim().is_empty());
    ids
}

#[async_trait]
impl ActionExecutor for PlaySoundExecutor {
    fn kind(&self) -> &'static str {
        "playSound"
    }

    // Los sonidos suenan en paralelo (se mezclan).
    fn concurrency(&self) -> Concurrency {
        Concurrency::Parallel
    }

    fn validate(&self, params: &Map<String, Value>) -> Result<()> {
        if sound_ids(params).is_empty() {
            return Err(AppError::Invalid("elige al menos un sonido".into()));
        }
        opt_number(params, "volume", 0.0, 100.0)?;
        opt_bool(params, "wait", true)?;
        Ok(())
    }

    async fn execute(&self, _ctx: &ActionContext, params: &Map<String, Value>) -> Result<()> {
        let ids = sound_ids(params);
        let id = ids
            .get(rand::random_range(0..ids.len().max(1)))
            .ok_or_else(|| AppError::Invalid("la acción no tiene sonido".into()))?;
        let sound = self
            .library
            .get(id)
            .await?
            .ok_or_else(|| AppError::Invalid(format!("el sonido «{id}» ya no existe en la biblioteca")))?;

        let action_volume = opt_number(params, "volume", 0.0, 100.0)?.unwrap_or(100.0);
        #[allow(clippy::cast_possible_truncation)]
        let volume = (f64::from(sound.volume) / 100.0 * action_volume / 100.0) as f32;
        let path = self.library.path_of(&sound);

        if opt_bool(params, "wait", true)? {
            self.audio.play(path, volume, TAG_SOUND).await
        } else {
            let audio = Arc::clone(&self.audio);
            tokio::spawn(async move {
                if let Err(e) = audio.play(path, volume, TAG_SOUND).await {
                    tracing::warn!(error = %e, "falló un sonido sin espera");
                }
            });
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use super::*;
    use crate::actions::clock::AppClock;
    use crate::audio::testing::FakeAudio;
    use crate::db::Db;
    use crate::sounds::testing::silent_wav;

    struct Rig {
        exec: PlaySoundExecutor,
        library: Arc<SoundLibrary>,
        audio: Arc<FakeAudio>,
        tmp: tempfile::TempDir,
    }

    async fn rig(fail: bool) -> Rig {
        let tmp = tempfile::tempdir().expect("tmp");
        let db = Db::open_memory().await.expect("db");
        let library = Arc::new(SoundLibrary::new(db, tmp.path().join("s"), Arc::new(AppClock::new())).expect("lib"));
        let audio = Arc::new(FakeAudio { fail, duration: Duration::from_millis(30), ..Default::default() });
        Rig { exec: PlaySoundExecutor::new(library.clone(), audio.clone()), library, audio, tmp }
    }

    async fn import(r: &Rig, name: &str, volume: u8) -> String {
        let p = r.tmp.path().join(format!("{name}.wav"));
        std::fs::write(&p, silent_wav(100)).expect("write");
        let s = r.library.import(&p, None).await.expect("import");
        r.library.update(&s.id, None, Some(volume)).await.expect("vol");
        s.id
    }

    fn ctx() -> ActionContext {
        ActionContext { rule_id: "r".into(), vars: Default::default() }
    }

    fn params(v: Value) -> Map<String, Value> {
        v.as_object().cloned().expect("objeto")
    }

    #[tokio::test]
    async fn plays_the_library_file_with_combined_volume() {
        let r = rig(false).await;
        let id = import(&r, "a", 50).await;
        r.exec.execute(&ctx(), &params(json!({"soundId": id, "volume": 50}))).await.expect("play");
        let played = r.audio.played.lock().expect("lock").clone();
        assert_eq!(played.len(), 1);
        assert!(played[0].0.ends_with(format!("{id}.wav")));
        assert!((played[0].1 - 0.25).abs() < 1e-6, "50% del sonido × 50% de la acción = 25%");
    }

    #[tokio::test]
    async fn default_volume_is_the_sounds_own_volume() {
        let r = rig(false).await;
        let id = import(&r, "a", 80).await;
        r.exec.execute(&ctx(), &params(json!({"soundId": id}))).await.expect("play");
        assert!((r.audio.played.lock().expect("lock")[0].1 - 0.8).abs() < 1e-6);
    }

    #[tokio::test]
    async fn picks_one_of_several_sounds() {
        let r = rig(false).await;
        let (a, b) = (import(&r, "a", 100).await, import(&r, "b", 100).await);
        for _ in 0..20 {
            r.exec.execute(&ctx(), &params(json!({"soundIds": [a, b]}))).await.expect("play");
        }
        let played = r.audio.played.lock().expect("lock").clone();
        let used_a = played.iter().any(|(p, _)| p.to_string_lossy().contains(&a));
        let used_b = played.iter().any(|(p, _)| p.to_string_lossy().contains(&b));
        assert!(used_a && used_b, "en 20 tiradas debían salir ambos");
    }

    #[tokio::test]
    async fn missing_sound_and_audio_failures_are_errors() {
        let r = rig(false).await;
        assert!(r.exec.execute(&ctx(), &params(json!({"soundId": "fantasma"}))).await.is_err());
        let r = rig(true).await;
        let id = import(&r, "a", 100).await;
        assert!(r.exec.execute(&ctx(), &params(json!({"soundId": id}))).await.is_err());
    }

    #[tokio::test]
    async fn wait_false_returns_before_the_sound_ends() {
        let r = rig(false).await;
        let id = import(&r, "a", 100).await;
        let t = std::time::Instant::now();
        r.exec.execute(&ctx(), &params(json!({"soundId": id, "wait": false}))).await.expect("play");
        assert!(t.elapsed() < Duration::from_millis(25));
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert_eq!(r.audio.played.lock().expect("lock").len(), 1);
    }

    #[tokio::test]
    async fn validation_rules() {
        let r = rig(false).await;
        let v = |p: Value| r.exec.validate(&params(p));
        assert!(v(json!({"soundId": "x"})).is_ok());
        assert!(v(json!({"soundIds": ["x", "y"], "volume": 0, "wait": false})).is_ok());
        assert!(v(json!({})).is_err());
        assert!(v(json!({"soundId": "  "})).is_err());
        assert!(v(json!({"soundId": "x", "volume": 101})).is_err());
        assert!(v(json!({"soundId": "x", "volume": "alto"})).is_err());
        assert!(v(json!({"soundId": "x", "wait": "si"})).is_err());
    }
}
