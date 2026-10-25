mod config;
mod update;

use adw::prelude::*;
use adw::{Application, ApplicationWindow, HeaderBar, ToolbarView, Window as AdwWindow};
use config::Config;
use gtk::{
    glib, Align, Box as GtkBox, Button, Image, Label, LinkButton, Orientation, Separator, Stack,
    StackTransitionType, ToggleButton,
};
use ksni::blocking::TrayMethods;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::OnceLock;
use webkit6::prelude::*;
use webkit6::{
    MemoryPressureSettings, NetworkSession, Settings, UserContentInjectedFrames,
    UserContentManager, UserScript, UserScriptInjectionTime, UserStyleLevel, UserStyleSheet,
    WebContext, WebView,
};

const APP_ID: &str = "io.github.OleksiyM.GnomeGCal";
const AGENDA_URL: &str = "https://calendar.google.com/calendar/u/0/r/agenda";
const X_HOME_URL: &str = "https://x.com/home";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppTab {
    Calendar,
    X,
}

static TRAY_HANDLE: OnceLock<ksni::blocking::Handle<GCalTray>> = OnceLock::new();
static CONFIG: OnceLock<Config> = OnceLock::new();
static LAST_ACTIVATION_TOKEN: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

fn main() -> glib::ExitCode {
    let cfg = Config::load_or_create();
    CONFIG.set(cfg).ok();

    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(build_ui);
    app.run()
}

/// Applies the theme scripts and styles into WebKit's UserContentManager for Google Calendar
/// so Google Calendar's JS (`matchMedia`) always matches Libadwaita's exact state.
fn update_calendar_theme(content_manager: &UserContentManager, is_dark: bool) {
    content_manager.remove_all_style_sheets();
    content_manager.remove_all_scripts();

    let scheme = if is_dark { "dark" } else { "light" };
    let thumb_color = if is_dark {
        "rgba(255, 255, 255, 0.25)"
    } else {
        "rgba(0, 0, 0, 0.25)"
    };

    let custom_css = format!(
        r#"
        :root {{
            color-scheme: {scheme} !important;
        }}

        /* Make Google header neat, slim, and compact */
        header[role="banner"] {{
            height: 48px !important;
            min-height: 48px !important;
            padding: 0 8px !important;
            box-sizing: border-box !important;
        }}

        /* Hide Google Calendar logo and branding ("19 Calendar") */
        header a[href*="calendar"],
        header a[aria-label*="Календарь" i],
        header a[aria-label*="Calendar" i],
        header a[title*="Календарь" i],
        header a[title*="Calendar" i] {{
            display: none !important;
        }}

        /* Hide Google search bar */
        header div[role="search"],
        header form {{
            display: none !important;
        }}

        /* Hide Google apps menu, account switcher, and help buttons */
        header a[href*="accounts.google.com"],
        header a[aria-label*="Аккаунт" i],
        header a[aria-label*="Account" i],
        header div[aria-label*="Приложения Google" i],
        header div[aria-label*="Google apps" i],
        header button[aria-label*="Справка" i],
        header button[aria-label*="Support" i] {{
            display: none !important;
        }}

        /* Clean slim scrollbar */
        ::-webkit-scrollbar {{
            width: 5px;
            height: 5px;
        }}
        ::-webkit-scrollbar-thumb {{
            background: {thumb_color};
            border-radius: 3px;
        }}
    "#
    );

    let stylesheet = UserStyleSheet::new(
        &custom_css,
        UserContentInjectedFrames::AllFrames,
        UserStyleLevel::User,
        &[],
        &[],
    );
    content_manager.add_style_sheet(&stylesheet);

    let js_theme_override = format!(
        r#"
        (function() {{
            const isDark = {is_dark};
            const origMatchMedia = window.matchMedia.bind(window);
            window.matchMedia = function(query) {{
                if (typeof query === 'string' && query.includes('prefers-color-scheme')) {{
                    const wantsDark = query.includes('dark');
                    const wantsLight = query.includes('light');
                    const matched = (wantsDark && isDark) || (wantsLight && !isDark);
                    return {{
                        matches: matched,
                        media: query,
                        onchange: null,
                        addListener: function() {{}},
                        removeListener: function() {{}},
                        addEventListener: function() {{}},
                        removeEventListener: function() {{}},
                        dispatchEvent: function() {{ return false; }}
                    }};
                }}
                return origMatchMedia(query);
            }};
        }})();
        "#
    );

    let script = UserScript::new(
        &js_theme_override,
        UserContentInjectedFrames::AllFrames,
        UserScriptInjectionTime::Start,
        &[],
        &[],
    );
    content_manager.add_script(&script);
}

/// Applies custom styles and scripts for X (Twitter) feed:
/// - Syncs dark/light theme via `night_mode` cookie and `matchMedia`
/// - Hides bulky right promotional sidebar on compact window widths
/// - Auto-selects the "Following" tab on `/home` if enabled in config
fn update_x_theme(content_manager: &UserContentManager, is_dark: bool, auto_following: bool) {
    content_manager.remove_all_style_sheets();
    content_manager.remove_all_scripts();

    let scheme = if is_dark { "dark" } else { "light" };
    let thumb_color = if is_dark {
        "rgba(255, 255, 255, 0.25)"
    } else {
        "rgba(0, 0, 0, 0.25)"
    };

    let custom_css = format!(
        r#"
        :root {{
            color-scheme: {scheme} !important;
        }}

        /* Clean slim scrollbar */
        ::-webkit-scrollbar {{
            width: 5px;
            height: 5px;
        }}
        ::-webkit-scrollbar-thumb {{
            background: {thumb_color};
            border-radius: 3px;
        }}
    "#
    );

    let stylesheet = UserStyleSheet::new(
        &custom_css,
        UserContentInjectedFrames::AllFrames,
        UserStyleLevel::User,
        &[],
        &[],
    );
    content_manager.add_style_sheet(&stylesheet);

    let night_mode_val = if is_dark { "2" } else { "0" };
    let js_x_script = format!(
        r#"
        (function() {{
            const isDark = {is_dark};
            const autoFollowing = {auto_following};

            // 1. Sync X night_mode cookie (2 = Lights Out dark, 0 = Light)
            try {{
                document.cookie = "night_mode={night_mode_val}; domain=.x.com; path=/; max-age=31536000; SameSite=Lax";
                document.cookie = "night_mode={night_mode_val}; domain=.twitter.com; path=/; max-age=31536000; SameSite=Lax";
            }} catch (_) {{}}

            // 2. Override prefers-color-scheme
            const origMatchMedia = window.matchMedia.bind(window);
            window.matchMedia = function(query) {{
                if (typeof query === 'string' && query.includes('prefers-color-scheme')) {{
                    const wantsDark = query.includes('dark');
                    const wantsLight = query.includes('light');
                    const matched = (wantsDark && isDark) || (wantsLight && !isDark);
                    return {{
                        matches: matched,
                        media: query,
                        onchange: null,
                        addListener: function() {{}},
                        removeListener: function() {{}},
                        addEventListener: function() {{}},
                        removeEventListener: function() {{}},
                        dispatchEvent: function() {{ return false; }}
                    }};
                }}
                return origMatchMedia(query);
            }};

            // 3. Auto-switch to "Following" tab once on initial /home load
            if (autoFollowing) {{
                let switched = false;
                let attempts = 0;
                const trySwitchFollowing = () => {{
                    if (switched || attempts > 30) return;
                    attempts++;
                    if (!location.pathname.startsWith('/home')) return;
                    const tabs = document.querySelectorAll('[role="tablist"] [role="tab"]');
                    if (tabs.length >= 2) {{
                        const secondTab = tabs[1];
                        if (secondTab.getAttribute('aria-selected') !== 'true') {{
                            secondTab.click();
                        }}
                        switched = true;
                    }} else {{
                        setTimeout(trySwitchFollowing, 350);
                    }}
                }};
                window.addEventListener('DOMContentLoaded', () => setTimeout(trySwitchFollowing, 400));
            }}
        }})();
        "#
    );

    let script = UserScript::new(
        &js_x_script,
        UserContentInjectedFrames::AllFrames,
        UserScriptInjectionTime::Start,
        &[],
        &[],
    );
    content_manager.add_script(&script);
}

fn is_internal_x_uri(uri_str: &str) -> bool {
    uri_str.contains("x.com")
        || uri_str.contains("twitter.com")
        || uri_str.contains("accounts.google.com")
        || uri_str.contains("appleid.apple.com")
}

fn build_ui(app: &Application) {
    // If window already exists, just bring it to front (prevents duplicate instances)
    if let Some(win) = app.active_window().or_else(|| app.windows().first().cloned()) {
        win.present();
        return;
    }

    let cfg = CONFIG.get().cloned().unwrap_or_default();

    // Configure Libadwaita ColorScheme according to config ("system" / "dark" / "light")
    let style_manager = adw::StyleManager::default();
    match cfg.window.theme.trim().to_lowercase().as_str() {
        "dark" => style_manager.set_color_scheme(adw::ColorScheme::ForceDark),
        "light" => style_manager.set_color_scheme(adw::ColorScheme::ForceLight),
        _ => style_manager.set_color_scheme(adw::ColorScheme::Default), // Follows GNOME 50 Auto/Dark/Light
    }

    gtk::Window::set_default_icon_name(APP_ID);

    // 1. Storage paths for authentication and shared cookies (Google + X)
    let mut data_dir = glib::user_data_dir();
    data_dir.push("gnome-gcal/data");
    let mut cache_dir = glib::user_cache_dir();
    cache_dir.push("gnome-gcal/cache");

    std::fs::create_dir_all(&data_dir).ok();
    std::fs::create_dir_all(&cache_dir).ok();

    let data_str = data_dir.to_str().unwrap_or("/tmp/gnome-gcal-data");
    let cache_str = cache_dir.to_str().unwrap_or("/tmp/gnome-gcal-cache");

    let network_session = NetworkSession::new(Some(data_str), Some(cache_str));

    if let Some(cookie_manager) = network_session.cookie_manager() {
        let mut cookie_file = PathBuf::from(data_str);
        cookie_file.push("cookies.sqlite");
        if let Some(cookie_path) = cookie_file.to_str() {
            cookie_manager.set_persistent_storage(
                cookie_path,
                webkit6::CookiePersistentStorage::Sqlite,
            );
        }
    }

    // 2. Separate UserContentManagers so Calendar CSS doesn't affect X and vice versa
    let is_dark_initial = style_manager.is_dark();
    let auto_following = cfg.window.x_auto_following;

    let cal_cm = UserContentManager::new();
    update_calendar_theme(&cal_cm, is_dark_initial);

    let x_cm = UserContentManager::new();
    update_x_theme(&x_cm, is_dark_initial, auto_following);

    // 3. Apply real hardware memory pressure settings to WebKit processes
    let mut mem_settings = MemoryPressureSettings::new();
    mem_settings.set_memory_limit(cfg.performance.memory_limit_mb);
    mem_settings.set_strict_threshold(0.75);
    mem_settings.set_conservative_threshold(0.5);
    mem_settings.set_kill_threshold(0.0);
    NetworkSession::set_memory_pressure_settings(&mut mem_settings);

    let web_context = WebContext::builder()
        .memory_pressure_settings(&mem_settings)
        .build();

    // 4. Engine settings
    let settings = Settings::builder()
        .enable_developer_extras(cfg!(debug_assertions))
        .enable_javascript(true)
        .enable_back_forward_navigation_gestures(false)
        .enable_media_stream(false)
        .enable_webrtc(false)
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
        .build();

    // Helper to set WebView background color so there's no white flash when loading in dark mode
    let set_wv_bg = |wv: &WebView, is_dark: bool, is_x: bool| {
        let bg = if is_dark {
            if is_x {
                gtk::gdk::RGBA::new(0.0, 0.0, 0.0, 1.0) // #000000 Lights Out for X
            } else {
                gtk::gdk::RGBA::new(0.118, 0.118, 0.118, 1.0) // #1e1e1e for Calendar
            }
        } else {
            gtk::gdk::RGBA::new(1.0, 1.0, 1.0, 1.0) // #ffffff
        };
        wv.set_background_color(&bg);
    };

    // 5A. Create Calendar WebView (loaded immediately)
    let cal_webview = WebView::builder()
        .web_context(&web_context)
        .network_session(&network_session)
        .user_content_manager(&cal_cm)
        .settings(&settings)
        .vexpand(true)
        .hexpand(true)
        .build();

    set_wv_bg(&cal_webview, is_dark_initial, false);

    cal_webview.connect_permission_request(|_, request| {
        request.deny();
        true
    });

    cal_webview.connect_decide_policy(|webview, decision, decision_type| {
        if decision_type == webkit6::PolicyDecisionType::NewWindowAction {
            if let Some(nav_decision) = decision.downcast_ref::<webkit6::NavigationPolicyDecision>() {
                if let Some(nav_action) = nav_decision.navigation_action() {
                    if let Some(request) = nav_action.request() {
                        if let Some(uri) = request.uri() {
                            let uri_str = uri.as_str();
                            if uri_str.contains("calendar.google.com") || uri_str.contains("accounts.google.com") {
                                webview.load_uri(uri_str);
                            } else {
                                let _ = gtk::gio::AppInfo::launch_default_for_uri(
                                    uri_str,
                                    None::<&gtk::gio::AppLaunchContext>,
                                );
                            }
                        }
                    }
                }
            }
            decision.ignore();
            return true;
        }
        false
    });

    let enable_cal = cfg.window.enable_calendar;
    let enable_x = cfg.window.enable_x;
    let initial_tab = if enable_cal {
        AppTab::Calendar
    } else {
        AppTab::X
    };

    cal_webview.set_zoom_level(cfg.window.zoom_level);
    if initial_tab == AppTab::Calendar {
        cal_webview.load_uri(AGENDA_URL);
    }

    // 5B. Create X WebView (Lazy-loaded unless X is the only enabled tab)
    let x_webview = WebView::builder()
        .web_context(&web_context)
        .network_session(&network_session)
        .user_content_manager(&x_cm)
        .settings(&settings)
        .vexpand(true)
        .hexpand(true)
        .build();

    set_wv_bg(&x_webview, is_dark_initial, true);

    x_webview.connect_permission_request(|_, request| {
        request.deny();
        true
    });

    x_webview.connect_decide_policy(|webview, decision, decision_type| {
        if decision_type == webkit6::PolicyDecisionType::NewWindowAction {
            if let Some(nav_decision) = decision.downcast_ref::<webkit6::NavigationPolicyDecision>() {
                if let Some(nav_action) = nav_decision.navigation_action() {
                    if let Some(request) = nav_action.request() {
                        if let Some(uri) = request.uri() {
                            let uri_str = uri.as_str();
                            if is_internal_x_uri(uri_str) {
                                webview.load_uri(uri_str);
                            } else {
                                let _ = gtk::gio::AppInfo::launch_default_for_uri(
                                    uri_str,
                                    None::<&gtk::gio::AppLaunchContext>,
                                );
                            }
                        }
                    }
                }
            }
            decision.ignore();
            return true;
        }
        false
    });

    x_webview.set_zoom_level(cfg.window.zoom_level);
    if initial_tab == AppTab::X {
        x_webview.load_uri(X_HOME_URL);
    }

    // State flags for Lazy Loading and Zero-RAM Deep Sleep
    let active_tab = Rc::new(RefCell::new(initial_tab));
    let cal_loaded = Rc::new(RefCell::new(initial_tab == AppTab::Calendar));
    let cal_terminated = Rc::new(RefCell::new(false));
    let x_loaded = Rc::new(RefCell::new(initial_tab == AppTab::X));
    let x_terminated = Rc::new(RefCell::new(false));

    // Listen to GNOME 50 live Dark/Light toggle and sync both WebViews
    let cal_cm_theme = cal_cm.clone();
    let x_cm_theme = x_cm.clone();
    let cal_wv_theme = cal_webview.clone();
    let x_wv_theme = x_webview.clone();
    let cal_loaded_theme = cal_loaded.clone();
    let cal_term_theme = cal_terminated.clone();
    let x_loaded_theme = x_loaded.clone();
    let x_term_theme = x_terminated.clone();

    style_manager.connect_dark_notify(move |sm| {
        let is_dark = sm.is_dark();
        update_calendar_theme(&cal_cm_theme, is_dark);
        update_x_theme(&x_cm_theme, is_dark, auto_following);

        set_wv_bg(&cal_wv_theme, is_dark, false);
        set_wv_bg(&x_wv_theme, is_dark, true);

        if *cal_loaded_theme.borrow() && !*cal_term_theme.borrow() {
            cal_wv_theme.reload();
        }
        if *x_loaded_theme.borrow() && !*x_term_theme.borrow() {
            x_wv_theme.reload();
        }
    });

    // 6. Stack for Calendar & X views
    let stack = Stack::new();
    stack.set_transition_type(StackTransitionType::Crossfade);
    stack.set_transition_duration(150);
    stack.add_named(&cal_webview, Some("calendar"));
    stack.add_named(&x_webview, Some("x"));
    stack.set_visible_child_name(match initial_tab {
        AppTab::Calendar => "calendar",
        AppTab::X => "x",
    });

    // 7. Build native HeaderBar
    let header_bar = HeaderBar::new();

    let refresh_btn = Button::from_icon_name("view-refresh-symbolic");
    refresh_btn.set_tooltip_text(Some("Refresh current tab"));

    let active_for_refresh = active_tab.clone();
    let cal_wv_refresh = cal_webview.clone();
    let x_wv_refresh = x_webview.clone();
    let cal_loaded_refresh = cal_loaded.clone();
    let cal_term_refresh = cal_terminated.clone();
    let x_loaded_refresh = x_loaded.clone();
    let x_term_refresh = x_terminated.clone();

    refresh_btn.connect_clicked(move |_| match *active_for_refresh.borrow() {
        AppTab::Calendar => {
            if !*cal_loaded_refresh.borrow() || *cal_term_refresh.borrow() {
                *cal_loaded_refresh.borrow_mut() = true;
                *cal_term_refresh.borrow_mut() = false;
                cal_wv_refresh.load_uri(AGENDA_URL);
            } else {
                cal_wv_refresh.reload();
            }
        }
        AppTab::X => {
            if !*x_loaded_refresh.borrow() || *x_term_refresh.borrow() {
                *x_loaded_refresh.borrow_mut() = true;
                *x_term_refresh.borrow_mut() = false;
                x_wv_refresh.load_uri(X_HOME_URL);
            } else {
                x_wv_refresh.reload();
            }
        }
    });
    header_bar.pack_start(&refresh_btn);

    // Right side of HeaderBar: [ 📅 Calendar | 𝕏 ] -> [ About ] -> Window controls [ _ ][ □ ][ ✕ ]
    // Note: pack_end adds widgets from right to left (closest to window controls first).
    let about_btn = Button::from_icon_name("help-about-symbolic");
    about_btn.set_tooltip_text(Some("About GnomeGCal"));
    header_bar.pack_end(&about_btn);

    let switcher_box = GtkBox::new(Orientation::Horizontal, 0);
    switcher_box.add_css_class("linked");

    let cal_tooltip = if enable_cal {
        "Google Calendar, Tasks & Keep (Alt+1)"
    } else {
        "Google Calendar (disabled in config.toml)"
    };
    let x_tooltip = if enable_x {
        "𝕏 Feed (Alt+2)"
    } else {
        "𝕏 Feed (disabled in config.toml)"
    };

    let btn_cal = ToggleButton::builder()
        .label("📅 Calendar")
        .active(initial_tab == AppTab::Calendar)
        .sensitive(enable_cal)
        .tooltip_text(cal_tooltip)
        .build();

    let btn_x = ToggleButton::builder()
        .label("𝕏")
        .group(&btn_cal)
        .active(initial_tab == AppTab::X)
        .sensitive(enable_x)
        .tooltip_text(x_tooltip)
        .build();

    switcher_box.append(&btn_cal);
    switcher_box.append(&btn_x);
    header_bar.pack_end(&switcher_box);

    let toolbar_view = ToolbarView::new();
    toolbar_view.add_top_bar(&header_bar);
    toolbar_view.set_content(Some(&stack));

    // 8. Application window configuration
    let initial_title = match initial_tab {
        AppTab::Calendar => "GnomeGCal — Calendar",
        AppTab::X => "GnomeGCal — 𝕏",
    };
    let window = ApplicationWindow::builder()
        .application(app)
        .title(initial_title)
        .icon_name(APP_ID)
        .default_width(cfg.window.width)
        .default_height(cfg.window.height)
        .hide_on_close(true)
        .content(&toolbar_view)
        .build();

    // Central tab-switching helper
    let switch_tab: Rc<dyn Fn(AppTab)> = {
        let active_tab = active_tab.clone();
        let stack = stack.clone();
        let window = window.clone();
        let btn_cal = btn_cal.clone();
        let btn_x = btn_x.clone();
        let cal_wv = cal_webview.clone();
        let x_wv = x_webview.clone();
        let cal_loaded = cal_loaded.clone();
        let cal_term = cal_terminated.clone();
        let x_loaded = x_loaded.clone();
        let x_term = x_terminated.clone();

        Rc::new(move |target: AppTab| {
            if target == AppTab::Calendar && !enable_cal {
                return;
            }
            if target == AppTab::X && !enable_x {
                return;
            }

            *active_tab.borrow_mut() = target;

            match target {
                AppTab::Calendar => {
                    if !btn_cal.is_active() {
                        btn_cal.set_active(true);
                    }
                    stack.set_visible_child_name("calendar");
                    window.set_title(Some("GnomeGCal — Calendar"));

                    if !*cal_loaded.borrow() || *cal_term.borrow() {
                        eprintln!("[GnomeGCal] Loading/Restoring Calendar WebView...");
                        *cal_loaded.borrow_mut() = true;
                        *cal_term.borrow_mut() = false;
                        cal_wv.load_uri(AGENDA_URL);
                    }
                }
                AppTab::X => {
                    if !btn_x.is_active() {
                        btn_x.set_active(true);
                    }
                    stack.set_visible_child_name("x");
                    window.set_title(Some("GnomeGCal — 𝕏"));

                    if !*x_loaded.borrow() {
                        eprintln!("[GnomeGCal] First switch to 𝕏 tab: lazy-loading {X_HOME_URL}...");
                        *x_loaded.borrow_mut() = true;
                        *x_term.borrow_mut() = false;
                        x_wv.load_uri(X_HOME_URL);
                    } else if *x_term.borrow() {
                        eprintln!("[GnomeGCal] Restoring 𝕏 WebView from deep sleep...");
                        *x_term.borrow_mut() = false;
                        x_wv.load_uri(X_HOME_URL);
                    }
                }
            }

            if let Some(handle) = TRAY_HANDLE.get() {
                handle.update(move |tray| {
                    tray.active_tab = target;
                });
            }
        })
    };

    // Connect HeaderBar toggle buttons
    let switch_for_cal_btn = switch_tab.clone();
    btn_cal.connect_toggled(move |btn| {
        if btn.is_active() {
            switch_for_cal_btn(AppTab::Calendar);
        }
    });

    let switch_for_x_btn = switch_tab.clone();
    btn_x.connect_toggled(move |btn| {
        if btn.is_active() {
            switch_for_x_btn(AppTab::X);
        }
    });

    // Register GActions for keyboard shortcuts (Alt+1 / Alt+2) and tray menu triggers
    let act_cal = gtk::gio::SimpleAction::new("switch-cal", None);
    act_cal.set_enabled(enable_cal);
    let switch_for_act_cal = switch_tab.clone();
    let win_for_act_cal = window.clone();
    act_cal.connect_activate(move |_, _| {
        present_window_cleanly(win_for_act_cal.upcast_ref::<gtk::Window>());
        switch_for_act_cal(AppTab::Calendar);
    });
    app.add_action(&act_cal);
    if enable_cal {
        app.set_accels_for_action("app.switch-cal", &["<Alt>1"]);
    }

    let act_x = gtk::gio::SimpleAction::new("switch-x", None);
    act_x.set_enabled(enable_x);
    let switch_for_act_x = switch_tab.clone();
    let win_for_act_x = window.clone();
    act_x.connect_activate(move |_, _| {
        present_window_cleanly(win_for_act_x.upcast_ref::<gtk::Window>());
        switch_for_act_x(AppTab::X);
    });
    app.add_action(&act_x);
    if enable_x {
        app.set_accels_for_action("app.switch-x", &["<Alt>2"]);
    }

    // Zoom actions: Ctrl+Plus / Ctrl+Minus / Ctrl+0
    let default_zoom = cfg.window.zoom_level;
    let act_zoom_in = gtk::gio::SimpleAction::new("zoom-in", None);
    let active_z_in = active_tab.clone();
    let cal_z_in = cal_webview.clone();
    let x_z_in = x_webview.clone();
    act_zoom_in.connect_activate(move |_, _| {
        let wv = match *active_z_in.borrow() {
            AppTab::Calendar => &cal_z_in,
            AppTab::X => &x_z_in,
        };
        wv.set_zoom_level((wv.zoom_level() + 0.1).min(2.5));
    });
    app.add_action(&act_zoom_in);
    app.set_accels_for_action("app.zoom-in", &["<Primary>plus", "<Primary>equal", "<Primary>KP_Add"]);

    let act_zoom_out = gtk::gio::SimpleAction::new("zoom-out", None);
    let active_z_out = active_tab.clone();
    let cal_z_out = cal_webview.clone();
    let x_z_out = x_webview.clone();
    act_zoom_out.connect_activate(move |_, _| {
        let wv = match *active_z_out.borrow() {
            AppTab::Calendar => &cal_z_out,
            AppTab::X => &x_z_out,
        };
        wv.set_zoom_level((wv.zoom_level() - 0.1).max(0.5));
    });
    app.add_action(&act_zoom_out);
    app.set_accels_for_action("app.zoom-out", &["<Primary>minus", "<Primary>KP_Subtract"]);

    let act_zoom_reset = gtk::gio::SimpleAction::new("zoom-reset", None);
    let active_z_rst = active_tab.clone();
    let cal_z_rst = cal_webview.clone();
    let x_z_rst = x_webview.clone();
    act_zoom_reset.connect_activate(move |_, _| {
        let wv = match *active_z_rst.borrow() {
            AppTab::Calendar => &cal_z_rst,
            AppTab::X => &x_z_rst,
        };
        wv.set_zoom_level(default_zoom);
    });
    app.add_action(&act_zoom_reset);
    app.set_accels_for_action("app.zoom-reset", &["<Primary>0", "<Primary>KP_0"]);

    let win_for_about = window.clone();
    about_btn.connect_clicked(move |_| {
        show_about_window(Some(&win_for_about));
    });

    std::mem::forget(app.hold());

    // 9. Track window visibility and handle Zero-RAM Deep Sleep for BOTH WebViews
    let sleep_source_id: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
    let sleep_source_clone = sleep_source_id.clone();
    let active_for_sleep = active_tab.clone();
    let cal_wv_sleep = cal_webview.clone();
    let x_wv_sleep = x_webview.clone();
    let cal_loaded_sleep = cal_loaded.clone();
    let cal_term_sleep = cal_terminated.clone();
    let x_loaded_sleep = x_loaded.clone();
    let x_term_sleep = x_terminated.clone();
    let deep_sleep_enabled = cfg.performance.deep_sleep_enabled;
    let sleep_timeout_secs = cfg.performance.sleep_timeout_minutes * 60;

    window.connect_visible_notify(move |win| {
        let is_vis = win.is_visible();
        if let Some(handle) = TRAY_HANDLE.get() {
            handle.update(move |tray| {
                tray.is_visible = is_vis;
            });
        }

        if is_vis {
            eprintln!("[GnomeGCal] Window is visible. Restoring active tab...");
            if let Some(source_id) = sleep_source_clone.borrow_mut().take() {
                source_id.remove();
            }
            match *active_for_sleep.borrow() {
                AppTab::Calendar => {
                    if *cal_term_sleep.borrow() || !*cal_loaded_sleep.borrow() {
                        eprintln!("[GnomeGCal] Calendar WebProcess was terminated, reloading agenda URL...");
                        *cal_loaded_sleep.borrow_mut() = true;
                        *cal_term_sleep.borrow_mut() = false;
                        cal_wv_sleep.load_uri(AGENDA_URL);
                    }
                }
                AppTab::X => {
                    if *x_term_sleep.borrow() || !*x_loaded_sleep.borrow() {
                        eprintln!("[GnomeGCal] 𝕏 WebProcess was terminated, reloading 𝕏 Home...");
                        *x_loaded_sleep.borrow_mut() = true;
                        *x_term_sleep.borrow_mut() = false;
                        x_wv_sleep.load_uri(X_HOME_URL);
                    }
                }
            }
        } else if deep_sleep_enabled {
            eprintln!("[GnomeGCal] Window hidden to tray. Scheduling deep sleep in {sleep_timeout_secs}s...");
            if let Some(prev_id) = sleep_source_clone.borrow_mut().take() {
                prev_id.remove();
            }
            let cal_wv_target = cal_wv_sleep.clone();
            let x_wv_target = x_wv_sleep.clone();
            let cal_loaded_flag = cal_loaded_sleep.clone();
            let cal_term_flag = cal_term_sleep.clone();
            let x_loaded_flag = x_loaded_sleep.clone();
            let x_term_flag = x_term_sleep.clone();
            let sleep_ref = sleep_source_clone.clone();

            let source_id = glib::timeout_add_seconds_local(sleep_timeout_secs, move || {
                eprintln!("[GnomeGCal] Deep sleep timer expired! Terminating WebProcesses (0 MB RAM)...");
                if *cal_loaded_flag.borrow() {
                    cal_wv_target.terminate_web_process();
                    *cal_term_flag.borrow_mut() = true;
                }

                if *x_loaded_flag.borrow() {
                    x_wv_target.terminate_web_process();
                    *x_term_flag.borrow_mut() = true;
                }

                sleep_ref.borrow_mut().take();
                glib::ControlFlow::Break
            });
            *sleep_source_clone.borrow_mut() = Some(source_id);
        }
    });

    if let Ok(handle) = (GCalTray {
        is_visible: true,
        active_tab: initial_tab,
        enable_calendar: enable_cal,
        enable_x,
    })
    .spawn()
    {
        let _ = TRAY_HANDLE.set(handle);
    }

    let mut last_day = glib::DateTime::now_local().map(|dt| dt.day_of_month()).unwrap_or(0);

    glib::timeout_add_seconds_local(60, move || {
        if let Ok(now) = glib::DateTime::now_local() {
            let current_day = now.day_of_month();
            if current_day != last_day {
                last_day = current_day;
                if let Some(handle) = TRAY_HANDLE.get() {
                    handle.update(|_tray| {});
                }
            }
        }
        glib::ControlFlow::Continue
    });

    window.present();

    // Hook into GdkToplevel state so clicking the headerbar's Minimize button [ _ ]
    // cleanly hides the window to the tray instead of leaving it stuck minimized in the dock!
    if let Some(surface) = window.surface() {
        if let Ok(toplevel) = surface.downcast::<gtk::gdk::Toplevel>() {
            let win_weak = window.downgrade();
            toplevel.connect_state_notify(move |tl| {
                if tl.state().contains(gtk::gdk::ToplevelState::MINIMIZED) {
                    let win_weak = win_weak.clone();
                    glib::idle_add_local_once(move || {
                        if let Some(win) = win_weak.upgrade() {
                            win.unminimize();
                            win.set_visible(false);
                        }
                    });
                }
            });
        }
    }
}

struct GCalTray {
    is_visible: bool,
    active_tab: AppTab,
    enable_calendar: bool,
    enable_x: bool,
}

impl ksni::Tray for GCalTray {
    fn id(&self) -> String {
        "io.github.OleksiyM.GnomeGCal".into()
    }

    fn icon_name(&self) -> String {
        String::new()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![
            generate_tray_icon(24, self.active_tab),
            generate_tray_icon(48, self.active_tab),
        ]
    }

    fn title(&self) -> String {
        match self.active_tab {
            AppTab::Calendar => "GnomeGCal — Calendar".into(),
            AppTab::X => "GnomeGCal — 𝕏".into(),
        }
    }

    fn provide_xdg_activation_token(&mut self, token: String) {
        if let Ok(mut guard) = LAST_ACTIVATION_TOKEN.lock() {
            *guard = Some(token);
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        toggle_window();
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;

        let mut items: Vec<ksni::MenuItem<Self>> = Vec::new();

        if self.is_visible {
            items.push(
                StandardItem {
                    label: "Minimize to tray".into(),
                    activate: Box::new(|_| {
                        hide_window_to_tray();
                    }),
                    ..Default::default()
                }
                .into(),
            );
            items.push(MenuItem::Separator);
        }

        items.push(
            StandardItem {
                label: "Open Calendar".into(),
                enabled: self.enable_calendar,
                activate: Box::new(|_| {
                    open_tab_from_tray("switch-cal");
                }),
                ..Default::default()
            }
            .into(),
        );

        items.push(
            StandardItem {
                label: "Open 𝕏".into(),
                enabled: self.enable_x,
                activate: Box::new(|_| {
                    open_tab_from_tray("switch-x");
                }),
                ..Default::default()
            }
            .into(),
        );

        items.push(MenuItem::Separator);

        items.push(
            StandardItem {
                label: "About GnomeGCal".into(),
                activate: Box::new(|_| {
                    show_about();
                }),
                ..Default::default()
            }
            .into(),
        );

        items.push(MenuItem::Separator);

        items.push(
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|_| {
                    glib::MainContext::default().invoke(|| {
                        if let Some(app) = gtk::gio::Application::default() {
                            app.quit();
                        }
                    });
                }),
                ..Default::default()
            }
            .into(),
        );

        items
    }
}

/// Cleanly restores and focuses the window on GNOME Wayland using the XDG activation
/// token provided by GNOME Shell's AppIndicator extension over DBus.
fn present_window_cleanly(win: &gtk::Window) {
    let token = LAST_ACTIVATION_TOKEN.lock().ok().and_then(|mut g| g.take());
    if let Some(ref t) = token {
        win.set_startup_id(t);
    }
    win.unminimize();
    win.set_visible(true);
    if let Some(ref t) = token {
        if let Some(surface) = win.surface() {
            if let Ok(toplevel) = surface.downcast::<gtk::gdk::Toplevel>() {
                toplevel.set_startup_id(t);
            }
        }
    }
    win.present();
}

fn open_tab_from_tray(action_name: &'static str) {
    glib::MainContext::default().invoke(move || {
        if let Some(app) = gtk::gio::Application::default() {
            app.activate_action(action_name, None);
        }
    });
}

fn hide_window_to_tray() {
    glib::MainContext::default().invoke(|| {
        if let Some(app) = gtk::gio::Application::default() {
            if let Some(gtk_app) = app.downcast_ref::<gtk::Application>() {
                let windows = gtk_app.windows();
                if let Some(win) = windows.first() {
                    win.set_visible(false);
                }
            }
        }
    });
}

fn toggle_window() {
    glib::MainContext::default().invoke(|| {
        if let Some(app) = gtk::gio::Application::default() {
            if let Some(gtk_app) = app.downcast_ref::<gtk::Application>() {
                let windows = gtk_app.windows();
                if let Some(win) = windows.first() {
                    if win.is_visible() {
                        win.set_visible(false);
                    } else {
                        present_window_cleanly(win);
                    }
                }
            }
        }
    });
}

fn show_about() {
    glib::MainContext::default().invoke(|| {
        if let Some(app) = gtk::gio::Application::default() {
            if let Some(gtk_app) = app.downcast_ref::<gtk::Application>() {
                let windows = gtk_app.windows();
                if let Some(win) = windows.first() {
                    present_window_cleanly(win);
                    if let Some(adw_win) = win.downcast_ref::<ApplicationWindow>() {
                        show_about_window(Some(adw_win));
                    }
                }
            }
        }
    });
}

fn show_about_window(parent: Option<&ApplicationWindow>) {
    let window = AdwWindow::builder()
        .title("About GnomeGCal")
        .default_width(380)
        .resizable(false)
        .modal(true)
        .build();

    if let Some(p) = parent {
        window.set_transient_for(Some(p));
    }

    let main_box = GtkBox::new(Orientation::Vertical, 0);

    let header = adw::HeaderBar::builder()
        .show_end_title_buttons(true)
        .show_start_title_buttons(false)
        .build();
    main_box.append(&header);

    let content_box = GtkBox::new(Orientation::Vertical, 10);
    content_box.set_margin_top(12);
    content_box.set_margin_bottom(24);
    content_box.set_margin_start(24);
    content_box.set_margin_end(24);
    content_box.set_halign(Align::Center);
    content_box.set_valign(Align::Center);

    let icon = Image::builder()
        .icon_name("x-office-calendar")
        .pixel_size(64)
        .build();
    content_box.append(&icon);

    let title = Label::builder()
        .label("GnomeGCal & 𝕏")
        .css_classes(vec!["title-2"])
        .build();
    content_box.append(&title);

    let ver_label = Label::builder()
        .label(format!("v{}", env!("CARGO_PKG_VERSION")))
        .css_classes(vec!["dim-label"])
        .build();
    content_box.append(&ver_label);

    let update_label = Label::builder()
        .label("Checking for updates…")
        .css_classes(vec!["dim-label"])
        .build();
    content_box.append(&update_label);

    let update_link = LinkButton::with_label(crate::update::RELEASES_URL, "Download new version");
    update_link.set_visible(false);
    content_box.append(&update_link);

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(crate::update::check_for_update());
    });

    let update_label_weak = update_label.downgrade();
    let update_link_weak = update_link.downgrade();

    glib::timeout_add_local(
        std::time::Duration::from_millis(200),
        move || {
            match rx.try_recv() {
                Ok(status) => {
                    if let (Some(label), Some(link)) = (update_label_weak.upgrade(), update_link_weak.upgrade()) {
                        match status {
                            crate::update::UpdateStatus::UpToDate => {
                                label.set_label("You are up to date ✓");
                            }
                            crate::update::UpdateStatus::Available(tag) => {
                                label.set_label(&format!("Update available: {tag}"));
                                label.remove_css_class("dim-label");
                                link.set_visible(true);
                            }
                            crate::update::UpdateStatus::Unknown => {
                                label.set_label("Could not check for updates");
                            }
                        }
                    }
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            }
        },
    );

    let desc = Label::builder()
        .label("Lightweight Google Calendar, Tasks, Keep & 𝕏\ndesktop hub for GNOME")
        .justify(gtk::Justification::Center)
        .build();
    desc.set_margin_top(4);
    content_box.append(&desc);

    let disclaimer = Label::builder()
        .label("Personal independent open-source project,\nnot affiliated with Google LLC, X Corp., or GNOME Foundation.")
        .css_classes(vec!["caption", "dim-label"])
        .justify(gtk::Justification::Center)
        .build();
    content_box.append(&disclaimer);

    let sep = Separator::new(Orientation::Horizontal);
    sep.set_margin_top(8);
    sep.set_margin_bottom(6);
    content_box.append(&sep);

    let links_box = GtkBox::new(Orientation::Horizontal, 6);
    links_box.set_halign(Align::Center);

    let btn_gh = LinkButton::with_label("https://github.com/OleksiyM/GnomeGCal", "GitHub");
    let btn_rel = LinkButton::with_label(crate::update::RELEASES_URL, "Releases");
    let btn_x = LinkButton::with_label("https://x.com/OleksiyML", "X");

    let sep_label = || {
        Label::builder()
            .label("|")
            .css_classes(vec!["dim-label"])
            .build()
    };

    links_box.append(&btn_gh);
    links_box.append(&sep_label());
    links_box.append(&btn_rel);
    links_box.append(&sep_label());
    links_box.append(&btn_x);

    content_box.append(&links_box);

    main_box.append(&content_box);
    window.set_content(Some(&main_box));
    window.present();
}

fn generate_tray_icon(size: i32, active_tab: AppTab) -> ksni::Icon {
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, size, size)
        .expect("Failed to create Cairo surface");
    let cr = cairo::Context::new(&surface).expect("Failed to create Cairo context");

    cr.set_operator(cairo::Operator::Clear);
    cr.paint().ok();
    cr.set_operator(cairo::Operator::Over);

    let scale = size as f64 / 48.0;

    match active_tab {
        AppTab::Calendar => {
            let pat = cairo::LinearGradient::new(0.0, 0.0, 0.0, size as f64);
            pat.add_color_stop_rgb(0.0, 0.310, 0.627, 1.0);
            pat.add_color_stop_rgb(1.0, 0.192, 0.525, 1.0);
            cr.set_source(&pat).ok();

            let x = 1.5 * scale;
            let y = 1.5 * scale;
            let w = 45.0 * scale;
            let h = 45.0 * scale;
            let r = 9.5 * scale;
            let degrees = std::f64::consts::PI / 180.0;

            cr.new_sub_path();
            cr.arc(x + w - r, y + r, r, -90.0 * degrees, 0.0 * degrees);
            cr.arc(x + w - r, y + h - r, r, 0.0 * degrees, 90.0 * degrees);
            cr.arc(x + r, y + h - r, r, 90.0 * degrees, 180.0 * degrees);
            cr.arc(x + r, y + r, r, 180.0 * degrees, 270.0 * degrees);
            cr.close_path();
            cr.fill().ok();

            let day_str = glib::DateTime::now_local()
                .map(|dt| dt.day_of_month().to_string())
                .unwrap_or_else(|_| "19".to_string());

            cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
            cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
            cr.set_font_size(27.0 * scale);

            if let Ok(extents) = cr.text_extents(&day_str) {
                let text_x = (size as f64 - extents.width()) / 2.0 - extents.x_bearing();
                let text_y = (size as f64 - extents.height()) / 2.0 - extents.y_bearing();
                cr.move_to(text_x, text_y);
                cr.show_text(&day_str).ok();
            }
        }
        AppTab::X => {
            // Full-size crisp white vector 𝕏 glyph matching GNOME top bar system icons
            cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);

            // 1. Thick hollow diagonal (top-left to bottom-right, spanning 6..42 on 48px grid)
            cr.new_sub_path();
            cr.move_to(6.0 * scale, 6.0 * scale);
            cr.line_to(17.5 * scale, 6.0 * scale);
            cr.line_to(42.0 * scale, 42.0 * scale);
            cr.line_to(30.5 * scale, 42.0 * scale);
            cr.close_path();
            cr.fill().ok();

            // Cut inner transparent slit inside the thick diagonal for authentic 𝕏 look
            cr.set_operator(cairo::Operator::Clear);
            cr.new_sub_path();
            cr.move_to(10.6 * scale, 9.4 * scale);
            cr.line_to(15.2 * scale, 9.4 * scale);
            cr.line_to(37.4 * scale, 38.6 * scale);
            cr.line_to(32.8 * scale, 38.6 * scale);
            cr.close_path();
            cr.fill().ok();

            // 2. Solid diagonal (top-right to bottom-left)
            cr.set_operator(cairo::Operator::Over);
            cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
            cr.set_line_width(4.2 * scale);
            cr.set_line_cap(cairo::LineCap::Butt);
            cr.move_to(40.0 * scale, 6.0 * scale);
            cr.line_to(8.0 * scale, 42.0 * scale);
            cr.stroke().ok();
        }
    }

    drop(cr);

    let data = surface.take_data().expect("Failed to get surface data");
    let mut argb_data = data.to_vec();
    for pixel in argb_data.chunks_exact_mut(4) {
        let b = pixel[0];
        let g = pixel[1];
        let r = pixel[2];
        let a = pixel[3];
        pixel[0] = a;
        pixel[1] = r;
        pixel[2] = g;
        pixel[3] = b;
    }

    ksni::Icon {
        width: size,
        height: size,
        data: argb_data,
    }
}
