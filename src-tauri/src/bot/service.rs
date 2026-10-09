//! Servicio del chatbot: escucha el bus y las recompensas, decide qué decir y lo encola en la salida.

use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Duration;

use tokio::sync::broadcast::error::RecvError;

use super::engine::{uses_points, BotEngine, ReplyCtx};
use super::model::BotConfig;
use super::outbox::{LogEntry, Outbox};
use super::timed::TimedScheduler;
use crate::actions::clock::Clock;
use crate::bus::EventBus;
use crate::db::Db;
use crate::error::Result;
use crate::events::{EventType, LiveEvent, Platform};
use crate::points::service::PointsService;
use crate::rules::engine::{DeniedReport, FiredReport, RuleEngine};
use crate::rules::template::Vars;

pub const KEY_BOT_CONFIG: &str = "bot_config";

pub struct BotService {
    cfg: RwLock<BotConfig>,
    engine: Mutex<BotEngine>,
    timed: Mutex<TimedScheduler>,
    outbox: Outbox,
    points: Arc<PointsService>,
    db: Db,
    clock: Arc<dyn Clock>,
}

fn rand01() -> f64 {
    rand::random::<f64>()
}

impl BotService {
    pub fn new(db: Db, clock: Arc<dyn Clock>, points: Arc<PointsService>, outbox: Outbox) -> Arc<Self> {
        Arc::new(Self {
            cfg: RwLock::new(BotConfig::default()),
            engine: Mutex::new(BotEngine::default()),
            timed: Mutex::new(TimedScheduler::default()),
            outbox,
            points,
            db,
            clock,
        })
    }

    pub async fn load_config(&self) -> Result<()> {
        let cfg = match self.db.get_setting(KEY_BOT_CONFIG).await? {
            Some(json) => serde_json::from_str::<BotConfig>(&json).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "configuración del bot ilegible; se usa la de fábrica");
                BotConfig::default()
            }),
            None => BotConfig::default(),
        }
        .sanitized();
        self.outbox.set_min_interval(Duration::from_millis(cfg.min_interval_ms));
        *self.cfg.write().unwrap_or_else(PoisonError::into_inner) = cfg;
        Ok(())
    }

    pub fn config(&self) -> BotConfig {
        self.cfg.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    pub async fn set_config(&self, cfg: BotConfig) -> Result<BotConfig> {
        let cfg = cfg.sanitized();
        cfg.validate()?;
        self.db.set_setting(KEY_BOT_CONFIG, &serde_json::to_string(&cfg)?).await?;
        self.outbox.set_min_interval(Duration::from_millis(cfg.min_interval_ms));
        *self.cfg.write().unwrap_or_else(PoisonError::into_inner) = cfg.clone();
        Ok(cfg)
    }

    pub fn log(&self) -> Vec<LogEntry> {
        self.outbox.log()
    }

    pub fn clear_log(&self) {
        self.outbox.clear_log();
    }

    /// Envía un mensaje de prueba (ignora el interruptor general, no la sesión de TikTok).
    pub fn say(&self, text: &str, source: &str) -> bool {
        self.outbox.enqueue(&super::engine::fit(text), source)
    }

    fn ctx<'a>(&self, points: Option<u64>, currency: &'a str, rand: &'a (dyn Fn() -> f64 + Sync)) -> ReplyCtx<'a> {
        ReplyCtx { points, currency, now_ms: self.clock.now_ms(), local: chrono::Local::now().naive_local(), rand }
    }

    /// Procesa un evento del bus: comandos, palabras clave, agradecimientos y `!puntos`/`!top`.
    pub async fn on_event(&self, ev: &LiveEvent) {
        let cfg = self.config();
        // El bot no habla por eventos de prueba (acabarían en el chat real) ni por eventos de otras plataformas:
        // solo sabe escribir en el chat de TikTok.
        if !cfg.enabled || ev.is_simulated() || ev.platform != Platform::Tiktok {
            return;
        }
        if ev.kind == EventType::Chat {
            if let Some(chat) = &ev.chat {
                if self.outbox.is_echo(&chat.text) {
                    return; // es lo que el propio bot acaba de escribir
                }
            }
            self.timed.lock().unwrap_or_else(PoisonError::into_inner).note_chat();
        }
        let currency = self.points.config().currency_name;
        let balance = if uses_points(&cfg) && !ev.user.id.is_empty() {
            self.points.balance(&ev.user.id).await.ok()
        } else {
            None
        };
        let rand: &(dyn Fn() -> f64 + Sync) = &rand01;
        let replies = self.engine.lock().unwrap_or_else(PoisonError::into_inner).on_event(&cfg, ev, &self.ctx(balance, &currency, rand));
        for r in replies {
            self.outbox.enqueue(&r.text, &r.source);
        }
        if ev.kind == EventType::Chat {
            self.builtin_commands(&cfg, ev, &currency).await;
        }
    }

    async fn builtin_commands(&self, cfg: &BotConfig, ev: &LiveEvent, currency: &str) {
        let pcfg = self.points.config();
        if !pcfg.enabled {
            return;
        }
        let rand: &(dyn Fn() -> f64 + Sync) = &rand01;
        let ctx = self.ctx(None, currency, rand);
        let mut vars: Vars = Vars::new();
        vars.insert("user".into(), ev.user.unique_id.clone());
        vars.insert("nickname".into(), ev.user.nickname.clone());

        let ask = |key: &str, cmd: &str| self.engine.lock().unwrap_or_else(PoisonError::into_inner).allow_builtin(key, cmd, ev, &ctx);

        if !pcfg.points_command.trim().is_empty() && ask("builtin:points", pcfg.points_command.trim().trim_start_matches('!')) {
            let points = self.points.balance(&ev.user.id).await.unwrap_or(0);
            let mut v = vars.clone();
            v.insert("points".into(), points.to_string());
            let text = self.engine.lock().unwrap_or_else(PoisonError::into_inner).render_with(&cfg.builtin.points, v, currency);
            self.outbox.enqueue(&text, "comando de puntos");
        }
        if !pcfg.top_command.trim().is_empty() && ask("builtin:top", pcfg.top_command.trim().trim_start_matches('!')) {
            let top = self.points.top(pcfg.top_size.clamp(1, 10)).await.unwrap_or_default();
            if top.is_empty() {
                return;
            }
            let list = top
                .iter()
                .enumerate()
                .map(|(i, v)| format!("{}. {} ({})", i + 1, if v.nickname.is_empty() { &v.unique_id } else { &v.nickname }, v.points))
                .collect::<Vec<_>>()
                .join(" · ");
            let mut v = vars;
            v.insert("top".into(), list);
            let text = self.engine.lock().unwrap_or_else(PoisonError::into_inner).render_with(&cfg.builtin.top, v, currency);
            self.outbox.enqueue(&text, "comando de top");
        }
    }

    /// Confirma un canje de recompensa en el chat.
    pub fn on_redeemed(&self, r: &FiredReport) {
        let cfg = self.config();
        let (Some(cost), Some(left), Some(user)) = (r.cost, r.points_left, r.user.as_deref()) else { return };
        if !cfg.enabled || !r.queued || r.platform != Platform::Tiktok {
            return;
        }
        let currency = self.points.config().currency_name;
        let mut v = Vars::new();
        v.insert("user".into(), user.trim_start_matches('@').to_string());
        v.insert("nickname".into(), r.nickname.clone().unwrap_or_default());
        v.insert("reward".into(), r.rule_name.clone());
        v.insert("cost".into(), cost.to_string());
        v.insert("points".into(), left.to_string());
        let text = self.engine.lock().unwrap_or_else(PoisonError::into_inner).render_with(&cfg.builtin.redeemed, v, &currency);
        self.outbox.enqueue(&text, "recompensa");
    }

    /// Avisa de que faltan puntos para una recompensa.
    pub fn on_denied(&self, d: &DeniedReport) {
        let cfg = self.config();
        if !cfg.enabled || d.platform != Platform::Tiktok {
            return;
        }
        let currency = self.points.config().currency_name;
        let mut v = Vars::new();
        v.insert("user".into(), d.user.trim_start_matches('@').to_string());
        v.insert("nickname".into(), d.nickname.clone());
        v.insert("reward".into(), d.rule_name.clone());
        v.insert("cost".into(), d.cost.to_string());
        v.insert("points".into(), d.have.to_string());
        let text = self.engine.lock().unwrap_or_else(PoisonError::into_inner).render_with(&cfg.builtin.denied, v, &currency);
        self.outbox.enqueue(&text, "recompensa (sin puntos)");
    }

    /// Dice los mensajes temporizados que toquen.
    pub fn tick_timed(&self) {
        let cfg = self.config();
        if !cfg.enabled {
            return;
        }
        let due = self.timed.lock().unwrap_or_else(PoisonError::into_inner).due(&cfg.timed_messages, self.clock.now_ms());
        for text in due {
            self.outbox.enqueue(&super::engine::fit(&text), "mensaje temporizado");
        }
    }

    pub fn spawn(self: &Arc<Self>, bus: &EventBus, rules: &Arc<RuleEngine>) {
        let (svc, mut events) = (Arc::clone(self), bus.subscribe());
        tokio::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(ev) => svc.on_event(&ev).await,
                    Err(RecvError::Lagged(n)) => tracing::warn!(missed = n, "el bot se quedó atrás en el bus"),
                    Err(RecvError::Closed) => break,
                }
            }
        });
        let (svc, mut fired) = (Arc::clone(self), rules.subscribe_fired());
        tokio::spawn(async move {
            loop {
                match fired.recv().await {
                    Ok(r) => svc.on_redeemed(&r),
                    Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => break,
                }
            }
        });
        let (svc, mut denied) = (Arc::clone(self), rules.subscribe_denied());
        tokio::spawn(async move {
            loop {
                match denied.recv().await {
                    Ok(d) => svc.on_denied(&d),
                    Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => break,
                }
            }
        });
        let svc = Arc::clone(self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(15));
            loop {
                tick.tick().await;
                svc.tick_timed();
            }
        });
    }
}

#[cfg(test)]
mod tests;
