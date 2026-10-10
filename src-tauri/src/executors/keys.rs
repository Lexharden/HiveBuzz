//! `pressKeys`: simula teclas y combinaciones, solo si la ventana en primer plano está en la lista
//! blanca.
//!
//! Parámetros: `keys` (`"ctrl+shift+f5"` o una lista de combinaciones que se pulsan en orden),
//! `holdMs` (cuánto se mantienen, 50 por defecto), `gapMs` (pausa entre combinaciones),
//! `targetWindows` (textos que deben aparecer en el título o en el nombre del `.exe` de la ventana
//! activa) y `anyWindow` (permiso explícito para no filtrar por ventana; por defecto no).
//! Sin lista blanca y sin `anyWindow: true` la regla ni siquiera se puede guardar: una tecla
//! enviada a la ventana equivocada (un navegador, el chat de TikTok) es peligrosa.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Map, Value};

use super::opt_bool;
use crate::actions::{ActionContext, ActionExecutor, Concurrency};
use crate::error::{AppError, Result};

const MAX_HOLD_MS: u64 = 10_000;
const MAX_GAP_MS: u64 = 5_000;
const MAX_CHORDS: usize = 20;
/// Duración máxima de toda la secuencia: muy por debajo del tiempo máximo de una acción en la cola.
/// Si la cola la abortara, el hilo bloqueante seguiría pulsando teclas mientras empieza la siguiente.
const MAX_TOTAL_MS: u64 = 60_000;
const MAX_TARGETS: usize = 20;

/// Ventana en primer plano.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    pub title: String,
    /// Nombre del ejecutable (`notepad.exe`), vacío si no se pudo averiguar.
    pub exe: String,
}

/// Una combinación: modificadores (se pulsan primero) y la tecla principal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chord {
    pub modifiers: Vec<u16>,
    pub key: u16,
}

/// Entrada de teclado del sistema operativo. Bloqueante: se ejecuta en `spawn_blocking`.
pub trait KeyBackend: Send + Sync {
    fn foreground(&self) -> Option<WindowInfo>;
    /// Pulsa la combinación, la mantiene `hold` y la suelta (siempre, incluso si algo falla).
    fn press(&self, chord: &Chord, hold: Duration) -> Result<()>;
}

// ---- Códigos de tecla virtual de Windows (valores fijos de la API) ----
const VK_BACK: u16 = 0x08;
const VK_TAB: u16 = 0x09;
const VK_RETURN: u16 = 0x0D;
const VK_SHIFT: u16 = 0x10;
const VK_CONTROL: u16 = 0x11;
const VK_MENU: u16 = 0x12;
const VK_ESCAPE: u16 = 0x1B;
const VK_SPACE: u16 = 0x20;

/// Traduce el nombre de una tecla (`a`, `f5`, `enter`, `ctrl`…) a su código virtual.
pub fn key_code(name: &str) -> Option<u16> {
    let n = name.trim().to_ascii_lowercase();
    let b = n.as_bytes();
    if b.len() == 1 && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit()) {
        return Some(u16::from(b[0].to_ascii_uppercase()));
    }
    if let Some(num) = n.strip_prefix('f').and_then(|d| d.parse::<u16>().ok()) {
        return (1..=24).contains(&num).then_some(0x6F + num);
    }
    Some(match n.as_str() {
        "ctrl" | "control" => VK_CONTROL,
        "shift" => VK_SHIFT,
        "alt" => VK_MENU,
        "enter" | "return" => VK_RETURN,
        "esc" | "escape" => VK_ESCAPE,
        "space" | "espacio" => VK_SPACE,
        "tab" => VK_TAB,
        "backspace" => VK_BACK,
        "delete" | "del" | "supr" => 0x2E,
        "insert" | "ins" => 0x2D,
        "home" => 0x24,
        "end" => 0x23,
        "pageup" | "pgup" => 0x21,
        "pagedown" | "pgdn" => 0x22,
        "left" => 0x25,
        "up" => 0x26,
        "right" => 0x27,
        "down" => 0x28,
        _ => return None,
    })
}

fn is_modifier(code: u16) -> bool {
    matches!(code, VK_SHIFT | VK_CONTROL | VK_MENU)
}

/// `"ctrl+shift+f5"` → combinación. Exactamente una tecla principal; los modificadores van antes.
pub fn parse_chord(text: &str) -> Result<Chord> {
    let mut modifiers = Vec::new();
    let mut main = None;
    for part in text.split('+') {
        let code = key_code(part).ok_or_else(|| AppError::Invalid(format!("tecla desconocida: «{}»", part.trim())))?;
        if is_modifier(code) {
            if !modifiers.contains(&code) {
                modifiers.push(code);
            }
        } else if main.replace(code).is_some() {
            return Err(AppError::Invalid(format!("«{text}» tiene más de una tecla principal")));
        }
    }
    let key = main.ok_or_else(|| AppError::Invalid(format!("«{text}» no tiene tecla principal (solo modificadores)")))?;
    Ok(Chord { modifiers, key })
}

fn parse_keys(params: &Map<String, Value>) -> Result<Vec<Chord>> {
    let texts: Vec<&str> = match params.get("keys") {
        Some(Value::String(s)) => vec![s.as_str()],
        Some(Value::Array(a)) => a
            .iter()
            .map(|v| v.as_str().ok_or_else(|| AppError::Invalid("«keys» solo admite textos".into())))
            .collect::<Result<_>>()?,
        _ => return Err(AppError::Invalid("falta el parámetro «keys»".into())),
    };
    if texts.is_empty() || texts.len() > MAX_CHORDS {
        return Err(AppError::Invalid(format!("«keys» necesita entre 1 y {MAX_CHORDS} combinaciones")));
    }
    texts.into_iter().map(parse_chord).collect()
}

fn parse_targets(params: &Map<String, Value>) -> Result<Vec<String>> {
    let Some(v) = params.get("targetWindows").filter(|v| !v.is_null()) else { return Ok(Vec::new()) };
    let arr = v.as_array().ok_or_else(|| AppError::Invalid("«targetWindows» debe ser una lista de textos".into()))?;
    if arr.len() > MAX_TARGETS {
        return Err(AppError::Invalid(format!("«targetWindows» admite hasta {MAX_TARGETS} entradas")));
    }
    Ok(arr
        .iter()
        .filter_map(Value::as_str)
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect())
}

fn ms(params: &Map<String, Value>, key: &str, default: u64, max: u64) -> Result<u64> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v
            .as_u64()
            .filter(|n| *n <= max)
            .ok_or_else(|| AppError::Invalid(format!("«{key}» debe ser un número entre 0 y {max}"))),
    }
}

/// ¿La ventana está permitida por la lista blanca? Compara sin distinguir mayúsculas.
pub fn window_allowed(win: &WindowInfo, targets: &[String]) -> bool {
    let (title, exe) = (win.title.to_lowercase(), win.exe.to_lowercase());
    targets.iter().filter(|t| !t.is_empty()).any(|t| title.contains(t.as_str()) || (!exe.is_empty() && exe.contains(t.as_str())))
}

pub struct PressKeysExecutor {
    backend: Arc<dyn KeyBackend>,
}

impl PressKeysExecutor {
    pub fn new(backend: Arc<dyn KeyBackend>) -> Self {
        Self { backend }
    }
}

#[async_trait]
impl ActionExecutor for PressKeysExecutor {
    fn kind(&self) -> &'static str {
        "pressKeys"
    }

    fn concurrency(&self) -> Concurrency {
        // Dos secuencias a la vez se mezclarían las teclas.
        Concurrency::Serial("keys")
    }

    fn validate(&self, params: &Map<String, Value>) -> Result<()> {
        let chords = parse_keys(params)?;
        let hold = ms(params, "holdMs", 50, MAX_HOLD_MS)?;
        let gap = ms(params, "gapMs", 50, MAX_GAP_MS)?;
        check_total(chords.len(), hold, gap)?;
        let any = opt_bool(params, "anyWindow", false)?;
        if parse_targets(params)?.is_empty() && !any {
            return Err(AppError::Invalid(
                "indica en «targetWindows» a qué ventanas se pueden enviar las teclas (o marca «anyWindow» a conciencia)".into(),
            ));
        }
        Ok(())
    }

    async fn execute(&self, _ctx: &ActionContext, params: &Map<String, Value>) -> Result<()> {
        let chords = parse_keys(params)?;
        let (hold_ms, gap_ms) = (ms(params, "holdMs", 50, MAX_HOLD_MS)?, ms(params, "gapMs", 50, MAX_GAP_MS)?);
        check_total(chords.len(), hold_ms, gap_ms)?;
        let (hold, gap) = (Duration::from_millis(hold_ms), Duration::from_millis(gap_ms));
        let targets = parse_targets(params)?;
        let any = opt_bool(params, "anyWindow", false)?;
        if targets.is_empty() && !any {
            return Err(AppError::Invalid("no hay ventanas permitidas para enviar teclas".into()));
        }
        let backend = Arc::clone(&self.backend);
        tokio::task::spawn_blocking(move || {
            for (i, chord) in chords.iter().enumerate() {
                if i > 0 {
                    std::thread::sleep(gap);
                }
                // Se comprueba antes de CADA combinación: si el foco cambia a mitad de la secuencia se aborta.
                if !any {
                    let win = backend.foreground().ok_or_else(|| AppError::Invalid("no se pudo saber qué ventana está activa".into()))?;
                    if !window_allowed(&win, &targets) {
                        return Err(AppError::Invalid(format!("la ventana activa («{}») no está en la lista blanca; no se enviaron teclas", win.title)));
                    }
                }
                backend.press(chord, hold)?;
            }
            Ok(())
        })
        .await
        .map_err(|e| AppError::Invalid(format!("la tarea de teclas falló: {e}")))?
    }
}

/// Backend para sistemas sin implementación: siempre falla con un mensaje claro.
pub struct UnsupportedKeys;

impl KeyBackend for UnsupportedKeys {
    fn foreground(&self) -> Option<WindowInfo> {
        None
    }

    fn press(&self, _chord: &Chord, _hold: Duration) -> Result<()> {
        Err(AppError::Invalid("la simulación de teclas solo está disponible en Windows por ahora".into()))
    }
}

/// El backend adecuado para este sistema.
pub fn system_backend() -> Arc<dyn KeyBackend> {
    #[cfg(windows)]
    {
        Arc::new(win::WindowsKeys)
    }
    #[cfg(not(windows))]
    {
        Arc::new(UnsupportedKeys)
    }
}

#[cfg(windows)]
mod win {
    use std::time::Duration;

    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE,
        MAPVK_VK_TO_VSC,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId};

    use super::{Chord, KeyBackend, WindowInfo};
    use crate::error::{AppError, Result};

    pub struct WindowsKeys;

    fn is_extended(vk: u16) -> bool {
        // Insert, Delete, Home, End, PageUp, PageDown y flechas.
        matches!(vk, 0x21..=0x28 | 0x2D | 0x2E)
    }

    fn send(vk: u16, up: bool) -> Result<()> {
        // SAFETY: MapVirtualKeyW solo lee sus argumentos.
        let scan = unsafe { MapVirtualKeyW(u32::from(vk), MAPVK_VK_TO_VSC) };
        let mut flags = KEYEVENTF_SCANCODE;
        if is_extended(vk) {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }
        if up {
            flags |= KEYEVENTF_KEYUP;
        }
        let input = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: 0, wScan: u16::try_from(scan).unwrap_or(0), dwFlags: flags, time: 0, dwExtraInfo: 0 } },
        };
        // SAFETY: `input` es una estructura INPUT válida y el tamaño pasado es el de INPUT.
        let sent = unsafe { SendInput(1, &input, i32::try_from(std::mem::size_of::<INPUT>()).unwrap_or(0)) };
        if sent == 1 {
            Ok(())
        } else {
            Err(AppError::Invalid("Windows rechazó la entrada de teclado (¿ventana con más privilegios?)".into()))
        }
    }

    fn exe_name(pid: u32) -> String {
        // SAFETY: se abre y se cierra el manejador; el búfer es local y su tamaño se pasa en `len`.
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return String::new();
            }
            let mut buf = [0u16; 1024];
            let mut len = u32::try_from(buf.len()).unwrap_or(0);
            let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len);
            CloseHandle(h);
            if ok == 0 {
                return String::new();
            }
            let path = String::from_utf16_lossy(&buf[..len as usize]);
            path.rsplit(['\\', '/']).next().unwrap_or("").to_string()
        }
    }

    impl KeyBackend for WindowsKeys {
        fn foreground(&self) -> Option<WindowInfo> {
            // SAFETY: llamadas de solo lectura sobre la ventana activa; los búferes son locales.
            unsafe {
                let hwnd = GetForegroundWindow();
                if hwnd.is_null() {
                    return None;
                }
                let mut buf = [0u16; 512];
                let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), i32::try_from(buf.len()).unwrap_or(0));
                let title = String::from_utf16_lossy(&buf[..usize::try_from(n).unwrap_or(0)]);
                let mut pid = 0u32;
                GetWindowThreadProcessId(hwnd, &mut pid);
                Some(WindowInfo { title, exe: if pid == 0 { String::new() } else { exe_name(pid) } })
            }
        }

        fn press(&self, chord: &Chord, hold: Duration) -> Result<()> {
            let mut down: Vec<u16> = Vec::new();
            let mut result = Ok(());
            for &vk in chord.modifiers.iter().chain(std::iter::once(&chord.key)) {
                match send(vk, false) {
                    Ok(()) => down.push(vk),
                    Err(e) => {
                        result = Err(e);
                        break;
                    }
                }
            }
            if result.is_ok() {
                std::thread::sleep(hold);
            }
            // Siempre se sueltan, en orden inverso, aunque algo haya fallado.
            for &vk in down.iter().rev() {
                if let Err(e) = send(vk, true) {
                    tracing::error!(error = %e, vk, "no se pudo soltar una tecla");
                    result = result.and(Err(e));
                }
            }
            result
        }
    }
}

fn check_total(chords: usize, hold_ms: u64, gap_ms: u64) -> Result<()> {
    let n = chords as u64;
    let total = n * hold_ms + n.saturating_sub(1) * gap_ms;
    if total > MAX_TOTAL_MS {
        return Err(AppError::Invalid(format!(
            "la secuencia de teclas duraría {} s; el máximo es {} s (menos teclas, o menos tiempo pulsadas o entre ellas)",
            total.div_ceil(1000),
            MAX_TOTAL_MS / 1000
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap_or_default()
    }

    fn ctx() -> ActionContext {
        ActionContext { rule_id: "r".into(), vars: Default::default() }
    }

    #[test]
    fn parses_key_names() {
        assert_eq!(key_code("A"), Some(0x41));
        assert_eq!(key_code("7"), Some(0x37));
        assert_eq!(key_code("F1"), Some(0x70));
        assert_eq!(key_code("f12"), Some(0x7B));
        assert_eq!(key_code("f24"), Some(0x87));
        assert_eq!(key_code("f25"), None);
        assert_eq!(key_code("Enter"), Some(VK_RETURN));
        assert_eq!(key_code("nope"), None);
    }

    #[test]
    fn parses_chords() {
        let c = parse_chord("Ctrl + Shift+F5").unwrap();
        assert_eq!(c, Chord { modifiers: vec![VK_CONTROL, VK_SHIFT], key: 0x74 });
        assert!(parse_chord("ctrl+shift").is_err(), "solo modificadores");
        assert!(parse_chord("a+b").is_err(), "dos teclas principales");
        assert!(parse_chord("ctrl+").is_err());
        assert!(parse_chord("").is_err());
    }

    #[test]
    fn window_whitelist_matches_title_or_exe() {
        let w = WindowInfo { title: "Minecraft 1.21".into(), exe: "javaw.exe".into() };
        assert!(window_allowed(&w, &["minecraft".into()]));
        assert!(window_allowed(&w, &["javaw".into()]));
        assert!(!window_allowed(&w, &["chrome".into()]));
        assert!(!window_allowed(&w, &[]));
        let no_exe = WindowInfo { title: "Doc".into(), exe: String::new() };
        assert!(!window_allowed(&no_exe, &["".into()]), "un texto vacío no permite todo");
    }

    #[test]
    fn validation_requires_a_whitelist_or_explicit_any_window() {
        let e = PressKeysExecutor::new(Arc::new(UnsupportedKeys));
        assert!(e.validate(&obj(json!({"keys": "f5"}))).is_err());
        assert!(e.validate(&obj(json!({"keys": "f5", "targetWindows": []}))).is_err());
        assert!(e.validate(&obj(json!({"keys": "f5", "targetWindows": ["obs"]}))).is_ok());
        assert!(e.validate(&obj(json!({"keys": ["f5", "ctrl+a"], "anyWindow": true}))).is_ok());
        assert!(e.validate(&obj(json!({"keys": "f5", "targetWindows": ["obs"], "holdMs": 999_999}))).is_err());
        assert!(e.validate(&obj(json!({"keys": "zz", "anyWindow": true}))).is_err());
        assert!(e.validate(&obj(json!({"targetWindows": ["obs"]}))).is_err());
        // 20 combinaciones × 10 s: cada valor es válido, pero el total no.
        let many: Vec<&str> = vec!["f5"; 20];
        assert!(e.validate(&obj(json!({"keys": many, "anyWindow": true, "holdMs": 10_000}))).is_err());
        assert!(e.validate(&obj(json!({"keys": many, "anyWindow": true, "holdMs": 1_000, "gapMs": 1_000}))).is_ok());
    }

    struct Fake {
        window: Mutex<Option<WindowInfo>>,
        pressed: Mutex<Vec<Chord>>,
    }

    impl Fake {
        fn new(title: &str) -> Arc<Self> {
            Arc::new(Self { window: Mutex::new(Some(WindowInfo { title: title.into(), exe: "game.exe".into() })), pressed: Mutex::new(Vec::new()) })
        }
    }

    impl KeyBackend for Fake {
        fn foreground(&self) -> Option<WindowInfo> {
            self.window.lock().unwrap().clone()
        }

        fn press(&self, chord: &Chord, _hold: Duration) -> Result<()> {
            self.pressed.lock().unwrap().push(chord.clone());
            Ok(())
        }
    }

    #[tokio::test]
    async fn presses_in_order_when_window_is_allowed() {
        let fake = Fake::new("My Game");
        let e = PressKeysExecutor::new(fake.clone());
        let p = obj(json!({"keys": ["f5", "ctrl+a"], "targetWindows": ["my game"], "holdMs": 1, "gapMs": 1}));
        e.execute(&ctx(), &p).await.unwrap();
        let pressed = fake.pressed.lock().unwrap().clone();
        assert_eq!(pressed.len(), 2);
        assert_eq!(pressed[0].key, 0x74);
        assert_eq!(pressed[1].modifiers, vec![VK_CONTROL]);
    }

    #[tokio::test]
    async fn refuses_when_the_active_window_is_not_whitelisted() {
        let fake = Fake::new("Chrome - TikTok");
        let e = PressKeysExecutor::new(fake.clone());
        let p = obj(json!({"keys": "f5", "targetWindows": ["my game"]}));
        let err = e.execute(&ctx(), &p).await.unwrap_err();
        assert!(err.to_string().contains("lista blanca"), "{err}");
        assert!(fake.pressed.lock().unwrap().is_empty(), "no debe enviarse ninguna tecla");
    }

    #[tokio::test]
    async fn refuses_when_the_foreground_is_unknown() {
        let fake = Fake::new("x");
        *fake.window.lock().unwrap() = None;
        let e = PressKeysExecutor::new(fake.clone());
        let p = obj(json!({"keys": "f5", "targetWindows": ["x"]}));
        assert!(e.execute(&ctx(), &p).await.is_err());
        assert!(fake.pressed.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn any_window_skips_the_check() {
        let fake = Fake::new("whatever");
        let e = PressKeysExecutor::new(fake.clone());
        e.execute(&ctx(), &obj(json!({"keys": "f5", "anyWindow": true, "holdMs": 1}))).await.unwrap();
        assert_eq!(fake.pressed.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn execution_without_whitelist_or_any_window_is_refused() {
        // Un job persistido antiguo o editado a mano no puede saltarse la validación.
        let fake = Fake::new("x");
        let e = PressKeysExecutor::new(fake.clone());
        assert!(e.execute(&ctx(), &obj(json!({"keys": "f5"}))).await.is_err());
        assert!(fake.pressed.lock().unwrap().is_empty());
    }
}
