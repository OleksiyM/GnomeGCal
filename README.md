# GnomeGCal

A lightweight, dedicated desktop hub for **Google Calendar**, **Google Tasks**, and **Google Keep** on **GNOME** (Wayland and X11).

Built with **Rust**, **GTK 4**, **Libadwaita**, and **WebKitGTK 6** — zero background runtimes, zero third-party D-Bus/async dependencies, and full control over RAM and disk usage.

---

## Highlights

* **All-in-One Google Workspace Hub**: Opens directly in **Agenda (Schedule)** view with full access to Day/Week/Month/Year views, **Google Tasks**, and the **Google Keep** side panel — untouched native Google UI with live GNOME Dark/Light theme sync (switches instantly without reloading the page).
* **Silent Login Autostart (`--hidden`)**: Automatically launches into the GNOME system tray at login (`autostart = true` by default). In `--hidden` mode, WebKit is **not initialized at all** until you first click the tray icon (**0 MB** WebKit RAM on startup).
* **Real Deep Sleep**: When hidden in the tray past `sleep_timeout_minutes`, GnomeGCal destroys the `WebView` and terminates `WebKitWebProcess`, then cleanly recreates it at your last open Calendar/Tasks URL on next click.
* **Native GLib Tray & Wayland Focus**: Pure `gio::DBusConnection` StatusNotifierItem + DBusMenu implementation with dynamic Cairo-rendered date badge (automatically flipped at midnight) and Wayland XDG Activation (`ProvideXdgActivationToken`) for instant, toast-free focus.
* **Automatic State & Cache Hygiene**: Remembers window dimensions, zoom level, and last Calendar/Tasks view in `~/.local/share/gnome-gcal/state.toml` without touching your `config.toml`. Automatically enforces the disk cache limit on cold start while preserving your Google login cookies.
* **Smart Link Security & Offline Recovery**: Strict `glib::Uri` host checks keep Calendar/Auth inside the app, unwrap `google.com/url?q=...` redirects, and launch external links (Drive, Meet, Docs, web) in your default browser. Shows a clean offline screen after laptop sleep and auto-loads as soon as Wi-Fi reconnects.

---

## CLI Options & Global Shortcut

```bash
gnome-gcal [OPTIONS]
```

* `--hidden` — Start silently in the system tray with zero WebKit memory usage until opened.
* `--toggle` — Show or hide the window (great for binding to a GNOME Custom Keyboard Shortcut such as `Super+C` with command `~/Applications/GnomeGCal/gnome-gcal --toggle`).
* `-v`, `--version` — Print version and exit.
* `-h`, `--help` — Show usage information.

---

## Memory & Performance Modes

| Mode | Setting | In-Tray WebProcess | Restore Time | Best For |
| :--- | :--- | :--- | :--- | :--- |
| **Mode A** (Fast & Ready) | `deep_sleep_enabled = false` | Alive (~250 MB cap) | **0 ms** (Instant) | Workstations with abundant RAM |
| **Mode B** (Deep Sleep, *Default*) | `deep_sleep_enabled = true` | **0 MB** (`WebView` destroyed) | ~1.0 s | Laptops, clean RAM, background idle |

### Configuration (`~/.config/gnome-gcal/config.toml`)

Created automatically on first launch (existing v2 configs remain compatible):

```toml
[general]
# Automatically start GnomeGCal silently in the tray on GNOME login (--hidden)
autostart = true

# Forward Google Calendar web notifications to GNOME desktop notifications
notifications_enabled = false

[performance]
# true = destroy idle WebView in tray (Mode B); false = keep alive (Mode A)
deep_sleep_enabled = true

# Idle minutes in tray before triggering deep sleep
sleep_timeout_minutes = 5

# Memory pressure ceiling for WebKit processes in MB
memory_limit_mb = 256

# Disk cache ceiling for ~/.cache/gnome-gcal/cache in MB (checked on cold start)
cache_limit_mb = 150

[window]
# Initial fallback window size (overridden automatically once resized)
width = 900
height = 820

# Initial fallback zoom level (overridden automatically by Ctrl++ / Ctrl+- / Ctrl+0)
zoom_level = 1.0

# Color scheme: "system" (follows GNOME Dark/Light toggle), "dark", or "light"
# Note: In Google Calendar Settings -> Appearance, select "Device default".
theme = "system"

# Browser User-Agent header for WebKit
user_agent = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/156.0.0.0 Safari/537.36"
```

---

## Install / Update

Installs per-user into `~/Applications/GnomeGCal` without `sudo`:

```bash
curl -fsSL https://raw.githubusercontent.com/OleksiyM/GnomeGCal/main/install.sh | bash
```

### Build from Source

Requirements: Rust toolchain, `gtk4`, `libadwaita`, `webkitgtk6.0`, `cairo`.

```bash
git clone https://github.com/OleksiyM/GnomeGCal.git
cd GnomeGCal
cargo build --release
./target/release/gnome-gcal
```

---

## Uninstall

Keep settings and Google login session (removes launcher & autostart entry):

```bash
bash ~/Applications/GnomeGCal/uninstall.sh
```

Purge all data (including cookies, cache, state, and configuration):

```bash
bash ~/Applications/GnomeGCal/uninstall.sh --purge
```

---

## Privacy & Security

* **Direct connection**: Traffic goes directly between local WebKitGTK and official `*.google.com` endpoints.
* **Local storage**:
  * Cookies & session: `~/.local/share/gnome-gcal/data/cookies.sqlite`
  * Window & view state: `~/.local/share/gnome-gcal/state.toml`
  * Disk cache: `~/.cache/gnome-gcal/cache` (auto-trimmed per `cache_limit_mb`)
  * Configuration: `~/.config/gnome-gcal/config.toml`
  * Autostart entry: `~/.config/autostart/io.github.OleksiyM.GnomeGCal.desktop`

---

## Disclaimer

Personal independent open-source project, **not** affiliated with, endorsed by, or sponsored by Google LLC or the GNOME Foundation.
