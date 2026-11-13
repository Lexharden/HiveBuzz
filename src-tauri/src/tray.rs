//! Icono de la bandeja del sistema: mostrar la ventana y salir.

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Runtime};

use crate::ui_sync::set_ui_visible;

const MAIN_WINDOW: &str = "main";

/// Elementos del menú, guardados para poder traducirlos cuando cambia el idioma.
struct TrayMenu<R: Runtime> {
    show: MenuItem<R>,
    quit: MenuItem<R>,
}

/// Textos del menú en el idioma de la app (el español es el de fábrica).
fn labels(language: &str) -> (&'static str, &'static str) {
    if language == "en" {
        ("Show HiveBuzz", "Quit")
    } else {
        ("Mostrar HiveBuzz", "Salir")
    }
}

pub fn show_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window(MAIN_WINDOW) {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        set_ui_visible(true);
    }
}

pub fn install<R: Runtime>(app: &AppHandle<R>, language: &str) -> tauri::Result<()> {
    let (show_label, quit_label) = labels(language);
    let show = MenuItem::with_id(app, "show", show_label, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", quit_label, true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("HiveBuzz")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    app.manage(TrayMenu { show, quit });
    Ok(())
}

/// Traduce el menú de la bandeja al idioma elegido (si el icono existe).
pub fn set_language<R: Runtime>(app: &AppHandle<R>, language: &str) {
    let Some(menu) = app.try_state::<TrayMenu<R>>() else {
        return;
    };
    let (show_label, quit_label) = labels(language);
    if let Err(e) = menu.show.set_text(show_label).and_then(|()| menu.quit.set_text(quit_label)) {
        tracing::warn!(error = %e, "no se pudo traducir el menú de la bandeja");
    }
}

#[cfg(test)]
mod tests {
    use super::labels;

    #[test]
    fn menu_labels_follow_the_language() {
        assert_eq!(labels("en"), ("Show HiveBuzz", "Quit"));
        assert_eq!(labels("es"), ("Mostrar HiveBuzz", "Salir"));
        assert_eq!(labels("fr"), ("Mostrar HiveBuzz", "Salir"), "idioma desconocido: el de fábrica");
    }
}
