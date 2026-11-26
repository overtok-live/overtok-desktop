//! OBS through the app (ADR 0010). A page can't always reach obs-websocket: the Mac app's web view
//! (WebKit) blocks `ws://` from an https page, and browsers ask before a site talks to the
//! computer. So the dashboard's OBS connection goes through these commands: one plain `ws://`
//! socket per connection, speaking only obs-websocket's JSON protocol, its messages handed to the
//! window that opened it. The app also reads obs-websocket's own settings file, so setup can offer
//! the port and password instead of asking the streamer to copy them.

use serde::Serialize;
use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{mpsc, Mutex};
use std::time::Duration;
use tauri::{Emitter, Manager, State, Url, WebviewWindow};
use tungstenite::client::IntoClientRequest;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::protocol::CloseFrame;
use tungstenite::{Message, WebSocket};

/// The only subprotocol the app speaks for the dashboard: obs-websocket's JSON.
pub const OBS_PROTOCOL: &str = "obswebsocket.json";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a read waits before the loop looks for messages to send.
const POLL: Duration = Duration::from_millis(25);

enum Out {
    Text(String),
    Close,
}

#[derive(Default)]
pub struct ObsSockets {
    next: AtomicU32,
    open: Mutex<HashMap<u32, mpsc::Sender<Out>>>,
}

/// What reaches the page about a socket: a message, or its close.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
enum SocketEvent {
    Message { id: u32, data: String },
    Closed { id: u32, code: u16, reason: String },
}

/// An obs-websocket address the app will open: `ws://` with a host, nothing else.
pub fn check_address(address: &str) -> Result<Url, String> {
    let url = Url::parse(address).map_err(|_| "address".to_string())?;
    if url.scheme() != "ws" || url.host_str().is_none() {
        return Err("address".into());
    }
    Ok(url)
}

fn open_socket(url: &Url) -> Result<WebSocket<TcpStream>, String> {
    let host = url.host_str().unwrap_or_default();
    let port = url.port_or_known_default().unwrap_or(4455);
    let addr = (host, port)
        .to_socket_addrs()
        .map_err(|_| "unreachable".to_string())?
        .next()
        .ok_or("unreachable")?;
    let stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).map_err(|_| "unreachable")?;
    stream.set_read_timeout(Some(CONNECT_TIMEOUT)).ok();
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|_| "address".to_string())?;
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        OBS_PROTOCOL.parse().expect("a valid header"),
    );
    let (socket, _) = tungstenite::client(request, stream).map_err(|_| "unreachable")?;
    socket.get_ref().set_read_timeout(Some(POLL)).ok();
    Ok(socket)
}

/// Opens a socket to obs-websocket; its messages and close come as `obs-socket` events.
#[tauri::command]
pub async fn obs_open(
    window: WebviewWindow,
    sockets: State<'_, ObsSockets>,
    url: String,
    protocol: String,
) -> Result<u32, String> {
    if protocol != OBS_PROTOCOL {
        return Err("protocol".into());
    }
    let url = check_address(&url)?;
    let mut socket = tauri::async_runtime::spawn_blocking(move || open_socket(&url))
        .await
        .map_err(|_| "unreachable".to_string())??;
    let id = sockets.next.fetch_add(1, Ordering::Relaxed) + 1;
    let (tx, rx) = mpsc::channel::<Out>();
    sockets.open.lock().unwrap().insert(id, tx);
    let label = window.label().to_string();
    let app = window.app_handle().clone();
    std::thread::spawn(move || {
        let send = |event: SocketEvent| {
            let _ = app.emit_to(label.as_str(), "obs-socket", event);
        };
        let (code, reason) = loop {
            let mut closing = false;
            while let Ok(out) = rx.try_recv() {
                match out {
                    Out::Text(text) => {
                        let _ = socket.send(Message::text(text));
                    }
                    Out::Close => {
                        let _ = socket.close(Some(CloseFrame {
                            code: CloseCode::Normal,
                            reason: "".into(),
                        }));
                        closing = true;
                    }
                }
            }
            match socket.read() {
                Ok(Message::Text(text)) => send(SocketEvent::Message {
                    id,
                    data: text.to_string(),
                }),
                Ok(Message::Close(frame)) => {
                    let _ = socket.flush();
                    break frame.map_or((1005, String::new()), |f| {
                        (u16::from(f.code), f.reason.to_string())
                    });
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(err))
                    if matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
                {
                    if closing {
                        let _ = socket.flush();
                    }
                }
                Err(tungstenite::Error::ConnectionClosed) => break (1000, String::new()),
                Err(_) => break (1006, String::new()),
            }
        };
        send(SocketEvent::Closed { id, code, reason });
        if let Some(sockets) = app.try_state::<ObsSockets>() {
            sockets.open.lock().unwrap().remove(&id);
        }
    });
    Ok(id)
}

#[tauri::command]
pub fn obs_send(sockets: State<ObsSockets>, id: u32, data: String) -> Result<(), String> {
    let open = sockets.open.lock().unwrap();
    let tx = open.get(&id).ok_or("closed")?;
    tx.send(Out::Text(data)).map_err(|_| "closed".to_string())
}

#[tauri::command]
pub fn obs_close(sockets: State<ObsSockets>, id: u32) {
    if let Some(tx) = sockets.open.lock().unwrap().get(&id) {
        let _ = tx.send(Out::Close);
    }
}

/// obs-websocket's settings on this computer, as setup needs them.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObsSettings {
    /// "Enable WebSocket server" is ticked.
    pub enabled: bool,
    pub port: u16,
    /// "Enable Authentication" is ticked; then the password is what OBS shows.
    pub auth: bool,
    pub password: Option<String>,
}

/// obs-websocket 5.3+ (OBS 30 and newer): `plugin_config/obs-websocket/config.json`.
pub fn parse_json_settings(text: &str) -> Option<ObsSettings> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let port = value.get("server_port")?.as_u64()?;
    Some(ObsSettings {
        enabled: value.get("server_enabled")?.as_bool()?,
        port: u16::try_from(port).ok()?,
        auth: value.get("auth_required")?.as_bool()?,
        password: value
            .get("server_password")
            .and_then(|p| p.as_str())
            .filter(|p| !p.is_empty())
            .map(str::to_string),
    })
}

/// obs-websocket 5.0–5.2 (OBS 28, 29): the `[OBSWebSocket]` section of `global.ini`.
pub fn parse_ini_settings(text: &str) -> Option<ObsSettings> {
    let mut in_section = false;
    let mut values: HashMap<&str, &str> = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_section = line == "[OBSWebSocket]";
        } else if in_section {
            if let Some((key, value)) = line.split_once('=') {
                values.insert(key.trim(), value.trim());
            }
        }
    }
    let truthy = |key: &str| values.get(key).is_some_and(|v| *v == "true");
    Some(ObsSettings {
        enabled: truthy("ServerEnabled"),
        port: values.get("ServerPort")?.parse().ok()?,
        auth: truthy("AuthRequired"),
        password: values
            .get("ServerPassword")
            .filter(|p| !p.is_empty())
            .map(|p| p.to_string()),
    })
}

/// Where OBS keeps its settings on this computer (Linux: also the Flatpak's).
fn obs_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut dirs = Vec::new();
    if cfg!(target_os = "macos") {
        dirs.extend(home.map(|h| h.join("Library/Application Support/obs-studio")));
    } else if cfg!(windows) {
        dirs.extend(std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("obs-studio")));
    } else {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| home.clone().map(|h| h.join(".config")));
        dirs.extend(config.map(|c| c.join("obs-studio")));
        dirs.extend(home.map(|h| h.join(".var/app/com.obsproject.Studio/config/obs-studio")));
    }
    dirs
}

fn read_settings(dir: &Path) -> Option<ObsSettings> {
    let json = dir.join("plugin_config/obs-websocket/config.json");
    if let Some(found) = std::fs::read_to_string(json).ok().and_then(|t| parse_json_settings(&t)) {
        return Some(found);
    }
    ["user.ini", "global.ini"]
        .iter()
        .filter_map(|name| std::fs::read_to_string(dir.join(name)).ok())
        .find_map(|text| parse_ini_settings(&text))
}

/// obs-websocket's settings, if OBS is installed here; none otherwise.
#[tauri::command]
pub fn obs_settings() -> Option<ObsSettings> {
    obs_dirs().iter().find_map(|dir| read_settings(dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_only_plain_obs_addresses() {
        assert!(check_address("ws://127.0.0.1:4455").is_ok());
        assert!(check_address("ws://192.168.1.20:4456").is_ok());
        assert!(check_address("wss://example.com").is_err());
        assert!(check_address("https://127.0.0.1:4455").is_err());
        assert!(check_address("not an address").is_err());
    }

    #[test]
    fn reads_obs_30_settings() {
        let text = r#"{"alerts_enabled":false,"auth_required":true,"first_load":false,
            "server_enabled":true,"server_password":"s3cret","server_port":4455}"#;
        assert_eq!(
            parse_json_settings(text),
            Some(ObsSettings {
                enabled: true,
                port: 4455,
                auth: true,
                password: Some("s3cret".into())
            })
        );
        let off = r#"{"auth_required":false,"server_enabled":false,"server_password":"","server_port":4456}"#;
        assert_eq!(
            parse_json_settings(off),
            Some(ObsSettings { enabled: false, port: 4456, auth: false, password: None })
        );
        assert_eq!(parse_json_settings("{}"), None);
    }

    #[test]
    fn reads_obs_28_settings_from_its_section_only() {
        let text = "[General]\nServerPort=1\n\n[OBSWebSocket]\nFirstLoad=false\nServerEnabled=true\nServerPort=4455\nAlertsEnabled=false\nAuthRequired=true\nServerPassword=abc123\n";
        assert_eq!(
            parse_ini_settings(text),
            Some(ObsSettings {
                enabled: true,
                port: 4455,
                auth: true,
                password: Some("abc123".into())
            })
        );
        assert_eq!(parse_ini_settings("[General]\nServerPort=4455\n"), None);
    }
}
