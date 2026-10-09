//! Cola de salida del bot: serializa los mensajes al chat respetando un ritmo mínimo, descarta los
//! que caducan esperando o se repiten, y recuerda lo enviado para que el bot no se responda a sí
//! mismo (sus mensajes vuelven por el chat como si los escribiera el streamer).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use serde::Serialize;
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::actions::clock::Clock;
use crate::error::Result;

/// Quién puede escribir en el chat (la conexión real, o uno falso en los tests).
#[async_trait]
pub trait ChatSender: Send + Sync {
    async fn send_chat(&self, text: String) -> Result<()>;
}

#[async_trait]
impl ChatSender for crate::connection::ConnectionService {
    async fn send_chat(&self, text: String) -> Result<()> {
        crate::connection::ConnectionService::send_chat(self, text).await
    }
}

#[derive(Debug, Clone, Copy)]
pub struct OutboxLimits {
    /// Mensajes máximos esperando turno.
    pub max_queue: usize,
    /// Un mensaje que espera más que esto ya no tiene sentido y se descarta.
    pub max_age: Duration,
    /// El mismo texto no se repite dentro de esta ventana.
    pub dedupe_window: Duration,
    /// Cuánto se recuerda lo enviado para detectar el eco.
    pub echo_window: Duration,
}

impl Default for OutboxLimits {
    fn default() -> Self {
        Self { max_queue: 30, max_age: Duration::from_secs(45), dedupe_window: Duration::from_secs(8), echo_window: Duration::from_secs(90) }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", content = "reason", rename_all = "camelCase")]
pub enum LogStatus {
    Sent,
    Failed(String),
    Dropped(String),
}

/// Una línea del registro del bot (lo que dijo, o intentó decir, y cómo fue).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub ts: i64,
    pub text: String,
    pub source: String,
    pub status: LogStatus,
}

const LOG_CAPACITY: usize = 60;

struct Queued {
    text: String,
    source: String,
    at: Instant,
}

struct Shared {
    log: Mutex<VecDeque<LogEntry>>,
    sent: Mutex<VecDeque<(Instant, String)>>,
    min_interval_ms: AtomicU64,
    clock: Arc<dyn Clock>,
    limits: OutboxLimits,
}

impl Shared {
    fn record(&self, text: &str, source: &str, status: LogStatus) {
        let mut log = self.log.lock().unwrap_or_else(PoisonError::into_inner);
        log.push_back(LogEntry { ts: self.clock.now_ms(), text: text.to_string(), source: source.to_string(), status });
        while log.len() > LOG_CAPACITY {
            log.pop_front();
        }
    }
}

/// Comparación tolerante: sin mayúsculas y con los espacios normalizados.
fn norm(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

#[derive(Clone)]
pub struct Outbox {
    tx: mpsc::Sender<Queued>,
    shared: Arc<Shared>,
}

impl Outbox {
    /// Arranca el trabajador. Debe llamarse dentro de un runtime de Tokio.
    pub fn start(sender: Arc<dyn ChatSender>, clock: Arc<dyn Clock>, min_interval: Duration, limits: OutboxLimits) -> Self {
        let (tx, rx) = mpsc::channel(limits.max_queue.max(1));
        let shared = Arc::new(Shared {
            log: Mutex::new(VecDeque::new()),
            sent: Mutex::new(VecDeque::new()),
            min_interval_ms: AtomicU64::new(u64::try_from(min_interval.as_millis()).unwrap_or(u64::MAX)),
            clock,
            limits,
        });
        tokio::spawn(worker(rx, sender, Arc::clone(&shared)));
        Self { tx, shared }
    }

    pub fn set_min_interval(&self, d: Duration) {
        self.shared.min_interval_ms.store(u64::try_from(d.as_millis()).unwrap_or(u64::MAX), Ordering::Relaxed);
    }

    /// Encola un mensaje. Devuelve `false` si la cola está llena (queda registrado como descartado).
    pub fn enqueue(&self, text: &str, source: &str) -> bool {
        let text = text.trim();
        if text.is_empty() {
            return false;
        }
        match self.tx.try_send(Queued { text: text.to_string(), source: source.to_string(), at: Instant::now() }) {
            Ok(()) => true,
            Err(_) => {
                self.shared.record(text, source, LogStatus::Dropped("la cola del bot está llena".into()));
                false
            }
        }
    }

    /// ¿Es este texto algo que el bot acaba de decir? (su propio eco en el chat)
    pub fn is_echo(&self, text: &str) -> bool {
        let wanted = norm(text);
        let window = self.shared.limits.echo_window;
        self.shared
            .sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .any(|(at, sent)| at.elapsed() <= window && *sent == wanted)
    }

    /// Últimas entradas del registro, de la más reciente a la más antigua.
    pub fn log(&self) -> Vec<LogEntry> {
        self.shared.log.lock().unwrap_or_else(PoisonError::into_inner).iter().rev().cloned().collect()
    }

    pub fn clear_log(&self) {
        self.shared.log.lock().unwrap_or_else(PoisonError::into_inner).clear();
    }
}

async fn worker(mut rx: mpsc::Receiver<Queued>, sender: Arc<dyn ChatSender>, shared: Arc<Shared>) {
    let mut last_send: Option<Instant> = None;
    while let Some(msg) = rx.recv().await {
        // Ritmo mínimo entre mensajes.
        if let Some(last) = last_send {
            let gap = Duration::from_millis(shared.min_interval_ms.load(Ordering::Relaxed));
            tokio::time::sleep_until(last + gap).await;
        }
        if msg.at.elapsed() > shared.limits.max_age {
            shared.record(&msg.text, &msg.source, LogStatus::Dropped("caducó esperando turno".into()));
            continue;
        }
        let key = norm(&msg.text);
        let repeated = shared
            .sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .any(|(at, t)| at.elapsed() <= shared.limits.dedupe_window && *t == key);
        if repeated {
            shared.record(&msg.text, &msg.source, LogStatus::Dropped("repetido hace unos segundos".into()));
            continue;
        }
        last_send = Some(Instant::now());
        match sender.send_chat(msg.text.clone()).await {
            Ok(()) => {
                let mut sent = shared.sent.lock().unwrap_or_else(PoisonError::into_inner);
                sent.push_back((Instant::now(), key));
                while sent.front().is_some_and(|(at, _)| at.elapsed() > shared.limits.echo_window) {
                    sent.pop_front();
                }
                drop(sent);
                shared.record(&msg.text, &msg.source, LogStatus::Sent);
            }
            Err(e) => {
                tracing::debug!(error = %e, "el bot no pudo escribir en el chat");
                shared.record(&msg.text, &msg.source, LogStatus::Failed(e.to_string()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::clock::AppClock;
    use crate::error::AppError;

    #[derive(Default)]
    struct FakeSender {
        sent: Mutex<Vec<(Duration, String)>>,
        fail_with: Mutex<Option<String>>,
        start: Option<Instant>,
    }

    impl FakeSender {
        fn new() -> Arc<Self> {
            Arc::new(Self { start: Some(Instant::now()), ..Default::default() })
        }
        fn texts(&self) -> Vec<String> {
            self.sent.lock().expect("lock").iter().map(|(_, t)| t.clone()).collect()
        }
    }

    #[async_trait]
    impl ChatSender for FakeSender {
        async fn send_chat(&self, text: String) -> Result<()> {
            if let Some(why) = self.fail_with.lock().expect("lock").clone() {
                return Err(AppError::Invalid(why));
            }
            let t = self.start.map_or(Duration::ZERO, |s| s.elapsed());
            self.sent.lock().expect("lock").push((t, text));
            Ok(())
        }
    }

    fn outbox(sender: &Arc<FakeSender>, interval_ms: u64) -> Outbox {
        Outbox::start(sender.clone(), Arc::new(AppClock::with_base(1_000)), Duration::from_millis(interval_ms), OutboxLimits::default())
    }

    async fn settle(ms: u64) {
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }

    #[tokio::test(start_paused = true)]
    async fn messages_go_out_in_order_respecting_the_minimum_interval() {
        let s = FakeSender::new();
        let o = outbox(&s, 2_000);
        for t in ["uno", "dos", "tres"] {
            assert!(o.enqueue(t, "test"));
        }
        settle(10_000).await;
        let sent = s.sent.lock().expect("lock").clone();
        assert_eq!(sent.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>(), ["uno", "dos", "tres"]);
        assert!(sent[1].0 - sent[0].0 >= Duration::from_millis(2_000));
        assert!(sent[2].0 - sent[1].0 >= Duration::from_millis(2_000));
        assert!(o.log().iter().all(|e| e.status == LogStatus::Sent));
    }

    #[tokio::test(start_paused = true)]
    async fn the_interval_can_change_at_runtime() {
        let s = FakeSender::new();
        let o = outbox(&s, 10_000);
        o.set_min_interval(Duration::from_millis(1_000));
        o.enqueue("a", "t");
        o.enqueue("b", "t");
        settle(1_500).await;
        assert_eq!(s.texts(), ["a", "b"]);
    }

    #[tokio::test(start_paused = true)]
    async fn stale_messages_are_dropped_instead_of_sent_late() {
        let s = FakeSender::new();
        let o = outbox(&s, 30_000); // el 2.º espera 30 s, el 3.º 60 s (> 45 s)
        for t in ["a", "b", "c"] {
            o.enqueue(t, "t");
        }
        settle(120_000).await;
        assert_eq!(s.texts(), ["a", "b"]);
        let log = o.log();
        assert_eq!(log[0].text, "c");
        assert!(matches!(&log[0].status, LogStatus::Dropped(r) if r.contains("caducó")));
    }

    #[tokio::test(start_paused = true)]
    async fn the_same_text_is_not_repeated_within_the_window() {
        let s = FakeSender::new();
        let o = outbox(&s, 1_000);
        o.enqueue("Únete al Discord", "t");
        o.enqueue("  únete   al discord ", "t"); // mismo texto, otra forma
        o.enqueue("otra cosa", "t");
        settle(5_000).await;
        assert_eq!(s.texts(), ["Únete al Discord", "otra cosa"]);
        assert!(o.log().iter().any(|e| matches!(&e.status, LogStatus::Dropped(r) if r.contains("repetido"))));
        // Pasada la ventana, ya puede decirse otra vez.
        settle(10_000).await;
        o.enqueue("Únete al Discord", "t");
        settle(2_000).await;
        assert_eq!(s.texts().len(), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn failures_are_logged_with_their_reason_and_do_not_stop_the_queue() {
        let s = FakeSender::new();
        *s.fail_with.lock().expect("lock") = Some("falta iniciar sesión".into());
        let o = outbox(&s, 1_000);
        o.enqueue("a", "t");
        settle(1_500).await;
        *s.fail_with.lock().expect("lock") = None;
        o.enqueue("b", "t");
        settle(3_000).await;
        assert_eq!(s.texts(), ["b"]);
        let log = o.log();
        assert_eq!(log[0].status, LogStatus::Sent);
        assert!(matches!(&log[1].status, LogStatus::Failed(r) if r.contains("iniciar sesión")));
        assert!(!o.is_echo("a"), "lo que falló no cuenta como enviado");
    }

    #[tokio::test(start_paused = true)]
    async fn sent_messages_are_recognised_as_echo_for_a_while() {
        let s = FakeSender::new();
        let o = outbox(&s, 1_000);
        o.enqueue("Hola   MUNDO", "t");
        settle(1_000).await;
        assert!(o.is_echo("hola mundo"));
        assert!(o.is_echo(" HOLA mundo "));
        assert!(!o.is_echo("otra cosa"));
        settle(100_000).await;
        assert!(!o.is_echo("hola mundo"), "pasada la ventana ya no es eco");
    }

    #[tokio::test(start_paused = true)]
    async fn a_full_queue_drops_new_messages_and_says_so() {
        let s = FakeSender::new();
        let o = Outbox::start(s.clone(), Arc::new(AppClock::with_base(0)), Duration::from_millis(10_000), OutboxLimits { max_queue: 2, ..Default::default() });
        // El trabajador toma el primero y se duerme; los siguientes llenan la cola.
        assert!(o.enqueue("1", "t"));
        settle(10).await;
        o.enqueue("2", "t");
        o.enqueue("3", "t");
        o.enqueue("4", "t");
        let accepted = [o.enqueue("5", "t"), o.enqueue("6", "t")];
        assert!(accepted.iter().any(|a| !a), "alguno debe rechazarse");
        assert!(o.log().iter().any(|e| matches!(&e.status, LogStatus::Dropped(r) if r.contains("llena"))));
    }

    #[tokio::test(start_paused = true)]
    async fn empty_messages_are_ignored_and_the_log_is_bounded() {
        let s = FakeSender::new();
        let o = outbox(&s, 1_000);
        assert!(!o.enqueue("   ", "t"));
        for i in 0..100 {
            *s.fail_with.lock().expect("lock") = Some("x".into());
            o.shared.record(&format!("m{i}"), "t", LogStatus::Sent);
        }
        assert_eq!(o.log().len(), LOG_CAPACITY);
        assert_eq!(o.log()[0].text, "m99");
        o.clear_log();
        assert!(o.log().is_empty());
    }
}
