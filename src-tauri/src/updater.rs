//! Updates: check on start and every 6 hours, download quietly, and install only when the
//! streamer isn't LIVE (unknown counts as LIVE). A `minimum` version in the manifest makes an
//! update required: it installs as soon as it may.

use serde::Serialize;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;

pub const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

/// Whether an update may install now: only when the dashboard said the streamer isn't LIVE.
pub fn may_install(live: Option<bool>) -> bool {
    live == Some(false)
}

/// Whether the running version is below the manifest's `minimum` (a broken release).
pub fn is_required(current: &str, minimum: Option<&str>) -> bool {
    let (Ok(current), Some(Ok(minimum))) = (
        semver::Version::parse(current),
        minimum.map(semver::Version::parse),
    ) else {
        return false;
    };
    current < minimum
}

#[derive(Clone, Serialize, Default, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateState {
    /// "idle", "downloading", "ready" or "error".
    pub status: &'static str,
    pub version: Option<String>,
    pub notes: Option<String>,
    pub required: bool,
}

#[derive(Default)]
pub struct Updates {
    pub state: Mutex<UpdateState>,
    pub live: Mutex<Option<bool>>,
    pending: Mutex<Option<(tauri_plugin_updater::Update, Vec<u8>)>>,
}

impl Updates {
    pub fn snapshot(&self) -> UpdateState {
        let state = self.state.lock().unwrap().clone();
        if state.status.is_empty() {
            UpdateState { status: "idle", ..state }
        } else {
            state
        }
    }
}

fn publish(app: &AppHandle, state: UpdateState) {
    *app.state::<Updates>().state.lock().unwrap() = state.clone();
    let _ = app.emit("update-state", state);
    crate::tray::refresh(app);
}

/// Checks for a new version and downloads it; installs at once only when required and allowed.
pub async fn check(app: AppHandle) {
    let Ok(updater) = app.updater() else { return };
    let update = match updater.check().await {
        Ok(Some(update)) => update,
        Ok(None) => return,
        Err(_) => return,
    };
    let current = app.package_info().version.to_string();
    let minimum = update
        .raw_json
        .get("minimum")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let required = is_required(&current, minimum.as_deref());
    publish(
        &app,
        UpdateState {
            status: "downloading",
            version: Some(update.version.clone()),
            notes: update.body.clone(),
            required,
        },
    );
    match update.download(|_, _| {}, || {}).await {
        Ok(bytes) => {
            *app.state::<Updates>().pending.lock().unwrap() = Some((update.clone(), bytes));
            publish(
                &app,
                UpdateState {
                    status: "ready",
                    version: Some(update.version.clone()),
                    notes: update.body.clone(),
                    required,
                },
            );
            let live = *app.state::<Updates>().live.lock().unwrap();
            if required && may_install(live) {
                let _ = install(&app);
            }
        }
        Err(_) => publish(
            &app,
            UpdateState { status: "error", version: Some(update.version), notes: None, required },
        ),
    }
}

/// Installs the downloaded update and restarts, unless the streamer is LIVE.
pub fn install(app: &AppHandle) -> Result<(), String> {
    let live = *app.state::<Updates>().live.lock().unwrap();
    if !may_install(live) {
        return Err("live".into());
    }
    let pending = app.state::<Updates>().pending.lock().unwrap().take();
    let Some((update, bytes)) = pending else {
        return Err("nothing to install".into());
    };
    update.install(bytes).map_err(|err| err.to_string())?;
    app.restart();
}

/// The dashboard says whether the streamer is LIVE; a required update waiting goes in right after.
pub fn set_live(app: &AppHandle, live: bool) {
    *app.state::<Updates>().live.lock().unwrap() = Some(live);
    let state = app.state::<Updates>().snapshot();
    let required_ready = state.status == "ready" && state.required;
    if !live && required_ready {
        let _ = install(app);
    }
}

pub fn start(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            check(app.clone()).await;
            tokio_sleep(CHECK_EVERY).await;
        }
    });
}

async fn tokio_sleep(duration: Duration) {
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        std::thread::sleep(duration);
        let _ = tx.send(());
    });
    let _ = tauri::async_runtime::spawn_blocking(move || rx.recv()).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_installs_while_live_or_unknown() {
        assert!(!may_install(None));
        assert!(!may_install(Some(true)));
        assert!(may_install(Some(false)));
    }

    #[test]
    fn requires_an_update_below_the_minimum() {
        assert!(is_required("0.1.0", Some("0.2.0")));
        assert!(!is_required("0.2.0", Some("0.2.0")));
        assert!(!is_required("0.3.0", Some("0.2.0")));
        assert!(!is_required("0.1.0", None));
        assert!(!is_required("0.1.0", Some("not a version")));
    }
}
