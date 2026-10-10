// El enlazador de MSVC informa «Creando biblioteca…» al generar la cdylib; es solo informativo.
#![allow(linker_messages)]

pub mod actions;
pub mod app;
pub mod audio;
pub mod backup;
pub mod bot;
pub mod bus;
pub mod commands;
pub mod commands_app;
pub mod commands_bot;
pub mod commands_integrations;
pub mod commands_spotify;
pub mod commands_twitch;
pub mod connection;
pub mod connections;
pub mod counters;
pub mod db;
pub mod error;
pub mod events;
pub mod executors;
pub mod goals;
pub mod interact;
pub mod leaderboard;
pub mod logs;
pub mod media;
pub mod overlay;
pub mod overlay_config;
pub mod points;
pub mod profiles;
pub mod prefs;
pub mod rules;
pub mod secrets;
pub mod session;
pub mod server;
pub mod simulator;
pub mod spotify;
pub mod stats;
pub mod source;
pub mod sounds;
pub mod timers;
#[cfg(test)]
pub mod testutil;
pub mod twitch;
pub mod tray;
pub mod tts;
pub mod updater;

use tauri::{Manager, RunEvent};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

use crate::app::AppState;

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,hive_buzz_lib=debug,sidecar=debug"));
    // Si ya hay un subscriber instalado (tests, recarga) no es un error.
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .with(logs::BufferLayer(logs::global().clone()))
        .try_init();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_tracing();

    let builder = tauri::Builder::default();
    // Debe ir primero: una segunda instancia abriría la misma base de datos, otro sidecar y otra
    // conexión, y repetiría las acciones pendientes de la cola. En su lugar se muestra la ventana.
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| tray::show_main(app)));
    let result = builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec!["--minimized"])))
        .on_window_event(|window, event| {
            // Con «cerrar a la bandeja» la ventana solo se oculta; se sale desde el menú de la bandeja.
            // Solo la ventana principal: la de inicio de sesión de TikTok debe cerrarse de verdad.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() != "main" {
                    return;
                }
                let to_tray = window.try_state::<AppState>().is_some_and(|s| s.prefs.get().close_to_tray);
                if to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .setup(|app| {
            // `HIVEBUZZ_DATA_DIR` permite usar otra carpeta de datos (pruebas, instalación portátil) sin tocar la real.
            let data_dir = match std::env::var_os("HIVEBUZZ_DATA_DIR").filter(|v| !v.is_empty()) {
                Some(dir) => std::path::PathBuf::from(dir),
                None => app.path().app_data_dir()?,
            };
            let handle = app.handle().clone();
            let state = tauri::async_runtime::block_on(AppState::init(handle, &data_dir))?;
            let prefs = state.prefs.get();
            app.manage(state);
            let (show, quit) = if prefs.language == "en" { ("Show HiveBuzz", "Quit") } else { ("Mostrar HiveBuzz", "Salir") };
            if let Err(e) = tray::install(app.handle(), show, quit) {
                tracing::warn!(error = %e, "no se pudo crear el icono de la bandeja");
            }
            // Icono de la ventana (barra de tareas, Alt+Tab): el de HiveBuzz, nunca el de Tauri por defecto.
            if let (Some(w), Some(icon)) = (app.get_webview_window("main"), app.default_window_icon()) {
                if let Err(e) = w.set_icon(icon.clone()) {
                    tracing::warn!(error = %e, "no se pudo poner el icono de la ventana");
                }
            }
            // Arranque con el sistema: sin ventana si así se pidió.
            if prefs.start_minimized && std::env::args().any(|a| a == "--minimized") {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.hide();
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::connect,
            commands::disconnect,
            commands::get_statuses,
            commands::get_app_info,
            commands::set_euler_api_key,
            commands::set_server_port,
            commands::simulate_event,
            commands::simulate_burst,
            commands::recent_events,
            commands::list_rules,
            commands::save_rule,
            commands::delete_rule,
            commands::set_rule_enabled,
            commands::test_rule,
            commands::list_action_types,
            commands::queue_stats,
            commands::clear_queue,
            commands::list_sounds,
            commands::import_sound,
            commands::update_sound,
            commands::delete_sound,
            commands::preview_sound,
            commands::stop_audio,
            commands::list_media,
            commands::import_media,
            commands::rename_media,
            commands::delete_media,
            commands::get_tts_config,
            commands::set_tts_config,
            commands::list_tts_voices,
            commands::get_tts_status,
            commands::tts_preview,
            commands::tts_skip,
            commands::install_piper,
            commands::install_piper_voice,
            commands::import_piper_voice,
            commands::delete_piper_voice,
            commands::list_overlays,
            commands::get_overlay_config,
            commands::set_overlay_config,
            commands::reset_overlay_config,
            commands::test_overlay,
            commands::list_goals,
            commands::save_goal,
            commands::delete_goal,
            commands::adjust_goal,
            commands::reset_goal,
            commands::list_timers,
            commands::save_timer,
            commands::delete_timer,
            commands::control_timer,
            commands::get_leaderboard,
            commands::clear_donor_history,
            commands::get_counters,
            commands::new_session,
            commands_bot::get_points_config,
            commands_bot::set_points_config,
            commands_bot::list_viewers,
            commands_bot::viewer_history,
            commands_bot::adjust_viewer_points,
            commands_bot::set_viewer_points,
            commands_bot::delete_viewer,
            commands_bot::clear_viewers,
            commands_bot::export_viewers_csv,
            commands_bot::import_viewers_csv,
            commands_bot::get_bot_config,
            commands_bot::set_bot_config,
            commands_bot::get_bot_log,
            commands_bot::clear_bot_log,
            commands_bot::bot_say,
            commands_bot::get_wheel_config,
            commands_bot::set_wheel_config,
            commands_bot::spin_wheel_test,
            commands_bot::get_poll,
            commands_bot::start_poll,
            commands_bot::stop_poll,
            commands_bot::clear_poll,
            commands_bot::has_tiktok_session,
            commands_bot::tiktok_login_start,
            commands_bot::tiktok_login_finish,
            commands_bot::tiktok_logout,
            commands_integrations::get_obs_config,
            commands_integrations::set_obs_config,
            commands_integrations::has_obs_password,
            commands_integrations::set_obs_password,
            commands_integrations::test_obs,
            commands_app::get_app_prefs,
            commands_app::set_app_prefs,
            commands_app::get_autostart,
            commands_app::set_autostart,
            commands_app::get_logs,
            commands_app::clear_logs,
            commands_app::export_logs,
            commands_app::list_streams,
            commands_app::get_stream,
            commands_app::delete_stream,
            commands_app::list_profiles,
            commands_app::active_profile,
            commands_app::save_profile,
            commands_app::apply_profile,
            commands_app::rename_profile,
            commands_app::delete_profile,
            commands_app::export_config,
            commands_app::import_config,
            commands_app::has_pending_import,
            commands_app::cancel_pending_import,
            commands_app::take_import_notice,
            commands_app::restart_app,
            commands_app::check_update,
            commands_app::install_update,
            commands_twitch::twitch_status,
            commands_twitch::twitch_get_config,
            commands_twitch::twitch_set_config,
            commands_twitch::twitch_login_start,
            commands_twitch::twitch_login_state,
            commands_twitch::twitch_login_cancel,
            commands_twitch::twitch_logout,
            commands_twitch::twitch_open_activation,
            commands_spotify::spotify_get_config,
            commands_spotify::spotify_set_config,
            commands_spotify::spotify_status,
            commands_spotify::spotify_connect,
            commands_spotify::spotify_disconnect,
            commands_spotify::spotify_queue_test,
        ])
        .build(tauri::generate_context!());

    match result {
        Ok(app) => app.run(|handle, event| {
            // Al salir se detiene el audio, la cola y el sidecar (si no, este último cierra al cerrarse su stdin).
            if let RunEvent::Exit = event {
                if let Some(state) = handle.try_state::<AppState>() {
                    state.shutdown();
                }
            }
        }),
        Err(e) => {
            tracing::error!(error = %e, "no se pudo iniciar HiveBuzz");
            eprintln!("No se pudo iniciar HiveBuzz: {e}");
            std::process::exit(1);
        }
    }
}
