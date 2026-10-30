mod state;

use std::sync::atomic::Ordering;
use std::sync::Arc;

use claude_local_core::{Config, TerminalApp};
use serde::{Deserialize, Serialize};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, State, WebviewWindow, WindowEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_positioner::{Position, WindowExt};

use state::{AppState, Snapshot};

pub(crate) const TRAY_ID: &str = "main";

#[tauri::command]
fn get_snapshot(state: State<Arc<AppState>>) -> Snapshot {
    state.snapshot()
}

#[tauri::command]
fn refresh(state: State<Arc<AppState>>) {
    state.wake();
}

#[tauri::command]
fn open_session(state: State<Arc<AppState>>, id: String) -> Result<(), String> {
    state.open_session(&id).map_err(|e| format!("{e:#}"))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TerminalOption {
    value: TerminalApp,
    label: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SettingsView {
    config: Config,
    terminals: Vec<TerminalOption>,
    claude_resolved: Option<String>,
    env_error: Option<String>,
    autostart: bool,
    config_path: Option<String>,
}

#[tauri::command]
fn get_settings(app: AppHandle, state: State<Arc<AppState>>) -> SettingsView {
    let (claude_resolved, env_error) = state.env_status();
    SettingsView {
        config: state.config(),
        terminals: TerminalApp::available()
            .iter()
            .map(|&value| TerminalOption {
                value,
                label: value.label(),
            })
            .collect(),
        claude_resolved,
        env_error,
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        config_path: Config::path().map(|p| p.display().to_string()),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveSettings {
    config: Config,
    autostart: bool,
}

#[tauri::command]
fn save_settings(
    app: AppHandle,
    state: State<Arc<AppState>>,
    settings: SaveSettings,
) -> Result<(), String> {
    let autolaunch = app.autolaunch();
    let result = if settings.autostart {
        autolaunch.enable()
    } else {
        autolaunch.disable()
    };
    result.map_err(|e| format!("launch at login: {e}"))?;
    state
        .set_config(settings.config)
        .map_err(|e| format!("{e:#}"))
}

fn main_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("main")
}

fn show_window(app: &AppHandle) {
    let Some(window) = main_window(app) else {
        return;
    };
    let position = if cfg!(target_os = "macos") {
        Position::TrayCenter
    } else {
        Position::TrayBottomRight
    };
    // Fails if the tray hasn't reported a position yet; then the window keeps
    // its last spot.
    let _ = window.move_window(position);
    let _ = window.show();
    let _ = window.set_focus();
    set_visible(app, true);
}

fn hide_window(app: &AppHandle) {
    if let Some(window) = main_window(app) {
        let _ = window.hide();
    }
    set_visible(app, false);
}

fn toggle_window(app: &AppHandle) {
    let visible = main_window(app).is_some_and(|w| w.is_visible().unwrap_or(false));
    if visible {
        hide_window(app);
    } else {
        show_window(app);
    }
}

fn set_visible(app: &AppHandle, visible: bool) {
    let state = app.state::<Arc<AppState>>();
    let was = state.visible.swap(visible, Ordering::SeqCst);
    if visible && !was {
        // Opening the list should show fresh data, not wait out the slow poll.
        state.wake();
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show Sessions", true, None::<&str>)?;
    let refresh = MenuItem::with_id(app, "refresh", "Refresh", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Claude Local", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&show, &refresh, &PredefinedMenuItem::separator(app)?, &quit],
    )?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(Image::from_bytes(include_bytes!("../icons/tray.png"))?)
        .icon_as_template(true)
        .tooltip("Claude Local")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_window(app),
            "refresh" => app.state::<Arc<AppState>>().wake(),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            tauri_plugin_positioner::on_tray_event(tray.app_handle(), &event);
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_window(app)
        }))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_positioner::init())
        .setup(|app| {
            // Tray-only on macOS: no Dock icon.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let state = Arc::new(AppState::new(Config::load()));
            app.manage(state.clone());
            build_tray(app.handle())?;
            state::spawn_poller(app.handle().clone(), state);
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the list hides it; the app lives in the tray.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                hide_window(window.app_handle());
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            refresh,
            open_session,
            get_settings,
            save_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running claude-local");
}
