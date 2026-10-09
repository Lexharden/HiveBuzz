//! Servicio de puntos: convierte los eventos del bus en puntos, los guarda por lotes y ofrece
//! saldo, gasto atómico, ranking y mantenimiento de la base de espectadores.

use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Duration;

use tokio::sync::broadcast::{self, error::RecvError};

use super::config::PointsConfig;
use super::logic::{Award, Awarder, Reason, Stats, Who};
use super::{HistoryEntry, SortKey, Viewer};
use crate::actions::clock::Clock;
use crate::bus::EventBus;
use crate::db::{Db, ImportMode, ImportReport, HISTORY_RETENTION_MS};
use crate::error::{AppError, Result};
use crate::session::SessionStarted;

/// Clave de la configuración en la tabla `settings`.
pub const KEY_POINTS_CONFIG: &str = "points_config";

#[derive(Debug, Clone, Copy)]
pub struct PointsTiming {
    /// Cada cuánto se vuelcan a la base de datos las ganancias acumuladas.
    pub flush_every: Duration,
    /// Cada cuánto se comprueba si toca repartir los puntos por ver.
    pub watch_check_every: Duration,
    /// Cada cuánto se purga la memoria y el historial viejo.
    pub maintenance_every: Duration,
}

impl Default for PointsTiming {
    fn default() -> Self {
        Self {
            flush_every: Duration::from_secs(1),
            watch_check_every: Duration::from_secs(5),
            maintenance_every: Duration::from_secs(600),
        }
    }
}

pub struct PointsService {
    cfg: RwLock<PointsConfig>,
    awarder: Mutex<Awarder>,
    pending: Mutex<Vec<Award>>,
    last_watch_ms: Mutex<i64>,
    db: Db,
    clock: Arc<dyn Clock>,
}

impl PointsService {
    pub fn new(db: Db, clock: Arc<dyn Clock>) -> Arc<Self> {
        let now = clock.now_ms();
        Arc::new(Self {
            cfg: RwLock::new(PointsConfig::default()),
            awarder: Mutex::new(Awarder::default()),
            pending: Mutex::new(Vec::new()),
            last_watch_ms: Mutex::new(now),
            db,
            clock,
        })
    }

    // ---- Configuración ----

    pub async fn load_config(&self) -> Result<()> {
        let cfg = match self.db.get_setting(KEY_POINTS_CONFIG).await? {
            Some(json) => serde_json::from_str::<PointsConfig>(&json).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "configuración de puntos ilegible; se usa la de fábrica");
                PointsConfig::default()
            }),
            None => PointsConfig::default(),
        };
        *self.cfg.write().unwrap_or_else(PoisonError::into_inner) = cfg.sanitized();
        Ok(())
    }

    pub fn config(&self) -> PointsConfig {
        self.cfg.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    pub async fn set_config(&self, cfg: PointsConfig) -> Result<PointsConfig> {
        let cfg = cfg.sanitized();
        self.db.set_setting(KEY_POINTS_CONFIG, &serde_json::to_string(&cfg)?).await?;
        *self.cfg.write().unwrap_or_else(PoisonError::into_inner) = cfg.clone();
        Ok(cfg)
    }

    // ---- Ganancias ----

    /// Procesa un evento: anota las ganancias que correspondan (se guardan en el próximo volcado).
    pub fn on_event(&self, ev: &crate::events::LiveEvent) -> usize {
        let cfg = self.config();
        if ev.is_simulated() && !cfg.count_simulated {
            return 0;
        }
        let awards = self.awarder.lock().unwrap_or_else(PoisonError::into_inner).on_event(&cfg, ev, self.clock.now_ms());
        let n = awards.len();
        self.pending.lock().unwrap_or_else(PoisonError::into_inner).extend(awards);
        n
    }

    /// Reparte los puntos por ver si ya pasó el intervalo configurado.
    pub fn watch_tick_if_due(&self) -> usize {
        let cfg = self.config();
        let now = self.clock.now_ms();
        let interval = i64::try_from(cfg.watch_interval_minutes).unwrap_or(1).saturating_mul(60_000);
        {
            let mut last = self.last_watch_ms.lock().unwrap_or_else(PoisonError::into_inner);
            if now.saturating_sub(*last) < interval {
                return 0;
            }
            *last = now;
        }
        let awards = self.awarder.lock().unwrap_or_else(PoisonError::into_inner).on_watch_tick(&cfg, now);
        let n = awards.len();
        self.pending.lock().unwrap_or_else(PoisonError::into_inner).extend(awards);
        n
    }

    /// Vuelca a la base las ganancias pendientes. Si falla, se reencolan para el siguiente intento.
    pub async fn flush(&self) -> Result<()> {
        let batch: Vec<Award> = std::mem::take(&mut *self.pending.lock().unwrap_or_else(PoisonError::into_inner));
        if batch.is_empty() {
            return Ok(());
        }
        if let Err(e) = self.db.apply_awards(&batch).await {
            let mut p = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
            let mut restored = batch;
            restored.append(&mut p);
            *p = restored;
            return Err(e);
        }
        Ok(())
    }

    // ---- Consultas y operaciones (siempre vuelcan antes: el saldo debe estar al día) ----

    pub async fn balance(&self, user_id: &str) -> Result<u64> {
        self.flush().await?;
        self.db.balance(user_id).await
    }

    pub async fn viewer_by_unique(&self, unique_id: &str) -> Result<Option<Viewer>> {
        self.flush().await?;
        self.db.find_viewer_by_unique(unique_id).await
    }

    pub async fn viewer(&self, user_id: &str) -> Result<Option<Viewer>> {
        self.flush().await?;
        self.db.get_viewer(user_id).await
    }

    pub async fn top(&self, limit: u32) -> Result<Vec<Viewer>> {
        self.flush().await?;
        self.db.top_viewers_by_points(limit).await
    }

    /// Gasta puntos en una recompensa. `None` si no alcanzaban (no se descuenta nada).
    pub async fn spend(&self, user_id: &str, cost: u64, reward: &str) -> Result<Option<u64>> {
        self.flush().await?;
        let reason = Reason::Spend(reward.chars().take(60).collect()).label();
        self.db.spend_points(user_id, cost, &reason, self.clock.now_ms()).await
    }

    /// Devuelve puntos (p. ej. si la recompensa no pudo ejecutarse). Reutiliza el id de la recompensa en el historial.
    pub async fn refund(&self, user_id: &str, amount: u64, reward: &str) -> Result<()> {
        self.flush().await?;
        let reason = format!("refund:{}", reward.chars().take(60).collect::<String>());
        self.db.adjust_points(user_id, i64::try_from(amount).unwrap_or(i64::MAX), &reason, self.clock.now_ms()).await.map(|_| ())
    }

    pub async fn adjust(&self, user_id: &str, delta: i64) -> Result<u64> {
        self.flush().await?;
        self.db.adjust_points(user_id, delta, "manual", self.clock.now_ms()).await
    }

    pub async fn set_points(&self, user_id: &str, value: u64) -> Result<()> {
        self.flush().await?;
        self.db.set_points(user_id, value, self.clock.now_ms()).await
    }

    /// Da (o quita) puntos a alguien por su `@usuario`; si no existe en la base, lo crea con 0 antes.
    pub async fn adjust_by_unique(&self, unique_id: &str, delta: i64) -> Result<Viewer> {
        self.flush().await?;
        let uid = unique_id.trim().trim_start_matches('@').to_lowercase();
        if uid.is_empty() {
            return Err(AppError::Invalid("indica un @usuario".into()));
        }
        let found = self.db.find_viewer_by_unique(&uid).await?;
        let user_id = match found {
            Some(v) => v.user_id,
            None => {
                let who = Who { user_id: format!("{}{uid}", super::PROVISIONAL_PREFIX), unique_id: uid.clone(), nickname: uid.clone(), avatar: String::new(), is_subscriber: false };
                self.db.apply_awards(&[Award { who: who.clone(), delta: 0, reason: Reason::Manual, stats: Stats::default(), ts: self.clock.now_ms() }]).await?;
                who.user_id
            }
        };
        self.db.adjust_points(&user_id, delta, "manual", self.clock.now_ms()).await?;
        self.db.get_viewer(&user_id).await?.ok_or_else(|| AppError::Invalid("el espectador desapareció".into()))
    }

    pub async fn list(&self, search: &str, sort: SortKey, limit: u32, offset: u32) -> Result<(Vec<Viewer>, u64)> {
        self.flush().await?;
        Ok((self.db.list_viewers(search, sort, limit, offset).await?, self.db.count_viewers(search).await?))
    }

    pub async fn history(&self, user_id: &str, limit: u32) -> Result<Vec<HistoryEntry>> {
        self.flush().await?;
        self.db.point_history(user_id, limit).await
    }

    pub async fn delete_viewer(&self, user_id: &str) -> Result<bool> {
        self.flush().await?;
        self.db.delete_viewer(user_id).await
    }

    pub async fn clear_all(&self) -> Result<()> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner).clear();
        self.db.clear_viewers().await
    }

    pub async fn export_csv(&self) -> Result<String> {
        self.flush().await?;
        self.db.export_viewers_csv().await
    }

    pub async fn import_csv(&self, text: &str, mode: ImportMode) -> Result<ImportReport> {
        self.flush().await?;
        self.db.import_viewers_csv(text, mode, self.clock.now_ms()).await
    }

    /// Un LIVE nuevo: nadie sigue «presente».
    pub fn on_session_started(&self) {
        self.awarder.lock().unwrap_or_else(PoisonError::into_inner).reset_session();
    }

    /// Arranca las tareas de fondo: eventos, puntos por ver, volcado y mantenimiento.
    pub fn spawn(self: &Arc<Self>, bus: &EventBus, mut sessions: broadcast::Receiver<SessionStarted>, timing: PointsTiming) {
        let this = Arc::clone(self);
        let mut events = bus.subscribe();
        tokio::spawn(async move {
            let mut sessions_open = true;
            loop {
                tokio::select! {
                    ev = events.recv() => match ev {
                        Ok(ev) => {
                            this.on_event(&ev);
                        }
                        Err(RecvError::Lagged(n)) => tracing::warn!(missed = n, "los puntos se quedaron atrás en el bus"),
                        Err(RecvError::Closed) => break,
                    },
                    s = sessions.recv(), if sessions_open => match s {
                        Ok(_) => this.on_session_started(),
                        Err(RecvError::Lagged(_)) => {}
                        Err(RecvError::Closed) => sessions_open = false,
                    },
                }
            }
        });

        let this = Arc::clone(self);
        tokio::spawn(async move {
            let mut flush = tokio::time::interval(timing.flush_every);
            let mut watch = tokio::time::interval(timing.watch_check_every);
            let mut maint = tokio::time::interval(timing.maintenance_every);
            loop {
                tokio::select! {
                    _ = flush.tick() => {
                        if let Err(e) = this.flush().await {
                            tracing::error!(error = %e, "no se pudieron guardar los puntos");
                        }
                    }
                    _ = watch.tick() => {
                        this.watch_tick_if_due();
                    }
                    _ = maint.tick() => {
                        let now = this.clock.now_ms();
                        let cfg = this.config();
                        this.awarder.lock().unwrap_or_else(PoisonError::into_inner).prune(&cfg, now);
                        match this.db.prune_point_history(now.saturating_sub(HISTORY_RETENTION_MS)).await {
                            Ok(0) => {}
                            Ok(n) => tracing::info!(deleted = n, "rotación del historial de puntos"),
                            Err(e) => tracing::warn!(error = %e, "falló la rotación del historial de puntos"),
                        }
                    }
                }
            }
        });
    }
}

#[async_trait::async_trait]
impl crate::rules::engine::PointsGate for PointsService {
    async fn balance(&self, user_id: &str) -> Result<u64> {
        PointsService::balance(self, user_id).await
    }

    async fn spend(&self, user_id: &str, cost: u64, reward: &str) -> Result<Option<u64>> {
        PointsService::spend(self, user_id, cost, reward).await
    }

    async fn refund(&self, user_id: &str, amount: u64, reward: &str) -> Result<()> {
        PointsService::refund(self, user_id, amount, reward).await
    }
}

#[cfg(test)]
mod tests;
