use gtk::glib;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const APP_ID: &str = "io.github.OleksiyM.GnomeGCal";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub general: GeneralConfig,
    pub performance: PerformanceConfig,
    pub window: WindowConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct GeneralConfig {
    pub autostart: bool,
    pub notifications_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PerformanceConfig {
    pub deep_sleep_enabled: bool,
    pub sleep_timeout_minutes: u32,
    pub memory_limit_mb: u32,
    pub cache_limit_mb: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct WindowConfig {
    pub width: i32,
    pub height: i32,
    pub zoom_level: f64,
    pub theme: String,
    pub user_agent: String,
}

pub fn default_user_agent() -> String {
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/156.0.0.0 Safari/537.36".to_string()
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            autostart: true,
            notifications_enabled: false,
        }
    }
}

impl Default for PerformanceConfig {
    fn default() -> Self {
        Self {
            deep_sleep_enabled: true,
            sleep_timeout_minutes: 5,
            memory_limit_mb: 256,
            cache_limit_mb: 150,
        }
    }
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: 900,
            height: 820,
            zoom_level: 1.0,
            theme: "system".to_string(),
            user_agent: default_user_agent(),
        }
    }
}

pub const DEFAULT_CONFIG_TEMPLATE: &str = r#"# ==============================================================================
#                      GnomeGCal Configuration File
# ==============================================================================
# Manual window resizes, zoom changes, and the last open view are saved
# automatically to ~/.local/share/gnome-gcal/state.toml without modifying this file.
#
# ------------------------------------------------------------------------------
#                         MEMORY & PERFORMANCE MODES
# ------------------------------------------------------------------------------
# * deep_sleep_enabled = false (Mode A: Fast & Ready)
#     Keeps WebView alive in the tray (~250 MB capped by memory_limit_mb), 0 ms wake.
# * deep_sleep_enabled = true  (Mode B: Deep Sleep, Default)
#     Destroys WebView after idle timeout (~15-50 MB native GTK/Network, 0 MB WebProcess),
#     then restores your last Calendar/Tasks URL on next click (~1 s).
# ==============================================================================

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
"#;

impl Config {
    pub fn load_or_create() -> Self {
        let config_dir = glib::user_config_dir().join("gnome-gcal");
        let config_path = config_dir.join("config.toml");

        if !config_path.exists() {
            fs::create_dir_all(&config_dir).ok();

            let legacy_path = glib::user_config_dir().join("gcal-agenda/config.toml");
            if legacy_path.exists() {
                if let Ok(content) = fs::read_to_string(&legacy_path) {
                    let _ = fs::write(&config_path, content);
                }
            } else {
                let _ = fs::write(&config_path, DEFAULT_CONFIG_TEMPLATE);
                return Self::default();
            }
        }

        match fs::read_to_string(&config_path) {
            Ok(content) => match toml::from_str::<Config>(&content) {
                Ok(cfg) => cfg,
                Err(err) => {
                    eprintln!("Failed to parse config.toml: {err}. Using defaults.");
                    Self::default()
                }
            },
            Err(_) => Self::default(),
        }
    }
}

fn autostart_desktop_path() -> PathBuf {
    glib::user_config_dir()
        .join("autostart")
        .join(format!("{APP_ID}.desktop"))
}

fn resolve_autostart_exec() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        let exe_str = exe.to_string_lossy();
        if !exe_str.contains("/target/") {
            return Some(exe);
        }
    }

    let installed = glib::home_dir().join("Applications/GnomeGCal/gnome-gcal");
    if installed.is_file() {
        Some(installed)
    } else {
        None
    }
}

fn render_autostart_desktop(exec_path: &Path) -> String {
    let exec_str = exec_path.to_string_lossy();
    let exec_cmd = if exec_str.contains(' ') {
        format!("\"{exec_str}\" --hidden")
    } else {
        format!("{exec_str} --hidden")
    };

    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=GnomeGCal\n\
         Comment=Lightweight Google Calendar, Tasks & Keep desktop client for GNOME\n\
         Exec={exec_cmd}\n\
         Icon={APP_ID}\n\
         Terminal=false\n\
         Categories=Office;Calendar;GTK;\n\
         StartupWMClass={APP_ID}\n\
         StartupNotify=false\n\
         X-GNOME-Autostart-enabled=true\n"
    )
}

/// Synchronizes `~/.config/autostart/io.github.OleksiyM.GnomeGCal.desktop` with `enabled`.
/// Skips writing when running directly from a Cargo `target/` binary and no installed binary exists.
pub fn sync_autostart(enabled: bool) {
    let path = autostart_desktop_path();
    let exec = resolve_autostart_exec();
    sync_autostart_at(&path, exec.as_deref(), enabled);
}

fn sync_autostart_at(path: &Path, exec_path: Option<&Path>, enabled: bool) {
    if !enabled {
        if path.exists() {
            let _ = fs::remove_file(path);
        }
        return;
    }

    let Some(exec) = exec_path else {
        return;
    };

    let desired = render_autostart_desktop(exec);
    if let Ok(existing) = fs::read_to_string(path)
        && existing == desired
    {
        return;
    }

    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, desired);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_template_parses_to_default_config() {
        let parsed: Config = toml::from_str(DEFAULT_CONFIG_TEMPLATE)
            .expect("DEFAULT_CONFIG_TEMPLATE must parse cleanly");
        assert_eq!(parsed, Config::default());
    }

    #[test]
    fn legacy_v2_config_parses_with_v3_defaults() {
        let legacy = r#"
[performance]
deep_sleep_enabled = false
sleep_timeout_minutes = 10
memory_limit_mb = 384

[window]
width = 1100
height = 820
zoom_level = 1.1
theme = "dark"
user_agent = "CustomAgent/1.0"
"#;
        let parsed: Config = toml::from_str(legacy).expect("legacy v2 config must parse");
        assert!(parsed.general.autostart);
        assert!(!parsed.general.notifications_enabled);
        assert!(!parsed.performance.deep_sleep_enabled);
        assert_eq!(parsed.performance.sleep_timeout_minutes, 10);
        assert_eq!(parsed.performance.memory_limit_mb, 384);
        assert_eq!(parsed.performance.cache_limit_mb, 150);
        assert_eq!(parsed.window.width, 1100);
        assert_eq!(parsed.window.theme, "dark");
        assert_eq!(parsed.window.user_agent, "CustomAgent/1.0");
    }

    #[test]
    fn sync_autostart_creates_and_removes_desktop_file() {
        let tmp = std::env::temp_dir().join(format!("gcal-autostart-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let desktop = tmp.join("autostart/io.github.OleksiyM.GnomeGCal.desktop");
        let exec = PathBuf::from("/home/user/Applications/GnomeGCal/gnome-gcal");

        sync_autostart_at(&desktop, Some(&exec), true);
        let content = fs::read_to_string(&desktop).expect("desktop file created");
        assert!(content.contains("Exec=/home/user/Applications/GnomeGCal/gnome-gcal --hidden"));
        assert!(content.contains("X-GNOME-Autostart-enabled=true"));

        sync_autostart_at(&desktop, Some(&exec), false);
        assert!(!desktop.exists());
        let _ = fs::remove_dir_all(&tmp);
    }
}
