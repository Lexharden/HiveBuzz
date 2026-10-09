//! Supervisor del sidecar Node: lo lanza, traduce su NDJSON a `SourceMessage`, le envía
//! comandos y, si muere, lo reinicia solo (backoff exponencial con jitter) restaurando
//! la conexión que el usuario tenía pedida.

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;

use super::protocol::{LogLevel, SidecarCommand, SidecarMessage};
use super::{ChatReply, ConnectTarget, LiveSource, SourceMessage, StatusUpdate};
use crate::error::{AppError, Result};
use crate::source::protocol::ConnectionState;

/// Un proceso del sidecar ya lanzado, visto como canales (así se puede simular en tests).
pub struct ProcessHandle {
    pub output: mpsc::Receiver<ProcessOutput>,
    pub stdin: mpsc::Sender<String>,
    pub kill: Box<dyn FnOnce() + Send>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessOutput {
    Stdout(String),
    Stderr(String),
    Exited(Option<i32>),
}

/// Lanza el proceso del sidecar. La implementación real vive en `tauri_spawner`.
pub trait Spawner: Send + Sync + 'static {
    fn spawn(&self) -> Result<ProcessHandle>;
}

#[derive(Debug, Clone, Copy)]
pub struct RestartConfig {
    pub base: Duration,
    pub max: Duration,
    /// Si el proceso vivió al menos esto, se considera sano y el backoff se reinicia.
    pub healthy_after: Duration,
}

impl Default for RestartConfig {
    fn default() -> Self {
        Self {
            base: Duration::from_secs(1),
            max: Duration::from_secs(60),
            healthy_after: Duration::from_secs(30),
        }
    }
}

/// Backoff exponencial con jitter: entre `cap/2` y `cap`, con `cap = min(max, base·2^intento)`.
pub fn backoff_delay(attempt: u32, base: Duration, max: Duration, jitter: f64) -> Duration {
    let exp = base.saturating_mul(2u32.saturating_pow(attempt.min(30)));
    exp.min(max).mul_f64(0.5 + 0.5 * jitter.clamp(0.0, 1.0))
}

/// Máximo de mensajes del bot esperando respuesta de TikTok.
const MAX_PENDING_CHAT: usize = 50;
/// Tiempo máximo esperando el resultado de un mensaje (el sidecar tiene su propio plazo, algo menor).
const CHAT_REPLY_TIMEOUT: Duration = Duration::from_secs(25);

enum Control {
    Connect(ConnectTarget),
    Disconnect,
    SendChat {
        request_id: String,
        text: String,
        reply: oneshot::Sender<ChatReply>,
    },
    Shutdown,
}

/// `LiveSource` respaldada por el sidecar.
pub struct SidecarSource {
    ctl: mpsc::UnboundedSender<Control>,
}

impl SidecarSource {
    /// Arranca el supervisor. Debe llamarse dentro de un runtime de Tokio.
    pub fn start<S: Spawner>(
        spawner: S,
        out: mpsc::Sender<SourceMessage>,
        cfg: RestartConfig,
    ) -> Self {
        let (ctl, rx) = mpsc::unbounded_channel();
        let supervisor = Supervisor {
            spawner,
            out,
            cfg,
            desired: None,
            attempt: 0,
            pending_chat: HashMap::new(),
        };
        tokio::spawn(supervisor.run(rx));
        Self { ctl }
    }

    /// Detiene el sidecar y el supervisor.
    pub fn shutdown(&self) {
        let _ = self.ctl.send(Control::Shutdown);
    }

    fn send(&self, c: Control) -> Result<()> {
        self.ctl
            .send(c)
            .map_err(|_| AppError::Sidecar("el supervisor del sidecar no está activo".into()))
    }
}

#[async_trait]
impl LiveSource for SidecarSource {
    async fn connect(&self, target: ConnectTarget) -> Result<()> {
        self.send(Control::Connect(target))
    }

    async fn disconnect(&self) -> Result<()> {
        self.send(Control::Disconnect)
    }

    async fn send_chat(&self, text: String) -> Result<()> {
        let (reply, rx) = oneshot::channel();
        self.send(Control::SendChat { request_id: uuid::Uuid::new_v4().to_string(), text, reply })?;
        match tokio::time::timeout(CHAT_REPLY_TIMEOUT, rx).await {
            Ok(Ok(Ok(()))) => Ok(()),
            Ok(Ok(Err(why))) => Err(AppError::Invalid(why)),
            Ok(Err(_)) => Err(AppError::Sidecar("el sidecar no respondió al mensaje".into())),
            Err(_) => Err(AppError::Invalid("TikTok tardó demasiado en confirmar el mensaje".into())),
        }
    }
}

enum SessionEnd {
    Shutdown,
    Died(Option<i32>),
}

struct Supervisor<S> {
    spawner: S,
    out: mpsc::Sender<SourceMessage>,
    cfg: RestartConfig,
    /// Conexión que el usuario tiene pedida; se restaura tras reiniciar el sidecar.
    desired: Option<ConnectTarget>,
    attempt: u32,
    /// Mensajes del bot enviados al sidecar que esperan su `chatResult`.
    pending_chat: HashMap<String, oneshot::Sender<ChatReply>>,
}

impl<S: Spawner> Supervisor<S> {
    async fn run(mut self, mut ctl: mpsc::UnboundedReceiver<Control>) {
        loop {
            let reason = match self.spawner.spawn() {
                Ok(proc) => {
                    let started = Instant::now();
                    let end = self.session(proc, &mut ctl).await;
                    // Ningún mensaje del bot sobrevive a su proceso: quien espera recibe un error.
                    self.fail_pending_chat("el sidecar se detuvo antes de confirmar el mensaje");
                    match end {
                        SessionEnd::Shutdown => return,
                        SessionEnd::Died(code) => {
                            if started.elapsed() >= self.cfg.healthy_after {
                                self.attempt = 0;
                            }
                            match code {
                                Some(c) => format!("el sidecar terminó (código {c})"),
                                None => "el sidecar terminó".to_string(),
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "no se pudo lanzar el sidecar");
                    format!("no se pudo lanzar el sidecar: {e}")
                }
            };

            let delay = backoff_delay(
                self.attempt,
                self.cfg.base,
                self.cfg.max,
                rand::random::<f64>(),
            );
            self.attempt = self.attempt.saturating_add(1);
            tracing::warn!(%reason, ?delay, attempt = self.attempt, "reiniciando el sidecar");

            // Solo se avisa a la UI si el usuario esperaba estar conectado.
            if self.desired.is_some() {
                self.emit(SourceMessage::Status(StatusUpdate {
                    state: ConnectionState::Reconnecting,
                    detail: Some(reason),
                    attempt: Some(self.attempt),
                    retry_in_ms: Some(u64::try_from(delay.as_millis()).unwrap_or(u64::MAX)),
                }))
                .await;
            }
            if self.wait(delay, &mut ctl).await {
                return;
            }
        }
    }

    /// Espera `delay` atendiendo comandos. Devuelve `true` si hay que terminar.
    async fn wait(&mut self, delay: Duration, ctl: &mut mpsc::UnboundedReceiver<Control>) -> bool {
        let sleep = tokio::time::sleep(delay);
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                () = &mut sleep => return false,
                c = ctl.recv() => match c {
                    None | Some(Control::Shutdown) => return true,
                    Some(Control::Connect(t)) => self.desired = Some(t),
                    Some(Control::Disconnect) => {
                        self.desired = None;
                        self.emit(SourceMessage::Status(StatusUpdate::new(ConnectionState::Disconnected))).await;
                    }
                    Some(Control::SendChat { reply, .. }) => {
                        let _ = reply.send(Err("el sidecar se está reiniciando".into()));
                    }
                },
            }
        }
    }

    fn fail_pending_chat(&mut self, why: &str) {
        for (_, tx) in self.pending_chat.drain() {
            let _ = tx.send(Err(why.to_string()));
        }
    }

    async fn session(
        &mut self,
        mut proc: ProcessHandle,
        ctl: &mut mpsc::UnboundedReceiver<Control>,
    ) -> SessionEnd {
        let mut ready = false;
        loop {
            tokio::select! {
                c = ctl.recv() => match c {
                    None | Some(Control::Shutdown) => {
                        send_cmd(&proc.stdin, &SidecarCommand::Shutdown).await;
                        (proc.kill)();
                        return SessionEnd::Shutdown;
                    }
                    Some(Control::Connect(t)) => {
                        self.desired = Some(t);
                        if ready {
                            self.send_connect(&proc.stdin).await;
                        }
                    }
                    Some(Control::Disconnect) => {
                        self.desired = None;
                        if ready {
                            send_cmd(&proc.stdin, &SidecarCommand::Disconnect).await;
                        } else {
                            self.emit(SourceMessage::Status(StatusUpdate::new(ConnectionState::Disconnected))).await;
                        }
                    }
                    Some(Control::SendChat { request_id, text, reply }) => {
                        // Se descartan las peticiones cuyo emisor ya se rindió (timeout) antes de acumular más.
                        self.pending_chat.retain(|_, tx| !tx.is_closed());
                        if !ready {
                            let _ = reply.send(Err("el sidecar aún no está listo".into()));
                        } else if self.pending_chat.len() >= MAX_PENDING_CHAT {
                            let _ = reply.send(Err("hay demasiados mensajes esperando respuesta de TikTok".into()));
                        } else {
                            self.pending_chat.insert(request_id.clone(), reply);
                            send_cmd(&proc.stdin, &SidecarCommand::SendChat { request_id, text }).await;
                        }
                    }
                },
                o = proc.output.recv() => match o {
                    Some(ProcessOutput::Stdout(line)) => self.on_line(&line, &proc.stdin, &mut ready).await,
                    Some(ProcessOutput::Stderr(line)) => tracing::debug!(target: "sidecar", "{line}"),
                    Some(ProcessOutput::Exited(code)) => return SessionEnd::Died(code),
                    None => return SessionEnd::Died(None),
                },
            }
        }
    }

    async fn on_line(&mut self, line: &str, stdin: &mpsc::Sender<String>, ready: &mut bool) {
        if line.trim().is_empty() {
            return;
        }
        match SidecarMessage::from_line(line) {
            Ok(SidecarMessage::Ready) => {
                *ready = true;
                self.send_connect(stdin).await;
            }
            Ok(SidecarMessage::Event { event }) => self.emit(SourceMessage::Event(event)).await,
            Ok(SidecarMessage::Status {
                state,
                detail,
                attempt,
                retry_in_ms,
            }) => {
                self.emit(SourceMessage::Status(StatusUpdate {
                    state,
                    detail,
                    attempt,
                    retry_in_ms,
                }))
                .await;
            }
            Ok(SidecarMessage::Viewers { count }) => self.emit(SourceMessage::Viewers(count)).await,
            Ok(SidecarMessage::ChatResult { request_id, ok, error }) => {
                // Un resultado sin petición (ya caducó) simplemente se ignora.
                if let Some(tx) = self.pending_chat.remove(&request_id) {
                    let _ = tx.send(if ok { Ok(()) } else { Err(error.unwrap_or_else(|| "TikTok rechazó el mensaje".into())) });
                }
            }
            Ok(SidecarMessage::Log { level, message }) => {
                match level {
                    LogLevel::Debug => tracing::debug!(target: "sidecar", "{message}"),
                    LogLevel::Info => tracing::info!(target: "sidecar", "{message}"),
                    LogLevel::Warn => tracing::warn!(target: "sidecar", "{message}"),
                    LogLevel::Error => tracing::error!(target: "sidecar", "{message}"),
                }
                self.emit(SourceMessage::Log { level, message }).await;
            }
            // Regla 5: una línea ilegible se registra y se sigue.
            Err(e) => {
                let snippet: String = line.chars().take(200).collect();
                tracing::warn!(error = %e, line = %snippet, "línea del sidecar no válida");
                self.emit(SourceMessage::Log {
                    level: LogLevel::Warn,
                    message: format!("línea del sidecar no válida: {e}"),
                })
                .await;
            }
        }
    }

    async fn send_connect(&self, stdin: &mpsc::Sender<String>) {
        if let Some(t) = &self.desired {
            send_cmd(
                stdin,
                &SidecarCommand::Connect {
                    unique_id: t.unique_id.clone(),
                    euler_api_key: t.euler_api_key.clone(),
                    session: t.session.clone(),
                },
            )
            .await;
        }
    }

    async fn emit(&self, msg: SourceMessage) {
        // Si nadie escucha, se descarta: la app se está cerrando.
        let _ = self.out.send(msg).await;
    }
}

async fn send_cmd(stdin: &mpsc::Sender<String>, cmd: &SidecarCommand) {
    match cmd.to_line() {
        Ok(line) => {
            if stdin.send(line).await.is_err() {
                tracing::warn!("el sidecar ya no acepta comandos");
            }
        }
        Err(e) => tracing::error!(error = %e, "no se pudo serializar el comando"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::testing::sample_event;
    use crate::source::protocol::SessionPayload;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    /// Lado "proceso" de un sidecar simulado.
    struct FakeProc {
        stdout: mpsc::Sender<ProcessOutput>,
        stdin: mpsc::Receiver<String>,
        killed: Arc<AtomicBool>,
    }

    impl FakeProc {
        async fn say(&self, line: &str) {
            self.stdout
                .send(ProcessOutput::Stdout(line.to_string()))
                .await
                .expect("stdout");
        }
        async fn die(&self, code: i32) {
            self.stdout
                .send(ProcessOutput::Exited(Some(code)))
                .await
                .expect("exit");
        }
        async fn next_cmd(&mut self) -> String {
            self.stdin.recv().await.expect("comando")
        }
    }

    fn fake() -> (ProcessHandle, FakeProc) {
        let (otx, orx) = mpsc::channel(64);
        let (itx, irx) = mpsc::channel(64);
        let killed = Arc::new(AtomicBool::new(false));
        let k = killed.clone();
        (
            ProcessHandle {
                output: orx,
                stdin: itx,
                kill: Box::new(move || k.store(true, Ordering::SeqCst)),
            },
            FakeProc {
                stdout: otx,
                stdin: irx,
                killed,
            },
        )
    }

    struct FakeSpawner {
        queue: Arc<Mutex<VecDeque<Result<ProcessHandle>>>>,
        spawns: Arc<Mutex<u32>>,
    }

    impl Spawner for FakeSpawner {
        fn spawn(&self) -> Result<ProcessHandle> {
            *self.spawns.lock().expect("lock") += 1;
            self.queue
                .lock()
                .expect("lock")
                .pop_front()
                .unwrap_or_else(|| Err(AppError::Sidecar("sin más procesos".into())))
        }
    }

    struct Harness {
        source: Arc<SidecarSource>,
        out: mpsc::Receiver<SourceMessage>,
        spawns: Arc<Mutex<u32>>,
    }

    fn harness(procs: Vec<Result<ProcessHandle>>) -> Harness {
        let spawns = Arc::new(Mutex::new(0));
        let spawner = FakeSpawner {
            queue: Arc::new(Mutex::new(procs.into())),
            spawns: spawns.clone(),
        };
        let (tx, out) = mpsc::channel(64);
        Harness {
            source: Arc::new(SidecarSource::start(spawner, tx, RestartConfig::default())),
            out,
            spawns,
        }
    }

    fn target(user: &str) -> ConnectTarget {
        ConnectTarget::new(user)
    }

    const CONNECT_ANA: &str = "{\"cmd\":\"connect\",\"uniqueId\":\"ana\"}\n";

    #[test]
    fn backoff_grows_and_caps_at_max() {
        let b = |a, j| backoff_delay(a, Duration::from_secs(1), Duration::from_secs(60), j);
        assert_eq!(b(0, 1.0), Duration::from_secs(1));
        assert_eq!(b(0, 0.0), Duration::from_millis(500));
        assert_eq!(b(3, 1.0), Duration::from_secs(8));
        assert_eq!(b(10, 1.0), Duration::from_secs(60));
        assert_eq!(b(u32::MAX, 1.0), Duration::from_secs(60));
    }

    #[tokio::test(start_paused = true)]
    async fn connect_before_ready_is_sent_once_ready() {
        let (handle, mut p) = fake();
        let h = harness(vec![Ok(handle)]);
        h.source.connect(target("ana")).await.expect("connect");
        p.say(r#"{"kind":"ready"}"#).await;
        assert_eq!(p.next_cmd().await, CONNECT_ANA);
    }

    #[tokio::test(start_paused = true)]
    async fn connect_after_ready_is_sent_immediately() {
        let (handle, mut p) = fake();
        let h = harness(vec![Ok(handle)]);
        p.say(r#"{"kind":"ready"}"#).await;
        h.source.connect(target("ana")).await.expect("connect");
        assert_eq!(p.next_cmd().await, CONNECT_ANA);
    }

    #[tokio::test(start_paused = true)]
    async fn forwards_events_status_and_logs_in_order() {
        let (handle, p) = fake();
        let mut h = harness(vec![Ok(handle)]);
        let ev = serde_json::to_string(&SidecarMessage::Event {
            event: Box::new(sample_event("e1")),
        })
        .expect("json");
        p.say(&ev).await;
        p.say(r#"{"kind":"status","state":"connected"}"#).await;
        p.say(r#"{"kind":"log","level":"warn","message":"hola"}"#).await;

        assert!(matches!(h.out.recv().await, Some(SourceMessage::Event(e)) if e.id == "e1"));
        assert_eq!(
            h.out.recv().await,
            Some(SourceMessage::Status(StatusUpdate::new(ConnectionState::Connected)))
        );
        assert_eq!(
            h.out.recv().await,
            Some(SourceMessage::Log {
                level: LogLevel::Warn,
                message: "hola".into()
            })
        );
    }

    #[tokio::test(start_paused = true)]
    async fn forwards_viewer_counts() {
        let (handle, p) = fake();
        let mut h = harness(vec![Ok(handle)]);
        p.say(r#"{"kind":"viewers","count":321}"#).await;
        assert_eq!(h.out.recv().await, Some(SourceMessage::Viewers(321)));
    }

    // ---- Mensajes del bot (send_chat) ----

    /// Lanza `send_chat` en segundo plano y devuelve el comando que le llega al sidecar.
    async fn start_send(h: &Harness, p: &mut FakeProc, text: &str) -> (tokio::task::JoinHandle<Result<()>>, String) {
        let src = h.source.clone();
        let text = text.to_string();
        let task = tokio::spawn(async move { src.send_chat(text).await });
        let line = p.next_cmd().await;
        (task, line)
    }

    fn request_id(line: &str) -> String {
        let v: serde_json::Value = serde_json::from_str(line.trim()).expect("json");
        assert_eq!(v["cmd"], "sendChat");
        v["requestId"].as_str().expect("requestId").to_string()
    }

    #[tokio::test(start_paused = true)]
    async fn send_chat_waits_for_the_matching_result() {
        let (handle, mut p) = fake();
        let h = harness(vec![Ok(handle)]);
        p.say(r#"{"kind":"ready"}"#).await;
        let (task, line) = start_send(&h, &mut p, "hola \"chat\"").await;
        let v: serde_json::Value = serde_json::from_str(line.trim()).expect("json");
        assert_eq!(v["text"], "hola \"chat\"");
        let id = request_id(&line);
        // Un resultado de otra petición no desbloquea a esta.
        p.say(r#"{"kind":"chatResult","requestId":"otra","ok":true}"#).await;
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(!task.is_finished());
        p.say(&format!(r#"{{"kind":"chatResult","requestId":"{id}","ok":true}}"#)).await;
        task.await.expect("join").expect("ok");
    }

    #[tokio::test(start_paused = true)]
    async fn send_chat_surfaces_the_reason_tiktok_gave() {
        let (handle, mut p) = fake();
        let h = harness(vec![Ok(handle)]);
        p.say(r#"{"kind":"ready"}"#).await;
        let (task, line) = start_send(&h, &mut p, "hola").await;
        let id = request_id(&line);
        p.say(&format!(r#"{{"kind":"chatResult","requestId":"{id}","ok":false,"error":"falta iniciar sesión"}}"#)).await;
        let err = task.await.expect("join").expect_err("debe fallar");
        assert!(err.to_string().contains("falta iniciar sesión"), "{err}");
    }

    #[tokio::test(start_paused = true)]
    async fn send_chat_fails_fast_when_the_sidecar_is_not_ready() {
        let (handle, _p) = fake();
        let h = harness(vec![Ok(handle)]);
        let err = h.source.send_chat("hola".into()).await.expect_err("sin Ready");
        assert!(err.to_string().contains("no está listo"), "{err}");
    }

    #[tokio::test(start_paused = true)]
    async fn pending_messages_fail_when_the_sidecar_dies() {
        let (handle, mut p) = fake();
        let h = harness(vec![Ok(handle)]);
        p.say(r#"{"kind":"ready"}"#).await;
        let (task, _line) = start_send(&h, &mut p, "hola").await;
        p.die(1).await;
        let err = task.await.expect("join").expect_err("debe fallar");
        assert!(err.to_string().contains("se detuvo"), "{err}");
    }

    #[tokio::test(start_paused = true)]
    async fn send_chat_gives_up_if_tiktok_never_answers() {
        let (handle, mut p) = fake();
        let h = harness(vec![Ok(handle)]);
        p.say(r#"{"kind":"ready"}"#).await;
        let (task, _line) = start_send(&h, &mut p, "hola").await;
        // Nunca llega el chatResult: tras el plazo se rinde (con el tiempo pausado avanza solo).
        let err = task.await.expect("join").expect_err("debe fallar");
        assert!(err.to_string().contains("tardó demasiado"), "{err}");
    }

    #[tokio::test(start_paused = true)]
    async fn the_session_travels_with_the_connect_command() {
        let (handle, mut p) = fake();
        let h = harness(vec![Ok(handle)]);
        let mut t = target("ana");
        t.session = Some(SessionPayload { session_id: "SID".into(), tt_target_idc: "IDC".into() });
        h.source.connect(t).await.expect("connect");
        p.say(r#"{"kind":"ready"}"#).await;
        let line = p.next_cmd().await;
        assert!(line.contains("\"session\":{\"sessionId\":\"SID\",\"ttTargetIdc\":\"IDC\"}"), "{line}");
    }

    #[tokio::test(start_paused = true)]
    async fn garbage_lines_are_logged_and_do_not_stop_the_supervisor() {
        let (handle, p) = fake();
        let mut h = harness(vec![Ok(handle)]);
        p.say("esto no es json").await;
        p.say(r#"{"kind":"nope"}"#).await;
        p.say(r#"{"kind":"status","state":"connected"}"#).await;
        assert!(matches!(
            h.out.recv().await,
            Some(SourceMessage::Log { level: LogLevel::Warn, .. })
        ));
        assert!(matches!(
            h.out.recv().await,
            Some(SourceMessage::Log { level: LogLevel::Warn, .. })
        ));
        assert!(matches!(h.out.recv().await, Some(SourceMessage::Status(_))));
    }

    #[tokio::test(start_paused = true)]
    async fn restarts_a_dead_sidecar_and_restores_the_connection() {
        let (h1, mut p1) = fake();
        let (h2, mut p2) = fake();
        let mut h = harness(vec![Ok(h1), Ok(h2)]);
        h.source.connect(target("ana")).await.expect("connect");
        p1.say(r#"{"kind":"ready"}"#).await;
        assert_eq!(p1.next_cmd().await, CONNECT_ANA);

        p1.die(1).await;
        match h.out.recv().await {
            Some(SourceMessage::Status(s)) => {
                assert_eq!(s.state, ConnectionState::Reconnecting);
                assert_eq!(s.attempt, Some(1));
                assert!(s.retry_in_ms.is_some());
                assert!(s.detail.expect("detail").contains("código 1"));
            }
            other => panic!("se esperaba Status, llegó {other:?}"),
        }

        // Tras el backoff aparece el segundo proceso y, al estar listo, se reconecta solo.
        p2.say(r#"{"kind":"ready"}"#).await;
        assert_eq!(p2.next_cmd().await, CONNECT_ANA);
        assert_eq!(*h.spawns.lock().expect("lock"), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn disconnect_clears_the_desired_connection() {
        let (h1, mut p1) = fake();
        let (h2, mut p2) = fake();
        let mut h = harness(vec![Ok(h1), Ok(h2)]);
        h.source.connect(target("ana")).await.expect("connect");
        p1.say(r#"{"kind":"ready"}"#).await;
        assert_eq!(p1.next_cmd().await, CONNECT_ANA);
        h.source.disconnect().await.expect("disconnect");
        assert_eq!(p1.next_cmd().await, "{\"cmd\":\"disconnect\"}\n");

        p1.die(1).await;
        p2.say(r#"{"kind":"ready"}"#).await;
        // No debe haber reconexión: el siguiente mensaje no puede ser un connect.
        tokio::time::sleep(Duration::from_secs(5)).await;
        assert!(p2.stdin.try_recv().is_err());
        // Y la UI no ve "reconnecting" porque no se esperaba estar conectado.
        assert!(h.out.try_recv().is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn retries_with_growing_backoff_when_spawn_fails() {
        let (good, mut p) = fake();
        let h = harness(vec![
            Err(AppError::Sidecar("a".into())),
            Err(AppError::Sidecar("b".into())),
            Ok(good),
        ]);
        h.source.connect(target("ana")).await.expect("connect");
        p.say(r#"{"kind":"ready"}"#).await;
        assert_eq!(p.next_cmd().await, CONNECT_ANA);
        assert_eq!(*h.spawns.lock().expect("lock"), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn shutdown_sends_the_command_and_kills_the_process() {
        let (handle, mut p) = fake();
        let h = harness(vec![Ok(handle)]);
        p.say(r#"{"kind":"ready"}"#).await;
        h.source.shutdown();
        assert_eq!(p.next_cmd().await, "{\"cmd\":\"shutdown\"}\n");
        // Deja que el supervisor termine.
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(p.killed.load(Ordering::SeqCst));
    }
}
