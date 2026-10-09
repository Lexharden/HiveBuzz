//! Interfaz común de los motores de síntesis de voz.

use std::ffi::OsStr;
use std::path::PathBuf;

use async_trait::async_trait;

use super::policy::VoiceInfo;
use crate::error::Result;

pub struct SynthRequest {
    pub text: String,
    /// Nombre de la voz dentro del motor (sin el prefijo `motor:`).
    pub voice: String,
    /// Velocidad, 0.5–2.0.
    pub rate: f64,
    /// Carpeta donde dejar el audio generado.
    pub out_dir: PathBuf,
}

#[async_trait]
pub trait TtsEngine: Send + Sync {
    /// Identificador del motor, prefijo de sus voces (`piper`, `sapi`, `edge`).
    fn id(&self) -> &'static str;

    /// Voces disponibles ahora mismo. Vacío si el motor no está instalado o no responde.
    async fn voices(&self) -> Vec<VoiceInfo>;

    /// Genera el audio y devuelve la ruta del archivo (WAV o MP3, ambos reproducibles).
    async fn synthesize(&self, req: &SynthRequest) -> Result<PathBuf>;
}

/// `motor:voz` → `(motor, voz)`.
pub fn split_voice_id(id: &str) -> Option<(&str, &str)> {
    id.split_once(':').filter(|(e, v)| !e.is_empty() && !v.is_empty())
}

/// Comando externo sin ventana de consola (en Windows) y que muere si se cancela la tarea.
pub(crate) fn hidden_command(program: impl AsRef<OsStr>) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(program);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    cmd.kill_on_drop(true);
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_voice_ids() {
        assert_eq!(split_voice_id("piper:es_MX-claude-high"), Some(("piper", "es_MX-claude-high")));
        // El nombre de la voz puede contener «:».
        assert_eq!(split_voice_id("sapi:Microsoft Sabina: es"), Some(("sapi", "Microsoft Sabina: es")));
        for bad in ["", "sinprefijo", ":voz", "motor:"] {
            assert_eq!(split_voice_id(bad), None, "{bad:?}");
        }
    }
}
