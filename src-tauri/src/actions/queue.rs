//! Cola de acciones con prioridad, concurrencia por tipo, límite de tamaño, caducidad y
//! persistencia.
//!
//! La unidad de la cola es el *paso* de un plan; el *job* (una ejecución de regla) coordina el
//! orden entre pasos (secuencia o paralelo, con retardos). Así la prioridad y la concurrencia se
//! aplican por acción: un TTS ocupado no bloquea a los sonidos.

use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde_json::{Map, Value};
use tokio::sync::Notify;

use super::clock::Clock;
use super::store::{Job, JobStore};
use super::{ActionContext, Concurrency, ExecutorRegistry};
use crate::error::Result;
use crate::rules::model::{PlanMode, Step};
use crate::rules::template::Vars;

#[derive(Debug, Clone, Copy)]
pub struct QueueConfig {
    /// Máximo de pasos esperando en la cola.
    pub max_pending: usize,
    /// Máximo de acciones ejecutándose a la vez (de cualquier tipo).
    pub max_parallel: usize,
    /// Una acción que tarde más que esto se aborta (libera su grupo serial).
    pub action_timeout: Duration,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            max_pending: 200,
            max_parallel: 32,
            action_timeout: Duration::from_secs(120),
        }
    }
}

/// Lo que una regla pide ejecutar.
#[derive(Debug, Clone)]
pub struct NewJob {
    pub rule_id: String,
    pub mode: PlanMode,
    pub steps: Vec<Step>,
    pub vars: Vars,
    pub priority: i32,
    pub ttl_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Queued,
    /// No cupo y no tenía prioridad suficiente para desplazar a nadie.
    Dropped,
    /// El plan no tenía pasos.
    Empty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueStats {
    pub pending: usize,
    pub running: usize,
    pub jobs: usize,
}

type Key = (Reverse<i32>, u64);

#[derive(Debug, Clone)]
struct Entry {
    job_id: String,
    step: usize,
    priority: i32,
    not_before_ms: i64,
    expires_ms: i64,
    group: Concurrency,
}

struct JobRt {
    job: Job,
    /// Pasos aún sin terminar (modo paralelo).
    remaining: usize,
}

#[derive(Default)]
struct State {
    pending: BTreeMap<Key, Entry>,
    jobs: HashMap<String, JobRt>,
    running_groups: HashSet<&'static str>,
    running_total: usize,
    seq: u64,
    shutdown: bool,
}

struct RunItem {
    job_id: String,
    step: usize,
    rule_id: String,
    kind: String,
    params: Map<String, Value>,
    vars: Vars,
    group: Concurrency,
}

/// Efectos a aplicar sobre el almacén una vez soltado el cerrojo.
#[derive(Default)]
struct StoreOps {
    delete: Vec<String>,
    progress: Vec<(String, usize)>,
}

struct Shared {
    state: Mutex<State>,
    notify: Notify,
    registry: ExecutorRegistry,
    store: Arc<dyn JobStore>,
    clock: Arc<dyn Clock>,
    cfg: QueueConfig,
}

pub struct ActionQueue {
    shared: Arc<Shared>,
}

impl ActionQueue {
    /// Arranca el despachador. Debe llamarse dentro de un runtime de Tokio.
    pub fn start(
        registry: ExecutorRegistry,
        store: Arc<dyn JobStore>,
        clock: Arc<dyn Clock>,
        cfg: QueueConfig,
    ) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            notify: Notify::new(),
            registry,
            store,
            clock,
            cfg,
        });
        tokio::spawn(dispatch_loop(Arc::clone(&shared)));
        Self { shared }
    }

    pub async fn enqueue(&self, new: NewJob) -> Result<Outcome> {
        let s = &self.shared;
        if new.steps.is_empty() {
            return Ok(Outcome::Empty);
        }
        let now = s.clock.now_ms();
        let job = Job {
            id: uuid::Uuid::new_v4().to_string(),
            rule_id: new.rule_id,
            mode: new.mode,
            steps: new.steps,
            vars: new.vars,
            priority: new.priority,
            created_ms: now,
            expires_ms: now.saturating_add(i64::try_from(new.ttl_ms).unwrap_or(i64::MAX)),
            next_step: 0,
        };
        let initial = match job.mode {
            PlanMode::Sequence => 1,
            PlanMode::Parallel => job.steps.len(),
        };

        let mut ops = StoreOps::default();
        {
            let mut st = s.lock();
            // Hacer sitio desplazando lo de menor prioridad, si el recién llegado vale más.
            while st.pending.len() + initial > s.cfg.max_pending {
                let Some((_, entry)) = st.pending.iter().next_back() else {
                    break;
                };
                if job.priority <= entry.priority {
                    return Ok(Outcome::Dropped);
                }
                let victim = entry.job_id.clone();
                cancel_job(&mut st, &victim, &mut ops);
            }
            s.insert_job(&mut st, job.clone(), job.next_step, now);
        }
        // Persistir fuera del cerrojo; un fallo no impide ejecutar en memoria.
        s.apply_ops(ops).await;
        if let Err(e) = s.store.save(&job).await {
            tracing::warn!(error = %e, "no se pudo persistir el job");
        }
        s.notify.notify_one();
        Ok(Outcome::Queued)
    }

    /// Recupera los jobs pendientes de una sesión anterior (descarta los caducados).
    pub async fn restore(&self) -> Result<usize> {
        let s = &self.shared;
        let jobs = s.store.load_all().await?;
        let now = s.clock.now_ms();
        let mut restored = 0;
        for job in jobs {
            if job.expires_ms <= now || job.steps.is_empty() || job.next_step >= job.steps.len() {
                let _ = s.store.delete(&job.id).await;
                continue;
            }
            let next = job.next_step;
            let mut st = s.lock();
            s.insert_job(&mut st, job, next, now);
            restored += 1;
        }
        s.notify.notify_one();
        Ok(restored)
    }

    /// Descarta todo lo que está en espera (lo que ya se ejecuta termina).
    pub async fn clear(&self) {
        let mut ops = StoreOps::default();
        {
            let mut st = self.shared.lock();
            let ids: Vec<String> = st.jobs.keys().cloned().collect();
            for id in ids {
                cancel_job(&mut st, &id, &mut ops);
            }
        }
        self.shared.apply_ops(ops).await;
    }

    pub fn stats(&self) -> QueueStats {
        let st = self.shared.lock();
        QueueStats {
            pending: st.pending.len(),
            running: st.running_total,
            jobs: st.jobs.len(),
        }
    }

    /// Detiene el despachador (lo que ya corre termina).
    pub fn shutdown(&self) {
        self.shared.lock().shutdown = true;
        self.shared.notify.notify_one();
    }
}

/// Quita un job y todos sus pasos en espera.
fn cancel_job(st: &mut State, job_id: &str, ops: &mut StoreOps) {
    st.pending.retain(|_, e| e.job_id != job_id);
    if st.jobs.remove(job_id).is_some() {
        ops.delete.push(job_id.to_string());
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // Un pánico previo no debe inutilizar la cola.
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn group_of(&self, kind: &str) -> Concurrency {
        self.registry.get(kind).map_or(Concurrency::Parallel, |e| e.concurrency())
    }

    /// Registra el job y deja en espera sus primeros pasos, empezando en `from`.
    fn insert_job(&self, st: &mut State, job: Job, from: usize, now: i64) {
        let indices: Vec<usize> = match job.mode {
            PlanMode::Sequence => vec![from],
            PlanMode::Parallel => (0..job.steps.len()).collect(),
        };
        let remaining = indices.len();
        for i in &indices {
            self.push_entry(st, &job, *i, now);
        }
        st.jobs.insert(job.id.clone(), JobRt { job, remaining });
    }

    fn push_entry(&self, st: &mut State, job: &Job, step: usize, now: i64) {
        let Some(s) = job.steps.get(step) else { return };
        st.seq += 1;
        let entry = Entry {
            job_id: job.id.clone(),
            step,
            priority: job.priority,
            not_before_ms: now.saturating_add(i64::try_from(s.delay_ms).unwrap_or(i64::MAX)),
            expires_ms: job.expires_ms,
            group: self.group_of(&s.action.kind),
        };
        st.pending.insert((Reverse(job.priority), st.seq), entry);
    }

    async fn apply_ops(&self, ops: StoreOps) {
        for id in ops.delete {
            if let Err(e) = self.store.delete(&id).await {
                tracing::warn!(error = %e, %id, "no se pudo borrar el job persistido");
            }
        }
        for (id, next) in ops.progress {
            if let Err(e) = self.store.set_progress(&id, next).await {
                tracing::warn!(error = %e, %id, "no se pudo guardar el progreso del job");
            }
        }
    }

    /// Elige la siguiente acción ejecutable. También descarta lo caducado.
    fn take_next(&self) -> (Option<RunItem>, Option<i64>, StoreOps) {
        let now = self.clock.now_ms();
        let mut ops = StoreOps::default();
        let mut st = self.lock();
        if st.shutdown {
            return (None, None, ops);
        }

        // 1. Caducidad: un job caducado se cancela entero.
        let expired: Vec<String> = st
            .pending
            .values()
            .filter(|e| e.expires_ms <= now)
            .map(|e| e.job_id.clone())
            .collect();
        for id in expired {
            tracing::debug!(job = %id, "job caducado sin ejecutarse");
            cancel_job(&mut st, &id, &mut ops);
        }

        // 2. Primera entrada (por prioridad) que ya toque y cuyo grupo esté libre.
        let mut wake_at: Option<i64> = None;
        let mut chosen: Option<Key> = None;
        if st.running_total < self.cfg.max_parallel {
            for (key, e) in &st.pending {
                if e.not_before_ms > now {
                    wake_at = Some(wake_at.map_or(e.not_before_ms, |w| w.min(e.not_before_ms)));
                    continue;
                }
                if let Concurrency::Serial(g) = e.group {
                    if st.running_groups.contains(g) {
                        continue;
                    }
                }
                chosen = Some(*key);
                break;
            }
        }
        // El próximo despertar también debe considerar la caducidad más cercana.
        if let Some(next_expiry) = st.pending.values().map(|e| e.expires_ms).min() {
            wake_at = Some(wake_at.map_or(next_expiry, |w| w.min(next_expiry)));
        }

        let Some(key) = chosen else {
            return (None, wake_at, ops);
        };
        let Some(entry) = st.pending.remove(&key) else {
            return (None, wake_at, ops);
        };
        let Some(rt) = st.jobs.get(&entry.job_id) else {
            return (None, Some(now), ops); // huérfana: reintenta de inmediato
        };
        let Some(step) = rt.job.steps.get(entry.step) else {
            return (None, Some(now), ops);
        };
        let item = RunItem {
            job_id: entry.job_id.clone(),
            step: entry.step,
            rule_id: rt.job.rule_id.clone(),
            kind: step.action.kind.clone(),
            params: step.action.params.clone(),
            vars: rt.job.vars.clone(),
            group: entry.group,
        };
        if let Concurrency::Serial(g) = entry.group {
            st.running_groups.insert(g);
        }
        st.running_total += 1;
        (Some(item), wake_at, ops)
    }

    /// Una acción terminó: libera su hueco y avanza el job.
    async fn complete(&self, item: &RunItem) {
        let now = self.clock.now_ms();
        let mut ops = StoreOps::default();
        {
            let mut st = self.lock();
            st.running_total = st.running_total.saturating_sub(1);
            if let Concurrency::Serial(g) = item.group {
                st.running_groups.remove(g);
            }
            let mut finished = false;
            let mut next: Option<(Job, usize)> = None;
            if let Some(rt) = st.jobs.get_mut(&item.job_id) {
                match rt.job.mode {
                    PlanMode::Sequence => {
                        let n = item.step + 1;
                        if n < rt.job.steps.len() {
                            rt.job.next_step = n;
                            next = Some((rt.job.clone(), n));
                        } else {
                            finished = true;
                        }
                    }
                    PlanMode::Parallel => {
                        rt.remaining = rt.remaining.saturating_sub(1);
                        finished = rt.remaining == 0;
                    }
                }
            }
            if let Some((job, n)) = next {
                self.push_entry(&mut st, &job, n, now);
                ops.progress.push((job.id, n));
            }
            if finished {
                st.jobs.remove(&item.job_id);
                ops.delete.push(item.job_id.clone());
            }
        }
        self.apply_ops(ops).await;
        self.notify.notify_one();
    }
}

async fn dispatch_loop(shared: Arc<Shared>) {
    loop {
        let (item, wake_at, ops) = shared.take_next();
        shared.apply_ops(ops).await;
        if shared.lock().shutdown {
            return;
        }
        if let Some(item) = item {
            let s = Arc::clone(&shared);
            tokio::spawn(async move { run_item(s, item).await });
            continue;
        }
        match wake_at {
            Some(at) => {
                let wait = u64::try_from((at - shared.clock.now_ms()).max(1)).unwrap_or(1);
                tokio::select! {
                    () = shared.notify.notified() => {}
                    () = tokio::time::sleep(Duration::from_millis(wait)) => {}
                }
            }
            None => shared.notify.notified().await,
        }
    }
}

async fn run_item(shared: Arc<Shared>, item: RunItem) {
    match shared.registry.get(&item.kind) {
        None => tracing::warn!(kind = %item.kind, rule = %item.rule_id, "no hay ejecutor para la acción; se omite"),
        Some(exec) => {
            let ctx = ActionContext {
                rule_id: item.rule_id.clone(),
                vars: item.vars.clone(),
            };
            let run = exec.execute(&ctx, &item.params);
            match tokio::time::timeout(shared.cfg.action_timeout, run).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    tracing::warn!(kind = %item.kind, rule = %item.rule_id, error = %e, "la acción falló");
                }
                Err(_) => {
                    tracing::warn!(kind = %item.kind, rule = %item.rule_id, "la acción superó el tiempo máximo y se abortó");
                }
            }
        }
    }
    shared.complete(&item).await;
}

#[cfg(test)]
mod tests;
