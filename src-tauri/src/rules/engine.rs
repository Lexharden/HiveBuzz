//! Motor de reglas: escucha el bus, evalúa las reglas y encola las acciones.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError, RwLock};

use serde::Serialize;
use tokio::sync::broadcast::{self, error::RecvError};

use super::eval::{evaluate, Blocked, Decision, EvalEnv, RuleState};
use super::matcher::{match_trigger, TriggerInput};
use super::model::{Rule, Trigger};
use super::sample::sample_event_for;
use super::template::{event_vars, Vars};
use super::validate_rule;
use crate::actions::clock::Clock;
use crate::actions::queue::{ActionQueue, NewJob, Outcome};
use crate::actions::ExecutorRegistry;
use crate::bus::EventBus;
use crate::db::Db;
use crate::error::{AppError, Result};
use crate::events::{EventType, LiveEvent, Platform};

/// Cada cuántos eventos se purgan los cooldowns de usuario vencidos.
const PRUNE_EVERY: u64 = 2_000;

/// Eventos internos que también pueden disparar reglas (los emiten las metas y los timers).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystemEvent {
    GoalReached(String),
    TimerEnded(String),
}

/// Quien puede disparar reglas con eventos internos. Las metas y los timers dependen de este
/// trait, no del motor concreto (así se prueban con un sumidero falso).
#[async_trait::async_trait]
pub trait SystemSink: Send + Sync {
    async fn fire(&self, ev: SystemEvent);
}

#[async_trait::async_trait]
impl SystemSink for RuleEngine {
    async fn fire(&self, ev: SystemEvent) {
        self.fire_system(&ev).await;
    }
}

/// Sistema de puntos visto desde el motor de reglas (así se prueba con uno falso).
#[async_trait::async_trait]
pub trait PointsGate: Send + Sync {
    async fn balance(&self, user_id: &str) -> Result<u64>;
    /// Gasta de forma atómica. `None` si no alcanzaban los puntos (no se descuenta nada).
    async fn spend(&self, user_id: &str, cost: u64, reward: &str) -> Result<Option<u64>>;
    /// Devuelve lo gastado (la recompensa no llegó a ejecutarse).
    async fn refund(&self, user_id: &str, amount: u64, reward: &str) -> Result<()>;
}

/// Aviso de que una regla se disparó (para mostrarlo en la UI y para que el chatbot confirme canjes).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FiredReport {
    pub rule_id: String,
    pub rule_name: String,
    pub ts: i64,
    /// `false` cuando la cola descartó el job por estar llena.
    pub queued: bool,
    /// `@usuario` que la disparó (si la causó un espectador).
    pub user: Option<String>,
    pub nickname: Option<String>,
    /// Puntos cobrados y saldo restante (si era una recompensa con coste).
    pub cost: Option<u64>,
    pub points_left: Option<u64>,
    /// Plataforma de quien la disparó (el bot solo confirma canjes en TikTok).
    pub platform: Platform,
}

/// Un espectador intentó una recompensa sin tener puntos suficientes.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeniedReport {
    pub rule_id: String,
    pub rule_name: String,
    pub user: String,
    pub nickname: String,
    pub cost: u64,
    pub have: u64,
    pub platform: Platform,
}

/// Prioridad por defecto: los regalos grandes se adelantan.
pub fn auto_priority(input: &TriggerInput) -> i32 {
    match input {
        TriggerInput::Live(ev) if ev.kind == EventType::Gift => {
            match ev.gift.as_ref().map_or(0, |g| g.coins) {
                30_000.. => 100,
                10_000.. => 90,
                1_000.. => 70,
                100.. => 40,
                10.. => 15,
                _ => 5,
            }
        }
        TriggerInput::Live(_) => 0,
        TriggerInput::GoalReached(_) | TriggerInput::TimerEnded(_) => 30,
    }
}

pub struct RuleEngine {
    rules: RwLock<Arc<Vec<Arc<Rule>>>>,
    states: Mutex<HashMap<String, RuleState>>,
    queue: Arc<ActionQueue>,
    registry: ExecutorRegistry,
    clock: Arc<dyn Clock>,
    db: Db,
    fired: broadcast::Sender<FiredReport>,
    denied: broadcast::Sender<DeniedReport>,
    points: OnceLock<Arc<dyn PointsGate>>,
    handled: Mutex<u64>,
}

impl RuleEngine {
    pub fn new(db: Db, queue: Arc<ActionQueue>, registry: ExecutorRegistry, clock: Arc<dyn Clock>) -> Arc<Self> {
        let (fired, _) = broadcast::channel(64);
        let (denied, _) = broadcast::channel(64);
        Arc::new(Self {
            rules: RwLock::new(Arc::new(Vec::new())),
            states: Mutex::new(HashMap::new()),
            queue,
            registry,
            clock,
            db,
            fired,
            denied,
            points: OnceLock::new(),
            handled: Mutex::new(0),
        })
    }

    pub fn subscribe_fired(&self) -> broadcast::Receiver<FiredReport> {
        self.fired.subscribe()
    }

    pub fn subscribe_denied(&self) -> broadcast::Receiver<DeniedReport> {
        self.denied.subscribe()
    }

    /// Conecta el sistema de puntos (necesario para las recompensas con coste).
    pub fn attach_points(&self, gate: Arc<dyn PointsGate>) {
        if self.points.set(gate).is_err() {
            tracing::warn!("el sistema de puntos ya estaba conectado al motor de reglas");
        }
    }

    /// Carga las reglas guardadas.
    pub async fn load(&self) -> Result<usize> {
        let rules = self.db.list_rules().await?;
        let n = rules.len();
        self.set_rules(rules);
        Ok(n)
    }

    pub fn list(&self) -> Vec<Rule> {
        self.snapshot().iter().map(|r| (**r).clone()).collect()
    }

    pub fn get(&self, id: &str) -> Option<Rule> {
        self.snapshot().iter().find(|r| r.id == id).map(|r| (**r).clone())
    }

    /// Valida, guarda y activa una regla (nueva o editada).
    pub async fn upsert(&self, rule: Rule) -> Result<()> {
        validate_rule(&rule)?;
        self.registry.validate_rule(&rule)?;
        self.db.save_rule(&rule, self.clock.now_ms()).await?;
        let mut rules = self.list();
        match rules.iter_mut().find(|r| r.id == rule.id) {
            Some(slot) => *slot = rule,
            None => rules.push(rule),
        }
        self.set_rules(rules);
        Ok(())
    }

    /// Sustituye todas las reglas (aplicar un perfil). Si alguna es inválida no se cambia nada.
    pub async fn replace_all(&self, rules: Vec<Rule>) -> Result<()> {
        let mut seen = std::collections::HashSet::new();
        for rule in &rules {
            validate_rule(rule)?;
            self.registry.validate_rule(rule)?;
            if !seen.insert(rule.id.clone()) {
                return Err(AppError::Invalid(format!("la regla «{}» está repetida", rule.name)));
            }
        }
        self.db.replace_rules(&rules, self.clock.now_ms()).await?;
        self.set_rules(rules);
        // Los cooldowns de reglas que ya no existen no sirven.
        let ids: std::collections::HashSet<String> = self.list().into_iter().map(|r| r.id).collect();
        self.states.lock().unwrap_or_else(PoisonError::into_inner).retain(|id, _| ids.contains(id));
        Ok(())
    }

    pub async fn delete(&self, id: &str) -> Result<bool> {
        let existed = self.db.delete_rule(id).await?;
        let rules: Vec<Rule> = self.list().into_iter().filter(|r| r.id != id).collect();
        self.set_rules(rules);
        self.states.lock().unwrap_or_else(PoisonError::into_inner).remove(id);
        Ok(existed)
    }

    pub async fn set_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        let mut rule = self.get(id).ok_or_else(|| AppError::Invalid("la regla no existe".into()))?;
        rule.enabled = enabled;
        self.upsert(rule).await
    }

    /// Escucha el bus hasta que se cierre.
    pub fn spawn(self: &Arc<Self>, bus: &EventBus) -> tokio::task::JoinHandle<()> {
        let engine = Arc::clone(self);
        let mut rx = bus.subscribe();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(ev) => {
                        engine.handle(TriggerInput::Live(&ev)).await;
                    }
                    Err(RecvError::Lagged(n)) => {
                        tracing::warn!(missed = n, "el motor de reglas se quedó atrás en el bus");
                    }
                    Err(RecvError::Closed) => break,
                }
            }
        })
    }

    /// Dispara las reglas que correspondan a un evento interno (meta alcanzada, timer terminado).
    pub async fn fire_system(&self, ev: &SystemEvent) -> usize {
        match ev {
            SystemEvent::GoalReached(id) => self.handle(TriggerInput::GoalReached(id)).await,
            SystemEvent::TimerEnded(id) => self.handle(TriggerInput::TimerEnded(id)).await,
        }
    }

    /// Llamada de la API local: encola las reglas activas cuyo trigger es `Api { name }`, con las
    /// variables recibidas. Ignora condiciones y coste (quien llama ya tiene el token). Devuelve
    /// cuántas reglas se encolaron.
    pub async fn fire_api(&self, name: &str, mut vars: Vars) -> usize {
        vars.insert("event".into(), "api".into());
        vars.insert("api".into(), name.to_string());
        let now_ms = self.clock.now_ms();
        let mut queued = 0;
        for rule in self.snapshot().iter() {
            if !rule.enabled || !matches!(&rule.trigger, Trigger::Api { name: n } if n.eq_ignore_ascii_case(name)) {
                continue;
            }
            let job = NewJob {
                rule_id: rule.id.clone(),
                mode: rule.plan.mode,
                steps: rule.plan.steps.clone(),
                vars: vars.clone(),
                priority: rule.priority.unwrap_or(30),
                ttl_ms: rule.ttl_ms,
            };
            let ok = matches!(self.queue.enqueue(job).await, Ok(Outcome::Queued));
            if ok {
                queued += 1;
            } else {
                tracing::warn!(rule = %rule.name, "la llamada a la API no se pudo encolar");
            }
            let _ = self.fired.send(FiredReport {
                rule_id: rule.id.clone(),
                rule_name: rule.name.clone(),
                ts: now_ms,
                queued: ok,
                user: None,
                nickname: None,
                cost: None,
                points_left: None,
                platform: Platform::Tiktok,
            });
        }
        queued
    }

    /// Evalúa todas las reglas contra la entrada y encola lo que corresponda.
    /// Devuelve cuántos jobs se encolaron.
    pub async fn handle(&self, input: TriggerInput<'_>) -> usize {
        let rules = self.snapshot();
        if rules.is_empty() {
            return 0;
        }
        let now_ms = self.clock.now_ms();
        let local = chrono::Local::now().naive_local();
        let user = match &input {
            TriggerInput::Live(ev) if !ev.user.id.trim().is_empty() => Some(&ev.user),
            _ => None,
        };
        let gate = self.points.get();
        let platform = match &input {
            TriggerInput::Live(ev) => ev.platform,
            _ => Platform::Tiktok,
        };

        // El saldo solo se consulta si alguna recompensa con coste podría dispararse con este evento
        // (si no, cada like iría a la base de datos).
        let balance = match (user, gate) {
            (Some(u), Some(g)) if costly_rule_may_match(&rules, &input) => g.balance(&u.id).await.map_err(|e| tracing::warn!(error = %e, "no se pudo consultar el saldo de puntos")).ok(),
            _ => None,
        };

        let mut firings = Vec::new();
        let mut broke = Vec::new();
        {
            let mut states = self.states.lock().unwrap_or_else(PoisonError::into_inner);
            for rule in rules.iter() {
                let state = states.entry(rule.id.clone()).or_default();
                let env = EvalEnv { now_ms, local, roll: rand::random::<f64>(), balance };
                match evaluate(rule, &input, state, &env) {
                    Decision::Fire { times, vars } => firings.push((Arc::clone(rule), times, vars)),
                    Decision::Blocked(Blocked::Points) => broke.push(Arc::clone(rule)),
                    _ => {}
                }
            }
            self.maybe_prune(&mut states, &rules, now_ms);
        }

        // Sin sistema de puntos conectado no se avisa de «no tienes puntos»: no sabemos su saldo.
        if let (Some(u), Some(_)) = (user, gate) {
            for rule in broke {
                let _ = self.denied.send(DeniedReport {
                    rule_id: rule.id.clone(),
                    rule_name: rule.name.clone(),
                    user: u.unique_id.clone(),
                    nickname: display_name(&u.nickname, &u.unique_id),
                    cost: rule.cost_points.unwrap_or(0),
                    have: balance.unwrap_or(0),
                    platform,
                });
            }
        }

        let priority_hint = auto_priority(&input);
        let mut queued = 0;
        for (rule, times, vars) in firings {
            for _ in 0..times {
                // Cobro (atómico): si entre medias el saldo bajó, no se ejecuta.
                let mut paid = None;
                if let (Some(cost), Some(u), Some(g)) = (rule.cost_points, user, gate) {
                    match g.spend(&u.id, cost, &rule.name).await {
                        Ok(Some(left)) => paid = Some((cost, left)),
                        Ok(None) => {
                            let _ = self.denied.send(DeniedReport {
                                rule_id: rule.id.clone(),
                                rule_name: rule.name.clone(),
                                user: u.unique_id.clone(),
                                nickname: display_name(&u.nickname, &u.unique_id),
                                cost,
                                have: g.balance(&u.id).await.unwrap_or(0),
                                platform,
                            });
                            continue;
                        }
                        Err(e) => {
                            tracing::error!(rule = %rule.name, error = %e, "no se pudo cobrar la recompensa");
                            continue;
                        }
                    }
                }

                let job = NewJob {
                    rule_id: rule.id.clone(),
                    mode: rule.plan.mode,
                    steps: rule.plan.steps.clone(),
                    vars: vars.clone(),
                    priority: rule.priority.unwrap_or(priority_hint),
                    ttl_ms: rule.ttl_ms,
                };
                let ok = match self.queue.enqueue(job).await {
                    Ok(Outcome::Queued) => {
                        queued += 1;
                        true
                    }
                    Ok(Outcome::Dropped) => {
                        tracing::warn!(rule = %rule.name, "cola llena: se descartó la ejecución");
                        false
                    }
                    Ok(Outcome::Empty) => false,
                    Err(e) => {
                        tracing::error!(rule = %rule.name, error = %e, "no se pudo encolar");
                        false
                    }
                };
                // Si la recompensa no llegó a ejecutarse, se devuelven los puntos.
                if !ok {
                    if let (Some((cost, _)), Some(u), Some(g)) = (paid, user, gate) {
                        if let Err(e) = g.refund(&u.id, cost, &rule.name).await {
                            tracing::error!(rule = %rule.name, error = %e, "no se pudieron devolver los puntos");
                        }
                        paid = None;
                    }
                }
                let _ = self.fired.send(FiredReport {
                    rule_id: rule.id.clone(),
                    rule_name: rule.name.clone(),
                    ts: now_ms,
                    queued: ok,
                    user: user.map(|u| u.unique_id.clone()),
                    nickname: user.map(|u| display_name(&u.nickname, &u.unique_id)),
                    cost: paid.map(|(c, _)| c),
                    points_left: paid.map(|(_, l)| l),
                    platform,
                });
            }
        }
        queued
    }

    /// Botón «probar»: ejecuta la regla con un evento de ejemplo, ignorando condiciones y cooldowns.
    pub async fn test_rule(&self, id: &str) -> Result<Outcome> {
        let rule = self.get(id).ok_or_else(|| AppError::Invalid("la regla no existe".into()))?;
        let sample = sample_event_for(&rule.trigger);
        let mut vars = sample.as_ref().map(event_vars).unwrap_or_default();
        // Variables propias del trigger (args, command, keyword…) y de los triggers internos.
        let input = match (&sample, &rule.trigger) {
            (Some(ev), _) => Some(TriggerInput::Live(ev)),
            _ => None,
        };
        if let Some(inp) = input {
            if let Some(hit) = match_trigger(&rule.trigger, &inp, &mut 0) {
                vars.extend(hit.vars);
            }
        } else {
            vars.insert("event".into(), "test".into());
        }
        let priority = rule.priority.unwrap_or_else(|| input.as_ref().map_or(30, auto_priority));
        self.queue
            .enqueue(NewJob {
                rule_id: rule.id.clone(),
                mode: rule.plan.mode,
                steps: rule.plan.steps.clone(),
                vars,
                priority,
                ttl_ms: rule.ttl_ms,
            })
            .await
    }

    // ---- Internos ----

    fn snapshot(&self) -> Arc<Vec<Arc<Rule>>> {
        Arc::clone(&self.rules.read().unwrap_or_else(PoisonError::into_inner))
    }

    fn set_rules(&self, rules: Vec<Rule>) {
        let arcs: Vec<Arc<Rule>> = rules.into_iter().map(Arc::new).collect();
        *self.rules.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(arcs);
    }

    fn maybe_prune(&self, states: &mut HashMap<String, RuleState>, rules: &[Arc<Rule>], now_ms: i64) {
        let mut n = self.handled.lock().unwrap_or_else(PoisonError::into_inner);
        *n += 1;
        if !(*n).is_multiple_of(PRUNE_EVERY) {
            return;
        }
        for rule in rules {
            if let Some(st) = states.get_mut(&rule.id) {
                st.prune(now_ms, rule.conditions.user_cooldown_ms);
            }
        }
    }
}

fn display_name(nickname: &str, unique_id: &str) -> String {
    if nickname.is_empty() {
        unique_id.to_string()
    } else {
        nickname.to_string()
    }
}

/// ¿Alguna regla activa con coste coincide con el disparador de esta entrada?
fn costly_rule_may_match(rules: &[Arc<Rule>], input: &TriggerInput) -> bool {
    rules
        .iter()
        .any(|r| r.enabled && r.cost_points.is_some() && match_trigger(&r.trigger, input, &mut 0).is_some())
}

/// Atajo para tests y el simulador: evento en vivo como entrada.
pub fn live(ev: &LiveEvent) -> TriggerInput<'_> {
    TriggerInput::Live(ev)
}

#[cfg(test)]
mod tests;
