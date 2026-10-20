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
    MemoryPressureSettings, NetworkSession, Settings, UserContentInjectedFrames,
    UserContentManager, UserScript, UserScriptInjectionTime, UserStyleLevel, UserStyleSheet,
    WebContext, WebView,
};

const APP_ID: &str = "io.github.OleksiyM.GnomeGCal";
const AGENDA_URL: &str = "https://calendar.google.com/calendar/u/0/r/agenda";

static TRAY_HANDLE: OnceLock<ksni::blocking::Handle<GCalTray>> = OnceLock::new();
static CONFIG: OnceLock<Config> = OnceLock::new();

fn main() -> glib::ExitCode {
    let cfg = Config::load_or_create();
    CONFIG.set(cfg).ok();

    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(build_ui);
    app.run()
}

/// Applies the theme scripts and styles into WebKit's UserContentManager
/// so Google Calendar's JS (`matchMedia`) always matches Libadwaita's exact state.
fn update_webview_theme(content_manager: &UserContentManager, is_dark: bool) {
    content_manager.remove_all_style_sheets();
    content_manager.remove_all_scripts();

    // Sync GTK settings property so WebKitGTK internal rendering matches Libadwaita
    // if let Some(gtk_settings) = gtk::Settings::default() {
    //     gtk_settings.set_gtk_application_prefer_dark_theme(is_dark);
    // }

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

    // Override matchMedia before Google Calendar scripts run so "Device default"
    // in Google Calendar settings ALWAYS sees the exact same theme as Libadwaita
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
        _ => style_manager.set_color_scheme(adw::ColorScheme::Default), // Follows GNOME 50 Auto/Dark/Light
    }

    gtk::Window::set_default_icon_name(APP_ID);

    // 1. Storage paths for Google authentication and cookies
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

    // 2. Custom CSS & JS theme injection
    let content_manager = UserContentManager::new();
    update_webview_theme(&content_manager, style_manager.is_dark());

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

    // 5. Create WebView instance
    let webview = WebView::builder()
        .web_context(&web_context)
        .network_session(&network_session)
        .user_content_manager(&content_manager)
        .settings(&settings)
        .vexpand(true)
        .hexpand(true)
        .build();

    // Set WebView background color so there's no white flash when loading in dark mode
    let set_wv_bg = |wv: &WebView, is_dark: bool| {
        let bg = if is_dark {
            gtk::gdk::RGBA::new(0.118, 0.118, 0.118, 1.0) // #1e1e1e
        } else {
            gtk::gdk::RGBA::new(1.0, 1.0, 1.0, 1.0)       // #ffffff
        };
        wv.set_background_color(&bg);
    };
    set_wv_bg(&webview, style_manager.is_dark());

    webview.connect_permission_request(|_, request| {
        request.deny();
        true
    });

    webview.connect_decide_policy(|webview, decision, decision_type| {
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

    webview.set_zoom_level(cfg.window.zoom_level);
    webview.load_uri(AGENDA_URL);

    // Flag tracking if WebProcess was killed while idle in tray
    let is_terminated = Rc::new(RefCell::new(false));

    // Listen to GNOME 50 live Dark/Light toggle and sync WebView immediately
    let cm_for_theme = content_manager.clone();
    let wv_for_theme = webview.clone();
    let term_for_theme = is_terminated.clone();
    style_manager.connect_dark_notify(move |sm| {
        let is_dark = sm.is_dark();
        update_webview_theme(&cm_for_theme, is_dark);
        set_wv_bg(&wv_for_theme, is_dark);

        // Reload only if WebProcess is currently alive
        if !*term_for_theme.borrow() {
            wv_for_theme.reload();
        }
    });

    // 6. Build native HeaderBar
    let header_bar = HeaderBar::new();

    let refresh_btn = Button::from_icon_name("view-refresh-symbolic");
    refresh_btn.set_tooltip_text(Some("Refresh agenda"));
    let wv_clone = webview.clone();
    let term_for_refresh = is_terminated.clone();
    refresh_btn.connect_clicked(move |_| {
        if *term_for_refresh.borrow() {
            *term_for_refresh.borrow_mut() = false;
            wv_clone.load_uri(AGENDA_URL);
        } else {
            wv_clone.reload();
        }
    });
    header_bar.pack_start(&refresh_btn);

    let about_btn = Button::from_icon_name("help-about-symbolic");
    about_btn.set_tooltip_text(Some("About GnomeGCal"));
    header_bar.pack_end(&about_btn);

    let toolbar_view = ToolbarView::new();
    toolbar_view.add_top_bar(&header_bar);
    toolbar_view.set_content(Some(&webview));

    // 7. Application window configuration
    let window = ApplicationWindow::builder()
        .application(app)
        .title("GnomeGCal")
        .icon_name(APP_ID)
        .default_width(cfg.window.width)
        .default_height(cfg.window.height)
        .hide_on_close(true)
        .content(&toolbar_view)
        .build();

    let win_for_about = window.clone();
    about_btn.connect_clicked(move |_| {
        show_about_window(Some(&win_for_about));
    });

    std::mem::forget(app.hold());

    // 8. Track window visibility and handle Deep Sleep
    let sleep_source_id: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
    let sleep_source_clone = sleep_source_id.clone();
    let wv_for_sleep = webview.clone();
    let is_terminated_clone = is_terminated.clone();
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
            eprintln!("[GnomeGCal] Window is visible. Restoring...");
            if let Some(source_id) = sleep_source_clone.borrow_mut().take() {
                source_id.remove();
            }
            if *is_terminated_clone.borrow() {
                eprintln!("[GnomeGCal] WebProcess was terminated, reloading agenda URL...");
                *is_terminated_clone.borrow_mut() = false;
                wv_for_sleep.load_uri(AGENDA_URL);
            }
        } else if deep_sleep_enabled {
            eprintln!("[GnomeGCal] Window hidden to tray. Scheduling deep sleep in {sleep_timeout_secs}s...");
            if let Some(prev_id) = sleep_source_clone.borrow_mut().take() {
                prev_id.remove();
            }
            let wv_target = wv_for_sleep.clone();
            let sleep_ref = sleep_source_clone.clone();
            let term_flag = is_terminated_clone.clone();

            let source_id = glib::timeout_add_seconds_local(sleep_timeout_secs, move || {
                eprintln!("[GnomeGCal] Deep sleep timer expired! Calling terminate_web_process()...");
                wv_target.terminate_web_process();
                *term_flag.borrow_mut() = true;
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
        "GnomeGCal".into()
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        toggle_window();
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;

        let toggle_label = if self.is_visible {
            "Minimize to tray"
        } else {
            "Open GnomeGCal"
        };

        vec![
            StandardItem {
                label: toggle_label.into(),
                activate: Box::new(|_| {
                    toggle_window();
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "About GnomeGCal".into(),
                activate: Box::new(|_| {
                    show_about();
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit GnomeGCal".into(),
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
        ]
    }
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
                        win.set_visible(true);
                        win.present();
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
                    win.set_visible(true);
                    win.present();
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
        .label("Lightweight Google Calendar, Tasks & Keep\ndesktop client for GNOME")
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

    let x = 3.0 * scale;
    let y = 3.0 * scale;
    let w = 42.0 * scale;
    let h = 42.0 * scale;
    let r = 9.0 * scale;
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
    cr.set_font_size(24.0 * scale);

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