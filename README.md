# GnomeGCal

A lightweight, dedicated desktop client for **Google Calendar**, **Google Tasks**, and **Google Keep** on **GNOME** (Wayland and X11).

Built with **Rust**, **GTK 4**, **Libadwaita**, and **WebKitGTK 6**.

---

## Highlights

* **All-in-one Google Workspace widget**: Starts directly in compact **Agenda (Schedule)** view, but provides full access to Day, Week, Month, Year views, full **Google Tasks** boards, and the slide-out **Google Keep** side panel.
* **Minimalist & Native**: Google's bloated headers and search bars are hidden via custom CSS. Integrates seamlessly into GNOME's Adwaita dark theme.
* **Dynamic System Tray**: Renders today's date dynamically on the modern Google Calendar icon using Cairo. Automatically flips to the new day at midnight.
* **Real Zero-RAM Deep Sleep**: Solves WebKit's notorious memory greed. When idle in the system tray, it completely terminates the rendering process, freeing memory down to ~30 MB (0 MB for WebProcess).
* **Smart Link Handling**: Google Calendar & authentication pages stay inside the app; external links (Google Drive, Gemini, Docs) automatically open in your default browser.
* **Strictly Private**: Direct HTTPS connection to Google. Login session and cookies are stored purely on your local machine (`~/.local/share/gnome-gcal/data/cookies.sqlite`). No analytics, telemetry, or third-party servers.

---

## Why Not Just a Browser PWA?

*“Can’t I just click 'Install as PWA' in Chrome and call it a day?”*  
You could. But here is what actually happens in reality:

1. **Memory (500–800 MB vs 0 MB)**: A browser PWA spawns a fleet of Chromium processes permanently hoarding RAM. When minimized to the tray, GnomeGCal's Deep Sleep physically terminates the idle WebProcess down to **0 MB**.
2. **Dynamic Tray Widget**: PWAs cannot live as native tray indicators. GnomeGCal renders today’s date dynamically on Cairo and flips it automatically at midnight.
3. **UI Surgery**: PWAs display Google’s huge search bar and branding headers that eat 30% of compact windows. GnomeGCal cuts the clutter via surgical CSS.
4. **Smart Links**: Click a Google Drive or external link in a PWA, and it hijacks your compact calendar window. GnomeGCal loads Calendar/Auth locally and cleanly launches external links in your default desktop browser.
5. **Zero Browser Dependency**: A lightweight 2.4 MB Rust binary with an isolated SQLite session. Closing, crashing, or wiping your main browser never interrupts your calendar.

---

## Memory & Performance Modes

Google Calendar is an intensive Single Page Application (SPA) that easily consumes 500–700 MB of RAM. **GnomeGCal** provides two battle-tested operating modes:

| Mode | Setting | In-Tray RAM | Restore Time | Best For |
| :--- | :--- | :--- | :--- | :--- |
| **Mode A** (Fast & Ready) | `deep_sleep_enabled = false` | ~250 MB | **0 ms** (Instant) | Workstations with abundant RAM, frequent checking |
| **Mode B** (Deep Sleep, *Default*) | `deep_sleep_enabled = true` | **0 MB** (WebProcess killed) | ~1.0 s | Laptops on battery, clean RAM, background idle |

### Configuration (`~/.config/gnome-gcal/config.toml`)

The configuration file is automatically created on first launch:

```toml
[performance]
# true = Mode B (terminate WebProcess in tray, drops to 0 MB RAM)
# false = Mode A (keep WebProcess alive, instant 0ms restoration)
deep_sleep_enabled = true

# Idle minutes in tray before triggering deep sleep
sleep_timeout_minutes = 5

# Memory pressure ceiling for WebKit processes in MB
memory_limit_mb = 256

[window]
width = 900
height = 820
zoom_level = 0.90
theme = "system"  # "system" (follows GNOME Dark/Light toggle), "dark", or "light"
```

---

## Install / Update

### Automatic Installer (Recommended)

Installs safely into `~/Applications/GnomeGCal` without `sudo` or root permissions:

```bash
curl -fsSL https://raw.githubusercontent.com/OleksiyM/GnomeGCal/main/install.sh | bash
```

The installer downloads the pre-built release archive for your architecture (`x86_64` or `aarch64`), verifies SHA-256 checksums (and Sigstore provenance if GitHub CLI `gh` is installed), and registers the application launcher in GNOME.

### Manual Build from Source

Requirements: Rust toolchain, `gtk4`, `libadwaita`, `webkitgtk6.0`, `cairo`.

```bash
git clone https://github.com/OleksiyM/GnomeGCal.git
cd GnomeGCal
cargo build --release
./target/release/gnome-gcal
```

---

## Uninstall

To remove GnomeGCal while keeping your settings and login session:

```bash
bash ~/Applications/GnomeGCal/uninstall.sh
```

To purge all data (including saved Google session cookies and configuration):

```bash
bash ~/Applications/GnomeGCal/uninstall.sh --purge
```

---

## Privacy & Security

* **Direct connection**: Network traffic flows strictly between your local WebKitGTK engine and official `*.google.com` endpoints.
* **Local data storage**: Authentication tokens, cookies, and local database are persisted strictly in your user directory:
  * Cookies & session: `~/.local/share/gnome-gcal/data/cookies.sqlite`
  * Cache: `~/.cache/gnome-gcal/`
  * Configuration: `~/.config/gnome-gcal/config.toml`
* No data is sent to the author or any third party.

---

## Disclaimer

This is a personal, independent open-source project. It is **not** affiliated with, endorsed by, or sponsored by Google LLC or the GNOME Foundation. All Google trademarks, service marks, and logos belong to Google LLC.
