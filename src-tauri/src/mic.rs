//! Detección de voz en el micrófono, para que el TTS no hable encima del streamer.
//!
//! Un hilo propio captura el micrófono (el `Stream` de `cpal` no es `Send`) y mide el nivel de
//! cada bloque de audio. Un detector por energía decide si el streamer está hablando y publica
//! el cambio por un canal `watch`. No se guarda ni se envía audio: solo se calcula el nivel.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::Serialize;
use tokio::sync::watch;

/// Nivel que se considera silencio absoluto (dBFS).
pub const SILENCE_DB: f32 = -100.0;
/// El nivel debe pasar el umbral al menos este tiempo seguido: un golpe en la mesa o un clic del
/// teclado no cuentan como hablar.
const ATTACK_MS: u64 = 120;

/// Cómo escuchar el micrófono.
#[derive(Debug, Clone, PartialEq)]
pub struct MicSettings {
    /// Nombre del micrófono; `None` = el predeterminado del sistema.
    pub device: Option<String>,
    /// Nivel (dBFS) a partir del cual se considera que hay voz.
    pub threshold_db: f32,
    /// Silencio necesario para dar por terminado lo que se dice (las pausas entre palabras no cuentan).
    pub hold_ms: u64,
}

/// Detector de voz por energía con histéresis temporal.
#[derive(Debug)]
pub struct Vad {
    threshold_db: f32,
    hold_ms: u64,
    above_since: Option<u64>,
    last_voice: Option<u64>,
    speaking: bool,
}

impl Vad {
    pub fn new(threshold_db: f32, hold_ms: u64) -> Self {
        Self { threshold_db, hold_ms, above_since: None, last_voice: None, speaking: false }
    }

    /// Cambia sensibilidad y silencio sin perder el estado (al mover el control mientras se habla).
    pub fn set_params(&mut self, threshold_db: f32, hold_ms: u64) {
        self.threshold_db = threshold_db;
        self.hold_ms = hold_ms;
    }

    /// Procesa el nivel de un bloque (`now_ms` crece) y dice si el streamer está hablando.
    pub fn feed(&mut self, level_db: f32, now_ms: u64) -> bool {
        if level_db >= self.threshold_db {
            let since = *self.above_since.get_or_insert(now_ms);
            if self.speaking || now_ms.saturating_sub(since) >= ATTACK_MS {
                self.speaking = true;
                self.last_voice = Some(now_ms);
            }
        } else {
            self.above_since = None;
            if self.speaking && self.last_voice.is_some_and(|t| now_ms.saturating_sub(t) >= self.hold_ms) {
                self.speaking = false;
            }
        }
        self.speaking
    }
}

/// Nivel RMS de unas muestras en dBFS (`-100` para silencio o vacío).
pub fn rms_db(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return SILENCE_DB;
    }
    let sum: f64 = samples.iter().map(|s| f64::from(*s) * f64::from(*s)).sum();
    #[allow(clippy::cast_precision_loss)]
    let rms = (sum / samples.len() as f64).sqrt();
    if rms <= 1e-5 {
        return SILENCE_DB;
    }
    #[allow(clippy::cast_possible_truncation)]
    let db = (20.0 * rms.log10()) as f32;
    db.max(SILENCE_DB)
}

/// Lo que el TTS necesita del micrófono (en las pruebas se sustituye por uno falso).
pub trait SpeechDetector: Send + Sync {
    /// Empieza a escuchar con esos ajustes (`None` = dejar de escuchar).
    fn configure(&self, settings: Option<MicSettings>);
    /// `true` mientras el streamer habla.
    fn subscribe(&self) -> watch::Receiver<bool>;
}

/// Estado para la interfaz (medidor de nivel y errores).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MicStatus {
    pub active: bool,
    pub speaking: bool,
    pub level_db: f32,
    pub error: Option<String>,
}

/// Un micrófono para elegir en la interfaz.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MicDevice {
    /// Nombre completo de Windows, p. ej. «Micrófono (Yeti Nano)»; es lo que se guarda.
    pub name: String,
    pub is_default: bool,
}

/// Sensibilidad y silencio que lee el hilo de captura en cada bloque: se cambian sin reabrir el micrófono.
struct LiveParams {
    threshold_db: AtomicU32,
    hold_ms: AtomicU64,
}

impl LiveParams {
    fn new(s: &MicSettings) -> Self {
        Self { threshold_db: AtomicU32::new(s.threshold_db.to_bits()), hold_ms: AtomicU64::new(s.hold_ms) }
    }

    fn store(&self, s: &MicSettings) {
        self.threshold_db.store(s.threshold_db.to_bits(), Ordering::Relaxed);
        self.hold_ms.store(s.hold_ms, Ordering::Relaxed);
    }

    fn load(&self) -> (f32, u64) {
        (f32::from_bits(self.threshold_db.load(Ordering::Relaxed)), self.hold_ms.load(Ordering::Relaxed))
    }
}

struct Running {
    device: Option<String>,
    params: Arc<LiveParams>,
    /// Al soltarlo, el hilo de captura termina.
    _stop: mpsc::Sender<()>,
}

#[derive(Default)]
struct Wanted {
    /// Lo que pide la configuración guardada del TTS.
    tts: Option<MicSettings>,
    /// Lo que se está probando en la interfaz (manda mientras la tarjeta está abierta).
    preview: Option<MicSettings>,
}

pub struct MicMonitor {
    speaking: Arc<watch::Sender<bool>>,
    level: Arc<AtomicU32>,
    error: Arc<Mutex<Option<String>>>,
    wanted: Mutex<Wanted>,
    running: Mutex<Option<Running>>,
}

impl Default for MicMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl MicMonitor {
    pub fn new() -> Self {
        Self {
            speaking: Arc::new(watch::Sender::new(false)),
            level: Arc::new(AtomicU32::new(SILENCE_DB.to_bits())),
            error: Arc::new(Mutex::new(None)),
            wanted: Mutex::new(Wanted::default()),
            running: Mutex::new(None),
        }
    }

    pub fn status(&self) -> MicStatus {
        let running = self.running.lock().unwrap_or_else(PoisonError::into_inner).is_some();
        let error = self.error.lock().unwrap_or_else(PoisonError::into_inner).clone();
        MicStatus {
            active: running && error.is_none(),
            speaking: *self.speaking.borrow(),
            level_db: f32::from_bits(self.level.load(Ordering::Relaxed)),
            error,
        }
    }
}

impl MicMonitor {
    /// Escucha con estos ajustes mientras se calibra en la interfaz, aunque no estén guardados
    /// (`None` = volver a lo guardado).
    pub fn preview(&self, settings: Option<MicSettings>) {
        self.wanted.lock().unwrap_or_else(PoisonError::into_inner).preview = settings;
        self.apply();
    }

    fn apply(&self) {
        let settings = {
            let w = self.wanted.lock().unwrap_or_else(PoisonError::into_inner);
            w.preview.clone().or_else(|| w.tts.clone())
        };
        let mut running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        match (running.as_ref(), &settings) {
            (None, None) => return,
            // Mismo micrófono: solo cambian sensibilidad o silencio, sin reabrirlo.
            (Some(r), Some(s)) if r.device == s.device => {
                r.params.store(s);
                return;
            }
            _ => {}
        }
        // Soltar el `Running` anterior detiene su hilo.
        *running = None;
        self.speaking.send_replace(false);
        self.level.store(SILENCE_DB.to_bits(), Ordering::Relaxed);
        *self.error.lock().unwrap_or_else(PoisonError::into_inner) = None;
        let Some(settings) = settings else {
            return;
        };
        let (stop_tx, stop_rx) = mpsc::channel();
        let params = Arc::new(LiveParams::new(&settings));
        let (speaking, level, error, p) = (Arc::clone(&self.speaking), Arc::clone(&self.level), Arc::clone(&self.error), Arc::clone(&params));
        let device = settings.device.clone();
        let spawned = std::thread::Builder::new().name("hivebuzz-mic".into()).spawn(move || {
            if let Err(e) = capture(device.as_deref(), &p, &speaking, &level, &stop_rx) {
                tracing::warn!(error = %e, "micrófono: no se pudo escuchar");
                *error.lock().unwrap_or_else(PoisonError::into_inner) = Some(e);
            }
            speaking.send_replace(false);
            level.store(SILENCE_DB.to_bits(), Ordering::Relaxed);
        });
        if let Err(e) = spawned {
            *self.error.lock().unwrap_or_else(PoisonError::into_inner) = Some(format!("no se pudo iniciar el hilo del micrófono: {e}"));
            return;
        }
        *running = Some(Running { device: settings.device, params, _stop: stop_tx });
    }
}

impl SpeechDetector for MicMonitor {
    fn configure(&self, settings: Option<MicSettings>) {
        self.wanted.lock().unwrap_or_else(PoisonError::into_inner).tts = settings;
        self.apply();
    }

    fn subscribe(&self) -> watch::Receiver<bool> {
        self.speaking.subscribe()
    }
}

/// Nombre con el que Windows muestra el micrófono: «Micrófono (Yeti Nano)». La descripción corta
/// de cpal suele ser solo «Micrófono», igual para todos, y no sirve para distinguirlos.
fn device_name(d: &cpal::Device) -> Option<String> {
    let desc = d.description().ok()?;
    Some(full_name(desc.name(), desc.extended().first().map(String::as_str), desc.driver()))
}

fn full_name(short: &str, friendly: Option<&str>, driver: Option<&str>) -> String {
    match (friendly, driver) {
        (Some(f), _) if !f.trim().is_empty() => f.to_string(),
        (_, Some(d)) if !d.trim().is_empty() && !short.contains(d) => format!("{short} ({d})"),
        _ => short.to_string(),
    }
}

/// ¿Es el micrófono guardado? También acepta el nombre corto que guardaban versiones anteriores.
fn matches_device(d: &cpal::Device, wanted: &str) -> bool {
    device_name(d).as_deref() == Some(wanted) || d.description().is_ok_and(|desc| desc.name() == wanted)
}

/// Micrófonos disponibles, primero el predeterminado del sistema.
pub fn input_devices() -> Vec<MicDevice> {
    let host = cpal::default_host();
    let default = host.default_input_device().and_then(|d| device_name(&d));
    let Ok(devices) = host.input_devices() else {
        return Vec::new();
    };
    let mut list: Vec<MicDevice> = devices
        .filter_map(|d| device_name(&d))
        .map(|name| MicDevice { is_default: default.as_deref() == Some(name.as_str()), name })
        .collect();
    list.sort_by(|a, b| b.is_default.cmp(&a.is_default).then_with(|| a.name.cmp(&b.name)));
    list.dedup_by(|a, b| a.name == b.name);
    list
}

/// Captura hasta que se suelte el emisor de `stop`. Solo devuelve error si no pudo empezar.
fn capture(
    device_wanted: Option<&str>,
    params: &Arc<LiveParams>,
    speaking: &Arc<watch::Sender<bool>>,
    level: &Arc<AtomicU32>,
    stop: &mpsc::Receiver<()>,
) -> Result<(), String> {
    let host = cpal::default_host();
    let device = match device_wanted {
        Some(name) => host
            .input_devices()
            .map_err(|e| format!("no se pudieron listar los micrófonos: {e}"))?
            .find(|d| matches_device(d, name))
            .ok_or_else(|| format!("no se encontró el micrófono «{name}»: ¿está conectado?"))?,
        None => host.default_input_device().ok_or("no hay ningún micrófono disponible")?,
    };
    let supported = device.default_input_config().map_err(|e| format!("el micrófono no da su formato: {e}"))?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();

    let opened = device_name(&device).unwrap_or_default();
    let (threshold_db, hold_ms) = params.load();
    let mut vad = Vad::new(threshold_db, hold_ms);
    let started = Instant::now();
    let (speaking, level, params) = (Arc::clone(speaking), Arc::clone(level), Arc::clone(params));
    let mut buf: Vec<f32> = Vec::new();
    let mut on_block = move |samples: &mut dyn Iterator<Item = f32>| {
        buf.clear();
        buf.extend(samples);
        let (threshold_db, hold_ms) = params.load();
        vad.set_params(threshold_db, hold_ms);
        let db = rms_db(&buf);
        level.store(db.to_bits(), Ordering::Relaxed);
        let now = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let talking = vad.feed(db, now);
        speaking.send_if_modified(|v| {
            let changed = *v != talking;
            *v = talking;
            changed
        });
    };
    let on_error = |e: cpal::StreamError| tracing::warn!(error = %e, "micrófono: error de captura");

    let stream = match format {
        cpal::SampleFormat::F32 => device.build_input_stream::<f32, _, _>(&config, move |d, _| on_block(&mut d.iter().copied()), on_error, None),
        cpal::SampleFormat::I16 => {
            device.build_input_stream::<i16, _, _>(&config, move |d, _| on_block(&mut d.iter().map(|s| f32::from(*s) / 32_768.0)), on_error, None)
        }
        cpal::SampleFormat::U16 => device.build_input_stream::<u16, _, _>(
            &config,
            move |d, _| on_block(&mut d.iter().map(|s| (f32::from(*s) - 32_768.0) / 32_768.0)),
            on_error,
            None,
        ),
        #[allow(clippy::cast_precision_loss)]
        cpal::SampleFormat::I32 => {
            device.build_input_stream::<i32, _, _>(&config, move |d, _| on_block(&mut d.iter().map(|s| *s as f32 / 2_147_483_648.0)), on_error, None)
        }
        other => return Err(format!("formato de micrófono no soportado: {other:?}")),
    }
    .map_err(|e| format!("no se pudo abrir el micrófono: {e}"))?;
    stream.play().map_err(|e| format!("no se pudo iniciar el micrófono: {e}"))?;
    tracing::info!(device = %opened, channels = config.channels, "micrófono: escuchando");

    // Hasta que `configure` suelte el emisor.
    let _ = stop.recv();
    drop(stream);
    Ok(())
}

#[cfg(test)]
pub mod testing {
    use super::*;

    /// Detector manejado a mano desde las pruebas.
    pub struct FakeMic {
        pub tx: watch::Sender<bool>,
        pub configured: Mutex<Vec<Option<MicSettings>>>,
    }

    impl FakeMic {
        pub fn new() -> Arc<Self> {
            Arc::new(Self { tx: watch::Sender::new(false), configured: Mutex::new(Vec::new()) })
        }

        pub fn set_speaking(&self, on: bool) {
            self.tx.send_replace(on);
        }
    }

    impl SpeechDetector for FakeMic {
        fn configure(&self, settings: Option<MicSettings>) {
            self.configured.lock().expect("lock").push(settings);
        }

        fn subscribe(&self) -> watch::Receiver<bool> {
            self.tx.subscribe()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rms_of_silence_and_of_a_full_scale_signal() {
        assert_eq!(rms_db(&[]), SILENCE_DB);
        assert_eq!(rms_db(&[0.0; 64]), SILENCE_DB);
        let full: Vec<f32> = (0..64).map(|i| if i % 2 == 0 { 1.0 } else { -1.0 }).collect();
        assert!(rms_db(&full).abs() < 0.01, "0 dBFS");
        let quiet = vec![0.01_f32; 64];
        assert!((rms_db(&quiet) - -40.0).abs() < 0.01, "0.01 → -40 dBFS");
    }

    #[test]
    fn a_short_click_is_not_speech() {
        let mut v = Vad::new(-40.0, 500);
        assert!(!v.feed(-10.0, 0));
        assert!(!v.feed(-10.0, 60), "aún no pasan los 120 ms");
        assert!(!v.feed(-80.0, 80), "fue un golpe");
        assert!(!v.feed(-10.0, 100));
        assert!(!v.feed(-10.0, 150), "el contador empezó de nuevo en 100");
    }

    #[test]
    fn speech_starts_after_the_attack_and_ends_after_the_hold() {
        let mut v = Vad::new(-40.0, 500);
        assert!(!v.feed(-20.0, 0));
        assert!(v.feed(-20.0, 130), "habla");
        // Pausas cortas entre palabras no cortan.
        assert!(v.feed(-70.0, 300));
        assert!(v.feed(-20.0, 400), "vuelve a hablar sin esperar el ataque");
        assert!(v.feed(-70.0, 600));
        assert!(v.feed(-70.0, 899));
        assert!(!v.feed(-70.0, 900), "500 ms de silencio desde la última voz");
    }

    /// Abre el micrófono real 1,5 s: `cargo test mic_live -- --ignored --nocapture`.
    #[test]
    #[ignore = "usa el micrófono real"]
    fn mic_live_reads_the_default_microphone() {
        eprintln!("micrófonos: {:?}", input_devices());
        let m = MicMonitor::new();
        m.configure(Some(MicSettings { device: None, threshold_db: -40.0, hold_ms: 800 }));
        let mut max = SILENCE_DB;
        for _ in 0..30 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            max = max.max(m.status().level_db);
        }
        let s = m.status();
        eprintln!("estado: {s:?}; nivel máximo {max:.1} dBFS");
        assert!(s.error.is_none(), "{:?}", s.error);
        assert!(s.active);
        m.configure(None);
        assert!(!m.status().active);
    }

    #[test]
    fn device_names_include_the_hardware() {
        assert_eq!(full_name("Micrófono", Some("Micrófono (Yeti Nano)"), Some("Yeti Nano")), "Micrófono (Yeti Nano)");
        assert_eq!(full_name("Micrófono", None, Some("Yeti Nano")), "Micrófono (Yeti Nano)");
        assert_eq!(full_name("Headset (Arctis)", None, Some("Arctis")), "Headset (Arctis)");
        assert_eq!(full_name("Micrófono", Some(" "), None), "Micrófono");
    }

    #[test]
    fn vad_params_change_without_losing_state() {
        let mut v = Vad::new(-40.0, 500);
        assert!(!v.feed(-30.0, 0));
        assert!(v.feed(-30.0, 130));
        v.set_params(-20.0, 200);
        assert!(v.feed(-30.0, 200), "-30 ya no llega al umbral, pero el silencio aún no dura 200 ms");
        assert!(!v.feed(-30.0, 330));
    }

    #[test]
    fn configure_is_idempotent_and_none_stops_listening() {
        let m = MicMonitor::new();
        m.configure(None);
        let s = m.status();
        assert!(!s.active && !s.speaking);
        assert_eq!(s.level_db, SILENCE_DB);
    }
}


