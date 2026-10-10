//! Motor SAPI de Windows (voces del sistema, sin descargas). En otras plataformas no hay voces.
//!
//! Además de las voces SAPI clásicas (`System.Speech`), lista las voces «OneCore» de Windows 10/11
//! (las que se añaden con los paquetes de idioma, p. ej. Microsoft Raúl), que `System.Speech` no ve;
//! esas se sintetizan con `Windows.Media.SpeechSynthesis`.
//!
//! El texto y la voz viajan por variables de entorno, nunca interpolados en el script, para que
//! un mensaje de chat no pueda inyectar comandos de PowerShell.

use std::path::PathBuf;
#[cfg(windows)]
use std::time::Duration;

use async_trait::async_trait;

use super::engine::{SynthRequest, TtsEngine};
use super::policy::VoiceInfo;
use crate::error::{AppError, Result};

#[cfg(windows)]
const SYNTH_SCRIPT: &str = "\
Add-Type -AssemblyName System.Speech
$s = New-Object System.Speech.Synthesis.SpeechSynthesizer
try {
  $classic = (-not $env:HB_VOICE) -or (@($s.GetInstalledVoices() | Where-Object { $_.VoiceInfo.Name -eq $env:HB_VOICE }).Count -gt 0)
  if ($classic) {
    if ($env:HB_VOICE) { $s.SelectVoice($env:HB_VOICE) }
    $s.Rate = [int]$env:HB_RATE
    $s.SetOutputToWaveFile($env:HB_OUT)
    $s.Speak($env:HB_TEXT)
  }
} finally { $s.Dispose() }
if (-not $classic) {
  $null = [Windows.Media.SpeechSynthesis.SpeechSynthesizer, Windows.Media.SpeechSynthesis, ContentType = WindowsRuntime]
  Add-Type -AssemblyName System.Runtime.WindowsRuntime
  $asTask = [System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object { $_.Name -eq 'AsTask' -and $_.IsGenericMethod -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1' } | Select-Object -First 1
  $voice = [Windows.Media.SpeechSynthesis.SpeechSynthesizer]::AllVoices | Where-Object { $_.DisplayName -eq $env:HB_VOICE } | Select-Object -First 1
  if (-not $voice) { throw \"voz no encontrada: $env:HB_VOICE\" }
  $w = New-Object Windows.Media.SpeechSynthesis.SpeechSynthesizer
  try {
    $w.Voice = $voice
    $w.Options.SpeakingRate = [double]::Parse($env:HB_WRATE, [Globalization.CultureInfo]::InvariantCulture)
    $task = $asTask.MakeGenericMethod([Windows.Media.SpeechSynthesis.SpeechSynthesisStream]).Invoke($null, @($w.SynthesizeTextToStreamAsync($env:HB_TEXT)))
    if (-not $task.Wait(25000)) { throw 'tiempo agotado' }
    $in = [System.IO.WindowsRuntimeStreamExtensions]::AsStreamForRead($task.Result)
    $fs = [System.IO.File]::Create($env:HB_OUT)
    try { $in.CopyTo($fs) } finally { $fs.Dispose(); $in.Dispose() }
  } finally { $w.Dispose() }
}";

/// Voces clásicas y, después, las OneCore que no estén ya como clásicas (`Microsoft Sabina` es la misma
/// voz que `Microsoft Sabina Desktop`). Si la API de OneCore no existe (Windows antiguo) se omite.
#[cfg(windows)]
const LIST_SCRIPT: &str = "\
Add-Type -AssemblyName System.Speech
$s = New-Object System.Speech.Synthesis.SpeechSynthesizer
$classic = @($s.GetInstalledVoices() | Where-Object { $_.Enabled } | ForEach-Object { $_.VoiceInfo })
$s.Dispose()
$classic | ForEach-Object { $_.Name + '|' + $_.Culture.Name }
try {
  $null = [Windows.Media.SpeechSynthesis.SpeechSynthesizer, Windows.Media.SpeechSynthesis, ContentType = WindowsRuntime]
  $names = @($classic | ForEach-Object { $_.Name })
  [Windows.Media.SpeechSynthesis.SpeechSynthesizer]::AllVoices |
    Where-Object { ($names -notcontains $_.DisplayName) -and ($names -notcontains ($_.DisplayName + ' Desktop')) } |
    ForEach-Object { $_.DisplayName + '|' + $_.Language }
} catch {}";

#[cfg(windows)]
const TIMEOUT: Duration = Duration::from_secs(30);

pub struct SapiEngine;

/// 0.5–2.0 → -10…10 (la escala de SAPI), con 1.0 = 0.
pub fn rate_to_sapi(rate: f64) -> i32 {
    #[allow(clippy::cast_possible_truncation)]
    let r = ((rate - 1.0) * 10.0).round().clamp(-10.0, 10.0) as i32;
    r
}

/// Interpreta la salida del script de listado (`Nombre|es-MX`, una por línea).
pub fn parse_voice_list(output: &str) -> Vec<VoiceInfo> {
    output
        .lines()
        .filter_map(|l| {
            let (name, lang) = l.trim().rsplit_once('|')?;
            let name = name.trim();
            (!name.is_empty()).then(|| VoiceInfo {
                id: format!("sapi:{name}"),
                engine: "sapi".into(),
                name: name.to_string(),
                lang: Some(lang.trim().to_string()).filter(|l| !l.is_empty()),
            })
        })
        .collect()
}

#[async_trait]
impl TtsEngine for SapiEngine {
    fn id(&self) -> &'static str {
        "sapi"
    }

    #[cfg(windows)]
    async fn voices(&self) -> Vec<VoiceInfo> {
        let run = super::engine::hidden_command("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", LIST_SCRIPT])
            .output();
        match tokio::time::timeout(TIMEOUT, run).await {
            Ok(Ok(out)) if out.status.success() => parse_voice_list(&String::from_utf8_lossy(&out.stdout)),
            Ok(Ok(out)) => {
                tracing::warn!(status = ?out.status, "SAPI: no se pudieron listar las voces");
                Vec::new()
            }
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "SAPI: no se pudo ejecutar PowerShell");
                Vec::new()
            }
            Err(_) => {
                tracing::warn!("SAPI: se agotó el tiempo al listar las voces");
                Vec::new()
            }
        }
    }

    #[cfg(not(windows))]
    async fn voices(&self) -> Vec<VoiceInfo> {
        Vec::new()
    }

    #[cfg(windows)]
    async fn synthesize(&self, req: &SynthRequest) -> Result<PathBuf> {
        let out = req.out_dir.join(format!("{}.wav", uuid::Uuid::new_v4()));
        let run = super::engine::hidden_command("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", SYNTH_SCRIPT])
            .env("HB_TEXT", &req.text)
            .env("HB_VOICE", &req.voice)
            .env("HB_RATE", rate_to_sapi(req.rate).to_string())
            // OneCore usa un multiplicador (1.0 = normal), con punto decimal siempre.
            .env("HB_WRATE", format!("{:.2}", req.rate.clamp(0.5, 2.0)))
            .env("HB_OUT", &out)
            .output();
        let output = tokio::time::timeout(TIMEOUT, run)
            .await
            .map_err(|_| AppError::Invalid("SAPI tardó demasiado en sintetizar".into()))??;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(AppError::Invalid(format!("SAPI falló: {}", err.trim().lines().next().unwrap_or("error desconocido"))));
        }
        let ok = tokio::fs::metadata(&out).await.is_ok_and(|m| m.len() > 44);
        if !ok {
            return Err(AppError::Invalid("SAPI no generó audio".into()));
        }
        Ok(out)
    }

    #[cfg(not(windows))]
    async fn synthesize(&self, _req: &SynthRequest) -> Result<PathBuf> {
        Err(AppError::Invalid("SAPI solo está disponible en Windows".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_mapping_is_centered_and_clamped() {
        assert_eq!(rate_to_sapi(1.0), 0);
        assert_eq!(rate_to_sapi(1.5), 5);
        assert_eq!(rate_to_sapi(2.0), 10);
        assert_eq!(rate_to_sapi(0.5), -5);
        assert_eq!(rate_to_sapi(9.0), 10);
        assert_eq!(rate_to_sapi(-3.0), -10);
    }

    #[test]
    fn parses_the_voice_listing() {
        let v = parse_voice_list("Microsoft Sabina Desktop|es-MX\r\nMicrosoft Zira Desktop|en-US\n\nrota\n|es-ES\n");
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].id, "sapi:Microsoft Sabina Desktop");
        assert_eq!(v[0].lang.as_deref(), Some("es-MX"));
        assert_eq!(v[1].name, "Microsoft Zira Desktop");
    }

    /// Prueba real contra las voces instaladas en Windows: genera un WAV (no necesita altavoces).
    #[cfg(windows)]
    #[tokio::test]
    async fn synthesizes_a_real_wav_with_the_installed_voice() {
        let engine = SapiEngine;
        let voices = engine.voices().await;
        let Some(voice) = voices.first() else {
            eprintln!("sin voces SAPI instaladas: se omite la prueba");
            return;
        };
        let dir = tempfile::tempdir().expect("tmp");
        let path = engine
            .synthesize(&SynthRequest {
                text: "Hola, esto es una prueba. $(Get-Date) \"comillas\" 'simples'".into(),
                voice: voice.name.clone(),
                rate: 1.2,
                out_dir: dir.path().to_path_buf(),
            })
            .await
            .expect("sintetiza");
        let bytes = std::fs::read(&path).expect("lee");
        assert!(bytes.len() > 1000, "el WAV debe tener audio");
        assert_eq!(&bytes[..4], b"RIFF");
        // El texto no se interpreta como código: no debe fallar por las comillas ni el `$()`.
        assert!(rodio::Decoder::try_from(std::fs::File::open(&path).expect("abre")).is_ok());
    }

    /// Las voces OneCore (las que no son «Desktop») también generan un WAV real.
    #[cfg(windows)]
    #[tokio::test]
    async fn synthesizes_with_a_onecore_voice_too() {
        let engine = SapiEngine;
        let voices = engine.voices().await;
        let names: Vec<_> = voices.iter().map(|v| v.name.as_str()).collect();
        let Some(voice) = voices.iter().find(|v| !v.name.ends_with(" Desktop")) else {
            eprintln!("sin voces OneCore instaladas: se omite la prueba");
            return;
        };
        assert!(!names.contains(&format!("{} Desktop", voice.name).as_str()), "no se duplica una voz clásica");
        let dir = tempfile::tempdir().expect("tmp");
        let path = engine
            .synthesize(&SynthRequest {
                text: "Hola, prueba con una voz de Windows. $(Get-Date) \"comillas\"".into(),
                voice: voice.name.clone(),
                rate: 1.3,
                out_dir: dir.path().to_path_buf(),
            })
            .await
            .expect("sintetiza con OneCore");
        let bytes = std::fs::read(&path).expect("lee");
        assert!(bytes.len() > 1000 && &bytes[..4] == b"RIFF", "WAV con audio");
        assert!(rodio::Decoder::try_from(std::fs::File::open(&path).expect("abre")).is_ok());
    }
}
