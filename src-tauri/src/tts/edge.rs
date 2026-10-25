//! Motor Edge-TTS (opcional): voces neuronales de Microsoft Edge por WebSocket.
//!
//! Es un servicio no oficial: Microsoft puede cambiar su protocolo o sus tokens en cualquier
//! momento. Por eso es opcional y, si falla, el error lo dice claramente (Piper/SAPI no se ven
//! afectados). Las partes puras (token, SSML, tramas) tienen pruebas; la conexión real se
//! verifica a mano.

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use sha2::{Digest, Sha256};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

use super::engine::{SynthRequest, TtsEngine};
use super::policy::VoiceInfo;
use crate::error::{AppError, Result};

const TRUSTED_CLIENT_TOKEN: &str = "6A5AA1D4EAFF4E9FB37E23D68491D6F4";
/// Debe parecerse a un Edge reciente: Microsoft rechaza (403) versiones muy viejas.
/// Si el servicio deja de responder, comprueba primero los hosts de `ENDPOINTS` (Microsoft los
/// ha movido) y después esta constante.
const CHROMIUM_VERSION: &str = "143.0.3650.75";

/// Host de síntesis y nombre del parámetro que lleva el token público. Desde octubre de 2026 el host
/// histórico responde 403 al WebSocket (primero en varios países, luego en todos); Edge sintetiza por
/// `api.msedgeservices.com`, que acepta el mismo token como `Ocp-Apim-Subscription-Key`. Se prueban en
/// orden y se conserva el antiguo como alternativa por si Microsoft lo reabre o retira el nuevo.
const ENDPOINTS: &[(&str, &str)] = &[
    ("wss://api.msedgeservices.com/tts/cognitiveservices/websocket/v1", "Ocp-Apim-Subscription-Key"),
    ("wss://speech.platform.bing.com/consumer/speech/synthesize/readaloud/edge/v1", "TrustedClientToken"),
];
const OUTPUT_FORMAT: &str = "audio-24khz-48kbitrate-mono-mp3";
/// Tiempo máximo de toda la síntesis (todos los servidores y un reintento por desfase de reloj).
const TIMEOUT: Duration = Duration::from_secs(40);
/// Tiempo máximo por servidor: si el primero se cuelga, el segundo todavía tiene margen para probarse.
const HOST_TIMEOUT: Duration = Duration::from_secs(12);

/// Voces del servicio de lectura de Edge en español, inglés, portugués, francés, italiano, alemán, japonés y
/// coreano (comprobadas contra su `voices/list` en octubre de 2026). Las «Multilingual» leen varios idiomas.
const VOICES: &[(&str, &str)] = &[
    ("es-AR-ElenaNeural", "es-AR"),
    ("es-AR-TomasNeural", "es-AR"),
    ("es-BO-MarceloNeural", "es-BO"),
    ("es-BO-SofiaNeural", "es-BO"),
    ("es-CL-CatalinaNeural", "es-CL"),
    ("es-CL-LorenzoNeural", "es-CL"),
    ("es-CO-GonzaloNeural", "es-CO"),
    ("es-CO-SalomeNeural", "es-CO"),
    ("es-CR-JuanNeural", "es-CR"),
    ("es-CR-MariaNeural", "es-CR"),
    ("es-CU-BelkysNeural", "es-CU"),
    ("es-CU-ManuelNeural", "es-CU"),
    ("es-DO-EmilioNeural", "es-DO"),
    ("es-DO-RamonaNeural", "es-DO"),
    ("es-EC-AndreaNeural", "es-EC"),
    ("es-EC-LuisNeural", "es-EC"),
    ("es-ES-AlvaroNeural", "es-ES"),
    ("es-ES-ElviraNeural", "es-ES"),
    ("es-ES-XimenaNeural", "es-ES"),
    ("es-GQ-JavierNeural", "es-GQ"),
    ("es-GQ-TeresaNeural", "es-GQ"),
    ("es-GT-AndresNeural", "es-GT"),
    ("es-GT-MartaNeural", "es-GT"),
    ("es-HN-CarlosNeural", "es-HN"),
    ("es-HN-KarlaNeural", "es-HN"),
    ("es-MX-DaliaNeural", "es-MX"),
    ("es-MX-JorgeNeural", "es-MX"),
    ("es-NI-FedericoNeural", "es-NI"),
    ("es-NI-YolandaNeural", "es-NI"),
    ("es-PA-MargaritaNeural", "es-PA"),
    ("es-PA-RobertoNeural", "es-PA"),
    ("es-PE-AlexNeural", "es-PE"),
    ("es-PE-CamilaNeural", "es-PE"),
    ("es-PR-KarinaNeural", "es-PR"),
    ("es-PR-VictorNeural", "es-PR"),
    ("es-PY-MarioNeural", "es-PY"),
    ("es-PY-TaniaNeural", "es-PY"),
    ("es-SV-LorenaNeural", "es-SV"),
    ("es-SV-RodrigoNeural", "es-SV"),
    ("es-US-AlonsoNeural", "es-US"),
    ("es-US-PalomaNeural", "es-US"),
    ("es-UY-MateoNeural", "es-UY"),
    ("es-UY-ValentinaNeural", "es-UY"),
    ("es-VE-PaolaNeural", "es-VE"),
    ("es-VE-SebastianNeural", "es-VE"),
    ("en-GB-LibbyNeural", "en-GB"),
    ("en-GB-MaisieNeural", "en-GB"),
    ("en-GB-RyanNeural", "en-GB"),
    ("en-GB-SoniaNeural", "en-GB"),
    ("en-GB-ThomasNeural", "en-GB"),
    ("en-US-AnaNeural", "en-US"),
    ("en-US-AndrewMultilingualNeural", "en-US"),
    ("en-US-AndrewNeural", "en-US"),
    ("en-US-AriaNeural", "en-US"),
    ("en-US-AvaMultilingualNeural", "en-US"),
    ("en-US-AvaNeural", "en-US"),
    ("en-US-BrianMultilingualNeural", "en-US"),
    ("en-US-BrianNeural", "en-US"),
    ("en-US-ChristopherNeural", "en-US"),
    ("en-US-EmmaMultilingualNeural", "en-US"),
    ("en-US-EmmaNeural", "en-US"),
    ("en-US-EricNeural", "en-US"),
    ("en-US-GuyNeural", "en-US"),
    ("en-US-JennyNeural", "en-US"),
    ("en-US-MichelleNeural", "en-US"),
    ("en-US-RogerNeural", "en-US"),
    ("en-US-SteffanNeural", "en-US"),
    ("pt-BR-AntonioNeural", "pt-BR"),
    ("pt-BR-FranciscaNeural", "pt-BR"),
    ("pt-BR-ThalitaMultilingualNeural", "pt-BR"),
    ("pt-PT-DuarteNeural", "pt-PT"),
    ("pt-PT-RaquelNeural", "pt-PT"),
    ("fr-FR-DeniseNeural", "fr-FR"),
    ("fr-FR-EloiseNeural", "fr-FR"),
    ("fr-FR-HenriNeural", "fr-FR"),
    ("fr-FR-RemyMultilingualNeural", "fr-FR"),
    ("fr-FR-VivienneMultilingualNeural", "fr-FR"),
    ("it-IT-DiegoNeural", "it-IT"),
    ("it-IT-ElsaNeural", "it-IT"),
    ("it-IT-GiuseppeMultilingualNeural", "it-IT"),
    ("it-IT-IsabellaNeural", "it-IT"),
    ("de-DE-AmalaNeural", "de-DE"),
    ("de-DE-ConradNeural", "de-DE"),
    ("de-DE-FlorianMultilingualNeural", "de-DE"),
    ("de-DE-KatjaNeural", "de-DE"),
    ("de-DE-KillianNeural", "de-DE"),
    ("de-DE-SeraphinaMultilingualNeural", "de-DE"),
    ("ja-JP-KeitaNeural", "ja-JP"),
    ("ja-JP-NanamiNeural", "ja-JP"),
    ("ko-KR-HyunsuMultilingualNeural", "ko-KR"),
    ("ko-KR-InJoonNeural", "ko-KR"),
    ("ko-KR-SunHiNeural", "ko-KR"),
];

pub struct EdgeEngine;

/// Token `Sec-MS-GEC`: SHA-256 (mayúsculas) de «ticks de Windows redondeados a 5 min» + token.
pub fn gec_token(unix_secs: u64) -> String {
    const WIN_EPOCH_OFFSET: u64 = 11_644_473_600;
    let mut ticks = unix_secs + WIN_EPOCH_OFFSET;
    ticks -= ticks % 300;
    ticks *= 10_000_000;
    let digest = Sha256::digest(format!("{ticks}{TRUSTED_CLIENT_TOKEN}").as_bytes());
    digest.iter().map(|b| format!("{b:02X}")).collect()
}

/// 1.0 → «+0%», 1.5 → «+50%», 0.5 → «-50%».
pub fn rate_percent(rate: f64) -> String {
    #[allow(clippy::cast_possible_truncation)]
    let pct = ((rate.clamp(0.5, 2.0) - 1.0) * 100.0).round() as i32;
    format!("{pct:+}%")
}

pub fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // Los caracteres de control no son válidos en XML 1.0.
            c if c.is_control() && !matches!(c, '\n' | '\t') => {}
            c => out.push(c),
        }
    }
    out
}

pub fn build_ssml(voice: &str, text: &str, rate: f64) -> String {
    // El idioma del SSML se deduce del nombre de la voz (`es-MX-DaliaNeural` → `es-MX`).
    let lang: String = voice.splitn(3, '-').take(2).collect::<Vec<_>>().join("-");
    format!(
        "<speak version='1.0' xmlns='http://www.w3.org/2001/10/synthesis' xml:lang='{}'>\
         <voice name='{}'><prosody pitch='+0Hz' rate='{}' volume='+0%'>{}</prosody></voice></speak>",
        xml_escape(&lang),
        xml_escape(voice),
        rate_percent(rate),
        xml_escape(text)
    )
}

/// En las tramas binarias, los 2 primeros bytes (big-endian) dan el largo de la cabecera de texto;
/// el resto es audio si la cabecera dice `Path:audio`.
pub fn audio_payload(frame: &[u8]) -> Option<&[u8]> {
    let header_len = usize::from(u16::from_be_bytes([*frame.first()?, *frame.get(1)?]));
    let header = frame.get(2..2 + header_len)?;
    let header = std::str::from_utf8(header).ok()?;
    header.contains("Path:audio").then(|| &frame[2 + header_len..])
}

/// Marca de tiempo al estilo JavaScript que usa el cliente oficial.
fn timestamp() -> String {
    chrono::Utc::now().format("%a %b %d %Y %H:%M:%S GMT+0000 (Coordinated Universal Time)").to_string()
}

fn unix_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

/// Segundos de diferencia entre el reloj del servidor (cabecera `Date`) y el local.
pub fn clock_skew_from_date_header(date: &str, local_unix: i64) -> Option<i64> {
    let server = chrono::DateTime::parse_from_rfc2822(date.trim()).ok()?.timestamp();
    Some(server - local_unix)
}

#[async_trait]
impl TtsEngine for EdgeEngine {
    fn id(&self) -> &'static str {
        "edge"
    }

    async fn voices(&self) -> Vec<VoiceInfo> {
        VOICES
            .iter()
            .map(|(name, lang)| VoiceInfo {
                id: format!("edge:{name}"),
                engine: "edge".into(),
                name: (*name).to_string(),
                lang: Some((*lang).to_string()),
            })
            .collect()
    }

    async fn synthesize(&self, req: &SynthRequest) -> Result<PathBuf> {
        if !VOICES.iter().any(|(n, _)| *n == req.voice) {
            return Err(AppError::Invalid(format!("voz de Edge desconocida: {}", req.voice)));
        }
        let attempt = async {
            let mut notes = Vec::new();
            let mut skew_known = 0;
            for &(host, key_param) in ENDPOINTS {
                let mut result = within_host_limit(host, fetch_audio(req, host, key_param, skew_known)).await;
                // 403 con cabecera `Date`: el reloj local está desfasado y el token GEC no vale.
                if let Err(Fetch::Forbidden { skew_secs: Some(skew) }) = result {
                    if skew != 0 {
                        skew_known = skew;
                        result = within_host_limit(host, fetch_audio(req, host, key_param, skew)).await;
                    }
                }
                match result {
                    Ok(audio) => return Ok(audio),
                    // Un 403 o un fallo de conexión en este host: se prueba el siguiente.
                    Err(e) => notes.push(format!("{}: {}", host.split('/').nth(2).unwrap_or(host), e.describe())),
                }
            }
            Err(AppError::Invalid(format!("Edge-TTS no respondió ({}); usa Piper o SAPI", notes.join("; "))))
        };
        let audio = tokio::time::timeout(TIMEOUT, attempt)
            .await
            .map_err(|_| AppError::Invalid("Edge-TTS tardó demasiado en responder".into()))??;
        let out = req.out_dir.join(format!("{}.mp3", uuid::Uuid::new_v4()));
        tokio::fs::write(&out, audio).await?;
        Ok(out)
    }
}

/// Corta un intento que no responde, para pasar al siguiente servidor en vez de agotar todo el tiempo.
async fn within_host_limit<T>(
    host: &str,
    attempt: impl std::future::Future<Output = std::result::Result<T, Fetch>>,
) -> std::result::Result<T, Fetch> {
    tokio::time::timeout(HOST_TIMEOUT, attempt)
        .await
        .unwrap_or_else(|_| Err(other(format!("Edge-TTS: {} no respondió en {} s", host.split('/').nth(2).unwrap_or(host), HOST_TIMEOUT.as_secs()))))
}

/// Fallo de una conexión a Edge-TTS.
enum Fetch {
    /// 403: token o versión rechazados; `skew_secs` es el desfase de reloj si el servidor lo reveló.
    Forbidden { skew_secs: Option<i64> },
    Other(String),
}

impl Fetch {
    fn describe(&self) -> &str {
        match self {
            Fetch::Forbidden { .. } => "rechazó la conexión (403)",
            Fetch::Other(m) => m,
        }
    }
}

fn other(msg: impl Into<String>) -> Fetch {
    Fetch::Other(msg.into())
}

/// URL del WebSocket: token público (con el nombre de parámetro propio de cada host), id de conexión y GEC.
pub fn connection_url(host: &str, key_param: &str, connection_id: &str, unix_secs: u64) -> String {
    format!(
        "{host}?{key_param}={TRUSTED_CLIENT_TOKEN}&ConnectionId={connection_id}\
         &Sec-MS-GEC={}&Sec-MS-GEC-Version=1-{CHROMIUM_VERSION}",
        gec_token(unix_secs)
    )
}

/// Petición de apertura del WebSocket, con las cabeceras que Microsoft exige a un cliente de Edge.
fn build_request(url: &str) -> std::result::Result<tokio_tungstenite::tungstenite::handshake::client::Request, Fetch> {
    let major = CHROMIUM_VERSION.split('.').next().unwrap_or("143");
    let mut request = url.into_client_request().map_err(|e| other(format!("URL inválida: {e}")))?;
    let muid = uuid::Uuid::new_v4().simple().to_string().to_uppercase();
    let h = request.headers_mut();
    let set = |h: &mut tokio_tungstenite::tungstenite::http::HeaderMap, k: &'static str, v: String| {
        if let Ok(v) = v.parse() {
            h.insert(k, v);
        }
    };
    // Sin `Accept` y `Accept-Encoding` el servidor nuevo responde `200 OK` vacío en vez de aceptar el
    // WebSocket (los clientes HTTP habituales las añaden solos; este no). Con el `User-Agent` y el
    // `Accept-Language` ausentes responde 403 o 200 igualmente.
    set(h, "Accept", "*/*".into());
    set(h, "Accept-Encoding", "gzip, deflate, br, zstd".into());
    set(h, "Pragma", "no-cache".into());
    set(h, "Cache-Control", "no-cache".into());
    set(h, "Origin", "chrome-extension://jdiccldimpdaibmpdkjnbmckianbfold".into());
    set(h, "Accept-Language", "en-US,en;q=0.9".into());
    set(h, "Cookie", format!("muid={muid};"));
    set(
        h,
        "User-Agent",
        format!("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{major}.0.0.0 Safari/537.36 Edg/{major}.0.0.0"),
    );
    Ok(request)
}

async fn fetch_audio(
    req: &SynthRequest,
    host: &str,
    key_param: &str,
    skew_secs: i64,
) -> std::result::Result<Vec<u8>, Fetch> {
    let unix = u64::try_from(unix_now() + skew_secs).unwrap_or(0);
    let connection_id = uuid::Uuid::new_v4().simple().to_string();
    let url = connection_url(host, key_param, &connection_id, unix);
    let request = build_request(&url)?;

    let (mut ws, _) = match tokio_tungstenite::connect_async(request).await {
        Ok(ok) => ok,
        Err(tokio_tungstenite::tungstenite::Error::Http(resp)) if resp.status().as_u16() == 403 => {
            let skew = resp
                .headers()
                .get("date")
                .and_then(|d| d.to_str().ok())
                .and_then(|d| clock_skew_from_date_header(d, unix_now()));
            return Err(Fetch::Forbidden { skew_secs: skew });
        }
        Err(tokio_tungstenite::tungstenite::Error::Http(resp)) => {
            return Err(other(format!("respondió HTTP {} en vez de abrir la conexión", resp.status().as_u16())));
        }
        Err(e) => return Err(other(format!("no se pudo conectar: {e}"))),
    };

    let ts = timestamp();
    let config = format!(
        "X-Timestamp:{ts}\r\nContent-Type:application/json; charset=utf-8\r\nPath:speech.config\r\n\r\n\
         {{\"context\":{{\"synthesis\":{{\"audio\":{{\"metadataoptions\":{{\"sentenceBoundaryEnabled\":\"false\",\
         \"wordBoundaryEnabled\":\"false\"}},\"outputFormat\":\"{OUTPUT_FORMAT}\"}}}}}}}}\r\n"
    );
    let ssml = format!(
        "X-RequestId:{}\r\nContent-Type:application/ssml+xml\r\nX-Timestamp:{ts}Z\r\nPath:ssml\r\n\r\n{}",
        uuid::Uuid::new_v4().simple(),
        build_ssml(&req.voice, &req.text, req.rate)
    );
    for msg in [config, ssml] {
        ws.send(Message::text(msg)).await.map_err(|e| other(format!("no se pudo enviar: {e}")))?;
    }

    let mut audio = Vec::new();
    while let Some(frame) = ws.next().await {
        match frame.map_err(|e| other(format!("conexión interrumpida: {e}")))? {
            Message::Binary(data) => {
                if let Some(chunk) = audio_payload(&data) {
                    audio.extend_from_slice(chunk);
                }
            }
            Message::Text(t) if t.contains("Path:turn.end") => break,
            Message::Close(_) => break,
            _ => {}
        }
    }
    let _ = ws.close(None).await;
    if audio.is_empty() {
        return Err(other("no devolvió audio"));
    }
    Ok(audio)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gec_token_matches_an_independent_reference() {
        // Calculado fuera de este código con .NET (SHA-256 sobre «ticks + token»).
        assert_eq!(gec_token(1_700_000_000), "42301B335578FEFDAE2637DED1ABD614505D432559EC08032B82048483726AFF");
    }

    #[test]
    fn gec_token_is_stable_within_a_five_minute_window() {
        let base = 1_700_000_100; // múltiplo de 300 en tiempo Windows
        let a = gec_token(base - base % 300);
        assert_eq!(a, gec_token(base - base % 300 + 299));
        assert_ne!(a, gec_token(base - base % 300 + 300));
    }

    #[test]
    fn clock_skew_is_read_from_the_http_date_header() {
        // 2026-10-08 12:00:00 UTC
        let server = 1_791_460_800;
        assert_eq!(clock_skew_from_date_header("Thu, 08 Oct 2026 12:00:00 GMT", server - 90), Some(90));
        assert_eq!(clock_skew_from_date_header("Thu, 08 Oct 2026 12:00:00 GMT", server + 30), Some(-30));
        assert_eq!(clock_skew_from_date_header("no es una fecha", 0), None);
    }

    #[test]
    fn each_host_gets_the_token_under_its_own_parameter_name() {
        let new = connection_url(ENDPOINTS[0].0, ENDPOINTS[0].1, "abc", 1_700_000_000);
        assert!(new.starts_with("wss://api.msedgeservices.com/tts/cognitiveservices/websocket/v1?Ocp-Apim-Subscription-Key="));
        let old = connection_url(ENDPOINTS[1].0, ENDPOINTS[1].1, "abc", 1_700_000_000);
        assert!(old.contains("?TrustedClientToken=6A5AA1D4EAFF4E9FB37E23D68491D6F4&ConnectionId=abc&Sec-MS-GEC="));
        assert!(new.ends_with("&Sec-MS-GEC-Version=1-143.0.3650.75"));
    }

    #[test]
    fn the_handshake_carries_the_headers_microsoft_requires() {
        let req = build_request(&connection_url(ENDPOINTS[0].0, ENDPOINTS[0].1, "abc", 1_700_000_000)).unwrap_or_else(|_| panic!("petición"));
        let h = req.headers();
        for name in ["accept", "accept-encoding", "accept-language", "user-agent", "origin", "cookie", "sec-websocket-key"] {
            assert!(h.contains_key(name), "falta la cabecera {name}");
        }
        assert!(h["user-agent"].to_str().unwrap_or_default().contains("Edg/143."));
    }

    #[test]
    fn timestamp_has_the_javascript_date_shape() {
        let t = timestamp();
        assert!(t.ends_with("GMT+0000 (Coordinated Universal Time)"), "{t}");
        // «Fri Oct 09 2026 06:17:43 GMT+0000 (Coordinated Universal Time)»
        assert_eq!(t.split_whitespace().count(), 9, "{t}");
    }

    #[test]
    fn rate_is_expressed_as_a_signed_percentage() {
        assert_eq!(rate_percent(1.0), "+0%");
        assert_eq!(rate_percent(1.5), "+50%");
        assert_eq!(rate_percent(0.5), "-50%");
        assert_eq!(rate_percent(9.0), "+100%");
    }

    #[test]
    fn text_cannot_break_out_of_the_ssml() {
        let ssml = build_ssml("es-MX-DaliaNeural", "</prosody></voice><evil a='1'>& \"hola\"\u{7}", 1.0);
        assert!(!ssml.contains("<evil"));
        assert!(ssml.contains("&lt;/prosody&gt;&lt;/voice&gt;&lt;evil a=&apos;1&apos;&gt;&amp; &quot;hola&quot;"));
        assert!(ssml.contains("xml:lang='es-MX'"));
        assert!(ssml.contains("<voice name='es-MX-DaliaNeural'>"));
        assert!(!ssml.contains('\u{7}'));
    }

    #[test]
    fn extracts_audio_from_binary_frames() {
        let header = b"X-RequestId:abc\r\nContent-Type:audio/mpeg\r\nPath:audio\r\n";
        let mut frame = u16::try_from(header.len()).expect("len").to_be_bytes().to_vec();
        frame.extend_from_slice(header);
        frame.extend_from_slice(&[1, 2, 3, 4]);
        assert_eq!(audio_payload(&frame), Some(&[1u8, 2, 3, 4][..]));

        let other = b"Path:response\r\n";
        let mut f2 = u16::try_from(other.len()).expect("len").to_be_bytes().to_vec();
        f2.extend_from_slice(other);
        f2.extend_from_slice(&[9, 9]);
        assert_eq!(audio_payload(&f2), None);
    }

    #[test]
    fn malformed_frames_do_not_panic() {
        for bad in [&[][..], &[0][..], &[0xFF, 0xFF, 1, 2][..], &[0, 5, b'a'][..], &[0, 2, 0xFF, 0xFE, 7][..]] {
            assert_eq!(audio_payload(bad), None);
        }
    }

    /// Prueba real contra el servicio de Microsoft (requiere red): `cargo test edge_live -- --ignored`.
    #[tokio::test]
    #[ignore = "usa la red y un servicio no oficial"]
    async fn edge_live_synthesizes_real_audio() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = EdgeEngine
            .synthesize(&SynthRequest {
                text: "Hola, esto es una prueba de HiveBuzz.".into(),
                voice: "es-MX-DaliaNeural".into(),
                rate: 1.1,
                out_dir: dir.path().into(),
            })
            .await
            .expect("Edge-TTS debe responder");
        let len = std::fs::metadata(&path).expect("meta").len();
        assert!(len > 2000, "audio demasiado corto: {len} bytes");
        assert!(rodio::Decoder::try_from(std::fs::File::open(&path).expect("abre")).is_ok(), "no decodifica como MP3");
    }

    #[test]
    fn the_voice_list_has_no_duplicates_and_each_language_matches_its_name() {
        let mut seen = std::collections::HashSet::new();
        for (name, lang) in VOICES {
            assert!(seen.insert(*name), "repetida: {name}");
            assert!(name.starts_with(&format!("{lang}-")) && name.ends_with("Neural"), "{name} / {lang}");
        }
        assert!(VOICES.iter().filter(|(_, l)| l.starts_with("es-")).count() >= 40, "todas las variantes del español");
    }

    #[tokio::test]
    async fn lists_the_curated_voices_and_rejects_unknown_ones() {
        let e = EdgeEngine;
        let v = e.voices().await;
        assert!(v.iter().any(|v| v.id == "edge:es-MX-DaliaNeural"));
        let dir = tempfile::tempdir().expect("tmp");
        let err = e
            .synthesize(&SynthRequest { text: "hola".into(), voice: "no-existe".into(), rate: 1.0, out_dir: dir.path().into() })
            .await
            .expect_err("voz desconocida");
        assert!(err.to_string().contains("desconocida"));
    }

    #[tokio::test(start_paused = true)]
    async fn a_hanging_host_is_cut_off_so_the_next_one_can_be_tried() {
        let hung = within_host_limit("wss://a.example/x", std::future::pending::<std::result::Result<(), Fetch>>()).await;
        match hung {
            Err(Fetch::Other(m)) => assert!(m.contains("a.example") && m.contains("12"), "{m}"),
            _ => panic!("debía fallar por tiempo"),
        }
        let ok = within_host_limit("wss://b.example/x", async { Ok::<_, Fetch>(7) }).await;
        assert!(matches!(ok, Ok(7)));
    }
}
