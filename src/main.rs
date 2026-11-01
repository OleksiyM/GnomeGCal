mod config;
mod update;

use adw::prelude::*;
use adw::{Application, ApplicationWindow, HeaderBar, ToolbarView, Window as AdwWindow};
use config::Config;
use gtk::{
    glib, Align, Box as GtkBox, Button, Image, Label, LinkButton, Orientation, Separator,
};
use ksni::blocking::TrayMethods;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::OnceLock;
use webkit6::prelude::*;
use webkit6::{
    HardwareAccelerationPolicy, MemoryPressureSettings, NetworkSession, Settings,
    UserContentInjectedFrames, UserContentManager, UserScript, UserScriptInjectionTime,
    UserStyleLevel, UserStyleSheet, WebContext, WebView,
};

const APP_ID: &str = "io.github.OleksiyM.GnomeGCal";
const AGENDA_URL: &str = "https://calendar.google.com/calendar/u/0/r/agenda";

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
        _ => style_manager.set_color_scheme(adw::ColorScheme::Default), // Follows GNOME Auto/Dark/Light
    }

    gtk::Window::set_default_icon_name(APP_ID);

    // 1. Storage paths for authentication and shared cookies
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

    // 2. UserContentManager for Google Calendar
    let is_dark_initial = style_manager.is_dark();
    let cal_cm = UserContentManager::new();
    update_calendar_theme(&cal_cm, is_dark_initial);

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
    let user_agent = if cfg.window.user_agent.trim().is_empty() {
        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/156.0.0.0 Safari/537.36".to_string()
    } else {
        cfg.window.user_agent.clone()
    };

    let settings = Settings::builder()
        .enable_developer_extras(cfg!(debug_assertions))
        .enable_javascript(true)
        .enable_back_forward_navigation_gestures(false)
        .enable_media_stream(false)
        .enable_webrtc(false)
        .enable_webaudio(false)
        .media_playback_requires_user_gesture(true)
        .hardware_acceleration_policy(HardwareAccelerationPolicy::Always)
        .user_agent(&user_agent)
        .build();

    // Helper to set WebView background color so there's no white flash when loading in dark mode
    let set_wv_bg = |wv: &WebView, is_dark: bool| {
        let bg = if is_dark {
            gtk::gdk::RGBA::new(0.118, 0.118, 0.118, 1.0) // #1e1e1e for Calendar
        } else {
            gtk::gdk::RGBA::new(1.0, 1.0, 1.0, 1.0) // #ffffff
        };
        wv.set_background_color(&bg);
    };

    // 5. Create Calendar WebView
    let cal_webview = WebView::builder()
        .web_context(&web_context)
        .network_session(&network_session)
        .user_content_manager(&cal_cm)
        .settings(&settings)
        .vexpand(true)
        .hexpand(true)
        .build();

    set_wv_bg(&cal_webview, is_dark_initial);

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

    cal_webview.set_zoom_level(cfg.window.zoom_level);
    cal_webview.load_uri(AGENDA_URL);

    // State flags for Zero-RAM Deep Sleep
    let cal_loaded = Rc::new(RefCell::new(true));
    let cal_terminated = Rc::new(RefCell::new(false));

    // Listen to GNOME live Dark/Light toggle and sync WebView
    let cal_cm_theme = cal_cm.clone();
    let cal_wv_theme = cal_webview.clone();
    let cal_loaded_theme = cal_loaded.clone();
    let cal_term_theme = cal_terminated.clone();

    style_manager.connect_dark_notify(move |sm| {
        let is_dark = sm.is_dark();
        update_calendar_theme(&cal_cm_theme, is_dark);
        set_wv_bg(&cal_wv_theme, is_dark);

        if *cal_loaded_theme.borrow() && !*cal_term_theme.borrow() {
            cal_wv_theme.reload();
        }
    });

    // 6. Build native HeaderBar
    let header_bar = HeaderBar::new();

    let refresh_btn = Button::from_icon_name("view-refresh-symbolic");
    refresh_btn.set_tooltip_text(Some("Refresh Calendar (Ctrl+R)"));

    let cal_wv_refresh = cal_webview.clone();
    let cal_loaded_refresh = cal_loaded.clone();
    let cal_term_refresh = cal_terminated.clone();

    refresh_btn.connect_clicked(move |_| {
        if !*cal_loaded_refresh.borrow() || *cal_term_refresh.borrow() {
            *cal_loaded_refresh.borrow_mut() = true;
            *cal_term_refresh.borrow_mut() = false;
            cal_wv_refresh.load_uri(AGENDA_URL);
        } else {
            cal_wv_refresh.reload();
        }
    });
    header_bar.pack_start(&refresh_btn);

    let about_btn = Button::from_icon_name("help-about-symbolic");
    about_btn.set_tooltip_text(Some("About GnomeGCal"));
    header_bar.pack_end(&about_btn);

    let toolbar_view = ToolbarView::new();
    toolbar_view.add_top_bar(&header_bar);
    toolbar_view.set_content(Some(&cal_webview));

    // 7. Application window configuration
    let window = ApplicationWindow::builder()
        .application(app)
        .title("GnomeGCal — Google Calendar")
        .icon_name(APP_ID)
        .default_width(cfg.window.width)
        .default_height(cfg.window.height)
        .hide_on_close(true)
        .content(&toolbar_view)
        .build();

    // Zoom actions: Ctrl+Plus / Ctrl+Minus / Ctrl+0
    let default_zoom = cfg.window.zoom_level;
    let act_zoom_in = gtk::gio::SimpleAction::new("zoom-in", None);
    let cal_z_in = cal_webview.clone();
    act_zoom_in.connect_activate(move |_, _| {
        cal_z_in.set_zoom_level((cal_z_in.zoom_level() + 0.1).min(2.5));
    });
    app.add_action(&act_zoom_in);
    app.set_accels_for_action("app.zoom-in", &["<Primary>plus", "<Primary>equal", "<Primary>KP_Add"]);

    let act_zoom_out = gtk::gio::SimpleAction::new("zoom-out", None);
    let cal_z_out = cal_webview.clone();
    act_zoom_out.connect_activate(move |_, _| {
        cal_z_out.set_zoom_level((cal_z_out.zoom_level() - 0.1).max(0.5));
    });
    app.add_action(&act_zoom_out);
    app.set_accels_for_action("app.zoom-out", &["<Primary>minus", "<Primary>KP_Subtract"]);

    let act_zoom_reset = gtk::gio::SimpleAction::new("zoom-reset", None);
    let cal_z_rst = cal_webview.clone();
    act_zoom_reset.connect_activate(move |_, _| {
        cal_z_rst.set_zoom_level(default_zoom);
    });
    app.add_action(&act_zoom_reset);
    app.set_accels_for_action("app.zoom-reset", &["<Primary>0", "<Primary>KP_0"]);

    // Reload action: Ctrl+R / F5
    let act_reload = gtk::gio::SimpleAction::new("reload", None);
    let cal_wv_reload = cal_webview.clone();
    act_reload.connect_activate(move |_, _| {
        cal_wv_reload.reload();
    });
    app.add_action(&act_reload);
    app.set_accels_for_action("app.reload", &["<Primary>r", "F5"]);

    let win_for_about = window.clone();
    about_btn.connect_clicked(move |_| {
        show_about_window(Some(&win_for_about));
    });

    std::mem::forget(app.hold());

    // 8. Track window visibility and handle Zero-RAM Deep Sleep
    let sleep_source_id: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
    let sleep_source_clone = sleep_source_id.clone();
    let cal_wv_sleep = cal_webview.clone();
    let cal_loaded_sleep = cal_loaded.clone();
    let cal_term_sleep = cal_terminated.clone();
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
            eprintln!("[GnomeGCal] Window is visible. Restoring Calendar...");
            if let Some(source_id) = sleep_source_clone.borrow_mut().take() {
                source_id.remove();
            }
            if *cal_term_sleep.borrow() || !*cal_loaded_sleep.borrow() {
                eprintln!("[GnomeGCal] Calendar WebProcess was terminated, reloading agenda URL...");
                *cal_loaded_sleep.borrow_mut() = true;
                *cal_term_sleep.borrow_mut() = false;
                cal_wv_sleep.load_uri(AGENDA_URL);
            }
        } else if deep_sleep_enabled {
            eprintln!("[GnomeGCal] Window hidden to tray. Scheduling deep sleep in {sleep_timeout_secs}s...");
            if let Some(prev_id) = sleep_source_clone.borrow_mut().take() {
                prev_id.remove();
            }
            let cal_wv_target = cal_wv_sleep.clone();
            let cal_loaded_flag = cal_loaded_sleep.clone();
            let cal_term_flag = cal_term_sleep.clone();
            let sleep_ref = sleep_source_clone.clone();

            let source_id = glib::timeout_add_seconds_local(sleep_timeout_secs, move || {
                eprintln!("[GnomeGCal] Deep sleep timer expired! Terminating WebProcess (0 MB RAM)...");
                if *cal_loaded_flag.borrow() {
                    cal_wv_target.terminate_web_process();
                    *cal_term_flag.borrow_mut() = true;
                }
                sleep_ref.borrow_mut().take();
                glib::ControlFlow::Break
            });
            *sleep_source_clone.borrow_mut() = Some(source_id);
        }
    });

    if let Ok(handle) = (GCalTray { is_visible: true }).spawn() {
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
}

impl ksni::Tray for GCalTray {
    fn id(&self) -> String {
        "io.github.OleksiyM.GnomeGCal".into()
    }

    fn icon_name(&self) -> String {
        String::new()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![generate_tray_icon(24), generate_tray_icon(48)]
    }

    fn title(&self) -> String {
        "GnomeGCal — Google Calendar".into()
    }

    fn provide_xdg_activation_token(&mut self, token: String) {
        if let Ok(mut guard) = LAST_ACTIVATION_TOKEN.lock() {
            *guard = Some(token);
        }
    }

    fn watcher_online(&self) {
        eprintln!("[GnomeGCal] StatusNotifierWatcher is back online. Restoring tray icon...");
    }

    fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool {
        eprintln!("[GnomeGCal] StatusNotifierWatcher offline ({reason:?}), waiting to reconnect...");
        true
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
                activate: Box::new(|_| {
                    glib::MainContext::default().invoke(|| {
                        if let Some(app) = gtk::gio::Application::default() {
                            if let Some(gtk_app) = app.downcast_ref::<gtk::Application>() {
                                let windows = gtk_app.windows();
                                if let Some(win) = windows.first() {
                                    present_window_cleanly(win);
                                }
                            }
                        }
                    });
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
        .label("GnomeGCal")
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
        .label("Lightweight Google Calendar, Tasks & Keep\ndesktop hub for GNOME")
        .justify(gtk::Justification::Center)
        .build();
    desc.set_margin_top(4);
    content_box.append(&desc);

    let disclaimer = Label::builder()
        .label("Personal independent open-source project,\nnot affiliated with Google LLC or GNOME Foundation.")
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

fn generate_tray_icon(size: i32) -> ksni::Icon {
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, size, size)
        .expect("Failed to create Cairo surface");
    let cr = cairo::Context::new(&surface).expect("Failed to create Cairo context");

    cr.set_operator(cairo::Operator::Clear);
    cr.paint().ok();
    cr.set_operator(cairo::Operator::Over);

    let scale = size as f64 / 48.0;

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
