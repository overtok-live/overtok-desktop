# Overtok for the desktop

The [Overtok](https://overtok.live) desktop app for Windows, macOS and Linux: a [Tauri](https://tauri.app) window on the Overtok dashboard that sits in the tray, starts with the computer, updates itself (never during a LIVE), connects to OBS and plays TikTok LIVE Studio's sounds as its own app.

Download it at **[overtok.live/download](https://overtok.live/download)**.

The installers aren't signed with a paid certificate, so Windows and macOS warn the first time you open the app; the Download page shows how to open it. Updates are signed with Overtok's own key and checked by the app before they install.

## Releases

Pushing a tag `desktop-v<version>` (matching `src-tauri/tauri.conf.json`) builds every system here and publishes the installers and update manifests to `downloads.overtok.live`. Running the workflow by hand builds Overtok Dev.

This repository is a copy of `apps/desktop` from Overtok's main (private) repository, where the app is developed.
