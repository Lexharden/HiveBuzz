//! Motor Piper: TTS neuronal local y offline (https://github.com/rhasspy/piper).
//! Se invoca como proceso externo: `piper --model voz.onnx --output_file salida.wav`, con el
//! texto por stdin.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::AsyncWriteExt;

use super::engine::{hidden_command, SynthRequest, TtsEngine};
use super::policy::VoiceInfo;
use crate::error::{AppError, Result};

const TIMEOUT: Duration = Duration::from_secs(45);

/// Dónde están el ejecutable y las voces (cambian si el usuario los reconfigura).
#[derive(Debug, Clone, Default)]
pub struct PiperPaths {
    pub exe: PathBuf,
    pub voices_dir: PathBuf,
}

pub struct PiperEngine {
    paths: Arc<RwLock<PiperPaths>>,
}

impl PiperEngine {
    pub fn new(paths: Arc<RwLock<PiperPaths>>) -> Self {
        Self { paths }
    }

    fn paths(&self) -> PiperPaths {
        self.paths.read().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

/// Piper usa `length_scale` (mayor = más lento): la inversa de la velocidad.
pub fn length_scale(rate: f64) -> f64 {
    1.0 / rate.clamp(0.5, 2.0)
}

/// Voces `.onnx` de una carpeta (cada una con su `.onnx.json`).
pub fn scan_voices(dir: &Path) -> Vec<VoiceInfo> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<VoiceInfo> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let name = path.file_name()?.to_str()?.strip_suffix(".onnx")?.to_string();
            let config = dir.join(format!("{name}.onnx.json"));
            config.is_file().then(|| VoiceInfo {
                id: format!("piper:{name}"),
                engine: "piper".into(),
                lang: voice_language(&config),
                name,
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// `language.code` del JSON de la voz (`es_MX` → `es-MX`).
fn voice_language(config: &Path) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(config).ok()?).ok()?;
    let code = v.get("language")?.get("code")?.as_str()?;
    Some(code.replace('_', "-"))
}

/// Nombre de voz seguro: sin separadores de ruta (impide salir de la carpeta de voces).
fn safe_voice_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(['/', '\\', ':']) && !name.contains("..")
}

#[async_trait]
impl TtsEngine for PiperEngine {
    fn id(&self) -> &'static str {
        "piper"
    }

    async fn voices(&self) -> Vec<VoiceInfo> {
        let p = self.paths();
        if !p.exe.is_file() {
            return Vec::new();
        }
        tokio::task::spawn_blocking(move || scan_voices(&p.voices_dir)).await.unwrap_or_default()
    }

    async fn synthesize(&self, req: &SynthRequest) -> Result<PathBuf> {
        let p = self.paths();
        if !p.exe.is_file() {
            return Err(AppError::Invalid("Piper no está instalado".into()));
        }
        if !safe_voice_name(&req.voice) {
            return Err(AppError::Invalid("nombre de voz no válido".into()));
        }
        let model = p.voices_dir.join(format!("{}.onnx", req.voice));
        if !model.is_file() {
            return Err(AppError::Invalid(format!("la voz «{}» no está instalada", req.voice)));
        }
        let out = req.out_dir.join(format!("{}.wav", uuid::Uuid::new_v4()));

        let mut child = hidden_command(&p.exe)
            .arg("--model")
            .arg(&model)
            .arg("--output_file")
            .arg(&out)
            .arg("--length_scale")
            .arg(format!("{:.2}", length_scale(req.rate)))
            // Piper necesita encontrar `espeak-ng-data` junto a su ejecutable.
            .current_dir(p.exe.parent().unwrap_or_else(|| Path::new(".")))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| AppError::Invalid(format!("no se pudo ejecutar Piper: {e}")))?;

        if let Some(mut stdin) = child.stdin.take() {
            // Una sola línea: Piper sintetiza una frase por línea.
            let line = req.text.replace(['\r', '\n'], " ");
            stdin.write_all(line.as_bytes()).await?;
            stdin.write_all(b"\n").await?;
            // Al soltar `stdin` se cierra la entrada y Piper termina.
        }
        let output = tokio::time::timeout(TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| AppError::Invalid("Piper tardó demasiado en sintetizar".into()))??;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(AppError::Invalid(format!("Piper falló: {}", err.trim().lines().last().unwrap_or("error desconocido"))));
        }
        if !tokio::fs::metadata(&out).await.is_ok_and(|m| m.len() > 44) {
            return Err(AppError::Invalid("Piper no generó audio".into()));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_scale_is_the_inverse_of_speed() {
        assert!((length_scale(1.0) - 1.0).abs() < 1e-9);
        assert!((length_scale(2.0) - 0.5).abs() < 1e-9);
        assert!((length_scale(0.5) - 2.0).abs() < 1e-9);
        assert!((length_scale(100.0) - 0.5).abs() < 1e-9, "se limita a 2.0×");
    }

    #[test]
    fn scans_only_voices_that_have_their_json() {
        let dir = tempfile::tempdir().expect("tmp");
        let w = |n: &str, c: &str| std::fs::write(dir.path().join(n), c).expect("write");
        w("es_MX-claude-high.onnx", "x");
        w("es_MX-claude-high.onnx.json", r#"{"language": {"code": "es_MX"}}"#);
        w("en_US-amy-medium.onnx", "x");
        w("en_US-amy-medium.onnx.json", "{}");
        w("huerfana.onnx", "x"); // sin JSON
        w("notas.txt", "x");
        let v = scan_voices(dir.path());
        assert_eq!(v.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(), ["piper:en_US-amy-medium", "piper:es_MX-claude-high"]);
        assert_eq!(v[1].lang.as_deref(), Some("es-MX"));
        assert_eq!(v[0].lang, None);
        assert!(scan_voices(&dir.path().join("no-existe")).is_empty());
    }

    #[test]
    fn voice_names_cannot_escape_the_folder() {
        for bad in ["", "../x", "a/b", "a\\b", "C:evil", ".."] {
            assert!(!safe_voice_name(bad), "{bad:?}");
        }
        assert!(safe_voice_name("es_MX-claude-high"));
    }

    #[tokio::test]
    async fn reports_clear_errors_when_not_installed_or_voice_missing() {
        let dir = tempfile::tempdir().expect("tmp");
        let paths = Arc::new(RwLock::new(PiperPaths { exe: dir.path().join("piper.exe"), voices_dir: dir.path().into() }));
        let engine = PiperEngine::new(paths.clone());
        assert!(engine.voices().await.is_empty());
        let req = |voice: &str| SynthRequest { text: "hola".into(), voice: voice.into(), rate: 1.0, out_dir: dir.path().into() };
        let e = engine.synthesize(&req("x")).await.expect_err("sin Piper");
        assert!(e.to_string().contains("no está instalado"));

        // Con un «ejecutable» presente pero sin la voz, el error nombra la voz.
        std::fs::write(dir.path().join("piper.exe"), "").expect("write");
        let e = engine.synthesize(&req("fantasma")).await.expect_err("sin voz");
        assert!(e.to_string().contains("fantasma"));
        let e = engine.synthesize(&req("../fuera")).await.expect_err("ruta");
        assert!(e.to_string().contains("no válido"));
    }
}
