mod autostart;
mod monitor;
mod runner;
mod scripts;

use std::sync::Mutex;
use tauri::{
    image::Image,
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, PhysicalPosition, PhysicalSize, WebviewWindow,
};

/// The raw app logo, unpadded, used for the tray icon. Kept separate from
/// `icons/icon.png` (Tauri's generated icon set), which adds platform
/// padding that makes the logo look shrunk at tray size.
const TRAY_LOGO_BYTES: &[u8] = include_bytes!("../icons/tray-source.png");

/// Moves the window to the bottom-right corner of the monitor's work area
/// (the screen space excluding the taskbar).
fn position_bottom_right(window: &WebviewWindow) {
    let monitor = match window.current_monitor() {
        Ok(Some(monitor)) => monitor,
        _ => return,
    };

    let work_area = monitor.work_area();

    let window_size = match window.outer_size() {
        Ok(size) => size,
        Err(_) => return,
    };

    let x = work_area.position.x + work_area.size.width as i32 - window_size.width as i32;
    let y = work_area.position.y + work_area.size.height as i32 - window_size.height as i32;

    let _ = window.set_position(PhysicalPosition::new(x, y));
}

/// Toggles the main window: hides it if visible, otherwise shows it
/// at its current position and focused.
fn toggle_main_window(app: &tauri::AppHandle) {
    let window = match app.get_webview_window("main") {
        Some(window) => window,
        None => return,
    };

    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
    } else {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Shows the custom tray menu window next to the tray click position.
fn show_tray_menu(app: &tauri::AppHandle, click: PhysicalPosition<f64>) {
    let window = match app.get_webview_window("traymenu") {
        Some(window) => window,
        None => return,
    };

    let size = window.outer_size().unwrap_or(PhysicalSize::new(230, 200));

    let x = click.x as i32 - size.width as i32;
    let y = click.y as i32 - size.height as i32;

    let _ = window.set_position(PhysicalPosition::new(x, y));
    let _ = window.show();
    let _ = window.set_focus();
}

/// Routes tray icon clicks: left toggles the main window, right opens the menu.
fn handle_tray_event(tray: &tauri::tray::TrayIcon, event: TrayIconEvent) {
    if let TrayIconEvent::Click {
        button,
        button_state: MouseButtonState::Up,
        position,
        ..
    } = event
    {
        match button {
            MouseButton::Left => toggle_main_window(tray.app_handle()),
            MouseButton::Right => show_tray_menu(tray.app_handle(), position),
            _ => {}
        }
    }
}

/// Builds the tray icon; both menus are handled by the app's own windows.
fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let icon = Image::from_bytes(TRAY_LOGO_BYTES)?;

    TrayIconBuilder::new()
        .icon(icon)
        .on_tray_icon_event(handle_tray_event)
        .build(app)?;

    Ok(())
}

/// Toggles the main window from the tray menu window.
#[tauri::command]
fn toggle_main(app: tauri::AppHandle) {
    toggle_main_window(&app);
}

/// Exits the application.
#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .invoke_handler(tauri::generate_handler![
            scripts::get_scripts,
            scripts::get_config_path,
            scripts::open_config_file,
            scripts::add_entry,
            scripts::update_entry,
            scripts::set_script_autostart,
            scripts::add_script,
            scripts::remove_script,
            runner::start_script,
            runner::stop_script,
            runner::stop_all_scripts,
            runner::pause_script,
            runner::resume_script,
            runner::send_input,
            runner::get_running_scripts,
            monitor::get_script_states,
            autostart::get_autostart_errors,
            toggle_main,
            quit_app
        ])
        .setup(|app| {
            let loaded = scripts::load_scripts(app.handle());
            app.manage(scripts::ScriptRegistry(Mutex::new(loaded)));
            app.manage(runner::RunningScripts::default());
            app.manage(monitor::PortStates::default());
            app.manage(autostart::StartupIssues::default());

            if let Some(window) = app.get_webview_window("main") {
                position_bottom_right(&window);
            }
            setup_tray(app)?;
            monitor::start_worker(app.handle().clone());
            autostart::start_projects(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            match event {
                // Closing the main window hides it to the tray instead of
                // quitting; the app only exits via the tray menu's Quit.
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    let _ = window.hide();
                    api.prevent_close();
                }
                // The tray menu behaves like a popup: it hides on focus loss.
                tauri::WindowEvent::Focused(false) => {
                    if window.label() == "traymenu" {
                        let _ = window.hide();
                    }
                }
                _ => {}
            }
        })
        .run(tauri::generate_context!())
        .expect("脚本管理器运行失败");
}
