use crate::config::APP_ID;
use gtk::glib::prelude::*;
use gtk::{gio, glib};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

const SNI_PATH: &str = "/StatusNotifierItem";
const MENU_PATH: &str = "/MenuBar";
const SNI_INTERFACE: &str = "org.kde.StatusNotifierItem";
const MENU_INTERFACE: &str = "com.canonical.dbusmenu";
const WATCHER_NAME: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";

const SNI_XML: &str = r#"
<node>
  <interface name="org.kde.StatusNotifierItem">
    <method name="ContextMenu">
      <arg name="x" type="i" direction="in"/>
      <arg name="y" type="i" direction="in"/>
    </method>
    <method name="Activate">
      <arg name="x" type="i" direction="in"/>
      <arg name="y" type="i" direction="in"/>
    </method>
    <method name="SecondaryActivate">
      <arg name="x" type="i" direction="in"/>
      <arg name="y" type="i" direction="in"/>
    </method>
    <method name="Scroll">
      <arg name="delta" type="i" direction="in"/>
      <arg name="orientation" type="s" direction="in"/>
    </method>
    <method name="ProvideXdgActivationToken">
      <arg name="token" type="s" direction="in"/>
    </method>
    <signal name="NewTitle"/>
    <signal name="NewIcon"/>
    <signal name="NewAttentionIcon"/>
    <signal name="NewOverlayIcon"/>
    <signal name="NewToolTip"/>
    <signal name="NewStatus">
      <arg name="status" type="s"/>
    </signal>
    <property name="Category" type="s" access="read"/>
    <property name="Id" type="s" access="read"/>
    <property name="Title" type="s" access="read"/>
    <property name="Status" type="s" access="read"/>
    <property name="WindowId" type="i" access="read"/>
    <property name="IconThemePath" type="s" access="read"/>
    <property name="Menu" type="o" access="read"/>
    <property name="ItemIsMenu" type="b" access="read"/>
    <property name="IconName" type="s" access="read"/>
    <property name="IconPixmap" type="a(iiay)" access="read"/>
    <property name="OverlayIconName" type="s" access="read"/>
    <property name="OverlayIconPixmap" type="a(iiay)" access="read"/>
    <property name="AttentionIconName" type="s" access="read"/>
    <property name="AttentionIconPixmap" type="a(iiay)" access="read"/>
    <property name="AttentionMovieName" type="s" access="read"/>
    <property name="ToolTip" type="(sa(iiay)ss)" access="read"/>
  </interface>
</node>
"#;

const MENU_XML: &str = r#"
<node>
  <interface name="com.canonical.dbusmenu">
    <method name="GetLayout">
      <arg name="parentId" type="i" direction="in"/>
      <arg name="recursionDepth" type="i" direction="in"/>
      <arg name="propertyNames" type="as" direction="in"/>
      <arg name="revision" type="u" direction="out"/>
      <arg name="layout" type="(ia{sv}av)" direction="out"/>
    </method>
    <method name="GetGroupProperties">
      <arg name="ids" type="ai" direction="in"/>
      <arg name="propertyNames" type="as" direction="in"/>
      <arg name="properties" type="a(ia{sv})" direction="out"/>
    </method>
    <method name="GetProperty">
      <arg name="id" type="i" direction="in"/>
      <arg name="name" type="s" direction="in"/>
      <arg name="value" type="v" direction="out"/>
    </method>
    <method name="Event">
      <arg name="id" type="i" direction="in"/>
      <arg name="eventId" type="s" direction="in"/>
      <arg name="data" type="v" direction="in"/>
      <arg name="timestamp" type="u" direction="in"/>
    </method>
    <method name="EventGroup">
      <arg name="events" type="a(isvu)" direction="in"/>
      <arg name="idErrors" type="ai" direction="out"/>
    </method>
    <method name="AboutToShow">
      <arg name="id" type="i" direction="in"/>
      <arg name="needUpdate" type="b" direction="out"/>
    </method>
    <method name="AboutToShowGroup">
      <arg name="ids" type="ai" direction="in"/>
      <arg name="updatesNeeded" type="ai" direction="out"/>
      <arg name="idErrors" type="ai" direction="out"/>
    </method>
    <signal name="ItemsPropertiesUpdated">
      <arg name="updatedProps" type="a(ia{sv})"/>
      <arg name="removedProps" type="a(ias)"/>
    </signal>
    <signal name="LayoutUpdated">
      <arg name="revision" type="u"/>
      <arg name="parent" type="i"/>
    </signal>
    <property name="Version" type="u" access="read"/>
    <property name="TextDirection" type="s" access="read"/>
    <property name="Status" type="s" access="read"/>
    <property name="IconThemePath" type="as" access="read"/>
  </interface>
</node>
"#;

const MENU_ID_MINIMIZE: i32 = 1;
const MENU_ID_SEP_TOP: i32 = 2;
const MENU_ID_OPEN: i32 = 3;
const MENU_ID_SEP_MID: i32 = 4;
const MENU_ID_ABOUT: i32 = 5;
const MENU_ID_SEP_BOT: i32 = 6;
const MENU_ID_QUIT: i32 = 7;

#[derive(Clone)]
pub struct TrayCallbacks {
    pub on_toggle: Rc<dyn Fn()>,
    pub on_show: Rc<dyn Fn()>,
    pub on_hide: Rc<dyn Fn()>,
    pub on_about: Rc<dyn Fn()>,
    pub on_quit: Rc<dyn Fn()>,
    pub on_activation_token: Rc<dyn Fn(String)>,
}

struct CachedIcons {
    day: i32,
    icons: Vec<(i32, i32, Vec<u8>)>,
}

struct TrayState {
    is_visible: bool,
    revision: u32,
    bus_name: String,
    cached: CachedIcons,
    retry_source: Option<glib::SourceId>,
    midnight_source: Option<glib::SourceId>,
}

pub struct TrayHandle {
    conn: gio::DBusConnection,
    state: Rc<RefCell<TrayState>>,
    sni_reg_id: Option<gio::RegistrationId>,
    menu_reg_id: Option<gio::RegistrationId>,
    bus_cleanup: Option<Box<dyn FnOnce()>>,
}

impl Drop for TrayHandle {
    fn drop(&mut self) {
        if let Some(src) = self.state.borrow_mut().retry_source.take() {
            src.remove();
        }
        if let Some(src) = self.state.borrow_mut().midnight_source.take() {
            src.remove();
        }
        if let Some(cleanup) = self.bus_cleanup.take() {
            cleanup();
        }
        if let Some(id) = self.sni_reg_id.take() {
            let _ = self.conn.unregister_object(id);
        }
        if let Some(id) = self.menu_reg_id.take() {
            let _ = self.conn.unregister_object(id);
        }
    }
}

impl TrayHandle {
    pub fn spawn(is_visible: bool, callbacks: TrayCallbacks) -> Result<Rc<Self>, String> {
        let conn = gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>)
            .map_err(|e| format!("Failed to connect to session D-Bus: {e}"))?;

        let sni_node = gio::DBusNodeInfo::for_xml(SNI_XML)
            .map_err(|e| format!("Invalid SNI XML: {e}"))?;
        let menu_node = gio::DBusNodeInfo::for_xml(MENU_XML)
            .map_err(|e| format!("Invalid DBusMenu XML: {e}"))?;

        let sni_iface = sni_node
            .lookup_interface(SNI_INTERFACE)
            .ok_or("Missing SNI interface info")?;
        let menu_iface = menu_node
            .lookup_interface(MENU_INTERFACE)
            .ok_or("Missing DBusMenu interface info")?;

        let day = current_local_day();
        let bus_name = format!("org.kde.StatusNotifierItem-{}-1", std::process::id());
        let state = Rc::new(RefCell::new(TrayState {
            is_visible,
            revision: 1,
            bus_name: bus_name.clone(),
            cached: CachedIcons {
                day,
                icons: build_icon_set(day),
            },
            retry_source: None,
            midnight_source: None,
        }));

        let sni_reg_id = register_sni(&conn, &sni_iface, &state, &callbacks)
            .map_err(|e| format!("Failed to register SNI object: {e}"))?;
        let menu_reg_id = register_menu(&conn, &menu_iface, &state, &callbacks)
            .map_err(|e| format!("Failed to register DBusMenu object: {e}"))?;

        let state_own = state.clone();
        let owner_id = gio::bus_own_name_on_connection(
            &conn,
            &bus_name,
            gio::BusNameOwnerFlags::NONE,
            move |_c, _name| {},
            move |_c, name| {
                eprintln!("[GnomeGCal] Lost tray D-Bus name {name}");
                if let Some(src) = state_own.borrow_mut().retry_source.take() {
                    src.remove();
                }
            },
        );

        let state_watch = state.clone();
        let state_vanish = state.clone();
        let watcher_id = gio::bus_watch_name_on_connection(
            &conn,
            WATCHER_NAME,
            gio::BusNameWatcherFlags::NONE,
            move |c, _name, _owner| {
                register_with_watcher_retry(&c, &state_watch, 0);
            },
            move |_c, _name| {
                eprintln!("[GnomeGCal] StatusNotifierWatcher offline, waiting to reconnect...");
                if let Some(src) = state_vanish.borrow_mut().retry_source.take() {
                    src.remove();
                }
            },
        );

        let handle = Rc::new(Self {
            conn,
            state,
            sni_reg_id: Some(sni_reg_id),
            menu_reg_id: Some(menu_reg_id),
            bus_cleanup: Some(Box::new(move || {
                gio::bus_unwatch_name(watcher_id);
                gio::bus_unown_name(owner_id);
            })),
        });

        schedule_midnight_refresh(&handle);
        Ok(handle)
    }

    pub fn set_window_visible(&self, visible: bool) {
        let rev = {
            let mut st = self.state.borrow_mut();
            if st.is_visible == visible {
                return;
            }
            st.is_visible = visible;
            st.revision = st.revision.saturating_add(1);
            st.revision
        };
        let params = glib::Variant::tuple_from_iter([rev.to_variant(), 0i32.to_variant()]);
        let _ = self.conn.emit_signal(
            None,
            MENU_PATH,
            MENU_INTERFACE,
            "LayoutUpdated",
            Some(&params),
        );
    }

    pub fn refresh_day_if_needed(&self) {
        let today = current_local_day();
        let changed = {
            let mut st = self.state.borrow_mut();
            if st.cached.day == today {
                false
            } else {
                st.cached = CachedIcons {
                    day: today,
                    icons: build_icon_set(today),
                };
                true
            }
        };
        if changed {
            let _ = self
                .conn
                .emit_signal(None, SNI_PATH, SNI_INTERFACE, "NewIcon", None);
        }
    }
}

fn schedule_midnight_refresh(handle: &Rc<TrayHandle>) {
    let secs = glib::DateTime::now_local()
        .map(|dt| seconds_until_next_midnight(&dt))
        .unwrap_or(3600);

    let weak_handle = Rc::downgrade(handle);
    let source_id = glib::timeout_add_seconds_local_once(secs, move || {
        if let Some(h) = weak_handle.upgrade() {
            h.state.borrow_mut().midnight_source = None;
            h.refresh_day_if_needed();
            schedule_midnight_refresh(&h);
        }
    });
    handle.state.borrow_mut().midnight_source = Some(source_id);
}

fn register_with_watcher_retry(
    conn: &gio::DBusConnection,
    state: &Rc<RefCell<TrayState>>,
    attempt: u32,
) {
    if let Some(src) = state.borrow_mut().retry_source.take() {
        src.remove();
    }

    let bus_name = state.borrow().bus_name.clone();
    let params = glib::Variant::tuple_from_iter([bus_name.to_variant()]);
    let conn_clone = conn.clone();
    let state_clone = state.clone();

    conn.call(
        Some(WATCHER_NAME),
        WATCHER_PATH,
        WATCHER_NAME,
        "RegisterStatusNotifierItem",
        Some(&params),
        None,
        gio::DBusCallFlags::NONE,
        2000,
        None::<&gio::Cancellable>,
        move |res| {
            if let Err(err) = res {
                if attempt < 15 {
                    let next_attempt = attempt + 1;
                    let conn_retry = conn_clone.clone();
                    let state_retry = state_clone.clone();
                    let source_id = glib::timeout_add_local_once(
                        std::time::Duration::from_millis(500),
                        move || {
                            state_retry.borrow_mut().retry_source = None;
                            register_with_watcher_retry(&conn_retry, &state_retry, next_attempt);
                        },
                    );
                    state_clone.borrow_mut().retry_source = Some(source_id);
                } else {
                    eprintln!("[GnomeGCal] Failed to register StatusNotifierItem: {err}");
                }
            }
        },
    );
}

fn register_sni(
    conn: &gio::DBusConnection,
    iface: &gio::DBusInterfaceInfo,
    state: &Rc<RefCell<TrayState>>,
    callbacks: &TrayCallbacks,
) -> Result<gio::RegistrationId, glib::Error> {
    let cb_methods = callbacks.clone();
    let state_props = state.clone();

    conn.register_object(SNI_PATH, iface)
        .method_call(
            move |_conn, _sender, _path, _iface, method, params, invocation| match method {
                "Activate" | "SecondaryActivate" => {
                    (cb_methods.on_toggle)();
                    invocation.return_value(None);
                }
                "Scroll" => {
                    invocation.return_value(None);
                }
                "ProvideXdgActivationToken" => {
                    if let Some(token) = params
                        .child_value(0)
                        .str()
                        .map(str::to_string)
                    {
                        (cb_methods.on_activation_token)(token);
                    }
                    invocation.return_value(None);
                }
                "ContextMenu" => {
                    invocation.return_dbus_error(
                        "org.freedesktop.DBus.Error.UnknownMethod",
                        "Not supported, please use Menu",
                    );
                }
                _ => {
                    invocation.return_dbus_error(
                        "org.freedesktop.DBus.Error.UnknownMethod",
                        "Unknown method",
                    );
                }
            },
        )
        .property(move |_conn, _sender, _path, _iface, prop_name| {
            match prop_name {
                "Category" => "ApplicationStatus".to_variant(),
                "Id" => APP_ID.to_variant(),
                "Title" => "GnomeGCal — Google Calendar".to_variant(),
                "Status" => "Active".to_variant(),
                "WindowId" => 0i32.to_variant(),
                "IconThemePath" => "".to_variant(),
                "Menu" => glib::variant::ObjectPath::try_from(MENU_PATH)
                    .expect("valid object path")
                    .to_variant(),
                "ItemIsMenu" => false.to_variant(),
                "IconName" => "".to_variant(),
                "IconPixmap" => {
                    let st = state_props.borrow();
                    icon_pixmap_variant(&st.cached.icons)
                }
                "OverlayIconName" | "AttentionIconName" | "AttentionMovieName" => "".to_variant(),
                "OverlayIconPixmap" | "AttentionIconPixmap" => empty_pixmap_variant(),
                "ToolTip" => tooltip_variant(),
                _ => "".to_variant(),
            }
        })
        .build()
}

fn register_menu(
    conn: &gio::DBusConnection,
    iface: &gio::DBusInterfaceInfo,
    state: &Rc<RefCell<TrayState>>,
    callbacks: &TrayCallbacks,
) -> Result<gio::RegistrationId, glib::Error> {
    let state_methods = state.clone();
    let cb_methods = callbacks.clone();

    conn.register_object(MENU_PATH, iface)
        .method_call(
            move |_conn, _sender, _path, _iface, method, params, invocation| match method {
                "GetLayout" => {
                    let parent_id = params.child_value(0).get::<i32>().unwrap_or(0);
                    let recursion_depth = params.child_value(1).get::<i32>().unwrap_or(-1);
                    let prop_names: Vec<String> =
                        params.child_value(2).get::<Vec<String>>().unwrap_or_default();

                    let st = state_methods.borrow();
                    if let Some(layout) =
                        build_menu_layout(parent_id, recursion_depth, &prop_names, st.is_visible)
                    {
                        let out = glib::Variant::tuple_from_iter([
                            st.revision.to_variant(),
                            layout,
                        ]);
                        invocation.return_value(Some(&out));
                    } else {
                        invocation.return_dbus_error(
                            "org.freedesktop.DBus.Error.InvalidArgs",
                            "parentId not found",
                        );
                    }
                }
                "GetGroupProperties" => {
                    let ids: Vec<i32> = params.child_value(0).get::<Vec<i32>>().unwrap_or_default();
                    let prop_names: Vec<String> =
                        params.child_value(1).get::<Vec<String>>().unwrap_or_default();
                    let is_visible = state_methods.borrow().is_visible;

                    let entries: Vec<glib::Variant> = menu_items_for_visibility(is_visible)
                        .into_iter()
                        .filter(|item| ids.is_empty() || ids.contains(&item.id))
                        .map(|item| {
                            glib::Variant::tuple_from_iter([
                                item.id.to_variant(),
                                item_props_variant(&item, &prop_names),
                            ])
                        })
                        .collect();

                    let entry_ty = glib::VariantTy::new("(ia{sv})").expect("valid variant type");
                    let array = glib::Variant::array_from_iter_with_type(entry_ty, entries);
                    invocation.return_value(Some(&glib::Variant::tuple_from_iter([array])));
                }
                "GetProperty" => {
                    let id = params.child_value(0).get::<i32>().unwrap_or(-1);
                    let prop_var = params.child_value(1);
                    let prop_name = prop_var.str().unwrap_or_default();
                    let is_visible = state_methods.borrow().is_visible;

                    let found = menu_items_for_visibility(is_visible)
                        .into_iter()
                        .find(|item| item.id == id)
                        .and_then(|item| item.property_value(prop_name));

                    if let Some(val) = found {
                        let out = glib::Variant::tuple_from_iter([val.to_variant()]);
                        invocation.return_value(Some(&out));
                    } else {
                        invocation.return_dbus_error(
                            "org.freedesktop.DBus.Error.InvalidArgs",
                            "Property or id not found",
                        );
                    }
                }
                "Event" => {
                    let id = params.child_value(0).get::<i32>().unwrap_or(-1);
                    let event_var = params.child_value(1);
                    let event_id = event_var.str().unwrap_or_default();
                    if event_id == "clicked" {
                        dispatch_menu_click(id, &cb_methods);
                    }
                    invocation.return_value(None);
                }
                "EventGroup" => {
                    let events = params.child_value(0);
                    for idx in 0..events.n_children() {
                        let ev = events.child_value(idx);
                        let id = ev.child_value(0).get::<i32>().unwrap_or(-1);
                        let event_var = ev.child_value(1);
                        let event_id = event_var.str().unwrap_or_default();
                        if event_id == "clicked" {
                            dispatch_menu_click(id, &cb_methods);
                        }
                    }
                    let empty_errors = glib::Variant::array_from_fixed_array::<i32>(&[]);
                    invocation.return_value(Some(&glib::Variant::tuple_from_iter([empty_errors])));
                }
                "AboutToShow" => {
                    let out = glib::Variant::tuple_from_iter([false.to_variant()]);
                    invocation.return_value(Some(&out));
                }
                "AboutToShowGroup" => {
                    let empty_a = glib::Variant::array_from_fixed_array::<i32>(&[]);
                    let empty_b = glib::Variant::array_from_fixed_array::<i32>(&[]);
                    let out = glib::Variant::tuple_from_iter([empty_a, empty_b]);
                    invocation.return_value(Some(&out));
                }
                _ => {
                    invocation.return_dbus_error(
                        "org.freedesktop.DBus.Error.UnknownMethod",
                        "Unknown method",
                    );
                }
            },
        )
        .property(|_conn, _sender, _path, _iface, prop_name| match prop_name {
            "Version" => 3u32.to_variant(),
            "TextDirection" => "ltr".to_variant(),
            "Status" => "normal".to_variant(),
            "IconThemePath" => Vec::<String>::new().to_variant(),
            _ => "".to_variant(),
        })
        .build()
}

fn dispatch_menu_click(id: i32, callbacks: &TrayCallbacks) {
    match id {
        MENU_ID_MINIMIZE => (callbacks.on_hide)(),
        MENU_ID_OPEN => (callbacks.on_show)(),
        MENU_ID_ABOUT => (callbacks.on_about)(),
        MENU_ID_QUIT => (callbacks.on_quit)(),
        _ => {}
    }
}

struct MenuItemSpec {
    id: i32,
    label: Option<&'static str>,
    is_separator: bool,
}

impl MenuItemSpec {
    fn property_value(&self, name: &str) -> Option<glib::Variant> {
        match name {
            "type" if self.is_separator => Some("separator".to_variant()),
            "label" => self.label.map(|l| l.to_variant()),
            "enabled" => Some(true.to_variant()),
            "visible" => Some(true.to_variant()),
            _ => None,
        }
    }

    fn all_props(&self, filter: &[String]) -> HashMap<String, glib::Variant> {
        let mut map = HashMap::new();
        let include = |k: &str| filter.is_empty() || filter.iter().any(|f| f == k);

        if self.is_separator {
            if include("type") {
                map.insert("type".to_string(), "separator".to_variant());
            }
        } else if let Some(label) = self.label
            && include("label")
        {
            map.insert("label".to_string(), label.to_variant());
        }
        if include("enabled") {
            map.insert("enabled".to_string(), true.to_variant());
        }
        if include("visible") {
            map.insert("visible".to_string(), true.to_variant());
        }
        map
    }
}

fn menu_items_for_visibility(is_visible: bool) -> Vec<MenuItemSpec> {
    let mut items = Vec::with_capacity(7);
    if is_visible {
        items.push(MenuItemSpec {
            id: MENU_ID_MINIMIZE,
            label: Some("Minimize to tray"),
            is_separator: false,
        });
        items.push(MenuItemSpec {
            id: MENU_ID_SEP_TOP,
            label: None,
            is_separator: true,
        });
    }
    items.push(MenuItemSpec {
        id: MENU_ID_OPEN,
        label: Some("Open Calendar"),
        is_separator: false,
    });
    items.push(MenuItemSpec {
        id: MENU_ID_SEP_MID,
        label: None,
        is_separator: true,
    });
    items.push(MenuItemSpec {
        id: MENU_ID_ABOUT,
        label: Some("About GnomeGCal"),
        is_separator: false,
    });
    items.push(MenuItemSpec {
        id: MENU_ID_SEP_BOT,
        label: None,
        is_separator: true,
    });
    items.push(MenuItemSpec {
        id: MENU_ID_QUIT,
        label: Some("Quit"),
        is_separator: false,
    });
    items
}

fn item_props_variant(item: &MenuItemSpec, filter: &[String]) -> glib::Variant {
    item.all_props(filter).to_variant()
}

fn build_menu_layout(
    parent_id: i32,
    recursion_depth: i32,
    property_names: &[String],
    is_visible: bool,
) -> Option<glib::Variant> {
    let items = menu_items_for_visibility(is_visible);
    let empty_children = || {
        glib::Variant::array_from_iter_with_type(
            glib::VariantTy::VARIANT,
            std::iter::empty::<glib::Variant>(),
        )
    };

    if parent_id == 0 {
        let mut root_props = HashMap::new();
        if property_names.is_empty()
            || property_names.iter().any(|p| p == "children-display")
        {
            root_props.insert("children-display".to_string(), "submenu".to_variant());
        }
        let children = if recursion_depth == 0 {
            empty_children()
        } else {
            let child_nodes: Vec<glib::Variant> = items
                .iter()
                .map(|item| {
                    let node = glib::Variant::tuple_from_iter([
                        item.id.to_variant(),
                        item_props_variant(item, property_names),
                        empty_children(),
                    ]);
                    node.to_variant()
                })
                .collect();
            glib::Variant::array_from_iter_with_type(glib::VariantTy::VARIANT, child_nodes)
        };
        return Some(glib::Variant::tuple_from_iter([
            0i32.to_variant(),
            root_props.to_variant(),
            children,
        ]));
    }

    items.iter().find(|item| item.id == parent_id).map(|item| {
        glib::Variant::tuple_from_iter([
            item.id.to_variant(),
            item_props_variant(item, property_names),
            empty_children(),
        ])
    })
}

pub fn empty_pixmap_variant() -> glib::Variant {
    let item_ty = glib::VariantTy::new("(iiay)").expect("valid pixmap type");
    glib::Variant::array_from_iter_with_type(item_ty, std::iter::empty::<glib::Variant>())
}

pub fn icon_pixmap_variant(icons: &[(i32, i32, Vec<u8>)]) -> glib::Variant {
    let item_ty = glib::VariantTy::new("(iiay)").expect("valid pixmap type");
    let entries = icons.iter().map(|(w, h, data)| {
        glib::Variant::tuple_from_iter([
            w.to_variant(),
            h.to_variant(),
            glib::Variant::array_from_fixed_array(data),
        ])
    });
    glib::Variant::array_from_iter_with_type(item_ty, entries)
}

pub fn tooltip_variant() -> glib::Variant {
    glib::Variant::tuple_from_iter([
        "".to_variant(),
        empty_pixmap_variant(),
        "GnomeGCal".to_variant(),
        "Google Calendar, Tasks & Keep".to_variant(),
    ])
}

fn current_local_day() -> i32 {
    glib::DateTime::now_local()
        .map(|dt| dt.day_of_month())
        .unwrap_or(19)
}

pub fn seconds_until_next_midnight(now: &glib::DateTime) -> u32 {
    let elapsed = now.hour() * 3600 + now.minute() * 60 + now.second();
    (86_402 - elapsed).clamp(1, 86_405) as u32
}

fn build_icon_set(day: i32) -> Vec<(i32, i32, Vec<u8>)> {
    vec![
        generate_tray_icon_for_day(24, day),
        generate_tray_icon_for_day(48, day),
    ]
}

/// Renders a rounded calendar badge for `day` and converts Cairo's premultiplied
/// native-endian ARGB32 surface into SNI's un-premultiplied network-byte-order `[A, R, G, B]`.
pub fn generate_tray_icon_for_day(size: i32, day: i32) -> (i32, i32, Vec<u8>) {
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, size, size)
        .expect("Failed to create Cairo surface");
    {
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

        let day_str = day.to_string();
        let font_size = if day < 10 { 26.5 * scale } else { 24.5 * scale };

        cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
        cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
        cr.set_font_size(font_size);

        if let Ok(extents) = cr.text_extents(&day_str) {
            let text_x = (size as f64 - extents.width()) / 2.0 - extents.x_bearing();
            let text_y = (size as f64 - extents.height()) / 2.0 - extents.y_bearing();
            cr.move_to(text_x, text_y);
            cr.show_text(&day_str).ok();
        }
    }

    surface.flush();
    let data = surface.take_data().expect("Failed to get surface data");
    let mut argb_data = Vec::with_capacity(data.len());

    for chunk in data.as_chunks::<4>().0 {
        let px = u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        let a = ((px >> 24) & 0xff) as u8;
        let r_pre = ((px >> 16) & 0xff) as u8;
        let g_pre = ((px >> 8) & 0xff) as u8;
        let b_pre = (px & 0xff) as u8;

        match a {
            0 => argb_data.extend_from_slice(&[0, 0, 0, 0]),
            255 => argb_data.extend_from_slice(&[255, r_pre, g_pre, b_pre]),
            _ => {
                let unpre = |c: u8| -> u8 {
                    ((c as u32 * 255 + (a as u32 / 2)) / a as u32).min(255) as u8
                };
                argb_data.extend_from_slice(&[a, unpre(r_pre), unpre(g_pre), unpre(b_pre)]);
            }
        }
    }

    (size, size, argb_data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_valid_unpremultiplied_argb_icon() {
        let (w, h, data) = generate_tray_icon_for_day(48, 26);
        assert_eq!(w, 48);
        assert_eq!(h, 48);
        assert_eq!(data.len(), 48 * 48 * 4);

        // Corners outside rounded rect must be transparent
        assert_eq!(&data[0..4], &[0, 0, 0, 0]);

        // Center pixel should be opaque (either white digit or blue badge)
        let center = (24 * 48 + 24) * 4;
        assert_eq!(data[center], 255);
    }

    #[test]
    fn dbus_variant_signatures_match_sni_and_dbusmenu_specs() {
        let icons = build_icon_set(7);
        let pixmap = icon_pixmap_variant(&icons);
        assert_eq!(pixmap.type_().as_str(), "a(iiay)");
        assert_eq!(empty_pixmap_variant().type_().as_str(), "a(iiay)");

        let tooltip = tooltip_variant();
        assert_eq!(tooltip.type_().as_str(), "(sa(iiay)ss)");

        let layout = build_menu_layout(0, -1, &[], true).expect("root layout");
        let full_reply = glib::Variant::tuple_from_iter([1u32.to_variant(), layout]);
        assert_eq!(full_reply.type_().as_str(), "(u(ia{sv}av))");
    }

    #[test]
    fn midnight_timer_computes_exact_delay() {
        let tz = glib::TimeZone::utc();
        let late = glib::DateTime::new(&tz, 2026, 11, 26, 23, 59, 59.0).unwrap();
        assert_eq!(seconds_until_next_midnight(&late), 3);

        let midnight = glib::DateTime::new(&tz, 2026, 11, 26, 0, 0, 2.0).unwrap();
        assert_eq!(seconds_until_next_midnight(&midnight), 86_400);
    }
}
