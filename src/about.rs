use crate::config::APP_ID;
use crate::update::{UpdateStatus, RELEASES_URL};
use adw::prelude::*;
use gtk::{
    gdk, gdk_pixbuf, gio, glib, Align, Box as GtkBox, Button, CssProvider, Image, Justification,
    Label, LinkButton, Orientation, Separator, Spinner,
};
use std::cell::{Cell, OnceCell};
use std::rc::Rc;

const BUNDLED_ICON_SVG: &[u8] = include_bytes!("../assets/google-calendar.svg");
const UPDATE_COMMAND: &str =
    "curl -fsSL https://raw.githubusercontent.com/OleksiyM/GnomeGCal/main/install.sh | bash";

thread_local! {
    static ABOUT_CSS_LOADED: OnceCell<()> = const { OnceCell::new() };
}

fn ensure_about_css() {
    ABOUT_CSS_LOADED.with(|cell| {
        cell.get_or_init(|| {
            let Some(display) = gdk::Display::default() else {
                return;
            };
            let provider = CssProvider::new();
            provider.load_from_string(
                "
                .gcal-version-pill {
                    padding: 2px 10px;
                    border-radius: 999px;
                    background-color: alpha(currentColor, 0.09);
                    font-size: 0.85em;
                    font-weight: 600;
                }
                .gcal-update-box {
                    padding: 6px 12px;
                    border-radius: 10px;
                    background-color: alpha(currentColor, 0.06);
                }
                .gcal-about-link {
                    padding: 4px 6px;
                    min-height: 0;
                    min-width: 0;
                }
                ",
            );
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        });
    });
}

fn create_app_icon() -> Image {
    let bytes = glib::Bytes::from_static(BUNDLED_ICON_SVG);
    let stream = gio::MemoryInputStream::from_bytes(&bytes);
    if let Ok(pixbuf) =
        gdk_pixbuf::Pixbuf::from_stream_at_scale(&stream, 152, 152, true, None::<&gio::Cancellable>)
    {
        let texture = gdk::Texture::for_pixbuf(&pixbuf);
        return Image::builder()
            .paintable(&texture)
            .pixel_size(76)
            .halign(Align::Center)
            .margin_bottom(4)
            .build();
    }

    Image::builder()
        .icon_name(APP_ID)
        .pixel_size(76)
        .halign(Align::Center)
        .margin_bottom(4)
        .build()
}

/// Presents a compact, custom `adw::Dialog` popup card styled after GnomeMeteo.
pub fn show_about_dialog(parent: &adw::ApplicationWindow) {
    ensure_about_css();

    let alive = Rc::new(Cell::new(true));
    let copy_reset_id: Rc<std::cell::RefCell<Option<glib::SourceId>>> =
        Rc::new(std::cell::RefCell::new(None));

    let content_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(8)
        .margin_top(0)
        .margin_bottom(22)
        .margin_start(22)
        .margin_end(22)
        .halign(Align::Fill)
        .build();

    content_box.append(&create_app_icon());

    content_box.append(
        &Label::builder()
            .label("GnomeGCal")
            .css_classes(["title-2"])
            .halign(Align::Center)
            .build(),
    );

    content_box.append(
        &Label::builder()
            .label(format!("v{}", env!("CARGO_PKG_VERSION")))
            .css_classes(["gcal-version-pill", "dim-label"])
            .halign(Align::Center)
            .build(),
    );

    // Live GitHub release update check block
    let update_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .halign(Align::Center)
        .margin_top(2)
        .margin_bottom(2)
        .build();

    let status_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .halign(Align::Center)
        .valign(Align::Center)
        .build();

    let spinner = Spinner::builder()
        .spinning(true)
        .valign(Align::Center)
        .build();
    let status_icon = Image::builder()
        .visible(false)
        .pixel_size(14)
        .valign(Align::Center)
        .build();
    let status_label = Label::builder()
        .label("Checking for updates…")
        .css_classes(["dim-label", "caption"])
        .valign(Align::Center)
        .build();

    status_row.append(&spinner);
    status_row.append(&status_icon);
    status_row.append(&status_label);
    update_box.append(&status_row);

    let action_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(4)
        .halign(Align::Center)
        .valign(Align::Center)
        .visible(false)
        .build();

    let releases_btn = LinkButton::with_label(RELEASES_URL, "Releases");
    releases_btn.add_css_class("gcal-about-link");

    let copy_btn = Button::builder()
        .icon_name("edit-copy-symbolic")
        .tooltip_text("Copy update command")
        .css_classes(["flat", "circular"])
        .valign(Align::Center)
        .build();

    {
        let alive_copy = alive.clone();
        let reset_id_copy = copy_reset_id.clone();
        copy_btn.connect_clicked(move |btn| {
            btn.clipboard().set_text(UPDATE_COMMAND);
            btn.set_icon_name("object-select-symbolic");
            btn.add_css_class("success");
            btn.set_tooltip_text(Some("Copied!"));

            if let Some(id) = reset_id_copy.borrow_mut().take() {
                id.remove();
            }

            let btn_weak = btn.downgrade();
            let alive_timer = alive_copy.clone();
            let reset_id_timer = reset_id_copy.clone();
            let source_id = glib::timeout_add_local_once(
                std::time::Duration::from_millis(1600),
                move || {
                    reset_id_timer.borrow_mut().take();
                    if alive_timer.get()
                        && let Some(b) = btn_weak.upgrade()
                    {
                        b.set_icon_name("edit-copy-symbolic");
                        b.remove_css_class("success");
                        b.set_tooltip_text(Some("Copy update command"));
                    }
                },
            );
            *reset_id_copy.borrow_mut() = Some(source_id);
        });
    }

    action_row.append(&releases_btn);
    action_row.append(&copy_btn);
    update_box.append(&action_row);
    content_box.append(&update_box);

    {
        let alive_check = alive.clone();
        let spinner_w = spinner.downgrade();
        let icon_w = status_icon.downgrade();
        let label_w = status_label.downgrade();
        let box_w = update_box.downgrade();
        let action_w = action_row.downgrade();

        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(crate::update::check_for_update).await;
            if !alive_check.get() {
                return;
            }
            let (Some(sp), Some(ic), Some(lb), Some(ub), Some(ar)) = (
                spinner_w.upgrade(),
                icon_w.upgrade(),
                label_w.upgrade(),
                box_w.upgrade(),
                action_w.upgrade(),
            ) else {
                return;
            };

            sp.stop();
            sp.set_visible(false);

            match result {
                Ok(UpdateStatus::UpToDate) => {
                    ic.set_icon_name(Some("object-select-symbolic"));
                    ic.set_css_classes(&["success"]);
                    ic.set_visible(true);
                    lb.set_label("You are up to date");
                }
                Ok(UpdateStatus::Available(tag)) => {
                    ic.set_icon_name(Some("software-update-available-symbolic"));
                    ic.set_css_classes(&["accent"]);
                    ic.set_visible(true);
                    lb.set_label(&format!("Update available: {tag}"));
                    lb.set_css_classes(&["heading", "caption"]);
                    ub.add_css_class("gcal-update-box");
                    ar.set_visible(true);
                }
                Ok(UpdateStatus::Unknown) | Err(_) => {
                    lb.set_label("Could not check for updates");
                }
            }
        });
    }

    content_box.append(
        &Label::builder()
            .label("Dedicated Google Calendar, Tasks & Keep\ndesktop hub for GNOME")
            .justify(Justification::Center)
            .wrap(true)
            .max_width_chars(38)
            .margin_top(4)
            .build(),
    );

    content_box.append(
        &Separator::builder()
            .orientation(Orientation::Horizontal)
            .margin_top(6)
            .margin_bottom(2)
            .build(),
    );

    let links_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(2)
        .halign(Align::Center)
        .valign(Align::Center)
        .build();

    let links = [
        ("Releases", RELEASES_URL),
        ("GitHub", "https://github.com/OleksiyM/GnomeGCal"),
        ("X (@oleksiyML)", "https://x.com/OleksiyML"),
    ];
    for (idx, (label, uri)) in links.iter().enumerate() {
        if idx > 0 {
            links_box.append(
                &Label::builder()
                    .label("|")
                    .css_classes(["dim-label"])
                    .build(),
            );
        }
        let btn = LinkButton::with_label(uri, label);
        btn.add_css_class("gcal-about-link");
        links_box.append(&btn);
    }
    content_box.append(&links_box);

    content_box.append(
        &Label::builder()
            .label("© 2026 OleksiyM · MIT License\nIndependent open-source project, not affiliated with Google LLC.")
            .justify(Justification::Center)
            .css_classes(["dim-label", "caption"])
            .margin_top(4)
            .build(),
    );

    let dialog = adw::Dialog::builder()
        .title("About GnomeGCal")
        .content_width(400)
        .build();

    let header_bar = adw::HeaderBar::builder()
        .show_title(false)
        .css_classes(["flat"])
        .build();
    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header_bar);
    toolbar_view.set_content(Some(&content_box));
    dialog.set_child(Some(&toolbar_view));

    dialog.connect_closed(move |_| {
        alive.set(false);
        if let Some(id) = copy_reset_id.borrow_mut().take() {
            id.remove();
        }
    });

    dialog.present(Some(parent));
}
