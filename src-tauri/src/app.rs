//! Estado global de la app y su inicialización (cableado de todos los módulos).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;

use crate::actions::clock::{AppClock, Clock};
use crate::bot::outbox::{ChatSender, Outbox, OutboxLimits};
use crate::bot::service::BotService;
use crate::actions::queue::{ActionQueue, QueueConfig};
use crate::actions::ExecutorRegistry;
use crate::audio::{AudioBackend, NullAudio, RodioBackend};
use crate::bus::EventBus;
use crate::connection::ConnectionService;
use crate::counters::CounterService;
use crate::db::{self, Db};
use crate::error::Result;
use crate::executors::alert::OverlayAlertExecutor;
use crate::executors::bot::{BotMessageExecutor, PointsAdjustExecutor};
use crate::executors::interact::{SpinWheelExecutor, StartPollExecutor};
use crate::executors::keys::{system_backend, PressKeysExecutor};
use crate::executors::netcmd::{TcpSendExecutor, WsSendExecutor};
use crate::executors::obs::{ObsExecutor, ObsService};
use crate::executors::webhook::WebhookExecutor;
use crate::interact::poll::PollService;
use crate::interact::wheel::WheelService;
use crate::executors::internal::{GoalAdjustExecutor, TimerControlExecutor};
use crate::executors::sound::PlaySoundExecutor;
use crate::executors::tts::TtsExecutor;
use crate::goals::service::{GoalService, GoalTiming};
use crate::leaderboard::{LeaderboardService, LeaderboardTiming};
use crate::media::MediaLibrary;
use crate::overlay::{OverlayHub, RecentEvents};
use crate::overlay_config::OverlayConfigService;
use crate::points::service::{PointsService, PointsTiming};
use crate::rules::engine::{RuleEngine, SystemSink};
use crate::connections::{ConnectionManager, PlatformStatus};
use crate::events::Platform;
use crate::prefs::PrefsService;
use crate::twitch::auth::TwitchAuth;
use crate::twitch::helix::Helix as TwitchHelix;
use crate::twitch::service::TwitchService;
use crate::twitch::source::TwitchSource;
use crate::profiles::ProfileService;
use crate::spotify::api::HttpSpotify;
use crate::spotify::auth::SpotifyAuth;
use crate::spotify::service::SongService;
use crate::stats::StatsService;
use crate::secrets::{self, KeyringStore, SecretStore};
use crate::server::{self, ServerHandle};
use crate::session::SessionService;
use crate::simulator::Simulator;
use crate::sounds::SoundLibrary;
use crate::source::sidecar::{RestartConfig, SidecarSource};
use crate::source::tauri_spawner::TauriSpawner;
use crate::source::LiveSource;
use crate::timers::service::{TimerService, TimerTiming};
use crate::tts::edge::EdgeEngine;
use crate::tts::engine::TtsEngine;
use crate::tts::piper::{PiperEngine, PiperPaths};
use crate::tts::provision::piper_exe_path;
use crate::tts::sapi::SapiEngine;
use crate::tts::service::{TtsDeps, TtsService};

/// Evento de Tauri con el estado de conexión (`StatusUpdate`).
pub const EVT_STATUS: &str = "connection-status";
/// Evento de Tauri con lotes de `LiveEvent` (array).
pub const EVT_EVENTS: &str = "live-events";
/// Evento de Tauri cuando una regla se dispara (`FiredReport`).
pub const EVT_RULE_FIRED: &str = "rule-fired";
/// Evento de Tauri con el progreso de las instalaciones del TTS (`Progress`).
pub const EVT_TTS_INSTALL: &str = "tts-install-progress";
/// Cada cuánto se envían a la UI los eventos acumulados.
const UI_BATCH_INTERVAL: Duration = Duration::from_millis(50);
/// Eventos recientes que se reenvían a un overlay recién conectado.
const OVERLAY_HISTORY: usize = 80;
/// Tiempo máximo para guardar el estado pendiente al cerrar la app.
const SHUTDOWN_FLUSH_TIMEOUT: Duration = Duration::from_secs(3);

pub struct AppState {
    /// Conexión de TikTok (la que escribe el bot). Todas están en `connections`.
    pub conn: Arc<ConnectionService>,
    pub connections: ConnectionManager,
    pub twitch: Arc<TwitchService>,
    pub db: Db,
    pub secrets: Arc<dyn SecretStore>,
    pub bus: EventBus,
    pub sim: Simulator,
    pub sidecar: Arc<SidecarSource>,
    pub clock: Arc<dyn Clock>,
    pub hub: OverlayHub,
    pub audio: Arc<dyn AudioBackend>,
    pub sounds: Arc<SoundLibrary>,
    pub media: Arc<MediaLibrary>,
    pub tts: Arc<TtsService>,
    pub tts_dir: PathBuf,
    pub queue: Arc<ActionQueue>,
    pub registry: ExecutorRegistry,
    pub rules: Arc<RuleEngine>,
    pub session: SessionService,
    pub overlay_cfg: OverlayConfigService,
    pub goals: Arc<GoalService>,
    pub timers: Arc<TimerService>,
    pub leaderboard: Arc<LeaderboardService>,
    pub counters: Arc<CounterService>,
    pub points: Arc<PointsService>,
    pub bot: Arc<BotService>,
    pub wheel: Arc<WheelService>,
    pub polls: Arc<PollService>,
    pub obs: Arc<ObsService>,
    pub prefs: Arc<PrefsService>,
    pub stats: Arc<StatsService>,
    pub spotify: Arc<SongService>,
    pub spotify_auth: Arc<SpotifyAuth>,
    pub profiles: Arc<ProfileService>,
    pub data_dir: PathBuf,
    pub pending_update: crate::updater::PendingUpdate,
    /// Resultado de la importación pendiente del arranque (se muestra una vez en la UI).
    pub import_notice: Mutex<Option<String>>,
    pub overlay_token: String,
    /// Puerto en el que quedó (o quiso quedar) el servidor local.
    pub server_port: u16,
    /// Si el servidor no pudo arrancar (p. ej. puerto ocupado), el motivo.
    pub server_error: Option<String>,
    _server: Mutex<Option<ServerHandle>>,
}

impl AppState {
    /// Debe llamarse dentro de un runtime de Tokio.
    pub async fn init(app: AppHandle, data_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        let db = Db::open(&data_dir.join("hivebuzz.db")).await?;
        // Una importación pendiente se aplica ahora, antes de que ningún servicio cargue nada.
        let import_notice = match crate::backup::apply_pending(&db, data_dir, chrono::Utc::now().timestamp_millis()).await {
            Ok(None) => None,
            Ok(Some(s)) => {
                tracing::info!(rules = s.rules, sounds = s.sounds, media = s.media, "configuración importada");
                Some(format!("ok:{}", s.rules))
            }
            Err(e) => {
                tracing::error!(error = %e, "no se pudo aplicar la importación pendiente");
                Some(format!("error:{e}"))
            }
        };
        let secrets: Arc<dyn SecretStore> = Arc::new(KeyringStore);
        let overlay_token = secrets::ensure_overlay_token(secrets.as_ref())?;

        let clock: Arc<dyn Clock> = Arc::new(AppClock::new());
        let bus = EventBus::new(2048);
        let hub = OverlayHub::new(256);

        // Fuente de eventos: el sidecar, supervisado y reiniciado automáticamente.
        let (tx, rx) = mpsc::channel(1024);
        let sidecar = Arc::new(SidecarSource::start(
            TauriSpawner::new(app.clone()),
            tx,
            RestartConfig::default(),
        ));
        let conn = ConnectionService::new(Arc::clone(&sidecar) as Arc<dyn LiveSource>);
        conn.spawn_pump(rx, bus.clone());

        // Twitch: chat sin cuenta; con sesión del dueño, también seguidores y estado del directo.
        let twitch_auth = TwitchAuth::new(Arc::clone(&secrets), Arc::clone(&clock))?;
        let twitch = TwitchService::new(db.clone(), Arc::clone(&twitch_auth));
        if let Err(e) = twitch.load_config().await {
            tracing::error!(error = %e, "no se pudo cargar la configuración de Twitch");
        }
        let (twitch_tx, twitch_rx) = mpsc::channel(1024);
        let twitch_source = TwitchSource::new(twitch_tx, Arc::clone(&twitch_auth), Arc::new(TwitchHelix::new(Arc::clone(&twitch_auth))?), Arc::clone(&clock));
        let twitch_conn = ConnectionService::with_platform(Arc::new(twitch_source) as Arc<dyn LiveSource>, Platform::Twitch);
        twitch_conn.spawn_pump(twitch_rx, bus.clone());
        let connections = ConnectionManager::new(Arc::clone(&conn), twitch_conn);
        let total_viewers = connections.spawn_total_viewers();

        // Sesión de transmisión: qué es «este LIVE» (rankings de sesión, metas con reinicio).
        let session = SessionService::new();
        session.spawn(&bus, conn.subscribe_status());

        // Audio, bibliotecas locales y TTS.
        let audio: Arc<dyn AudioBackend> = match RodioBackend::start() {
            Ok(a) => Arc::new(a),
            Err(e) => {
                tracing::error!(error = %e, "el audio no se pudo iniciar; las acciones de sonido fallarán");
                Arc::new(NullAudio)
            }
        };
        let sounds = Arc::new(SoundLibrary::new(db.clone(), data_dir.join("sounds"), Arc::clone(&clock))?);
        let media = Arc::new(MediaLibrary::new(db.clone(), data_dir.join("media"), Arc::clone(&clock))?);

        let tts_dir = data_dir.join("tts");
        let default_piper = PiperPaths { exe: piper_exe_path(&tts_dir), voices_dir: tts_dir.join("voices") };
        let piper_paths = Arc::new(RwLock::new(default_piper.clone()));
        let engines: Vec<Arc<dyn TtsEngine>> = vec![
            Arc::new(PiperEngine::new(Arc::clone(&piper_paths))),
            Arc::new(SapiEngine),
            Arc::new(EdgeEngine),
        ];
        let tts = TtsService::new(TtsDeps {
            engines,
            audio: Arc::clone(&audio),
            clock: Arc::clone(&clock),
            db: db.clone(),
            cache_dir: tts_dir.join("cache"),
            piper_paths,
            default_piper,
        })?;
        tts.load_config().await?;

        // Configuración de los overlays (se publica retenida para que cualquiera que se conecte la reciba).
        let overlay_cfg = OverlayConfigService::new(db.clone(), hub.clone());
        if let Err(e) = overlay_cfg.publish_all().await {
            tracing::warn!(error = %e, "no se pudo publicar la configuración de los overlays");
        }

        // Metas, timers, ranking y contadores.
        let goals = GoalService::new(db.clone(), hub.clone());
        if let Err(e) = goals.load().await {
            tracing::error!(error = %e, "no se pudieron cargar las metas");
        }
        let timers = TimerService::new(db.clone(), hub.clone(), Arc::clone(&clock));
        if let Err(e) = timers.load().await {
            tracing::error!(error = %e, "no se pudieron cargar los timers");
        }

        // Puntos y chatbot (el bot escribe por la conexión; sin sesión de TikTok sus envíos fallan y se registran).
        let points = PointsService::new(db.clone(), Arc::clone(&clock));
        if let Err(e) = points.load_config().await {
            tracing::error!(error = %e, "no se pudo cargar la configuración de puntos");
        }
        let outbox = Outbox::start(
            Arc::clone(&conn) as Arc<dyn ChatSender>,
            Arc::clone(&clock),
            Duration::from_secs(2),
            OutboxLimits::default(),
        );
        let bot = BotService::new(db.clone(), Arc::clone(&clock), Arc::clone(&points), outbox);
        if let Err(e) = bot.load_config().await {
            tracing::error!(error = %e, "no se pudo cargar la configuración del bot");
        }

        let wheel = WheelService::new(db.clone(), hub.clone(), Arc::clone(&bot));
        if let Err(e) = wheel.load().await {
            tracing::error!(error = %e, "no se pudo cargar la ruleta");
        }
        let polls = PollService::new(hub.clone(), Arc::clone(&bot), Arc::clone(&clock));
        let prefs = Arc::new(PrefsService::new(db.clone()));
        if let Err(e) = prefs.load().await {
            tracing::error!(error = %e, "no se pudieron cargar las preferencias");
        }
        // Spotify: PKCE sin servidor propio; peticiones por chat y «Sonando ahora».
        let spotify_auth = SpotifyAuth::new(Arc::clone(&secrets), Arc::clone(&clock))?;
        let spotify_api = HttpSpotify::new(Arc::clone(&spotify_auth))?;
        let spotify = SongService::new(
            db.clone(),
            Arc::clone(&spotify_auth),
            Arc::new(spotify_api),
            Arc::clone(&points) as Arc<dyn crate::rules::engine::PointsGate>,
            Arc::clone(&bot) as Arc<dyn crate::spotify::service::ChatOut>,
            hub.clone(),
            Arc::clone(&clock),
        );
        if let Err(e) = spotify.load_config().await {
            tracing::error!(error = %e, "no se pudo cargar la configuración de Spotify");
        }
        let obs = ObsService::new(db.clone(), Arc::clone(&secrets));
        if let Err(e) = obs.load_config().await {
            tracing::error!(error = %e, "no se pudo cargar la configuración de OBS");
        }

        // Ejecutores: aquí se enchufan; agregar uno nuevo no toca el núcleo.
        let mut registry = ExecutorRegistry::new();
        registry.register(Arc::new(PlaySoundExecutor::new(Arc::clone(&sounds), Arc::clone(&audio))));
        registry.register(Arc::new(OverlayAlertExecutor::new(Arc::clone(&media), hub.clone())));
        registry.register(Arc::new(TtsExecutor::new(Arc::clone(&tts))));
        registry.register(Arc::new(GoalAdjustExecutor::new(Arc::clone(&goals))));
        registry.register(Arc::new(TimerControlExecutor::new(Arc::clone(&timers))));
        registry.register(Arc::new(BotMessageExecutor::new(Arc::clone(&bot))));
        registry.register(Arc::new(PointsAdjustExecutor::new(Arc::clone(&points))));
        registry.register(Arc::new(SpinWheelExecutor::new(Arc::clone(&wheel))));
        registry.register(Arc::new(StartPollExecutor::new(Arc::clone(&polls))));
        // Fase 5: integraciones.
        registry.register(Arc::new(WebhookExecutor::new()?));
        registry.register(Arc::new(TcpSendExecutor));
        registry.register(Arc::new(WsSendExecutor));
        registry.register(Arc::new(PressKeysExecutor::new(system_backend())));
        registry.register(Arc::new(ObsExecutor::new(Arc::clone(&obs))));

        // Cola de acciones (persistente) y motor de reglas.
        let queue = Arc::new(ActionQueue::start(
            registry.clone(),
            Arc::new(db.clone()),
            Arc::clone(&clock),
            QueueConfig::default(),
        ));
        match queue.restore().await {
            Ok(0) => {}
            Ok(n) => tracing::info!(restored = n, "acciones pendientes recuperadas"),
            Err(e) => tracing::warn!(error = %e, "no se pudieron recuperar las acciones pendientes"),
        }
        tts.attach_queue(Arc::clone(&queue));
        wheel.attach_queue(Arc::clone(&queue));
        let rules = RuleEngine::new(db.clone(), Arc::clone(&queue), registry.clone(), Arc::clone(&clock));
        match rules.load().await {
            Ok(n) => tracing::info!(rules = n, "reglas cargadas"),
            Err(e) => tracing::error!(error = %e, "no se pudieron cargar las reglas"),
        }

        rules.attach_points(Arc::clone(&points) as Arc<dyn crate::rules::engine::PointsGate>);

        // Metas y timers avisan al motor de reglas («meta alcanzada», «timer terminado»).
        let sink: Arc<dyn SystemSink> = rules.clone();
        goals.attach_sink(Arc::clone(&sink));
        timers.attach_sink(sink);

        let profiles = ProfileService::new(db.clone(), Arc::clone(&rules), overlay_cfg.clone(), Arc::clone(&clock));
        if let Err(e) = profiles.load().await {
            tracing::error!(error = %e, "no se pudo cargar el perfil activo");
        }
        let leaderboard = LeaderboardService::new(db.clone(), hub.clone(), Arc::new(LeaderboardService::local_today));
        leaderboard.publish().await;
        let counters = CounterService::new(hub.clone());
        let recent = RecentEvents::new(OVERLAY_HISTORY);
        let stats = StatsService::new(db.clone(), Arc::clone(&clock));

        // Todo lo que escucha el bus.
        rules.spawn(&bus);
        tts.spawn(&bus);
        goals.spawn(&bus, session.subscribe(), GoalTiming::default());
        timers.spawn(&bus, TimerTiming::default());
        leaderboard.spawn(&bus, session.subscribe(), LeaderboardTiming::default());
        counters.spawn(&bus, total_viewers.clone(), session.subscribe(), Duration::from_millis(250));
        recent.spawn(&bus);
        stats.spawn(&bus, total_viewers, session.subscribe(), Duration::from_secs(20));
        points.spawn(&bus, session.subscribe(), PointsTiming::default());
        bot.spawn(&bus, &rules);
        polls.spawn(&bus);
        spotify.spawn(&bus);

        // Persistencia del log de eventos y su rotación (7 días).
        db::spawn_log_writer(db.clone(), &bus);
        db::spawn_rotation(db.clone());

        // Servidor local para overlays y API. Un fallo aquí no debe impedir usar la app.
        let server_port = db.server_port().await?;
        let server_deps = server::ServerDeps {
            bus: bus.clone(),
            hub: hub.clone(),
            recent,
            media_dir: media.dir().to_path_buf(),
            token: overlay_token.clone(),
            api: Some(Arc::clone(&rules) as server::api::SharedApi),
            oauth: Some(Arc::clone(&spotify_auth) as server::oauth::SharedOAuth),
        };
        let (server, server_error) = match server::start(&server_deps, server_port).await {
            Ok(h) => (Some(h), None),
            Err(e) => {
                tracing::error!(error = %e, "el servidor local no arrancó");
                (None, Some(e.to_string()))
            }
        };

        spawn_ui_forwarders(app, &connections, &bus, &rules);

        Ok(Self {
            conn,
            connections,
            twitch,
            db,
            secrets,
            sim: Simulator::new(bus.clone()),
            bus,
            sidecar,
            clock,
            hub,
            audio,
            sounds,
            media,
            tts,
            tts_dir,
            queue,
            registry,
            rules,
            session,
            overlay_cfg,
            goals,
            timers,
            leaderboard,
            counters,
            points,
            bot,
            wheel,
            polls,
            obs,
            prefs,
            stats,
            spotify,
            spotify_auth,
            profiles,
            data_dir: data_dir.to_path_buf(),
            pending_update: crate::updater::PendingUpdate::default(),
            import_notice: Mutex::new(import_notice),
            overlay_token,
            server_port,
            server_error,
            _server: Mutex::new(server),
        })
    }

    /// Cierre ordenado: guarda el estado pendiente y detiene audio, cola y sidecar.
    pub fn shutdown(&self) {
        self.audio.stop_all();
        self.queue.shutdown();
        self.sidecar.shutdown();
        let (goals, timers, board, points, stats) =
            (Arc::clone(&self.goals), Arc::clone(&self.timers), Arc::clone(&self.leaderboard), Arc::clone(&self.points), Arc::clone(&self.stats));
        // Mejor esfuerzo y con tope de tiempo: cerrar la app no debe quedarse esperando a la base de datos.
        tauri::async_runtime::block_on(async move {
            let flush = async {
                goals.flush().await;
                timers.flush().await;
                if let Err(e) = points.flush().await {
                    tracing::warn!(error = %e, "no se pudieron guardar los puntos pendientes");
                }
                if let Err(e) = stats.flush().await {
                    tracing::warn!(error = %e, "no se pudieron guardar las estadísticas pendientes");
                }
                if let Err(e) = board.flush().await {
                    tracing::warn!(error = %e, "no se pudieron guardar las donaciones pendientes");
                }
            };
            if tokio::time::timeout(SHUTDOWN_FLUSH_TIMEOUT, flush).await.is_err() {
                tracing::warn!("se agotó el tiempo guardando el estado al cerrar");
            }
        });
    }
}

/// Reenvía a la UI el estado de conexión, los eventos (en lotes) y los disparos de reglas.
fn spawn_ui_forwarders(app: AppHandle, connections: &ConnectionManager, bus: &EventBus, rules: &Arc<RuleEngine>) {
    for (platform, mut status) in connections.subscribe_statuses() {
        let status_app = app.clone();
        tokio::spawn(async move {
            while status.changed().await.is_ok() {
                let current = PlatformStatus { platform, status: status.borrow_and_update().clone() };
                if let Err(e) = status_app.emit(EVT_STATUS, current) {
                    tracing::warn!(error = %e, "no se pudo emitir el estado a la UI");
                }
            }
        });
    }

    let mut fired = rules.subscribe_fired();
    let fired_app = app.clone();
    tokio::spawn(async move {
        loop {
            match fired.recv().await {
                Ok(report) => {
                    let _ = fired_app.emit(EVT_RULE_FIRED, report);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    let mut events = bus.subscribe();
    tokio::spawn(async move {
        let mut pending = Vec::new();
        let mut tick = tokio::time::interval(UI_BATCH_INTERVAL);
        loop {
            tokio::select! {
                ev = events.recv() => match ev {
                    Ok(ev) => pending.push(ev),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(missed = n, "la UI se quedó atrás en el bus");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
                _ = tick.tick() => {
                    if !pending.is_empty() {
                        if let Err(e) = app.emit(EVT_EVENTS, &pending) {
                            tracing::warn!(error = %e, "no se pudo emitir eventos a la UI");
                        }
                        pending.clear();
                    }
                }
            }
        }
    });
}
