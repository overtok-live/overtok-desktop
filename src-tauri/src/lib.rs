//! Overtok for the desktop (ADR 0010): a window on the hosted dashboard, in the tray while it
//! runs, starting with the computer and updating itself. The dashboard calls the few commands
//! below; the capabilities let only its own origin do so.

mod channel;
mod obs;
mod signin;
mod sound;
mod tray;
mod updater;

use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, Manager, State, Url, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_opener::OpenerExt;

use channel::Channel;
use updater::{UpdateState, Updates};

/// The sign-in the app is waiting for: the `state` it sent the browser with.
#[derive(Default)]
struct PendingSignIn(Mutex<Option<String>>);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    version: String,
    channel: &'static str,
    app_url: String,
    os: &'static str,
    /// The chip the app was built for: `x86_64` or `aarch64`.
    arch: &'static str,
}

#[tauri::command]
fn app_info(app: AppHandle, channel: State<Channel>) -> AppInfo {
    AppInfo {
        version: app.package_info().version.to_string(),
        channel: channel.name,
        app_url: channel.app_url.clone(),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
    }
}

/// Opens the browser to sign in; the answer comes back as a deep link.
#[tauri::command]
fn begin_sign_in(
    app: AppHandle,
    channel: State<Channel>,
    pending: State<PendingSignIn>,
) -> Result<(), String> {
    use rand::Rng;
    let state: String = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(32)
        .map(char::from)
        .collect();
    let url = signin::connect_url(&channel.app_url, &state, channel.scheme);
    *pending.0.lock().unwrap() = Some(state);
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|err| err.to_string())
}

/// Opens an https page in the system browser.
#[tauri::command]
fn open_external(app: AppHandle, url: String) -> Result<(), String> {
    let parsed = Url::parse(&url).map_err(|err| err.to_string())?;
    if parsed.scheme() != "https" {
        return Err("only https".into());
    }
    app.opener()
        .open_url(parsed.as_str(), None::<&str>)
        .map_err(|err| err.to_string())
}

#[tauri::command]
fn set_live(app: AppHandle, live: bool) {
    updater::set_live(&app, live);
}

#[tauri::command]
fn update_state(updates: State<Updates>) -> UpdateState {
    updates.snapshot()
}

#[tauri::command]
fn install_update(app: AppHandle) -> Result<(), String> {
    updater::install(&app)
}

#[tauri::command]
fn get_autostart(app: AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    let launch = app.autolaunch();
    let result = if enabled { launch.enable() } else { launch.disable() };
    result.map_err(|err| err.to_string())
}

/// A deep link arrived (the browser signed in): the window signs in with its ticket.
fn handle_links(app: &AppHandle, links: &[Url]) {
    let channel = app.state::<Channel>();
    let expected = app.state::<PendingSignIn>().0.lock().unwrap().clone();
    for link in links {
        if let Some(ticket) = signin::parse_deep_link(link.as_str(), channel.scheme, expected.as_deref()) {
            *app.state::<PendingSignIn>().0.lock().unwrap() = None;
            if let (Some(window), Ok(url)) = (
                app.get_webview_window("main"),
                Url::parse(&signin::ticket_url(&channel.app_url, &ticket)),
            ) {
                let _ = window.navigate(url);
            }
            tray::show_window(app);
        }
    }
}

/// The first run turns starting with the computer on; the streamer can turn it off in Settings.
fn first_run(app: &AppHandle) {
    let Ok(dir) = app.path().app_config_dir() else { return };
    let marker = dir.join("first-run-done");
    if marker.exists() {
        return;
    }
    let _ = std::fs::create_dir_all(&dir);
    let _ = app.autolaunch().enable();
    let _ = std::fs::write(marker, b"1");
}

fn create_window(app: &AppHandle) -> tauri::Result<()> {
    let channel = app.state::<Channel>().inner().clone();
    let title = if channel.name == "dev" { "Overtok Dev" } else { "Overtok" };
    let opener = app.clone();
    WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title(title)
        .inner_size(1360.0, 880.0)
        .min_inner_size(900.0, 620.0)
        .center()
        .background_color(tauri::window::Color(13, 13, 12, 255))
        .on_navigation(move |url| {
            if channel.stays_in_window(url) {
                return true;
            }
            // Leaving the dashboard (a help page, checkout, Google): the system browser.
            if url.scheme() == "https" {
                let _ = opener.opener().open_url(url.as_str(), None::<&str>);
            }
            false
        })
        .build()?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // First: a second launch (or a deep link on Windows and Linux) goes to the running app.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_window(app);
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        .manage(PendingSignIn::default())
        .manage(Updates::default())
        .manage(obs::ObsSockets::default())
        .setup(|app| {
            let handle = app.handle().clone();
            let identifier = app.config().identifier.clone();
            app.manage(Channel::for_identifier(
                &identifier,
                std::env::var("OVERTOK_APP_URL").ok(),
            ));
            create_window(&handle)?;
            // Started with the computer: stay in the tray until the streamer opens it.
            if std::env::args().any(|arg| arg == "--hidden") {
                if let Some(window) = handle.get_webview_window("main") {
                    let _ = window.hide();
                }
            }
            tray::create(&handle)?;
            sound::restore(&handle);
            first_run(&handle);
            #[cfg(any(windows, target_os = "linux"))]
            {
                let _ = app.deep_link().register_all();
            }
            let links = handle.clone();
            app.deep_link().on_open_url(move |event| {
                handle_links(&links, &event.urls());
            });
            if let Ok(Some(urls)) = app.deep_link().get_current() {
                handle_links(&handle, &urls);
            }
            updater::start(&handle);
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window keeps Overtok in the tray.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            begin_sign_in,
            open_external,
            set_live,
            update_state,
            install_update,
            get_autostart,
            set_autostart,
            obs::obs_open,
            obs::obs_send,
            obs::obs_close,
            obs::obs_settings,
            sound::sound_window,
            sound::sound_window_on
        ])
        .build(tauri::generate_context!())
        .expect("error while building Overtok")
        .run(|app, event| {
            // macOS: a click on the Dock icon (or opening the app again) while the window is
            // hidden in the tray brings it back.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                tray::show_window(app);
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (app, event);
        });
}
