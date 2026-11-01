# Changelog

All notable changes to **GnomeGCal** will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.1.1] - 2026-11-01

### Fixed
- **System Tray Reconnection**: Implemented automatic retry loop in `ksni` StatusNotifierItem registration when GNOME Shell or `appindicatorsupport` reloads (e.g. during screen lock, system sleep, or extension restarts). Added `watcher_online` / `watcher_offline` lifecycle hooks in `GCalTray` to ensure the tray icon is never dropped or silently terminated.

### Changed
- **Automated Release Notes**: Integrated commit-based "What’s New" release notes generator and compare diff links into `.github/workflows/release.yml`.

## [1.1.0] - 2026-10-25

### Added
- **𝕏 (Twitter) Feed Tab (`https://x.com/home`)**: Integrated a second workspace tab with shared persistent SQLite cookie session (`cookies.sqlite`) and hardware memory pressure ceiling (`MemoryPressureSettings`).
- **Lazy Loading & Dual Zero-RAM Deep Sleep**: The `𝕏` WebView is only initialized when first opened (0 MB overhead if unused). When minimized to the tray, Deep Sleep terminates both Calendar and `𝕏` WebKit processes down to 0 MB RAM and restores only the active tab on wake.
- **HeaderBar Tab Switcher & Shortcuts**: Added a linked `[ 📅 Calendar | 𝕏 ]` switcher in the header bar with `Alt+1` / `Alt+2` tab shortcuts and `Ctrl++` / `Ctrl+-` / `Ctrl+0` zoom controls.
- **Dynamic Tray Icon & Expanded Tray Menu**: System tray icon dynamically switches between the Cairo-rendered date badge (Calendar) and a crisp vector `𝕏` glyph (`𝕏` tab), with direct `Open Calendar`, `Open 𝕏`, and `Minimize to tray` actions.
- **𝕏 Theme Sync & Auto-Following**: Automatically syncs GNOME Dark/Light mode with `x.com` (`night_mode` cookie + `prefers-color-scheme`) and optionally switches the home timeline to the **Following** tab (`x_auto_following = true`).
- **Configurable Tab Toggles**: Added `enable_calendar` and `enable_x` flags in `config.toml` to disable either tab (disables its header button and tray menu item, falling back to Calendar if both are set to `false`).

### Fixed
- **Wayland Focus & Window Activation from Tray**: Implemented `ProvideXdgActivationToken` in the vendored `ksni` StatusNotifierItem interface and wired it to GTK4's `set_startup_id`, eliminating GNOME Shell's `"GnomeGCal is ready"` notification and ensuring the window always focuses cleanly.
- **HeaderBar Minimize Button**: Clicking the window's minimize button (`[ _ ]`) now cleanly hides the window to the system tray instead of leaving it stuck minimized in the dock.

## [1.0.0] - 2026-10-20

### Added
- Initial stable release of **GnomeGCal**.
- Full Google Calendar, Google Tasks, and Google Keep workspace client inside a native GNOME Libadwaita window.
- System tray integration (AppIndicator / StatusNotifierItem) with dynamic Cairo-rendered calendar icon showing current day of month.
- Automatic midnight tray icon update.
- Two memory management modes:
  - **Mode A (Fast & Ready)**: WebKit hardware memory pressure ceiling (`MemoryPressureSettings`) capped to 256 MB.
  - **Mode B (Deep Sleep)**: Automatically terminates idle WebKit processes when hidden in system tray longer than `sleep_timeout_minutes` (drops RAM to 0 MB for WebProcess).
- Clean selective CSS injection: hides Google branding, apps launcher, and search bar in Agenda view while keeping burger menu, date navigation, and right companion side panel.
- Safe external link navigation: calendar and auth URLs load inside the widget; external links (GDrive, Gemini, etc.) open in the user's default browser.
- Native Adwaita dark mode theme consistency.
- Automatic session and cookie persistence via SQLite (`cookies.sqlite`).
- Native Libadwaita About dialog with update checking via GitHub Releases API.
- Zero-sudo `install.sh` and `uninstall.sh` with provenance verification.
