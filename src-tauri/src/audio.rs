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
    }
}
