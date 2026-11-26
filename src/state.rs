use crate::config::Config;
use crate::url_policy::is_restorable_workspace_uri;
use gtk::glib;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const AGENDA_URL: &str = "https://calendar.google.com/calendar/u/0/r/agenda";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AppStateFile {
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub zoom_level: Option<f64>,
    pub last_uri: Option<String>,
    pub cache_migrated_v3: bool,
}

impl AppStateFile {
    pub fn state_path() -> PathBuf {
        glib::user_data_dir().join("gnome-gcal/state.toml")
    }

    pub fn load() -> Self {
        Self::load_from(&Self::state_path())
    }

    pub fn load_from(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(content) => toml::from_str::<Self>(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) {
        self.save_to(&Self::state_path());
    }

    pub fn save_to(&self, path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let Ok(serialized) = toml::to_string_pretty(self) else {
            return;
        };
        let tmp_path = path.with_extension("toml.tmp");
        if fs::write(&tmp_path, serialized).is_ok()
            && fs::rename(&tmp_path, path).is_err()
        {
            let _ = fs::remove_file(&tmp_path);
        }
    }

    pub fn effective_width(&self, cfg: &Config) -> i32 {
        self.width.unwrap_or(cfg.window.width).max(360)
    }

    pub fn effective_height(&self, cfg: &Config) -> i32 {
        self.height.unwrap_or(cfg.window.height).max(320)
    }

    pub fn effective_zoom(&self, cfg: &Config) -> f64 {
        self.zoom_level
            .unwrap_or(cfg.window.zoom_level)
            .clamp(0.5, 2.5)
    }

    pub fn effective_uri(&self) -> &str {
        match self.last_uri.as_deref() {
            Some(uri) if is_restorable_workspace_uri(uri) => uri,
            _ => AGENDA_URL,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_uri_filtering() {
        let tmp = std::env::temp_dir().join(format!("gcal-state-test-{}.toml", std::process::id()));
        let _ = fs::remove_file(&tmp);

        let state = AppStateFile {
            width: Some(1120),
            height: Some(860),
            zoom_level: Some(1.15),
            last_uri: Some("https://calendar.google.com/calendar/u/0/r/week".to_string()),
            cache_migrated_v3: true,
        };
        state.save_to(&tmp);

        let loaded = AppStateFile::load_from(&tmp);
        assert_eq!(loaded, state);
        assert_eq!(
            loaded.effective_uri(),
            "https://calendar.google.com/calendar/u/0/r/week"
        );

        let auth_state = AppStateFile {
            last_uri: Some("https://accounts.google.com/v3/signin/identifier".to_string()),
            ..Default::default()
        };
        assert_eq!(auth_state.effective_uri(), AGENDA_URL);

        let tasks_state = AppStateFile {
            last_uri: Some("https://tasks.google.com/embed/?origin=https://calendar.google.com".to_string()),
            ..Default::default()
        };
        assert_eq!(
            tasks_state.effective_uri(),
            "https://tasks.google.com/embed/?origin=https://calendar.google.com"
        );

        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn clamps_dimensions_and_zoom() {
        let cfg = Config::default();
        let state = AppStateFile {
            width: Some(100),
            height: Some(100),
            zoom_level: Some(9.0),
            ..Default::default()
        };
        assert_eq!(state.effective_width(&cfg), 360);
        assert_eq!(state.effective_height(&cfg), 320);
        assert_eq!(state.effective_zoom(&cfg), 2.5);
    }
}
