//! Ejecutor `overlayAlert`: muestra una alerta en el overlay `/overlay/alerts`.
//!
//! Parámetros (los textos admiten variables como `{nickname}`):
//! `title`, `text`, `mediaId` (de la biblioteca de medios), `imageUrl` (p. ej. `{giftimage}`),
//! `showAvatar`, `durationMs` (500–60000, por defecto 5000) y `wait` (esperar a que termine la
//! alerta antes de seguir con el plan; por defecto `true`).
//!
//! Sonido opcional (`soundId`/`soundIds`, `soundVolume` 0–100): suena justo cuando el overlay empieza a
//! mostrar la alerta (el overlay avisa por WebSocket cuando su vídeo está reproduciéndose), así imagen y
//! audio salen a la vez aunque haya otras alertas en cola. Si el overlay no está abierto, suena tras unos
//! segundos de todos modos.
//!
//! Pantalla (`screen`, 1 por defecto): cada pantalla es un overlay distinto con su estilo y su cola, así que
//! alertas de pantallas distintas se ven a la vez. Si la pantalla ya no existe se usa la 1.
//! Sonido del vídeo (`videoVolume`, 0–100, por defecto 100): si el vídeo de la biblioteca trae audio, suena en
//! el overlay (0 lo silencia). En OBS, el audio sale por el escritorio o, si activas «Controlar audio mediante
//! OBS» en la fuente Navegador, por su propio canal del mezclador.
//! Fondo del vídeo (`keyMode`): `black` quita el fondo negro y `green` el verde, para vídeos sin canal alfa.
//! Ajustes de esta alerta sobre el estilo de su pantalla: `animation` (`pop`/`slide`/`fade`) y `scale` (40–250 %).
//!
//! Dentro de una pantalla las alertas se muestran de una en una: el overlay no las superpone.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Map, Value};

use super::sound::{resolve, sound_ids};
use super::{opt_bool, opt_number};
use crate::actions::{ActionContext, ActionExecutor, Concurrency};
use crate::audio::{AudioBackend, TAG_SOUND};
use crate::error::{AppError, Result};
use crate::media::MediaLibrary;
use crate::overlay::OverlayHub;
use crate::overlay_config::schema::{alert_screen_id, MAX_ALERT_SCREENS};
use crate::overlay_config::OverlayConfigService;
use crate::sounds::SoundLibrary;

const DEFAULT_DURATION_MS: f64 = 5000.0;
/// Cuánto se espera el aviso del overlay antes de reproducir el sonido igualmente.
const ACK_TIMEOUT: Duration = Duration::from_secs(15);

pub struct OverlayAlertExecutor {
    media: Arc<MediaLibrary>,
    hub: OverlayHub,
    sounds: Arc<SoundLibrary>,
    audio: Arc<dyn AudioBackend>,
    screens: OverlayConfigService,
}

impl OverlayAlertExecutor {
    pub fn new(media: Arc<MediaLibrary>, hub: OverlayHub, sounds: Arc<SoundLibrary>, audio: Arc<dyn AudioBackend>, screens: OverlayConfigService) -> Self {
        Self { media, hub, sounds, audio, screens }
    }
}

/// Pantalla pedida por la acción (1 si no dice nada o no es válida).
fn wanted_screen(params: &Map<String, Value>) -> usize {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    params.get("screen").and_then(Value::as_f64).filter(|n| n.is_finite() && *n >= 1.0).map_or(1, |n| n as usize)
}

fn text_param<'a>(params: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    params.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

#[async_trait]
impl ActionExecutor for OverlayAlertExecutor {
    fn kind(&self) -> &'static str {
        "overlayAlert"
    }

    fn concurrency(&self) -> Concurrency {
        Concurrency::Serial("alerts")
    }

    /// Una cola por pantalla: las alertas de pantallas distintas no se esperan entre sí.
    fn concurrency_for(&self, params: &Map<String, Value>) -> Concurrency {
        Concurrency::Serial(alert_screen_id(wanted_screen(params)).unwrap_or("alerts"))
    }

    fn validate(&self, params: &Map<String, Value>) -> Result<()> {
        let has_content = ["title", "text", "mediaId", "imageUrl"].iter().any(|k| text_param(params, k).is_some()) || !sound_ids(params).is_empty();
        if !has_content {
            return Err(AppError::Invalid("la alerta necesita título, texto o un medio".into()));
        }
        opt_number(params, "durationMs", 500.0, 60_000.0)?;
        opt_number(params, "soundVolume", 0.0, 100.0)?;
        #[allow(clippy::cast_precision_loss)]
        opt_number(params, "screen", 1.0, MAX_ALERT_SCREENS as f64)?;
        opt_number(params, "scale", 40.0, 250.0)?;
        opt_number(params, "videoVolume", 0.0, 100.0)?;
        if let Some(k) = text_param(params, "keyMode") {
            if !["black", "green"].contains(&k) {
                return Err(AppError::Invalid("fondo de vídeo desconocido".into()));
            }
        }
        if let Some(a) = text_param(params, "animation") {
            if !["pop", "slide", "fade"].contains(&a) {
                return Err(AppError::Invalid("animación desconocida".into()));
            }
        }
        opt_bool(params, "wait", true)?;
        opt_bool(params, "showAvatar", false)?;
        Ok(())
    }

    async fn execute(&self, ctx: &ActionContext, params: &Map<String, Value>) -> Result<()> {
        let duration = opt_number(params, "durationMs", 500.0, 60_000.0)?.unwrap_or(DEFAULT_DURATION_MS);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let duration_ms = duration as u64;

        let media = match text_param(params, "mediaId") {
            Some(id) => match self.media.get(id).await? {
                Some(m) => Some(json!({ "kind": m.kind, "url": m.url_path() })),
                None => {
                    // Una alerta sin su imagen sigue siendo útil: se muestra solo el texto.
                    tracing::warn!(media = %id, "el medio de la alerta ya no existe");
                    None
                }
            },
            None => None,
        };
        let avatar = if opt_bool(params, "showAvatar", false)? {
            ctx.vars.get("avatar").filter(|a| !a.is_empty()).cloned()
        } else {
            None
        };

        // Un sonido que ya no existe no debe impedir la alerta: se avisa y sale sin sonido.
        let sound = if sound_ids(params).is_empty() {
            None
        } else {
            match resolve(&self.sounds, params, "soundVolume").await {
                Ok(s) => Some(s),
                Err(e) => {
                    tracing::warn!(error = %e, "la alerta sale sin sonido");
                    None
                }
            }
        };

        let channel = self.screens.resolve_screen(wanted_screen(params)).await?;
        let id = uuid::Uuid::new_v4().to_string();
        // Se pide el aviso antes de publicar: así no puede llegar antes de que alguien lo espere.
        let shown = sound.as_ref().map(|_| self.hub.expect_ack(&id));

        let payload = json!({
            "id": id,
            "sync": sound.is_some(),
            "title": text_param(params, "title").map(|t| ctx.render(t)),
            "text": text_param(params, "text").map(|t| ctx.render(t)),
            "imageUrl": text_param(params, "imageUrl").map(|t| ctx.render(t)).filter(|u| !u.is_empty()),
            "media": media,
            "avatar": avatar,
            "durationMs": duration_ms,
            "animation": text_param(params, "animation"),
            "keyMode": text_param(params, "keyMode"),
            "videoVolume": opt_number(params, "videoVolume", 0.0, 100.0)?.unwrap_or(100.0),
            "scale": opt_number(params, "scale", 40.0, 250.0)?,
        });
        self.hub.publish(channel, payload);

        if let (Some((path, volume)), Some(shown)) = (sound, shown) {
            let (audio, hub) = (Arc::clone(&self.audio), self.hub.clone());
            tokio::spawn(async move {
                if tokio::time::timeout(ACK_TIMEOUT, shown).await.is_err() {
                    hub.forget_ack(&id);
                }
                if let Err(e) = audio.play(path, volume, TAG_SOUND).await {
                    tracing::warn!(error = %e, "falló el sonido de la alerta");
                }
            });
        }

        if opt_bool(params, "wait", true)? {
            tokio::time::sleep(Duration::from_millis(duration_ms)).await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::actions::clock::AppClock;
    use crate::audio::testing::FakeAudio;
    use crate::db::Db;
    use crate::sounds::testing::silent_wav;

    struct Rig {
        exec: OverlayAlertExecutor,
        hub: OverlayHub,
        media: Arc<MediaLibrary>,
        sounds: Arc<SoundLibrary>,
        audio: Arc<FakeAudio>,
        screens: OverlayConfigService,
        tmp: tempfile::TempDir,
    }

    async fn rig() -> Rig {
        let tmp = tempfile::tempdir().expect("tmp");
        let db = Db::open_memory().await.expect("db");
        let clock = Arc::new(AppClock::new());
        let media = Arc::new(MediaLibrary::new(db.clone(), tmp.path().join("m"), clock.clone()).expect("lib"));
        let sounds = Arc::new(SoundLibrary::new(db.clone(), tmp.path().join("s"), clock).expect("sounds"));
        let audio = Arc::new(FakeAudio { duration: Duration::from_millis(10), ..Default::default() });
        let hub = OverlayHub::new(16);
        let screens = OverlayConfigService::new(db, hub.clone());
        Rig { exec: OverlayAlertExecutor::new(media.clone(), hub.clone(), sounds.clone(), audio.clone(), screens.clone()), hub, media, sounds, audio, screens, tmp }
    }

    async fn import_sound(r: &Rig) -> String {
        let p = r.tmp.path().join("ding.wav");
        std::fs::write(&p, silent_wav(100)).expect("write");
        r.sounds.import(&p, None).await.expect("import").id
    }

    fn ctx() -> ActionContext {
        ActionContext {
            rule_id: "r".into(),
            vars: [
                ("nickname".to_string(), "Ana".to_string()),
                ("gift".to_string(), "Rose".to_string()),
                ("avatar".to_string(), "https://x/a.png".to_string()),
                ("giftimage".to_string(), "https://x/rose.png".to_string()),
            ]
            .into(),
        }
    }

    fn params(v: Value) -> Map<String, Value> {
        v.as_object().cloned().expect("objeto")
    }

    #[tokio::test]
    async fn publishes_a_rendered_alert_to_the_alerts_channel() {
        let r = rig().await;
        let mut rx = r.hub.subscribe();
        let p = params(json!({
            "title": "{nickname}", "text": "envió una {gift}", "imageUrl": "{giftimage}",
            "showAvatar": true, "wait": false, "durationMs": 3000
        }));
        r.exec.execute(&ctx(), &p).await.expect("exec");
        let m = rx.recv().await.expect("msg");
        assert_eq!(m.channel, "alerts");
        assert_eq!(m.data["title"], "Ana");
        assert_eq!(m.data["text"], "envió una Rose");
        assert_eq!(m.data["imageUrl"], "https://x/rose.png");
        assert_eq!(m.data["avatar"], "https://x/a.png");
        assert_eq!(m.data["durationMs"], 3000);
        assert!(m.data["media"].is_null());
        assert!(!m.data["id"].as_str().unwrap_or("").is_empty());
    }

    #[tokio::test]
    async fn includes_library_media_and_survives_a_missing_one() {
        let r = rig().await;
        let src = r.tmp.path().join("fuego.gif");
        std::fs::write(&src, b"GIF89a").expect("write");
        let m = r.media.import(&src, None).await.expect("import");
        let mut rx = r.hub.subscribe();

        r.exec.execute(&ctx(), &params(json!({"mediaId": m.id, "wait": false}))).await.expect("exec");
        let got = rx.recv().await.expect("msg");
        assert_eq!(got.data["media"]["kind"], "image");
        assert_eq!(got.data["media"]["url"], m.url_path());

        r.exec.execute(&ctx(), &params(json!({"mediaId": "fantasma", "text": "hola", "wait": false}))).await.expect("exec");
        let got = rx.recv().await.expect("msg");
        assert!(got.data["media"].is_null());
        assert_eq!(got.data["text"], "hola");
    }

    #[tokio::test]
    async fn avatar_is_only_sent_when_requested() {
        let r = rig().await;
        let mut rx = r.hub.subscribe();
        r.exec.execute(&ctx(), &params(json!({"text": "x", "wait": false}))).await.expect("exec");
        assert!(rx.recv().await.expect("msg").data["avatar"].is_null());
    }

    #[tokio::test]
    async fn wait_blocks_for_the_duration_and_wait_false_does_not() {
        let r = rig().await;
        let t = Instant::now();
        r.exec.execute(&ctx(), &params(json!({"text": "x", "wait": false}))).await.expect("exec");
        assert!(t.elapsed() < Duration::from_millis(100));
        let t = Instant::now();
        r.exec.execute(&ctx(), &params(json!({"text": "x", "durationMs": 500}))).await.expect("exec");
        assert!(t.elapsed() >= Duration::from_millis(450));
    }

    #[tokio::test]
    async fn validation_rules() {
        let r = rig().await;
        let v = |p: Value| r.exec.validate(&params(p));
        assert!(v(json!({"text": "hola"})).is_ok());
        assert!(v(json!({"mediaId": "m"})).is_ok());
        assert!(v(json!({"imageUrl": "{giftimage}"})).is_ok());
        assert!(v(json!({})).is_err());
        assert!(v(json!({"text": "  "})).is_err());
        assert!(v(json!({"text": "x", "durationMs": 100})).is_err());
        assert!(v(json!({"text": "x", "durationMs": 70000})).is_err());
        assert!(v(json!({"text": "x", "wait": 1})).is_err());
    }

    async fn plays(r: &Rig) -> usize {
        r.audio.played.lock().expect("lock").len()
    }

    #[tokio::test]
    async fn the_sound_waits_for_the_overlay_to_confirm_it_is_showing_the_alert() {
        let r = rig().await;
        let sound = import_sound(&r).await;
        let mut rx = r.hub.subscribe();
        r.exec.execute(&ctx(), &params(json!({"text": "hola", "soundId": sound, "wait": false}))).await.expect("exec");
        let m = rx.recv().await.expect("msg");
        assert_eq!(m.data["sync"], true);
        // Hasta que el overlay no avisa, no suena nada.
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert_eq!(plays(&r).await, 0);
        r.hub.ack(m.data["id"].as_str().expect("id"));
        for _ in 0..50 {
            if plays(&r).await == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(plays(&r).await, 1);
    }

    #[tokio::test]
    async fn an_alert_without_sound_does_not_wait_for_anything() {
        let r = rig().await;
        let mut rx = r.hub.subscribe();
        r.exec.execute(&ctx(), &params(json!({"text": "hola", "wait": false}))).await.expect("exec");
        assert_eq!(rx.recv().await.expect("msg").data["sync"], false);
        assert_eq!(plays(&r).await, 0);
    }

    #[tokio::test]
    async fn a_missing_sound_still_shows_the_alert() {
        let r = rig().await;
        let mut rx = r.hub.subscribe();
        r.exec.execute(&ctx(), &params(json!({"text": "hola", "soundId": "fantasma", "wait": false}))).await.expect("exec");
        let m = rx.recv().await.expect("msg");
        assert_eq!((m.data["text"].clone(), m.data["sync"].clone()), (json!("hola"), json!(false)));
    }

    #[tokio::test]
    async fn a_sound_alone_is_valid_content() {
        let r = rig().await;
        assert!(r.exec.validate(&params(json!({"soundId": "s"}))).is_ok());
        assert!(r.exec.validate(&params(json!({"text": "x", "soundVolume": 120}))).is_err());
    }

    #[tokio::test]
    async fn alerts_are_shown_one_at_a_time() {
        assert_eq!(rig().await.exec.concurrency(), Concurrency::Serial("alerts"));
    }

    #[tokio::test]
    async fn each_screen_gets_its_own_channel_and_queue() {
        let r = rig().await;
        let mut rx = r.hub.subscribe();
        for (screen, channel) in [(json!(null), "alerts"), (json!(3), "alerts3"), (json!(5), "alerts5")] {
            r.exec.execute(&ctx(), &params(json!({"text": "x", "wait": false, "screen": screen}))).await.expect("exec");
            assert_eq!(rx.recv().await.expect("msg").channel, channel);
        }
        let queue = |s: Value| r.exec.concurrency_for(&params(json!({"screen": s})));
        assert_eq!(queue(json!(1)), Concurrency::Serial("alerts"));
        assert_eq!(queue(json!(4)), Concurrency::Serial("alerts4"));
        assert_ne!(queue(json!(2)), queue(json!(3)));
        assert_eq!(r.exec.concurrency_for(&params(json!({}))), Concurrency::Serial("alerts"));
    }

    #[tokio::test]
    async fn a_screen_that_no_longer_exists_falls_back_to_the_first() {
        let r = rig().await;
        r.screens.set_screens(2).await.expect("n");
        let mut rx = r.hub.subscribe();
        r.exec.execute(&ctx(), &params(json!({"text": "x", "wait": false, "screen": 4}))).await.expect("exec");
        assert_eq!(rx.recv().await.expect("msg").channel, "alerts");
    }

    #[tokio::test]
    async fn per_alert_animation_and_scale_travel_with_the_alert() {
        let r = rig().await;
        let mut rx = r.hub.subscribe();
        r.exec.execute(&ctx(), &params(json!({"text": "x", "wait": false, "animation": "slide", "scale": 150}))).await.expect("exec");
        let m = rx.recv().await.expect("msg");
        assert_eq!((m.data["animation"].clone(), m.data["scale"].as_f64()), (json!("slide"), Some(150.0)));
        assert!(m.data["keyMode"].is_null());
        assert_eq!(m.data["videoVolume"].as_f64(), Some(100.0), "los vídeos suenan por defecto");
        r.exec.execute(&ctx(), &params(json!({"text": "x", "wait": false, "keyMode": "green"}))).await.expect("exec");
        assert_eq!(rx.recv().await.expect("msg").data["keyMode"], "green");
        let v = |p: Value| r.exec.validate(&params(p));
        assert!(v(json!({"text": "x", "screen": 21})).is_err());
        assert!(v(json!({"text": "x", "screen": 0})).is_err());
        assert!(v(json!({"text": "x", "scale": 10})).is_err());
        assert!(v(json!({"text": "x", "animation": "spin"})).is_err());
        assert!(v(json!({"text": "x", "keyMode": "blue"})).is_err());
        assert!(v(json!({"text": "x", "videoVolume": 101})).is_err());
        assert!(v(json!({"text": "x", "videoVolume": 0})).is_ok());
        assert!(v(json!({"text": "x", "keyMode": "black"})).is_ok());
        assert!(v(json!({"text": "x", "keyMode": ""})).is_ok());
        assert!(v(json!({"text": "x", "screen": 20, "scale": 250, "animation": "fade"})).is_ok());
    }
}
