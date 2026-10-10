//! Reproducción de audio. El dispositivo de salida de `rodio` no es `Send`, así que vive en un
//! hilo propio y el resto de la app habla con él a través de `AudioBackend`.

use std::fs::File;
use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::oneshot;

use crate::error::{AppError, Result};

/// Etiqueta de un grupo de reproducciones (para cortar solo el TTS, p. ej.).
pub type Tag = &'static str;
pub const TAG_SOUND: Tag = "sound";
pub const TAG_TTS: Tag = "tts";

#[async_trait]
pub trait AudioBackend: Send + Sync {
    /// Reproduce un archivo con el volumen dado (1.0 = original). Se resuelve al terminar
    /// (también cuando se corta con `stop_tag` / `stop_all`).
    async fn play(&self, path: PathBuf, volume: f32, tag: Tag) -> Result<()>;

    /// Corta lo que esté sonando con esa etiqueta.
    fn stop_tag(&self, tag: Tag);

    /// Corta todo lo que esté sonando.
    fn stop_all(&self);

    /// Pausa lo que suene con esa etiqueta; `play` sigue pendiente hasta que termine o se corte.
    fn pause_tag(&self, tag: Tag);

    /// Reanuda lo pausado, retrocediendo antes `rewind` (`Duration::MAX` = desde el principio).
    fn resume_tag(&self, tag: Tag, rewind: Duration);
}

enum Cmd {
    Play {
        path: PathBuf,
        volume: f32,
        tag: Tag,
        done: oneshot::Sender<std::result::Result<(), String>>,
    },
    StopTag(Tag),
    StopAll,
    PauseTag(Tag),
    ResumeTag(Tag, Duration),
}

struct Active {
    player: rodio::Player,
    tag: Tag,
    done: Option<oneshot::Sender<std::result::Result<(), String>>>,
}

/// Sustituto cuando no se pudo iniciar el audio: la app sigue funcionando y las acciones de
/// sonido fallan con un mensaje claro en vez de impedir el arranque.
pub struct NullAudio;

#[async_trait]
impl AudioBackend for NullAudio {
    async fn play(&self, _path: PathBuf, _volume: f32, _tag: Tag) -> Result<()> {
        Err(AppError::Invalid("el audio no está disponible en este equipo".into()))
    }

    fn stop_tag(&self, _tag: Tag) {}

    fn stop_all(&self) {}

    fn pause_tag(&self, _tag: Tag) {}

    fn resume_tag(&self, _tag: Tag, _rewind: Duration) {}
}

/// Backend real, sobre el dispositivo de audio predeterminado del sistema.
pub struct RodioBackend {
    tx: mpsc::Sender<Cmd>,
}

impl RodioBackend {
    pub fn start() -> Result<Self> {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("hivebuzz-audio".into())
            .spawn(move || audio_thread(&rx))
            .map_err(|e| AppError::Invalid(format!("no se pudo iniciar el hilo de audio: {e}")))?;
        Ok(Self { tx })
    }
}

#[async_trait]
impl AudioBackend for RodioBackend {
    async fn play(&self, path: PathBuf, volume: f32, tag: Tag) -> Result<()> {
        let (done, rx) = oneshot::channel();
        self.tx
            .send(Cmd::Play { path, volume, tag, done })
            .map_err(|_| AppError::Invalid("el hilo de audio no está activo".into()))?;
        match rx.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(AppError::Invalid(e)),
            Err(_) => Err(AppError::Invalid("la reproducción se interrumpió".into())),
        }
    }

    fn stop_tag(&self, tag: Tag) {
        let _ = self.tx.send(Cmd::StopTag(tag));
    }

    fn stop_all(&self) {
        let _ = self.tx.send(Cmd::StopAll);
    }

    fn pause_tag(&self, tag: Tag) {
        let _ = self.tx.send(Cmd::PauseTag(tag));
    }

    fn resume_tag(&self, tag: Tag, rewind: Duration) {
        let _ = self.tx.send(Cmd::ResumeTag(tag, rewind));
    }
}

fn audio_thread(rx: &mpsc::Receiver<Cmd>) {
    let mut device: Option<rodio::MixerDeviceSink> = None;
    let mut active: Vec<Active> = Vec::new();

    loop {
        match rx.recv_timeout(Duration::from_millis(40)) {
            Ok(Cmd::Play { path, volume, tag, done }) => {
                match start_playback(&mut device, &path, volume) {
                    Ok(player) => active.push(Active {
                        player,
                        tag,
                        done: Some(done),
                    }),
                    Err(e) => {
                        tracing::warn!(path = %path.display(), error = %e, "no se pudo reproducir");
                        let _ = done.send(Err(e));
                    }
                }
            }
            Ok(Cmd::StopTag(tag)) => {
                active.retain_mut(|a| {
                    if a.tag != tag {
                        return true;
                    }
                    a.player.stop();
                    if let Some(d) = a.done.take() {
                        let _ = d.send(Ok(()));
                    }
                    false
                });
            }
            Ok(Cmd::StopAll) => {
                for mut a in active.drain(..) {
                    a.player.stop();
                    if let Some(d) = a.done.take() {
                        let _ = d.send(Ok(()));
                    }
                }
                // Se reabre el dispositivo en la próxima reproducción (p. ej. si cambió de salida).
                device = None;
            }
            Ok(Cmd::PauseTag(tag)) => {
                for a in active.iter().filter(|a| a.tag == tag) {
                    a.player.pause();
                }
            }
            Ok(Cmd::ResumeTag(tag, rewind)) => {
                for a in active.iter().filter(|a| a.tag == tag) {
                    if !rewind.is_zero() {
                        let to = a.player.get_pos().saturating_sub(rewind);
                        // Si el formato no permite buscar, se sigue desde donde se pausó.
                        if let Err(e) = a.player.try_seek(to) {
                            tracing::debug!(error = %e, "no se pudo retroceder el audio");
                        }
                    }
                    a.player.play();
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        // Recoge lo terminado, y corta lo que nadie espera ya (acción abortada por tiempo).
        active.retain_mut(|a| {
            let abandoned = a.done.as_ref().is_some_and(oneshot::Sender::is_closed);
            if abandoned {
                a.player.stop();
                return false;
            }
            if a.player.empty() {
                if let Some(d) = a.done.take() {
                    let _ = d.send(Ok(()));
                }
                return false;
            }
            true
        });
    }
}

fn start_playback(
    device: &mut Option<rodio::MixerDeviceSink>,
    path: &std::path::Path,
    volume: f32,
) -> std::result::Result<rodio::Player, String> {
    if device.is_none() {
        let opened = rodio::DeviceSinkBuilder::open_default_sink()
            .map_err(|e| format!("no hay dispositivo de audio disponible: {e}"))?;
        *device = Some(opened);
    }
    let Some(dev) = device.as_ref() else {
        return Err("no hay dispositivo de audio disponible".into());
    };
    let file = File::open(path).map_err(|e| format!("no se pudo abrir el archivo: {e}"))?;
    let source = rodio::Decoder::try_from(file).map_err(|e| format!("formato de audio no válido: {e}"))?;
    let player = rodio::Player::connect_new(dev.mixer());
    player.set_volume(volume.clamp(0.0, 2.0));
    player.append(source);
    Ok(player)
}

#[cfg(test)]
pub mod testing {
    use std::sync::{Arc, Mutex};

    use super::*;

    /// Backend que solo registra lo que se le pide reproducir.
    #[derive(Default)]
    pub struct FakeAudio {
        pub played: Arc<Mutex<Vec<(PathBuf, f32)>>>,
        pub tags: Arc<Mutex<Vec<Tag>>>,
        pub stopped: Arc<Mutex<Vec<String>>>,
        pub fail: bool,
        pub duration: Duration,
    }

    #[async_trait]
    impl AudioBackend for FakeAudio {
        async fn play(&self, path: PathBuf, volume: f32, tag: Tag) -> Result<()> {
            self.played.lock().expect("lock").push((path, volume));
            self.tags.lock().expect("lock").push(tag);
            tokio::time::sleep(self.duration).await;
            if self.fail {
                Err(AppError::Invalid("sin audio".into()))
            } else {
                Ok(())
            }
        }

        fn stop_tag(&self, tag: Tag) {
            self.stopped.lock().expect("lock").push(tag.to_string());
        }

        fn stop_all(&self) {
            self.stopped.lock().expect("lock").push("*".to_string());
        }

        fn pause_tag(&self, tag: Tag) {
            self.stopped.lock().expect("lock").push(format!("pause:{tag}"));
        }

        fn resume_tag(&self, tag: Tag, rewind: Duration) {
            let r = if rewind == Duration::MAX { "start".to_string() } else { rewind.as_millis().to_string() };
            self.stopped.lock().expect("lock").push(format!("resume:{tag}:{r}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    /// WAV de `secs` segundos con un tono (16 bits, mono, 16 kHz).
    fn tone_wav(path: &std::path::Path, secs: u32) {
        let rate = 16_000u32;
        let n = rate * secs;
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + n * 2).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * 2).to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&(n * 2).to_le_bytes());
        for i in 0..n {
            #[allow(clippy::cast_possible_truncation)]
            let v = ((f64::from(i) * 440.0 * std::f64::consts::TAU / f64::from(rate)).sin() * 2_000.0) as i16;
            b.extend_from_slice(&v.to_le_bytes());
        }
        std::fs::write(path, b).expect("wav");
    }

    /// Reproduce de verdad (necesita salida de audio): `cargo test audio_live -- --ignored`.
    /// La pausa y el retroceso alargan la reproducción lo esperado.
    #[tokio::test]
    #[ignore = "usa la salida de audio real"]
    async fn audio_live_pause_and_rewind_extend_the_playback() {
        let dir = tempfile::tempdir().expect("tmp");
        let wav = dir.path().join("t.wav");
        tone_wav(&wav, 2);
        let audio = Arc::new(RodioBackend::start().expect("audio"));
        let started = std::time::Instant::now();
        let a2 = Arc::clone(&audio);
        let play = tokio::spawn(async move { a2.play(wav, 0.05, TAG_TTS).await });
        tokio::time::sleep(Duration::from_millis(800)).await;
        audio.pause_tag(TAG_TTS);
        tokio::time::sleep(Duration::from_millis(500)).await;
        audio.resume_tag(TAG_TTS, Duration::from_millis(600));
        play.await.expect("join").expect("suena");
        let took = started.elapsed();
        eprintln!("duró {took:?}");
        // 2 s de audio + 0,5 s en pausa + 0,6 s repetidos ≈ 3,1 s.
        assert!(took >= Duration::from_millis(2_900) && took <= Duration::from_millis(3_800), "{took:?}");
    }
}
