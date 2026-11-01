# GnomeGCal

A lightweight, dedicated desktop hub for **Google Calendar**, **Google Tasks**, and **Google Keep** on **GNOME** (Wayland and X11).

Built with **Rust**, **GTK 4**, **Libadwaita**, and **WebKitGTK 6**.

---

## Highlights

* **All-in-one Google Workspace Hub**: Starts directly in compact **Agenda (Schedule)** view with full access to Day/Week/Month/Year views, **Google Tasks**, and **Google Keep** companion side panel.
* **Minimalist & Distraction-Free**: Google's bloated headers and search bars are hidden via custom CSS, keeping navigation compact and clean. Syncs live with GNOME's Dark/Light theme.
* **Dynamic System Tray & Wayland-Native Activation**: Renders today's date dynamically on Cairo and updates automatically at midnight. Supports XDG Activation tokens (`ProvideXdgActivationToken`) for instant, toast-free window restoration on GNOME Wayland.
* **Real Zero-RAM Deep Sleep**: Solves WebKit's memory greed. When idle in the system tray, it physically terminates the WebKit rendering process, freeing memory down to **0 MB** for WebProcess.
* **Smart Link Handling**: Calendar and authentication pages stay inside the app; external links (Google Drive, Gemini, docs, web) automatically open in your default desktop browser.
* **Configurable User-Agent**: User-Agent can be customized in `config.toml` (defaults to modern Chrome for Linux) to ensure seamless compatibility with Google services without outdated browser warnings.
* **Strictly Private**: Direct HTTPS connection to Google. Login sessions and cookies are stored purely on your local machine (`~/.local/share/gnome-gcal/data/cookies.sqlite`). No analytics, telemetry, or third-party servers.

---

## Why Not Just Use a Google Calendar PWA?

You could. GnomeGCal exists because we wanted the full Google Calendar experience to behave like a real GNOME desktop application — not just a browser tab with its own window.

Here is what that difference actually means in practice:

1. **Memory (500–800 MB vs 0 MB)**: A browser PWA spawns a fleet of Chromium processes permanently hoarding RAM. When minimized to the tray, GnomeGCal's Deep Sleep physically terminates the idle WebProcess down to **0 MB**.
2. **Dynamic Tray Widget**: PWAs cannot live as native tray indicators. GnomeGCal renders today’s date dynamically on Cairo and flips it automatically at midnight.
3. **UI Surgery**: PWAs display Google’s huge search bar and branding headers that eat 30% of compact windows. GnomeGCal cuts the clutter via surgical CSS.
4. **Smart Links**: Click a Google Drive or external link in a PWA, and it hijacks your compact calendar window. GnomeGCal loads Calendar/Auth locally and cleanly launches external links in your default desktop browser.
5. **Zero Browser Dependency**: A lightweight isolated Rust binary with its own SQLite session. Closing, crashing, or wiping your main browser never interrupts your calendar.

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
width = 1020
height = 820
zoom_level = 1.0
theme = "system"          # "system" (follows GNOME Dark/Light toggle), "dark", or "light"
user_agent = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/156.0.0.0 Safari/537.36"
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
