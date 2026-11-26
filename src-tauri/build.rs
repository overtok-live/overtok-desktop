fn main() {
    // The commands the dashboard may call, as permissions the capabilities grant by name.
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "app_info",
            "begin_sign_in",
            "open_external",
            "set_live",
            "update_state",
            "install_update",
            "get_autostart",
            "set_autostart",
            "obs_open",
            "obs_send",
            "obs_close",
            "obs_settings",
            "sound_window",
            "sound_window_on",
            "app_audio",
        ]),
    ))
    .expect("failed to run the Tauri build script");
}
