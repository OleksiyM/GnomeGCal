# Changelog

All notable changes to **GnomeGCal** will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
