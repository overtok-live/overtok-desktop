//! The sound window inside the app (ADR 0010): LIVE Studio can send one app's sound to a LIVE
//! (Audio mixer › Add audio source › Application), so the app plays the scene's sounds in a hidden
//! window of its own, and the streamer picks Overtok there: nothing else on the computer goes out.
//! The app plays sound without a click (web views here allow autoplay), keeps it on across
//! restarts and opens only the dashboard's own sound page.

use tauri::{AppHandle, Manager, State, Url, WebviewUrl, WebviewWindowBuilder};

use crate::channel::Channel;

pub const LABEL: &str = "sound";
const FILE: &str = "sound-page";

/// The sound page the app will play: the dashboard's `/o/<token>/sound`, marked as the app's.
pub fn sound_url(channel: &Channel, page: &str) -> Option<Url> {
    let url = Url::parse(page).ok()?;
    if url.origin().ascii_serialization() != channel.app_origin() {
        return None;
    }
    let parts: Vec<&str> = url.path().trim_matches('/').split('/').collect();
    let token_ok = |t: &str| !t.is_empty() && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if parts.len() != 3 || parts[0] != "o" || !token_ok(parts[1]) || parts[2] != "sound" {
        return None;
    }
    let mut clean = Url::parse(&channel.app_origin()).ok()?;
    clean.set_path(url.path());
    clean.set_query(Some("app=1"));
    Some(clean)
}

fn open(app: &AppHandle, url: Url) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(LABEL) {
        if window.url().ok().as_ref() != Some(&url) {
            window.navigate(url)?;
        }
        return Ok(());
    }
    let channel = app.state::<Channel>().inner().clone();
    WebviewWindowBuilder::new(app, LABEL, WebviewUrl::External(url))
        .title("Overtok sound")
        .inner_size(440.0, 720.0)
        .visible(false)
        .on_navigation(move |url| channel.app_origin() == url.origin().ascii_serialization())
        .build()?;
    Ok(())
}

fn saved(app: &AppHandle) -> Option<std::path::PathBuf> {
    app.path().app_config_dir().ok().map(|dir| dir.join(FILE))
}

/// Turns the sound window on (with the dashboard's sound page) or off (none); remembered.
#[tauri::command]
pub fn sound_window(app: AppHandle, channel: State<Channel>, page: Option<String>) -> Result<(), String> {
    let Some(page) = page else {
        if let Some(window) = app.get_webview_window(LABEL) {
            let _ = window.destroy();
        }
        if let Some(file) = saved(&app) {
            let _ = std::fs::remove_file(file);
        }
        return Ok(());
    };
    let url = sound_url(&channel, &page).ok_or("page")?;
    if let Some(file) = saved(&app) {
        let _ = file.parent().map(std::fs::create_dir_all);
        let _ = std::fs::write(file, url.as_str());
    }
    open(&app, url).map_err(|err| err.to_string())
}

/// Whether the sound window is on.
#[tauri::command]
pub fn sound_window_on(app: AppHandle) -> bool {
    app.get_webview_window(LABEL).is_some()
}

/// At start: the sound window comes back if it was on.
pub fn restore(app: &AppHandle) {
    let channel = app.state::<Channel>().inner().clone();
    let url = saved(app)
        .and_then(|file| std::fs::read_to_string(file).ok())
        .and_then(|page| sound_url(&channel, page.trim()));
    if let Some(url) = url {
        let _ = open(app, url);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prod() -> Channel {
        Channel::for_identifier("live.overtok.desktop", None)
    }

    #[test]
    fn plays_only_the_dashboards_sound_page() {
        let ok = sound_url(&prod(), "https://app.overtok.live/o/abc_D-1/sound?x=1").unwrap();
        assert_eq!(ok.as_str(), "https://app.overtok.live/o/abc_D-1/sound?app=1");
        assert!(sound_url(&prod(), "https://evil.example/o/abc/sound").is_none());
        assert!(sound_url(&prod(), "https://app.overtok.live/o/abc").is_none());
        assert!(sound_url(&prod(), "https://app.overtok.live/o/abc/scene").is_none());
        assert!(sound_url(&prod(), "https://app.overtok.live/o/a%2Fb/sound").is_none());
        assert!(sound_url(&prod(), "http://app.overtok.live/o/abc/sound").is_none());
    }
}
