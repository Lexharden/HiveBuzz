//! Ruleta de premios. El sorteo lo hace Rust (nunca el overlay): se elige el ganador con pesos,
//! se manda al overlay para que lo anime y, al terminar, se ejecuta el plan de acciones del premio.

use std::sync::{Arc, Mutex, OnceLock, PoisonError, RwLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::actions::queue::{ActionQueue, NewJob};
use crate::bot::service::BotService;
use crate::db::Db;
use crate::error::{AppError, Result};
use crate::overlay::OverlayHub;
use crate::rules::model::ActionPlan;
use crate::rules::template::{render, Vars};

pub const KEY_WHEEL_CONFIG: &str = "wheel_config";
pub const CHANNEL: &str = "wheel";
const MAX_SEGMENTS: usize = 24;
const MAX_WEIGHT: u32 = 10_000;
/// Pausa tras la animación para que el público vea el premio antes de ejecutarlo.
const REVEAL_PAUSE: Duration = Duration::from_millis(1_500);
const PRIZE_PRIORITY: i32 = 60;
/// Un premio ya ganado no debe perderse porque la cola esté ocupada: caduca tarde.
const PRIZE_TTL_MS: u64 = 5 * 60_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    pub id: String,
    pub label: String,
    /// Probabilidad relativa (0 = nunca sale).
    #[serde(default = "one")]
    pub weight: u32,
    #[serde(default = "default_color")]
    pub color: String,
    /// Qué ocurre cuando este premio sale.
    #[serde(default = "empty_plan")]
    pub plan: ActionPlan,
}

fn one() -> u32 {
    1
}
fn default_color() -> String {
    "#f59e0b".into()
}
fn empty_plan() -> ActionPlan {
    ActionPlan { mode: Default::default(), steps: Vec::new() }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WheelConfig {
    #[serde(default)]
    pub segments: Vec<Segment>,
    /// Duración de la animación del giro.
    #[serde(default = "default_spin_ms")]
    pub spin_ms: u64,
    /// Mensaje del bot al salir un premio (vacío = no anuncia). Variables: {user}, {nickname}, {prize}.
    #[serde(default = "default_announce")]
    pub announce: String,
}

fn default_spin_ms() -> u64 {
    6_000
}
fn default_announce() -> String {
    "🎡 {nickname} giró la ruleta y salió: {prize}".into()
}

impl Default for WheelConfig {
    fn default() -> Self {
        Self { segments: Vec::new(), spin_ms: default_spin_ms(), announce: default_announce() }
    }
}

impl WheelConfig {
    pub fn sanitized(mut self) -> Self {
        self.spin_ms = self.spin_ms.clamp(1_000, 30_000);
        for s in &mut self.segments {
            s.label = s.label.trim().chars().take(60).collect();
            s.weight = s.weight.min(MAX_WEIGHT);
            if !is_hex(&s.color) {
                s.color = default_color();
            }
        }
        self
    }

    pub fn validate(&self) -> Result<()> {
        if self.segments.len() > MAX_SEGMENTS {
            return Err(AppError::Invalid(format!("la ruleta admite hasta {MAX_SEGMENTS} premios")));
        }
        let mut ids = std::collections::HashSet::new();
        for s in &self.segments {
            if s.id.trim().is_empty() || !ids.insert(s.id.clone()) {
                return Err(AppError::Invalid("cada premio necesita un id único".into()));
            }
            if s.label.is_empty() {
                return Err(AppError::Invalid("hay un premio sin nombre".into()));
            }
        }
        if !self.segments.is_empty() && self.segments.iter().all(|s| s.weight == 0) {
            return Err(AppError::Invalid("al menos un premio debe tener probabilidad mayor que 0".into()));
        }
        Ok(())
    }
}

fn is_hex(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// Elige un premio con probabilidad proporcional a su peso. `roll` está en [0, 1).
pub fn pick(segments: &[Segment], roll: f64) -> Option<usize> {
    let total: u64 = segments.iter().map(|s| u64::from(s.weight)).sum();
    if total == 0 {
        return None;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_precision_loss)]
    let mut target = ((roll.clamp(0.0, 0.999_999_999) * total as f64) as u64).min(total - 1);
    for (i, s) in segments.iter().enumerate() {
        let w = u64::from(s.weight);
        if target < w {
            return Some(i);
        }
        target -= w;
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpinResult {
    pub index: usize,
    pub label: String,
}

pub type RandFn = Arc<dyn Fn() -> f64 + Send + Sync>;

pub struct WheelService {
    cfg: RwLock<WheelConfig>,
    hub: OverlayHub,
    /// La cola se conecta después de crear el servicio (el ejecutor de la ruleta debe existir antes que la cola).
    queue: OnceLock<Arc<ActionQueue>>,
    bot: Arc<BotService>,
    db: Db,
    rand: RandFn,
    reveal_pause: Duration,
    spins: Mutex<u64>,
}

impl WheelService {
    pub fn new(db: Db, hub: OverlayHub, bot: Arc<BotService>) -> Arc<Self> {
        Self::with_rand(db, hub, bot, Arc::new(rand::random::<f64>), REVEAL_PAUSE)
    }

    pub fn with_rand(db: Db, hub: OverlayHub, bot: Arc<BotService>, rand: RandFn, reveal_pause: Duration) -> Arc<Self> {
        Arc::new(Self { cfg: RwLock::new(WheelConfig::default()), hub, queue: OnceLock::new(), bot, db, rand, reveal_pause, spins: Mutex::new(0) })
    }

    pub fn attach_queue(&self, queue: Arc<ActionQueue>) {
        if self.queue.set(queue).is_err() {
            tracing::warn!("la ruleta ya tenía una cola conectada");
        }
    }

    pub async fn load(&self) -> Result<()> {
        let cfg = match self.db.get_setting(KEY_WHEEL_CONFIG).await? {
            Some(json) => serde_json::from_str::<WheelConfig>(&json).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "configuración de la ruleta ilegible; se usa la de fábrica");
                WheelConfig::default()
            }),
            None => WheelConfig::default(),
        };
        *self.cfg.write().unwrap_or_else(PoisonError::into_inner) = cfg.sanitized();
        self.publish_idle();
        Ok(())
    }

    pub fn config(&self) -> WheelConfig {
        self.cfg.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    pub async fn set_config(&self, cfg: WheelConfig) -> Result<WheelConfig> {
        let cfg = cfg.sanitized();
        cfg.validate()?;
        self.db.set_setting(KEY_WHEEL_CONFIG, &serde_json::to_string(&cfg)?).await?;
        *self.cfg.write().unwrap_or_else(PoisonError::into_inner) = cfg.clone();
        self.publish_idle();
        Ok(cfg)
    }

    fn view(cfg: &WheelConfig) -> serde_json::Value {
        cfg.segments.iter().map(|s| json!({ "label": s.label, "color": s.color, "weight": s.weight })).collect()
    }

    /// Estado en reposo (retenido): un overlay recién abierto dibuja la ruleta quieta.
    fn publish_idle(&self) {
        let cfg = self.config();
        self.hub.publish_retained(CHANNEL, json!({ "kind": "idle", "segments": Self::view(&cfg) }));
    }

    /// Gira la ruleta. Termina cuando acaba la animación y el premio se ha encolado, así que un
    /// ejecutor `Serial` convierte varios giros pedidos a la vez en giros uno tras otro.
    pub async fn spin(&self, vars: Vars) -> Result<SpinResult> {
        let cfg = self.config();
        let index = pick(&cfg.segments, (self.rand)()).ok_or_else(|| AppError::Invalid("la ruleta no tiene premios con probabilidad".into()))?;
        let Some(winner) = cfg.segments.get(index).cloned() else {
            return Err(AppError::Invalid("premio fuera de rango".into()));
        };
        let id = {
            let mut n = self.spins.lock().unwrap_or_else(PoisonError::into_inner);
            *n += 1;
            *n
        };
        self.hub.publish(
            CHANNEL,
            json!({
                "kind": "spin",
                "id": id,
                "segments": Self::view(&cfg),
                "winner": index,
                "durationMs": cfg.spin_ms,
                "user": vars.get("nickname").or_else(|| vars.get("user")).cloned().unwrap_or_default(),
            }),
        );
        tokio::time::sleep(Duration::from_millis(cfg.spin_ms) + self.reveal_pause).await;

        let mut vars = vars;
        vars.insert("prize".into(), winner.label.clone());
        if !cfg.announce.trim().is_empty() {
            self.bot.say(&render(&cfg.announce, &vars), "ruleta");
        }
        if !winner.plan.steps.is_empty() {
            let job = NewJob {
                rule_id: format!("wheel:{}", winner.id),
                mode: winner.plan.mode,
                steps: winner.plan.steps.clone(),
                vars,
                priority: PRIZE_PRIORITY,
                ttl_ms: PRIZE_TTL_MS,
                refund: None,
            };
            match self.queue.get() {
                Some(queue) => {
                    if let Err(e) = queue.enqueue(job).await {
                        tracing::error!(prize = %winner.label, error = %e, "no se pudo encolar el premio de la ruleta");
                    }
                }
                None => tracing::error!(prize = %winner.label, "la ruleta no tiene cola de acciones conectada"),
            }
        }
        Ok(SpinResult { index, label: winner.label })
    }
}

#[cfg(test)]
mod tests;
