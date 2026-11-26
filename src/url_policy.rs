use gtk::glib;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlAction {
    AllowInApp,
    RedirectInApp(String),
    OpenExternal(String),
    Block,
}

const IN_APP_GOOGLE_HOSTS: &[&str] = &[
    "calendar.google.com",
    "tasks.google.com",
    "keep.google.com",
    "ogs.google.com",
    "clients6.google.com",
    "apis.google.com",
    "content.googleapis.com",
    "accounts.google.com",
    "accounts.youtube.com",
    "myaccount.google.com",
    "gds.google.com",
    "consent.google.com",
];

const EXTERNAL_OPEN_SCHEMES: &[&str] = &["https", "http", "mailto"];

pub fn is_in_app_google_host(host: &str) -> bool {
    let lower = host.trim().to_ascii_lowercase();
    IN_APP_GOOGLE_HOSTS.contains(&lower.as_str())
}

pub fn is_trusted_google_data_domain(domain: &str) -> bool {
    let lower = domain.trim().trim_start_matches('.').to_ascii_lowercase();
    const TRUSTED_SUFFIXES: &[&str] = &[
        "google.com",
        "gstatic.com",
        "googleapis.com",
        "googleusercontent.com",
        "youtube.com",
    ];
    TRUSTED_SUFFIXES
        .iter()
        .any(|suffix| lower == *suffix || lower.ends_with(&format!(".{suffix}")))
}

pub fn is_restorable_workspace_uri(uri: &str) -> bool {
    let Ok(parsed) = glib::Uri::parse(uri, glib::UriFlags::NONE) else {
        return false;
    };
    let scheme = parsed.scheme();
    let Some(host) = parsed.host() else {
        return false;
    };
    scheme.eq_ignore_ascii_case("https")
        && (host.eq_ignore_ascii_case("calendar.google.com")
            || host.eq_ignore_ascii_case("tasks.google.com"))
}

pub fn unwrap_google_redirect(uri: &str) -> Option<String> {
    let parsed = glib::Uri::parse(uri, glib::UriFlags::NONE).ok()?;
    let scheme = parsed.scheme();
    if !scheme.eq_ignore_ascii_case("https") && !scheme.eq_ignore_ascii_case("http") {
        return None;
    }
    let host = parsed.host()?;
    if !host.eq_ignore_ascii_case("www.google.com") && !host.eq_ignore_ascii_case("google.com") {
        return None;
    }
    if parsed.path() != "/url" {
        return None;
    }
    let query = parsed.query()?;

    for pair in query.split('&') {
        let Some((raw_key, raw_val)) = pair.split_once('=') else {
            continue;
        };
        if raw_key == "q" || raw_key == "url" {
            let plus_decoded = raw_val.replace('+', " ");
            let decoded = glib::Uri::unescape_string(&plus_decoded, None)
                .map(|s| s.to_string())
                .unwrap_or(plus_decoded);
            if let Ok(target) = glib::Uri::parse(&decoded, glib::UriFlags::NONE) {
                let target_scheme = target.scheme();
                if target_scheme.eq_ignore_ascii_case("https")
                    || target_scheme.eq_ignore_ascii_case("http")
                {
                    return Some(decoded);
                }
            }
        }
    }
    None
}

fn classify_target(target_uri: &str, was_unwrapped: bool) -> UrlAction {
    let Ok(parsed) = glib::Uri::parse(target_uri, glib::UriFlags::NONE) else {
        return UrlAction::Block;
    };
    let scheme = parsed.scheme().to_ascii_lowercase();
    if !EXTERNAL_OPEN_SCHEMES.contains(&scheme.as_str()) {
        return UrlAction::Block;
    }

    if scheme == "https"
        && let Some(host) = parsed.host()
        && is_in_app_google_host(&host)
    {
        return if was_unwrapped {
            UrlAction::RedirectInApp(target_uri.to_string())
        } else {
            UrlAction::AllowInApp
        };
    }

    UrlAction::OpenExternal(target_uri.to_string())
}

pub fn classify_new_window(uri: &str) -> UrlAction {
    let (target, unwrapped) = match unwrap_google_redirect(uri) {
        Some(u) => (u, true),
        None => (uri.to_string(), false),
    };

    match classify_target(&target, unwrapped) {
        UrlAction::AllowInApp => UrlAction::RedirectInApp(target),
        other => other,
    }
}

pub fn classify_main_navigation(uri: &str, is_user_link_click: bool) -> UrlAction {
    if !is_user_link_click {
        return UrlAction::AllowInApp;
    }

    let (target, unwrapped) = match unwrap_google_redirect(uri) {
        Some(u) => (u, true),
        None => (uri.to_string(), false),
    };

    classify_target(&target, unwrapped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_host_spoofing_in_query_string() {
        let evil = "https://evil.com/?next=calendar.google.com";
        assert_eq!(
            classify_new_window(evil),
            UrlAction::OpenExternal(evil.to_string())
        );
        assert_eq!(
            classify_main_navigation(evil, true),
            UrlAction::OpenExternal(evil.to_string())
        );
    }

    #[test]
    fn unwraps_google_redirect_urls() {
        let wrapped =
            "https://www.google.com/url?q=https%3A%2F%2Fmeet.google.com%2Fabc-defg-hij&sa=D";
        assert_eq!(
            classify_new_window(wrapped),
            UrlAction::OpenExternal("https://meet.google.com/abc-defg-hij".to_string())
        );

        let wrapped_cal =
            "https://www.google.com/url?q=https%3A%2F%2Fcalendar.google.com%2Fcalendar%2Fu%2F0%2Fr%2Fweek";
        assert_eq!(
            classify_main_navigation(wrapped_cal, true),
            UrlAction::RedirectInApp(
                "https://calendar.google.com/calendar/u/0/r/week".to_string()
            )
        );
    }

    #[test]
    fn allows_workspace_and_auth_in_app_and_opens_external_google_apps_outside() {
        let week = "https://calendar.google.com/calendar/u/0/r/week";
        assert_eq!(classify_main_navigation(week, true), UrlAction::AllowInApp);
        assert_eq!(
            classify_new_window(week),
            UrlAction::RedirectInApp(week.to_string())
        );
        assert!(is_restorable_workspace_uri(week));

        let auth = "https://accounts.google.com/v3/signin/identifier";
        assert_eq!(classify_main_navigation(auth, true), UrlAction::AllowInApp);
        assert!(!is_restorable_workspace_uri(auth));

        let drive = "https://drive.google.com/file/d/123/view";
        assert_eq!(
            classify_main_navigation(drive, true),
            UrlAction::OpenExternal(drive.to_string())
        );
        assert_eq!(
            classify_new_window(drive),
            UrlAction::OpenExternal(drive.to_string())
        );
    }

    #[test]
    fn blocks_dangerous_schemes_and_allows_non_click_navigation() {
        assert_eq!(classify_new_window("javascript:alert(1)"), UrlAction::Block);
        assert_eq!(
            classify_main_navigation("file:///etc/passwd", true),
            UrlAction::Block
        );
        assert_eq!(
            classify_main_navigation("about:blank", false),
            UrlAction::AllowInApp
        );
        assert_eq!(
            classify_new_window("mailto:alex@example.com"),
            UrlAction::OpenExternal("mailto:alex@example.com".to_string())
        );
    }
}
