use gtk::glib;
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub performance: PerformanceConfig,
    pub window: WindowConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceConfig {
    pub deep_sleep_enabled: bool,
    pub sleep_timeout_minutes: u32,
    pub memory_limit_mb: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowConfig {
    pub width: i32,
    pub height: i32,
    pub zoom_level: f64,
    #[serde(default = "default_theme")]
    pub theme: String,
}

fn default_theme() -> String {
    "system".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            performance: PerformanceConfig {
                deep_sleep_enabled: true,
                sleep_timeout_minutes: 5,
                memory_limit_mb: 256,
            },
            window: WindowConfig {
                width: 900,
                height: 820,
                zoom_level: 0.90,
                theme: default_theme(),
            },
        }
    }
}

pub const DEFAULT_CONFIG_TEMPLATE: &str = r#"# ==============================================================================
#                      GnomeGCal Configuration File
# ==============================================================================
# Controls memory usage, sleep behavior, and window settings for GnomeGCal.
#
# ------------------------------------------------------------------------------
#                         MEMORY & PERFORMANCE MODES
# ------------------------------------------------------------------------------
# WebKitGTK runs Google Calendar as a heavy SPA (~500 MB RAM).
#
# +----------------------------------------------------------------------------+
# | MODE A: Fast & Ready                                                       |
# | -------------------------------------------------------------------------- |
# | * Setting:      deep_sleep_enabled = false                                 |
# | * In-tray RAM:  ~250-300 MB (capped via WebKit MemoryPressureSettings)     |
# | * Restore time: 0 ms (Instantaneous, page stays live in background)       |
# | * Best for:     Desktops & laptops with plenty of RAM, frequent checking   |
# +----------------------------------------------------------------------------+
# | MODE B: Deep Sleep (Default - Kill WebProcess when idle in tray)           |
# | -------------------------------------------------------------------------- |
# | * Setting:      deep_sleep_enabled = true                                  |
# | * In-tray RAM:  0 MB for WebProcess! (Only ~30 MB for native GTK UI process)|
# | * Restore time: ~1.0 - 1.5 s (WebKit process restarts on tray click)       |
# | * Best for:     Laptops on battery, clean RAM, long background idle        |
# +----------------------------------------------------------------------------+
# ==============================================================================

[performance]
# Mode Selection:
#   false = MODE A (Keep WebProcess alive, 0ms instant display)
#   true  = MODE B (Physically terminate WebProcess in tray, drops to 0 MB RAM)
deep_sleep_enabled = true

# How long window must stay hidden in tray before terminating WebProcess (minutes).
# Only applies when deep_sleep_enabled = true.
sleep_timeout_minutes = 5

# Memory pressure ceiling for WebKit processes in MB.
# WebKit aggressively evicts caches and frees pages to OS when reaching this limit.
memory_limit_mb = 256

[window]
# Default window size in pixels
width = 900
height = 820

# UI scaling inside WebKit (0.90 = 90% zoom for compact layout)
zoom_level = 0.90

# Color scheme: "system" (follows GNOME Dark/Light toggle), "dark", or "light"
# Note: In Google Calendar Settings (Gear icon -> Appearance), select "Device default".
theme = "system"
"#;

impl Config {
    pub fn load_or_create() -> Self {
        let config_dir = glib::user_config_dir().join("gnome-gcal");
        let config_path = config_dir.join("config.toml");

        if !config_path.exists() {
            fs::create_dir_all(&config_dir).ok();

            // Migrate legacy gcal-agenda config if present
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