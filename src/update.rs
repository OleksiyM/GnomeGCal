//! Update check for GnomeGCal: asks the GitHub API for the latest stable
//! release and compares it with the running version. Uses `curl` so no HTTP/TLS
//! dependency is needed, with a 10-minute in-memory cache.

use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const LATEST_RELEASE_API: &str =
    "https://api.github.com/repos/OleksiyM/GnomeGCal/releases/latest";

pub const RELEASES_URL: &str = "https://github.com/OleksiyM/GnomeGCal/releases/latest";

const CACHE_TTL: Duration = Duration::from_secs(10 * 60);

static UPDATE_CACHE: Mutex<Option<(Instant, UpdateStatus)>> = Mutex::new(None);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    UpToDate,
    Available(String),
    Unknown,
}

/// Parses `X.Y.Z` or `vX.Y.Z` into a comparable tuple.
pub fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let mut parts = s.trim().trim_start_matches('v').split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

pub fn compare(latest_tag: &str, current: &str) -> UpdateStatus {
    match (parse_version(latest_tag), parse_version(current)) {
        (Some(latest), Some(cur)) if latest > cur => {
            UpdateStatus::Available(latest_tag.trim().to_string())
        }
        (Some(_), Some(_)) => UpdateStatus::UpToDate,
        _ => UpdateStatus::Unknown,
    }
}

fn fetch_latest_tag() -> Option<String> {
    let output = Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--location",
            "--max-time",
            "8",
            "--proto",
            "=https",
            "-H",
            "Accept: application/vnd.github+json",
            "-A",
            concat!("GnomeGCal/", env!("CARGO_PKG_VERSION")),
            LATEST_RELEASE_API,
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    json.get("tag_name")?.as_str().map(str::to_string)
}

/// Blocking network call; run it off the GTK main thread via `gio::spawn_blocking`.
pub fn check_for_update() -> UpdateStatus {
    if let Ok(guard) = UPDATE_CACHE.lock()
        && let Some((checked_at, status)) = guard.as_ref()
        && checked_at.elapsed() < CACHE_TTL
    {
        return status.clone();
    }

    let status = match fetch_latest_tag() {
        Some(tag) => compare(&tag, env!("CARGO_PKG_VERSION")),
        None => UpdateStatus::Unknown,
    };

    if status != UpdateStatus::Unknown
        && let Ok(mut guard) = UPDATE_CACHE.lock()
    {
        *guard = Some((Instant::now(), status.clone()));
    }

    status
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_semver_tags() {
        assert_eq!(parse_version("v3.0.0"), Some((3, 0, 0)));
        assert_eq!(parse_version(" 2.10.4 "), Some((2, 10, 4)));
        assert_eq!(parse_version("3.0"), None);
        assert_eq!(parse_version("v3.0.0.1"), None);
        assert_eq!(parse_version("v3.0.0-rc1"), None);
    }

    #[test]
    fn compares_versions_accurately() {
        assert_eq!(
            compare("v3.1.0", "3.0.0"),
            UpdateStatus::Available("v3.1.0".to_string())
        );
        assert_eq!(compare("v3.0.0", "3.0.0"), UpdateStatus::UpToDate);
        assert_eq!(compare("v2.9.9", "3.0.0"), UpdateStatus::UpToDate);
        assert_eq!(compare("invalid", "3.0.0"), UpdateStatus::Unknown);
    }
}
