mod about;
mod config;
mod state;
mod tray;
mod update;
mod url_policy;

use adw::prelude::*;
use adw::{Application, ApplicationWindow, HeaderBar, StatusPage, ToolbarView};
use config::{Config, APP_ID};
use gtk::{gio, glib, Box as GtkBox, Button, Orientation, Stack};
use state::AppStateFile;
use std::cell::{Cell, RefCell};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use tray::{TrayCallbacks, TrayHandle};
use url_policy::{
    classify_main_navigation, classify_new_window, is_restorable_workspace_uri,
    is_trusted_google_data_domain, UrlAction,
};
use webkit6::prelude::*;
use webkit6::{
    HardwareAccelerationPolicy, MemoryPressureSettings, NetworkSession,
    NotificationPermissionRequest, PolicyDecisionType, Settings, UserContentInjectedFrames,
    UserContentManager, UserScript, UserScriptInjectionTime, UserStyleLevel, UserStyleSheet,
    WebContext, WebProcessTerminationReason, WebView, WebsiteDataTypes,
};

struct WebKitEnv {
    network_session: NetworkSession,
    web_context: WebContext,
    content_manager: UserContentManager,
    settings: Settings,
}

struct AppController {
    app: Application,
    window: ApplicationWindow,
    stack: Stack,
    webview_slot: GtkBox,
    cfg: Config,
    state: RefCell<AppStateFile>,
    webkit_env: RefCell<Option<WebKitEnv>>,
    webview: RefCell<Option<WebView>>,
    purged_third_party: Cell<bool>,
    intentional_sleep: Cell<bool>,
    needs_network_reload: Cell<bool>,
    sleep_source_id: RefCell<Option<glib::SourceId>>,
    crash_retry_source: RefCell<Option<glib::SourceId>>,
    activation_token: RefCell<Option<String>>,
    tray: RefCell<Option<Rc<TrayHandle>>>,
    hold_guard: RefCell<Option<gio::ApplicationHoldGuard>>,
}

fn main() -> glib::ExitCode {
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "-h" | "--help" => {
                println!(
                    "GnomeGCal {} — Lightweight Google Calendar, Tasks & Keep hub\n\n\
                     Usage:\n  gnome-gcal [OPTIONS]\n\n\
                     Options:\n  \
                       --hidden       Start silently in the system tray (0 MB WebKit)\n  \
                       --toggle       Toggle window visibility (for global shortcuts)\n  \
                       -v, --version  Show version and exit\n  \
                       -h, --help     Show this help and exit",
                    env!("CARGO_PKG_VERSION")
                );
                return glib::ExitCode::SUCCESS;
            }
            "-V" | "-v" | "--version" => {
                println!("gnome-gcal {}", env!("CARGO_PKG_VERSION"));
                return glib::ExitCode::SUCCESS;
            }
            _ => {}
        }
    }

    let app = Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();

    let controller_slot: Rc<RefCell<Option<Rc<AppController>>>> = Rc::new(RefCell::new(None));
    let slot_cmd = controller_slot.clone();

    app.connect_command_line(move |app, cmd| {
        let mut hidden = false;
        let mut toggle = false;
        for arg in cmd.arguments().into_iter().skip(1) {
            let s = arg.to_string_lossy();
            match s.as_ref() {
                "--hidden" => hidden = true,
                "--toggle" => toggle = true,
                _ => {}
            }
        }

        let startup_id = cmd
            .platform_data()
            .and_then(|v| v.get::<std::collections::HashMap<String, glib::Variant>>())
            .and_then(|map| {
                map.get("desktop-startup-id")
                    .and_then(|v| v.str().map(str::to_string))
            });

        let is_initial = slot_cmd.borrow().is_none();
        let ctrl = if let Some(existing) = slot_cmd.borrow().clone() {
            existing
        } else {
            let created = initialize_controller(app);
            *slot_cmd.borrow_mut() = Some(created.clone());
            created
        };

        if let Some(token) = startup_id {
            *ctrl.activation_token.borrow_mut() = Some(token);
        }

        ctrl.refresh_tray_day();

        if toggle {
            ctrl.toggle_window();
        } else if hidden && is_initial {
            ctrl.window.set_visible(false);
            ctrl.set_tray_visible(false);
        } else {
            ctrl.show_and_focus();
        }

        glib::ExitCode::SUCCESS
    });

    let slot_shutdown = controller_slot.clone();
    app.connect_shutdown(move |_| {
        if let Some(ctrl) = slot_shutdown.borrow_mut().take() {
            ctrl.cancel_timers();
            ctrl.persist_state();
            ctrl.tray.borrow_mut().take();
            ctrl.hold_guard.borrow_mut().take();
        }
    });

    app.run()
}

fn initialize_controller(app: &Application) -> Rc<AppController> {
    let cfg = Config::load_or_create();
    let mut state = AppStateFile::load();

    config::sync_autostart(cfg.general.autostart);
    run_cold_start_cache_hygiene(&cfg, &mut state);

    let style_manager = adw::StyleManager::default();
    match cfg.window.theme.trim().to_lowercase().as_str() {
        "dark" => style_manager.set_color_scheme(adw::ColorScheme::ForceDark),
        "light" => style_manager.set_color_scheme(adw::ColorScheme::ForceLight),
        _ => style_manager.set_color_scheme(adw::ColorScheme::Default),
    }

    gtk::Window::set_default_icon_name(APP_ID);

    let header_bar = HeaderBar::new();

    let refresh_btn = Button::from_icon_name("view-refresh-symbolic");
    refresh_btn.set_tooltip_text(Some("Refresh Calendar (Ctrl+R)"));
    header_bar.pack_start(&refresh_btn);

    let about_btn = Button::from_icon_name("help-about-symbolic");
    about_btn.set_tooltip_text(Some("About GnomeGCal"));
    header_bar.pack_end(&about_btn);

    let webview_slot = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .vexpand(true)
        .hexpand(true)
        .build();

    let retry_btn = Button::builder()
        .label("Retry")
        .css_classes(["suggested-action", "pill"])
        .halign(gtk::Align::Center)
        .build();

    let offline_page = StatusPage::builder()
        .icon_name("network-wireless-offline-symbolic")
        .title("Waiting for Network")
        .description("Google Calendar will load automatically once the connection is back.")
        .child(&retry_btn)
        .build();

    let stack = Stack::builder()
        .vexpand(true)
        .hexpand(true)
        .build();
    stack.add_named(&webview_slot, Some("webview"));
    stack.add_named(&offline_page, Some("offline"));
    stack.set_visible_child_name("webview");

    let toolbar_view = ToolbarView::new();
    toolbar_view.add_top_bar(&header_bar);
    toolbar_view.set_content(Some(&stack));

    let window = ApplicationWindow::builder()
        .application(app)
        .title("GnomeGCal — Google Calendar")
        .icon_name(APP_ID)
        .default_width(state.effective_width(&cfg))
        .default_height(state.effective_height(&cfg))
        .hide_on_close(true)
        .content(&toolbar_view)
        .build();

    let hold_guard = app.hold();

    let ctrl = Rc::new(AppController {
        app: app.clone(),
        window: window.clone(),
        stack,
        webview_slot,
        cfg,
        state: RefCell::new(state),
        webkit_env: RefCell::new(None),
        webview: RefCell::new(None),
        purged_third_party: Cell::new(false),
        intentional_sleep: Cell::new(false),
        needs_network_reload: Cell::new(false),
        sleep_source_id: RefCell::new(None),
        crash_retry_source: RefCell::new(None),
        activation_token: RefCell::new(None),
        tray: RefCell::new(None),
        hold_guard: RefCell::new(Some(hold_guard)),
    });

    setup_actions(&ctrl);
    setup_window_hooks(&ctrl);
    setup_theme_and_network(&ctrl, &style_manager);

    {
        let weak = Rc::downgrade(&ctrl);
        refresh_btn.connect_clicked(move |_| {
            if let Some(c) = weak.upgrade() {
                c.reload_or_restore();
            }
        });
    }
    {
        let weak = Rc::downgrade(&ctrl);
        retry_btn.connect_clicked(move |_| {
            if let Some(c) = weak.upgrade() {
                c.reload_or_restore();
            }
        });
    }
    {
        let weak = Rc::downgrade(&ctrl);
        about_btn.connect_clicked(move |_| {
            if let Some(c) = weak.upgrade() {
                about::show_about_dialog(&c.window);
            }
        });
    }

    let callbacks = TrayCallbacks {
        on_toggle: {
            let weak = Rc::downgrade(&ctrl);
            Rc::new(move || {
                if let Some(c) = weak.upgrade() {
                    c.toggle_window();
                }
            })
        },
        on_show: {
            let weak = Rc::downgrade(&ctrl);
            Rc::new(move || {
                if let Some(c) = weak.upgrade() {
                    c.show_and_focus();
                }
            })
        },
        on_hide: {
            let weak = Rc::downgrade(&ctrl);
            Rc::new(move || {
                if let Some(c) = weak.upgrade() {
                    c.hide_to_tray();
                }
            })
        },
        on_quit: {
            let app_quit = app.clone();
            Rc::new(move || {
                app_quit.quit();
            })
        },
        on_activation_token: {
            let weak = Rc::downgrade(&ctrl);
            Rc::new(move |token| {
                if let Some(c) = weak.upgrade() {
                    *c.activation_token.borrow_mut() = Some(token);
                }
            })
        },
    };

    match TrayHandle::spawn(false, callbacks) {
        Ok(handle) => {
            *ctrl.tray.borrow_mut() = Some(handle);
        }
        Err(err) => {
            eprintln!("[GnomeGCal] Tray unavailable: {err}");
        }
    }

    ctrl
}

impl AppController {
    fn cancel_timers(&self) {
        if let Some(id) = self.sleep_source_id.borrow_mut().take() {
            id.remove();
        }
        if let Some(id) = self.crash_retry_source.borrow_mut().take() {
            id.remove();
        }
    }

    fn set_tray_visible(&self, visible: bool) {
        if let Some(tray) = self.tray.borrow().as_ref() {
            tray.set_window_visible(visible);
        }
    }

    fn refresh_tray_day(&self) {
        if let Some(tray) = self.tray.borrow().as_ref() {
            tray.refresh_day_if_needed();
        }
    }

    fn persist_state(&self) {
        let mut st = self.state.borrow_mut();
        let w = self.window.width();
        let h = self.window.height();
        if w >= 360 && h >= 320 {
            st.width = Some(w);
            st.height = Some(h);
        }
        if let Some(wv) = self.webview.borrow().as_ref() {
            st.zoom_level = Some(wv.zoom_level());
            if let Some(uri) = wv.uri()
                && is_restorable_workspace_uri(uri.as_str())
            {
                st.last_uri = Some(uri.to_string());
            }
        }
        st.save();
    }

    fn show_and_focus(self: &Rc<Self>) {
        self.refresh_tray_day();
        if let Some(id) = self.sleep_source_id.borrow_mut().take() {
            id.remove();
        }
        self.ensure_webview();

        let token = self.activation_token.borrow_mut().take();
        if let Some(ref t) = token {
            self.window.set_startup_id(t);
        }
        self.window.unminimize();
        self.window.set_visible(true);
        if let Some(ref t) = token
            && let Some(surface) = self.window.surface()
            && let Ok(toplevel) = surface.downcast::<gtk::gdk::Toplevel>()
        {
            toplevel.set_startup_id(t);
        }
        self.window.present();
    }

    fn hide_to_tray(&self) {
        self.window.set_visible(false);
    }

    fn toggle_window(self: &Rc<Self>) {
        if self.window.is_visible() {
            self.hide_to_tray();
        } else {
            self.show_and_focus();
        }
    }

    fn reload_or_restore(self: &Rc<Self>) {
        self.needs_network_reload.set(false);
        self.stack.set_visible_child_name("webview");
        if let Some(wv) = self.webview.borrow().clone() {
            wv.reload();
        } else {
            self.ensure_webview();
        }
    }

    fn destroy_webview_for_sleep(&self) {
        self.intentional_sleep.set(true);
        if let Some(wv) = self.webview.borrow_mut().take() {
            if let Some(uri) = wv.uri()
                && is_restorable_workspace_uri(uri.as_str())
            {
                self.state.borrow_mut().last_uri = Some(uri.to_string());
            }
            self.state.borrow_mut().zoom_level = Some(wv.zoom_level());
            self.state.borrow().save();

            self.webview_slot.remove(&wv);
            wv.terminate_web_process();
        }
        // Drop WebContext and NetworkSession too so background ServiceWorker and Network processes exit cleanly.
        self.webkit_env.borrow_mut().take();
        self.intentional_sleep.set(false);
    }

    fn ensure_webkit_env(&self) {
        if self.webkit_env.borrow().is_some() {
            return;
        }

        let mut data_dir = glib::user_data_dir();
        data_dir.push("gnome-gcal/data");
        let mut cache_dir = glib::user_cache_dir();
        cache_dir.push("gnome-gcal/cache");

        fs::create_dir_all(&data_dir).ok();
        fs::create_dir_all(&cache_dir).ok();

        let data_str = data_dir.to_str().unwrap_or("/tmp/gnome-gcal-data");
        let cache_str = cache_dir.to_str().unwrap_or("/tmp/gnome-gcal-cache");

        // Must be set before creating NetworkSession so WebKit applies it to the NetworkProcess as well.
        let mut mem_settings = MemoryPressureSettings::new();
        mem_settings.set_memory_limit(self.cfg.performance.memory_limit_mb);
        mem_settings.set_strict_threshold(0.75);
        mem_settings.set_conservative_threshold(0.5);
        mem_settings.set_kill_threshold(0.0);
        mem_settings.set_poll_interval(10.0);
        NetworkSession::set_memory_pressure_settings(&mut mem_settings);

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

        if !self.purged_third_party.replace(true) {
            purge_non_google_website_data(&network_session);
        }

        let is_dark = adw::StyleManager::default().is_dark();
        let content_manager = UserContentManager::new();
        update_calendar_theme(&content_manager, is_dark);

        let web_context = WebContext::builder()
            .memory_pressure_settings(&mem_settings)
            .build();

        let user_agent = if self.cfg.window.user_agent.trim().is_empty() {
            config::default_user_agent()
        } else {
            self.cfg.window.user_agent.clone()
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

        *self.webkit_env.borrow_mut() = Some(WebKitEnv {
            network_session,
            web_context,
            content_manager,
            settings,
        });
    }

    fn ensure_webview(self: &Rc<Self>) {
        if self.webview.borrow().is_some() {
            return;
        }
        self.ensure_webkit_env();

        let env_ref = self.webkit_env.borrow();
        let Some(env) = env_ref.as_ref() else {
            return;
        };

        let wv = WebView::builder()
            .web_context(&env.web_context)
            .network_session(&env.network_session)
            .user_content_manager(&env.content_manager)
            .settings(&env.settings)
            .vexpand(true)
            .hexpand(true)
            .build();

        let is_dark = adw::StyleManager::default().is_dark();
        set_webview_bg(&wv, is_dark);

        let zoom = self.state.borrow().effective_zoom(&self.cfg);
        wv.set_zoom_level(zoom);

        let notifications_enabled = self.cfg.general.notifications_enabled;
        wv.connect_permission_request(move |_, request| {
            if notifications_enabled && request.is::<NotificationPermissionRequest>() {
                request.allow();
            } else {
                request.deny();
            }
            true
        });

        let app_notif = self.app.clone();
        let weak_notif = Rc::downgrade(self);
        wv.connect_show_notification(move |_, notification| {
            if !notifications_enabled {
                return false;
            }
            let title = notification
                .title()
                .map(|s| s.to_string())
                .unwrap_or_else(|| "Google Calendar".to_string());
            let gio_notif = gio::Notification::new(&title);
            if let Some(body) = notification.body()
                && !body.is_empty()
            {
                gio_notif.set_body(Some(body.as_str()));
            }
            app_notif.send_notification(Some("gcal-web-notification"), &gio_notif);
            if let Some(c) = weak_notif.upgrade() {
                c.refresh_tray_day();
            }
            true
        });

        wv.connect_decide_policy(|webview, decision, decision_type| {
            let Some(nav_decision) = decision.downcast_ref::<webkit6::NavigationPolicyDecision>()
            else {
                return false;
            };
            let Some(nav_action) = nav_decision.navigation_action() else {
                return false;
            };
            let Some(request) = nav_action.request() else {
                return false;
            };
            let Some(uri) = request.uri() else {
                return false;
            };

            let action = match decision_type {
                PolicyDecisionType::NewWindowAction => classify_new_window(uri.as_str()),
                PolicyDecisionType::NavigationAction => {
                    let is_user_click = nav_action.navigation_type()
                        == webkit6::NavigationType::LinkClicked
                        || (nav_action.is_user_gesture() && nav_action.frame_name().is_none());
                    classify_main_navigation(uri.as_str(), is_user_click)
                }
                _ => return false,
            };

            match action {
                UrlAction::AllowInApp => false,
                UrlAction::RedirectInApp(target) => {
                    decision.ignore();
                    webview.load_uri(&target);
                    true
                }
                UrlAction::OpenExternal(target) => {
                    decision.ignore();
                    let _ = gio::AppInfo::launch_default_for_uri(
                        &target,
                        None::<&gio::AppLaunchContext>,
                    );
                    true
                }
                UrlAction::Block => {
                    decision.ignore();
                    true
                }
            }
        });

        {
            let weak = Rc::downgrade(self);
            wv.connect_uri_notify(move |webview| {
                if let (Some(c), Some(uri)) = (weak.upgrade(), webview.uri())
                    && is_restorable_workspace_uri(uri.as_str())
                {
                    c.state.borrow_mut().last_uri = Some(uri.to_string());
                }
            });
        }

        {
            let weak = Rc::downgrade(self);
            wv.connect_load_failed(move |_webview, _event, _uri, err| {
                if err.matches(webkit6::NetworkError::Cancelled) {
                    return false;
                }
                if let Some(c) = weak.upgrade()
                    && !gio::NetworkMonitor::default().is_network_available()
                {
                    c.needs_network_reload.set(true);
                    c.stack.set_visible_child_name("offline");
                    return true;
                }
                false
            });
        }

        {
            let weak = Rc::downgrade(self);
            wv.connect_web_process_terminated(move |dead_wv, reason| {
                let Some(c) = weak.upgrade() else {
                    return;
                };
                if c.intentional_sleep.get()
                    || reason == WebProcessTerminationReason::TerminatedByApi
                    || !c.window.is_visible()
                {
                    return;
                }

                eprintln!("[GnomeGCal] WebProcess terminated unexpectedly ({reason:?}); recovering...");
                if let Some(old) = c.crash_retry_source.borrow_mut().take() {
                    old.remove();
                }

                // Clear the crashed WebView on idle/timeout and re-create cleanly
                let dead_clone = dead_wv.clone();
                let weak_retry = Rc::downgrade(&c);
                let src = glib::timeout_add_local_once(
                    std::time::Duration::from_millis(300),
                    move || {
                        let Some(c2) = weak_retry.upgrade() else {
                            return;
                        };
                        c2.crash_retry_source.borrow_mut().take();
                        let matches_current = c2
                            .webview
                            .borrow()
                            .as_ref()
                            .is_some_and(|cur| cur == &dead_clone);
                        if matches_current {
                            if let Some(removed) = c2.webview.borrow_mut().take() {
                                c2.webview_slot.remove(&removed);
                            }
                            if c2.window.is_visible() {
                                c2.ensure_webview();
                            }
                        }
                    },
                );
                *c.crash_retry_source.borrow_mut() = Some(src);
            });
        }

        let target_uri = self.state.borrow().effective_uri().to_string();
        if gio::NetworkMonitor::default().is_network_available() {
            self.needs_network_reload.set(false);
            self.stack.set_visible_child_name("webview");
            wv.load_uri(&target_uri);
        } else {
            self.needs_network_reload.set(true);
            self.stack.set_visible_child_name("offline");
        }

        self.webview_slot.append(&wv);
        *self.webview.borrow_mut() = Some(wv);
    }
}

fn setup_actions(ctrl: &Rc<AppController>) {
    let act_zoom_in = gio::SimpleAction::new("zoom-in", None);
    {
        let weak = Rc::downgrade(ctrl);
        act_zoom_in.connect_activate(move |_, _| {
            if let Some(c) = weak.upgrade()
                && let Some(wv) = c.webview.borrow().as_ref()
            {
                let next = (wv.zoom_level() + 0.1).min(2.5);
                wv.set_zoom_level(next);
                c.state.borrow_mut().zoom_level = Some(next);
                c.state.borrow().save();
            }
        });
    }
    ctrl.app.add_action(&act_zoom_in);
    ctrl.app.set_accels_for_action(
        "app.zoom-in",
        &["<Primary>plus", "<Primary>equal", "<Primary>KP_Add"],
    );

    let act_zoom_out = gio::SimpleAction::new("zoom-out", None);
    {
        let weak = Rc::downgrade(ctrl);
        act_zoom_out.connect_activate(move |_, _| {
            if let Some(c) = weak.upgrade()
                && let Some(wv) = c.webview.borrow().as_ref()
            {
                let next = (wv.zoom_level() - 0.1).max(0.5);
                wv.set_zoom_level(next);
                c.state.borrow_mut().zoom_level = Some(next);
                c.state.borrow().save();
            }
        });
    }
    ctrl.app.add_action(&act_zoom_out);
    ctrl.app.set_accels_for_action(
        "app.zoom-out",
        &["<Primary>minus", "<Primary>KP_Subtract"],
    );

    let act_zoom_reset = gio::SimpleAction::new("zoom-reset", None);
    {
        let weak = Rc::downgrade(ctrl);
        act_zoom_reset.connect_activate(move |_, _| {
            if let Some(c) = weak.upgrade() {
                let default_zoom = c.cfg.window.zoom_level.clamp(0.5, 2.5);
                if let Some(wv) = c.webview.borrow().as_ref() {
                    wv.set_zoom_level(default_zoom);
                }
                c.state.borrow_mut().zoom_level = Some(default_zoom);
                c.state.borrow().save();
            }
        });
    }
    ctrl.app.add_action(&act_zoom_reset);
    ctrl.app
        .set_accels_for_action("app.zoom-reset", &["<Primary>0", "<Primary>KP_0"]);

    let act_reload = gio::SimpleAction::new("reload", None);
    {
        let weak = Rc::downgrade(ctrl);
        act_reload.connect_activate(move |_, _| {
            if let Some(c) = weak.upgrade() {
                c.reload_or_restore();
            }
        });
    }
    ctrl.app.add_action(&act_reload);
    ctrl.app
        .set_accels_for_action("app.reload", &["<Primary>r", "F5"]);
}

fn setup_window_hooks(ctrl: &Rc<AppController>) {
    // Re-attach Toplevel minimize listener whenever the window is realized,
    // because hiding the window destroys the underlying GdkSurface.
    {
        let weak = Rc::downgrade(ctrl);
        ctrl.window.connect_realize(move |win| {
            let Some(surface) = win.surface() else {
                return;
            };
            let Ok(toplevel) = surface.downcast::<gtk::gdk::Toplevel>() else {
                return;
            };
            let win_weak = win.downgrade();
            let ctrl_weak = weak.clone();
            toplevel.connect_state_notify(move |tl| {
                if tl.state().contains(gtk::gdk::ToplevelState::MINIMIZED) {
                    let win_weak = win_weak.clone();
                    let ctrl_weak = ctrl_weak.clone();
                    glib::idle_add_local_once(move || {
                        if let Some(win) = win_weak.upgrade() {
                            win.unminimize();
                            win.set_visible(false);
                        }
                        if let Some(c) = ctrl_weak.upgrade() {
                            c.set_tray_visible(false);
                        }
                    });
                }
            });
        });
    }

    let deep_sleep_enabled = ctrl.cfg.performance.deep_sleep_enabled;
    let sleep_timeout_secs = ctrl.cfg.performance.sleep_timeout_minutes.saturating_mul(60);

    let weak = Rc::downgrade(ctrl);
    ctrl.window.connect_visible_notify(move |win| {
        let Some(c) = weak.upgrade() else {
            return;
        };
        let is_vis = win.is_visible();
        c.set_tray_visible(is_vis);

        if is_vis {
            if let Some(id) = c.sleep_source_id.borrow_mut().take() {
                id.remove();
            }
            c.ensure_webview();
            return;
        }

        c.persist_state();

        if deep_sleep_enabled {
            if let Some(prev) = c.sleep_source_id.borrow_mut().take() {
                prev.remove();
            }
            let weak_sleep = Rc::downgrade(&c);
            let source_id = glib::timeout_add_seconds_local_once(sleep_timeout_secs, move || {
                let Some(c2) = weak_sleep.upgrade() else {
                    return;
                };
                c2.sleep_source_id.borrow_mut().take();
                if !c2.window.is_visible() {
                    c2.destroy_webview_for_sleep();
                }
            });
            *c.sleep_source_id.borrow_mut() = Some(source_id);
        }
    });
}

fn setup_theme_and_network(ctrl: &Rc<AppController>, style_manager: &adw::StyleManager) {
    {
        let weak = Rc::downgrade(ctrl);
        style_manager.connect_dark_notify(move |sm| {
            let Some(c) = weak.upgrade() else {
                return;
            };
            let is_dark = sm.is_dark();
            if let Some(env) = c.webkit_env.borrow().as_ref() {
                update_calendar_theme(&env.content_manager, is_dark);
            }
            if let Some(wv) = c.webview.borrow().as_ref() {
                set_webview_bg(wv, is_dark);
                let js = format!(
                    "window.__gcalApplyTheme && window.__gcalApplyTheme({});",
                    if is_dark { "true" } else { "false" }
                );
                wv.evaluate_javascript(&js, None, None, None::<&gio::Cancellable>, |_| {});
            }
        });
    }

    {
        let weak = Rc::downgrade(ctrl);
        gio::NetworkMonitor::default().connect_network_changed(move |_, available| {
            if !available {
                return;
            }
            let Some(c) = weak.upgrade() else {
                return;
            };
            if c.needs_network_reload.get() {
                c.needs_network_reload.set(false);
                c.stack.set_visible_child_name("webview");
                if let Some(wv) = c.webview.borrow().clone() {
                    let target = c.state.borrow().effective_uri().to_string();
                    wv.load_uri(&target);
                } else if c.window.is_visible() {
                    c.ensure_webview();
                }
            }
        });
    }
}

fn set_webview_bg(wv: &WebView, is_dark: bool) {
    let bg = if is_dark {
        gtk::gdk::RGBA::new(0.118, 0.118, 0.118, 1.0)
    } else {
        gtk::gdk::RGBA::new(1.0, 1.0, 1.0, 1.0)
    };
    wv.set_background_color(&bg);
}

/// Applies non-intrusive system styles (scrollbar + color-scheme) and a live
/// `matchMedia` wrapper so GNOME Dark/Light switches happen instantly without `reload()`.
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
            let currentDark = {is_dark};
            const listeners = new Set();
            const origMatchMedia = window.matchMedia.bind(window);

            function computeMatches(query, dark) {{
                const wantsDark = query.includes('dark');
                const wantsLight = query.includes('light');
                return (wantsDark && dark) || (wantsLight && !dark);
            }}

            window.matchMedia = function(query) {{
                if (typeof query === 'string' && query.includes('prefers-color-scheme')) {{
                    const legacyListeners = new Set();
                    const mql = {{
                        media: query,
                        onchange: null,
                        get matches() {{
                            return computeMatches(query, currentDark);
                        }},
                        addListener: function(fn) {{
                            if (typeof fn === 'function') {{
                                legacyListeners.add(fn);
                                listeners.add({{ mql, query, fn }});
                            }}
                        }},
                        removeListener: function(fn) {{
                            legacyListeners.delete(fn);
                            for (const item of Array.from(listeners)) {{
                                if (item.mql === mql && item.fn === fn) listeners.delete(item);
                            }}
                        }},
                        addEventListener: function(type, fn) {{
                            if (type === 'change' && typeof fn === 'function') {{
                                listeners.add({{ mql, query, fn }});
                            }}
                        }},
                        removeEventListener: function(type, fn) {{
                            if (type === 'change') {{
                                for (const item of Array.from(listeners)) {{
                                    if (item.mql === mql && item.fn === fn) listeners.delete(item);
                                }}
                            }}
                        }},
                        dispatchEvent: function() {{ return false; }}
                    }};
                    return mql;
                }}
                return origMatchMedia(query);
            }};

            window.__gcalApplyTheme = function(nextDark) {{
                currentDark = !!nextDark;
                if (document.documentElement) {{
                    document.documentElement.style.colorScheme = currentDark ? 'dark' : 'light';
                }}
                for (const item of Array.from(listeners)) {{
                    const matches = computeMatches(item.query, currentDark);
                    const ev = {{ matches, media: item.query }};
                    try {{
                        item.fn.call(item.mql, ev);
                        if (typeof item.mql.onchange === 'function') {{
                            item.mql.onchange(ev);
                        }}
                    }} catch (_) {{}}
                }}
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

fn dir_size_bytes(path: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    let mut total = 0u64;
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            total = total.saturating_add(dir_size_bytes(&entry.path()));
        } else {
            total = total.saturating_add(meta.len());
        }
    }
    total
}

fn run_cold_start_cache_hygiene(cfg: &Config, state: &mut AppStateFile) {
    let cache_dir = glib::user_cache_dir().join("gnome-gcal/cache");
    let limit_bytes = cfg.performance.cache_limit_mb.saturating_mul(1024 * 1024);
    let current_bytes = dir_size_bytes(&cache_dir);

    if !state.cache_migrated_v3 || (limit_bytes > 0 && current_bytes > limit_bytes) {
        let _ = fs::remove_dir_all(&cache_dir);
        let _ = fs::create_dir_all(&cache_dir);
        state.cache_migrated_v3 = true;
        state.save();
    }
}

fn purge_non_google_website_data(network_session: &NetworkSession) {
    let Some(dm) = network_session.website_data_manager() else {
        return;
    };
    let types = WebsiteDataTypes::ALL.difference(WebsiteDataTypes::COOKIES);
    let dm_clone = dm.clone();
    dm.fetch(types, None::<&gio::Cancellable>, move |res| {
        let Ok(items) = res else {
            return;
        };
        let to_remove: Vec<&webkit6::WebsiteData> = items
            .iter()
            .filter(|item| {
                item.name()
                    .map(|n| !is_trusted_google_data_domain(n.as_str()))
                    .unwrap_or(false)
            })
            .collect();
        if !to_remove.is_empty() {
            dm_clone.remove(types, &to_remove, None::<&gio::Cancellable>, |_| {});
        }
    });
}
