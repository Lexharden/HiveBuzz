use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Map, Value};
use tokio::time::{sleep, Instant};

use super::*;
use crate::actions::clock::AppClock;
use crate::actions::store::MemoryJobStore;
use crate::actions::{ActionExecutor, Concurrency, ExecutorRegistry};
use crate::error::AppError;
use crate::rules::model::{ActionSpec, PlanMode, Step};

#[derive(Debug, Clone, PartialEq)]
struct Rec {
    label: String,
    start: u64,
    end: u64,
}

type Log = Arc<Mutex<Vec<Rec>>>;

struct Probe {
    kind: &'static str,
    conc: Concurrency,
    dur: Duration,
    fail: bool,
    log: Log,
    t0: Instant,
}

fn ms(t0: Instant) -> u64 {
    u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[async_trait]
impl ActionExecutor for Probe {
    fn kind(&self) -> &'static str {
        self.kind
    }
    fn concurrency(&self) -> Concurrency {
        self.conc
    }
    async fn execute(&self, ctx: &ActionContext, params: &Map<String, Value>) -> Result<()> {
        let label = ctx.render(params.get("label").and_then(Value::as_str).unwrap_or("?"));
        let start = ms(self.t0);
        sleep(self.dur).await;
        self.log.lock().expect("lock").push(Rec { label, start, end: ms(self.t0) });
        if self.fail {
            return Err(AppError::Invalid("falla a propósito".into()));
        }
        Ok(())
    }
}

struct Rig {
    queue: ActionQueue,
    store: Arc<MemoryJobStore>,
    log: Log,
}

fn probe(kind: &'static str, conc: Concurrency, dur_ms: u64, fail: bool, log: &Log, t0: Instant) -> Arc<Probe> {
    Arc::new(Probe { kind, conc, dur: Duration::from_millis(dur_ms), fail, log: log.clone(), t0 })
}

/// `snd` (paralelo, 10 ms), `tts` (serial, 1000 ms), `bad` (falla), `slow` (serial, 10 s).
fn rig(cfg: QueueConfig) -> Rig {
    let log: Log = Arc::default();
    let t0 = Instant::now();
    let mut reg = ExecutorRegistry::new();
    reg.register(probe("snd", Concurrency::Parallel, 10, false, &log, t0));
    reg.register(probe("tts", Concurrency::Serial("tts"), 1000, false, &log, t0));
    reg.register(probe("bad", Concurrency::Parallel, 10, true, &log, t0));
    reg.register(probe("slow", Concurrency::Serial("tts"), 10_000, false, &log, t0));
    let store = Arc::new(MemoryJobStore::default());
    let queue = ActionQueue::start(reg, store.clone(), Arc::new(AppClock::with_base(1_000_000)), cfg);
    Rig { queue, store, log }
}

fn step(kind: &str, label: &str, delay_ms: u64) -> Step {
    Step { delay_ms, action: ActionSpec::new(kind, json!({ "label": label })) }
}

fn job(steps: Vec<Step>, mode: PlanMode, priority: i32, ttl_ms: u64) -> NewJob {
    NewJob { rule_id: "r".into(), mode, steps, vars: Vars::new(), priority, ttl_ms, refund: None }
}

fn seq(steps: Vec<Step>) -> NewJob {
    job(steps, PlanMode::Sequence, 0, 600_000)
}

fn starts(rig: &Rig) -> Vec<(String, u64)> {
    let mut v: Vec<_> = rig.log.lock().expect("lock").iter().map(|r| (r.label.clone(), r.start)).collect();
    v.sort_by_key(|(_, s)| *s);
    v
}

fn labels_by_start(rig: &Rig) -> Vec<String> {
    starts(rig).into_iter().map(|(l, _)| l).collect()
}

async fn settle(ms: u64) {
    sleep(Duration::from_millis(ms)).await;
}

#[tokio::test(start_paused = true)]
async fn sequence_runs_in_order_and_delays_start_after_the_previous_step() {
    let r = rig(QueueConfig::default());
    r.queue.enqueue(seq(vec![step("snd", "A", 0), step("snd", "B", 500), step("snd", "C", 0)])).await.expect("enq");
    settle(5_000).await;
    // A: 0–10. B: espera 500 tras terminar A → 510–520. C: 520.
    assert_eq!(starts(&r), [("A".into(), 0), ("B".into(), 510), ("C".into(), 520)]);
}

#[tokio::test(start_paused = true)]
async fn parallel_plan_runs_steps_concurrently_each_with_its_own_delay() {
    let r = rig(QueueConfig::default());
    r.queue.enqueue(job(vec![step("snd", "A", 0), step("snd", "B", 100), step("snd", "C", 100)], PlanMode::Parallel, 0, 60_000)).await.expect("enq");
    settle(1_000).await;
    assert_eq!(starts(&r), [("A".into(), 0), ("B".into(), 100), ("C".into(), 100)]);
}

#[tokio::test(start_paused = true)]
async fn serial_group_runs_one_at_a_time_without_blocking_other_kinds() {
    let r = rig(QueueConfig::default());
    r.queue.enqueue(seq(vec![step("tts", "T1", 0)])).await.expect("enq");
    r.queue.enqueue(seq(vec![step("tts", "T2", 0)])).await.expect("enq");
    r.queue.enqueue(seq(vec![step("snd", "S", 0)])).await.expect("enq");
    settle(5_000).await;
    let s = starts(&r);
    assert!(s.contains(&("T1".into(), 0)));
    assert!(s.contains(&("S".into(), 0)), "el sonido no debe esperar al TTS: {s:?}");
    assert!(s.contains(&("T2".into(), 1000)), "el 2.º TTS espera al 1.º: {s:?}");
}

#[tokio::test(start_paused = true)]
async fn higher_priority_jumps_the_line_inside_a_busy_group() {
    let r = rig(QueueConfig::default());
    r.queue.enqueue(job(vec![step("tts", "running", 0)], PlanMode::Sequence, 0, 600_000)).await.expect("enq");
    settle(10).await;
    r.queue.enqueue(job(vec![step("tts", "low", 0)], PlanMode::Sequence, 0, 600_000)).await.expect("enq");
    r.queue.enqueue(job(vec![step("tts", "high", 0)], PlanMode::Sequence, 50, 600_000)).await.expect("enq");
    settle(10_000).await;
    assert_eq!(labels_by_start(&r), ["running", "high", "low"]);
}

#[tokio::test(start_paused = true)]
async fn equal_priority_is_fifo() {
    let r = rig(QueueConfig::default());
    for l in ["1", "2", "3", "4"] {
        r.queue.enqueue(seq(vec![step("tts", l, 0)])).await.expect("enq");
    }
    settle(10_000).await;
    assert_eq!(labels_by_start(&r), ["1", "2", "3", "4"]);
}

#[tokio::test(start_paused = true)]
async fn full_queue_drops_newcomers_of_equal_priority_and_evicts_lower_ones() {
    let r = rig(QueueConfig { max_pending: 2, ..QueueConfig::default() });
    r.queue.enqueue(seq(vec![step("tts", "running", 0)])).await.expect("enq");
    settle(10).await; // pasa a ejecución; la cola queda vacía
    assert_eq!(r.queue.enqueue(seq(vec![step("tts", "low1", 0)])).await.expect("enq"), Outcome::Queued);
    assert_eq!(r.queue.enqueue(seq(vec![step("tts", "low2", 0)])).await.expect("enq"), Outcome::Queued);
    assert_eq!(r.queue.enqueue(seq(vec![step("tts", "low3", 0)])).await.expect("enq"), Outcome::Dropped);
    let high = job(vec![step("tts", "high", 0)], PlanMode::Sequence, 10, 600_000);
    assert_eq!(r.queue.enqueue(high).await.expect("enq"), Outcome::Queued);
    settle(20_000).await;
    // "low2" (el más reciente de los de menor prioridad) fue desplazado.
    assert_eq!(labels_by_start(&r), ["running", "high", "low1"]);
    assert_eq!(r.store.len(), 0, "los jobs descartados también se borran del almacén");
}

#[tokio::test(start_paused = true)]
async fn expired_jobs_are_dropped_without_running() {
    let r = rig(QueueConfig::default());
    r.queue.enqueue(seq(vec![step("tts", "running", 0)])).await.expect("enq");
    settle(10).await;
    r.queue.enqueue(job(vec![step("tts", "stale", 0)], PlanMode::Sequence, 0, 300)).await.expect("enq");
    settle(5_000).await;
    assert_eq!(labels_by_start(&r), ["running"]);
    assert_eq!(r.queue.stats(), QueueStats { pending: 0, running: 0, jobs: 0 });
    assert!(r.store.is_empty());
}

#[tokio::test(start_paused = true)]
async fn persists_while_pending_and_forgets_on_completion() {
    let r = rig(QueueConfig::default());
    r.queue.enqueue(seq(vec![step("snd", "A", 0), step("snd", "B", 1000)])).await.expect("enq");
    settle(100).await; // A terminó; B espera su retardo
    let saved = r.store.load_all().await.expect("load");
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].next_step, 1, "el progreso se guarda para poder reanudar");
    settle(5_000).await;
    assert!(r.store.is_empty());
}

#[tokio::test(start_paused = true)]
async fn restore_resumes_sequences_and_discards_expired_jobs() {
    let r = rig(QueueConfig::default());
    let now = 1_000_000;
    let mk = |id: &str, next_step, expires_ms| Job {
        id: id.into(),
        rule_id: "r".into(),
        mode: PlanMode::Sequence,
        steps: vec![step("snd", "first", 0), step("snd", "second", 0)],
        vars: Vars::new(),
        priority: 0,
        created_ms: now - 1000,
        expires_ms,
        next_step,
        refund: None,
    };
    r.store.save(&mk("alive", 1, now + 60_000)).await.expect("save");
    r.store.save(&mk("dead", 0, now - 1)).await.expect("save");
    assert_eq!(r.queue.restore().await.expect("restore"), 1);
    settle(1_000).await;
    assert_eq!(labels_by_start(&r), ["second"], "solo se ejecuta lo que faltaba");
    assert!(r.store.is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_failing_or_unknown_action_does_not_stop_the_sequence() {
    let r = rig(QueueConfig::default());
    r.queue.enqueue(seq(vec![step("bad", "fails", 0), step("nope", "unknown", 0), step("snd", "after", 0)])).await.expect("enq");
    settle(1_000).await;
    assert_eq!(labels_by_start(&r), ["fails", "after"]);
    assert_eq!(r.queue.stats().jobs, 0);
}

#[tokio::test(start_paused = true)]
async fn action_timeout_frees_the_serial_group() {
    let cfg = QueueConfig { action_timeout: Duration::from_millis(1_000), ..QueueConfig::default() };
    let r = rig(cfg);
    r.queue.enqueue(seq(vec![step("slow", "stuck", 0)])).await.expect("enq");
    r.queue.enqueue(seq(vec![step("tts", "next", 0)])).await.expect("enq");
    settle(5_000).await;
    // "stuck" nunca termina de registrarse (se aborta a los 1000 ms) y "next" arranca en ese momento.
    assert_eq!(starts(&r), [("next".into(), 1000)]);
}

#[tokio::test(start_paused = true)]
async fn clear_discards_everything_waiting() {
    let r = rig(QueueConfig::default());
    r.queue.enqueue(seq(vec![step("tts", "running", 0)])).await.expect("enq");
    settle(10).await;
    r.queue.enqueue(seq(vec![step("tts", "w1", 0)])).await.expect("enq");
    r.queue.enqueue(seq(vec![step("tts", "w2", 0)])).await.expect("enq");
    r.queue.clear().await;
    settle(10_000).await;
    assert_eq!(labels_by_start(&r), ["running"]);
    assert!(r.store.is_empty());
}

#[tokio::test(start_paused = true)]
async fn empty_plan_is_ignored() {
    let r = rig(QueueConfig::default());
    assert_eq!(r.queue.enqueue(seq(vec![])).await.expect("enq"), Outcome::Empty);
    assert!(r.store.is_empty());
}

#[tokio::test(start_paused = true)]
async fn templates_use_the_job_variables() {
    let r = rig(QueueConfig::default());
    let mut j = seq(vec![step("snd", "Hola {nickname}", 0)]);
    j.vars.insert("nickname".into(), "Ana".into());
    r.queue.enqueue(j).await.expect("enq");
    settle(100).await;
    assert_eq!(labels_by_start(&r), ["Hola Ana"]);
}

#[tokio::test(start_paused = true)]
async fn global_parallel_cap_is_respected() {
    let r = rig(QueueConfig { max_parallel: 2, ..QueueConfig::default() });
    for l in ["a", "b", "c"] {
        r.queue.enqueue(seq(vec![step("snd", l, 0)])).await.expect("enq");
    }
    settle(1_000).await;
    // Dos arrancan en 0; la tercera espera a que termine alguna (10 ms).
    assert_eq!(starts(&r), [("a".into(), 0), ("b".into(), 0), ("c".into(), 10)]);
}

fn paid(steps: Vec<Step>, priority: i32, ttl_ms: u64, who: &str) -> NewJob {
    NewJob { refund: Some(Refund { user_id: who.into(), cost: 50, reward: "premio".into() }), ..job(steps, PlanMode::Sequence, priority, ttl_ms) }
}

fn refund_log(r: &Rig) -> Arc<Mutex<Vec<String>>> {
    let got: Arc<Mutex<Vec<String>>> = Arc::default();
    let g = Arc::clone(&got);
    r.queue.on_discard(Arc::new(move |rf| g.lock().expect("lock").push(rf.user_id)));
    got
}

#[tokio::test(start_paused = true)]
async fn paid_jobs_discarded_before_running_are_refunded() {
    let r = rig(QueueConfig { max_pending: 1, ..QueueConfig::default() });
    let got = refund_log(&r);
    r.queue.enqueue(seq(vec![step("tts", "running", 0)])).await.expect("enq");
    settle(10).await;
    // Desplazado por uno de más prioridad.
    r.queue.enqueue(paid(vec![step("tts", "evicted", 0)], 0, 600_000, "ana")).await.expect("enq");
    r.queue.enqueue(job(vec![step("tts", "vip", 0)], PlanMode::Sequence, 10, 600_000)).await.expect("enq");
    // Caducado en la cola (el grupo «tts» está ocupado 10 s).
    settle(3_000).await;
    r.queue.enqueue(seq(vec![step("slow", "busy", 0)])).await.expect("enq");
    settle(10).await;
    r.queue.enqueue(paid(vec![step("tts", "stale", 0)], 0, 300, "bob")).await.expect("enq");
    settle(15_000).await;
    assert_eq!(*got.lock().expect("lock"), ["ana", "bob"]);
    // Uno que se ejecuta no se devuelve, ni uno que la cola rechaza al encolar (eso lo hace quien encola).
    r.queue.enqueue(paid(vec![step("snd", "ok", 0)], 0, 600_000, "carla")).await.expect("enq");
    settle(1_000).await;
    assert_eq!(got.lock().expect("lock").len(), 2);
    assert!(r.store.is_empty());
}
