//! Ejecutor `overlayAlert`: muestra una alerta en el overlay `/overlay/alerts`.
//!
//! Parámetros (los textos admiten variables como `{nickname}`):
//! `title`, `text`, `mediaId` (de la biblioteca de medios), `imageUrl` (p. ej. `{giftimage}`),
//! `showAvatar`, `durationMs` (500–60000, por defecto 5000) y `wait` (esperar a que termine la
//! alerta antes de seguir con el plan; por defecto `true`).
//!
//! Las alertas se muestran de una en una (grupo serial): el overlay no las superpone.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Map, Value};

use super::{opt_bool, opt_number};
use crate::actions::{ActionContext, ActionExecutor, Concurrency};
use crate::error::{AppError, Result};
use crate::media::MediaLibrary;
use crate::overlay::OverlayHub;

pub const CHANNEL: &str = "alerts";
const DEFAULT_DURATION_MS: f64 = 5000.0;

pub struct OverlayAlertExecutor {
    media: Arc<MediaLibrary>,
    hub: OverlayHub,
}

impl OverlayAlertExecutor {
    pub fn new(media: Arc<MediaLibrary>, hub: OverlayHub) -> Self {
        Self { media, hub }
    }
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

    fn validate(&self, params: &Map<String, Value>) -> Result<()> {
        let has_content = ["title", "text", "mediaId", "imageUrl"].iter().any(|k| text_param(params, k).is_some());
        if !has_content {
            return Err(AppError::Invalid("la alerta necesita título, texto o un medio".into()));
        }
        opt_number(params, "durationMs", 500.0, 60_000.0)?;
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

        let payload = json!({
            "id": uuid::Uuid::new_v4().to_string(),
            "title": text_param(params, "title").map(|t| ctx.render(t)),
            "text": text_param(params, "text").map(|t| ctx.render(t)),
            "imageUrl": text_param(params, "imageUrl").map(|t| ctx.render(t)).filter(|u| !u.is_empty()),
            "media": media,
            "avatar": avatar,
            "durationMs": duration_ms,
        });
        self.hub.publish(CHANNEL, payload);

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
    use crate::db::Db;

    struct Rig {
        exec: OverlayAlertExecutor,
        hub: OverlayHub,
        media: Arc<MediaLibrary>,
        tmp: tempfile::TempDir,
    }

    async fn rig() -> Rig {
        let tmp = tempfile::tempdir().expect("tmp");
        let db = Db::open_memory().await.expect("db");
        let media = Arc::new(MediaLibrary::new(db, tmp.path().join("m"), Arc::new(AppClock::new())).expect("lib"));
        let hub = OverlayHub::new(16);
        Rig { exec: OverlayAlertExecutor::new(media.clone(), hub.clone()), hub, media, tmp }
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

    #[tokio::test]
    async fn alerts_are_shown_one_at_a_time() {
        assert_eq!(rig().await.exec.concurrency(), Concurrency::Serial("alerts"));
    }
}
