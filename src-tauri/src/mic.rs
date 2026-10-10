//! Detección de voz en el micrófono, para que el TTS no hable encima del streamer.
//!
//! Un hilo propio captura el micrófono (el `Stream` de `cpal` no es `Send`) y mide el nivel de
//! cada bloque de audio. Un detector por energía decide si el streamer está hablando y publica
//! el cambio por un canal `watch`. No se guarda ni se envía audio: solo se calcula el nivel.

use std::sync::atomic::{AtomicU32, Ordering};
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

struct Running {
    settings: MicSettings,
    /// Al soltarlo, el hilo de captura termina.
    _stop: mpsc::Sender<()>,
}

pub struct MicMonitor {
    speaking: Arc<watch::Sender<bool>>,
    level: Arc<AtomicU32>,
    error: Arc<Mutex<Option<String>>>,
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

impl SpeechDetector for MicMonitor {
    fn configure(&self, settings: Option<MicSettings>) {
        let mut running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        if running.as_ref().map(|r| &r.settings) == settings.as_ref() {
            return;
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
        let (speaking, level, error) = (Arc::clone(&self.speaking), Arc::clone(&self.level), Arc::clone(&self.error));
        let s = settings.clone();
        let spawned = std::thread::Builder::new().name("hivebuzz-mic".into()).spawn(move || {
            if let Err(e) = capture(&s, &speaking, &level, &stop_rx) {
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
        *running = Some(Running { settings, _stop: stop_tx });
    }

    fn subscribe(&self) -> watch::Receiver<bool> {
        self.speaking.subscribe()
    }
}

fn device_name(d: &cpal::Device) -> Option<String> {
    d.description().ok().map(|desc| desc.name().to_string())
}

/// Micrófonos disponibles (por nombre).
pub fn input_devices() -> Vec<String> {
    let host = cpal::default_host();
    let Ok(devices) = host.input_devices() else {
        return Vec::new();
    };
    let mut names: Vec<String> = devices.filter_map(|d| device_name(&d)).collect();
    names.sort();
    names.dedup();
    names
}

/// Captura hasta que se suelte el emisor de `stop`. Solo devuelve error si no pudo empezar.
fn capture(
    settings: &MicSettings,
    speaking: &Arc<watch::Sender<bool>>,
    level: &Arc<AtomicU32>,
    stop: &mpsc::Receiver<()>,
) -> Result<(), String> {
    let host = cpal::default_host();
    let device = match &settings.device {
        Some(name) => host
            .input_devices()
            .map_err(|e| format!("no se pudieron listar los micrófonos: {e}"))?
            .find(|d| device_name(d).as_deref() == Some(name.as_str()))
            .ok_or_else(|| format!("no se encontró el micrófono «{name}»"))?,
        None => host.default_input_device().ok_or("no hay ningún micrófono disponible")?,
    };
    let supported = device.default_input_config().map_err(|e| format!("el micrófono no da su formato: {e}"))?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();

    let mut vad = Vad::new(settings.threshold_db, settings.hold_ms);
    let started = Instant::now();
    let (speaking, level) = (Arc::clone(speaking), Arc::clone(level));
    let mut buf: Vec<f32> = Vec::new();
    let mut on_block = move |samples: &mut dyn Iterator<Item = f32>| {
        buf.clear();
        buf.extend(samples);
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
    tracing::info!(device = settings.device.as_deref().unwrap_or("(predeterminado)"), "micrófono: escuchando");

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
    fn configure_is_idempotent_and_none_stops_listening() {
        let m = MicMonitor::new();
        m.configure(None);
        let s = m.status();
        assert!(!s.active && !s.speaking);
        assert_eq!(s.level_db, SILENCE_DB);
    }
}

