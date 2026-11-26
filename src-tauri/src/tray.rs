//! The tray icon: Overtok keeps running there with its window closed. Its menu opens the window,
//! restarts into a downloaded update (not during a LIVE), and quits.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use crate::updater::Updates;

const TRAY_ID: &str = "overtok";

pub fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let version = app.package_info().version.to_string();
    let update = app.state::<Updates>().snapshot();
    let open = MenuItem::with_id(app, "open", "Open Overtok", true, None::<&str>)?;
    let restart = MenuItem::with_id(
        app,
        "update",
        match update.status {
            "ready" => format!(
                "Restart to update to {}",
                update.version.clone().unwrap_or_default()
            ),
            _ => "Up to date".to_string(),
        },
        update.status == "ready",
        None::<&str>,
    )?;
    let about = MenuItem::with_id(app, "version", format!("Overtok {version}"), false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Overtok", true, None::<&str>)?;
    Menu::with_items(
        app,
        &[&open, &restart, &PredefinedMenuItem::separator(app)?, &about, &quit],
    )
}

/// Redraws the menu (an update got ready).
pub fn refresh(app: &AppHandle) {
    if let (Some(tray), Ok(menu)) = (app.tray_by_id(TRAY_ID), menu(app)) {
        let _ = tray.set_menu(Some(menu));
    }
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Overtok")
        .menu(&menu(app)?)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_window(app),
            "update" => {
                if crate::updater::install(app).is_err() {
                    show_window(app);
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}
