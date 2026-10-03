//! Nexus-style building blocks: pages, groups and option rows.

use crate::{cmd, paths};
use gtk::pango;
use gtk::prelude::*;
use std::path::PathBuf;
use std::rc::Rc;

// ---------- Page / group ----------

pub struct Page {
    pub root: gtk::ScrolledWindow,
    pub body: gtk::Box,
}

pub fn page(section: &str, title: &str, description: &str, files: &[PathBuf]) -> Page {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.add_css_class("settings-page");
    body.add_css_class(&format!("page-{section}"));

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    header.add_css_class("section-header");
    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.set_hexpand(true);
    let t = gtk::Label::new(Some(title));
    t.add_css_class("section-title");
    t.set_xalign(0.0);
    let d = gtk::Label::new(Some(description));
    d.add_css_class("section-description");
    d.set_xalign(0.0);
    d.set_wrap(true);
    d.set_visible(!description.is_empty());
    text.append(&t);
    text.append(&d);
    header.append(&text);
    if !files.is_empty() {
        header.append(&open_config_button(files));
    }
    body.append(&header);

    body.set_hexpand(true);

    let root = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        // Scrolls with wheel, trackpad and keyboard; no visible scrollbar.
        .vscrollbar_policy(gtk::PolicyType::External)
        .child(&body)
        .vexpand(true)
        .build();
    Page { root, body }
}

impl Page {
    /// For pages whose content scrolls itself (tables): the page stops scrolling
    /// and hands its height to the last child.
    pub fn fill(&self) {
        self.root.set_vscrollbar_policy(gtk::PolicyType::Never);
        self.body.set_vexpand(true);
        self.body.add_css_class("fill");
    }

    /// No header and no padding: the page is all content (the player).
    pub fn bare(&self) {
        if let Some(header) = self.body.first_child() {
            header.set_visible(false);
        }
        self.body.add_css_class("bare");
    }

    pub fn group(&self, title: &str) -> Group {
        let wrapper = gtk::Box::new(gtk::Orientation::Vertical, 0);
        wrapper.add_css_class("settings-group");
        if !title.is_empty() {
            let l = gtk::Label::new(Some(&title.to_uppercase()));
            l.add_css_class("group-title");
            l.set_xalign(0.0);
            wrapper.append(&l);
        }
        let list = gtk::Box::new(gtk::Orientation::Vertical, 6);
        wrapper.append(&list);
        self.body.append(&wrapper);
        Group { wrapper, list }
    }
}

pub fn banner(text: &str, warning: bool) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    b.add_css_class("banner");
    if warning {
        b.add_css_class("warning");
    }
    let icon = gtk::Image::from_icon_name(if warning { "dialog-warning-symbolic" } else { "dialog-information-symbolic" });
    icon.set_valign(gtk::Align::Start);
    let l = gtk::Label::new(None);
    l.set_markup(text);
    l.set_wrap(true);
    l.set_xalign(0.0);
    l.set_hexpand(true);
    b.append(&icon);
    b.append(&l);
    b
}

#[derive(Clone)]
pub struct Group {
    pub wrapper: gtk::Box,
    pub list: gtk::Box,
}

impl Group {
    pub fn add(&self, w: &impl IsA<gtk::Widget>) {
        self.list.append(w);
    }

    pub fn note(&self, text: &str) {
        let l = gtk::Label::new(None);
        l.set_markup(text);
        l.add_css_class("group-note");
        l.set_xalign(0.0);
        l.set_wrap(true);
        // Notes sit just under the group title.
        self.wrapper.insert_child_after(&l, self.wrapper.first_child().as_ref());
    }
}

// ---------- Open config ----------

pub fn open_config_button(files: &[PathBuf]) -> gtk::Widget {
    let make_content = || {
        let b = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        b.append(&gtk::Image::from_icon_name("text-editor-symbolic"));
        b.append(&gtk::Label::new(Some("Open config")));
        b
    };
    if files.len() == 1 {
        let path = files[0].clone();
        let button = gtk::Button::new();
        button.set_child(Some(&make_content()));
        button.add_css_class("open-config");
        button.set_valign(gtk::Align::Center);
        button.set_tooltip_text(Some(&paths::pretty(&path)));
        button.connect_clicked(move |_| cmd::open_in_editor(&path));
        return button.upcast();
    }
    let menu = gtk::MenuButton::new();
    menu.set_child(Some(&make_content()));
    menu.add_css_class("open-config");
    menu.set_valign(gtk::Align::Center);
    let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let popover = gtk::Popover::new();
    for path in files {
        let b = gtk::Button::with_label(&paths::pretty(path));
        b.add_css_class("flat");
        if let Some(label) = b.child().and_downcast::<gtk::Label>() {
            label.set_xalign(0.0);
            label.add_css_class("mono");
        }
        let p = path.clone();
        let pop = popover.clone();
        b.connect_clicked(move |_| {
            pop.popdown();
            cmd::open_in_editor(&p);
        });
        list.append(&b);
    }
    popover.set_child(Some(&list));
    menu.set_popover(Some(&popover));
    menu.upcast()
}

// ---------- Rows ----------

/// An option card: title and description on the left, control on the right.
pub fn row(title: &str, desc: &str, control: Option<&gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    row.add_css_class("settings-option");
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_valign(gtk::Align::Center);
    text.set_hexpand(true);
    let t = gtk::Label::new(Some(title));
    t.add_css_class("settings-option-title");
    t.set_xalign(0.0);
    t.set_wrap(true);
    text.append(&t);
    if !desc.is_empty() {
        let d = gtk::Label::new(None);
        d.set_markup(desc);
        d.add_css_class("settings-option-description");
        d.set_xalign(0.0);
        d.set_wrap(true);
        d.set_wrap_mode(pango::WrapMode::WordChar);
        text.append(&d);
    }
    row.append(&text);
    if let Some(c) = control {
        c.set_valign(gtk::Align::Center);
        row.append(c);
    }
    row
}

pub fn switch_row(title: &str, desc: &str, active: bool, on_change: impl Fn(bool) + 'static) -> (gtk::Box, gtk::Switch) {
    let sw = gtk::Switch::new();
    sw.set_active(active);
    sw.connect_active_notify(move |s| on_change(s.is_active()));
    let r = row(title, desc, Some(sw.upcast_ref()));
    (r, sw)
}

pub fn dropdown(options: &[(String, String)], current: &str) -> gtk::DropDown {
    let labels: Vec<&str> = options.iter().map(|(_, l)| l.as_str()).collect();
    let dd = gtk::DropDown::from_strings(&labels);
    if let Some(i) = options.iter().position(|(id, _)| id == current) {
        dd.set_selected(i as u32);
    } else {
        dd.set_selected(gtk::INVALID_LIST_POSITION);
    }
    dd
}

pub fn opts(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
}

pub fn choice_row(
    title: &str,
    desc: &str,
    options: Vec<(String, String)>,
    current: &str,
    on_change: impl Fn(String) + 'static,
) -> (gtk::Box, gtk::DropDown) {
    let dd = dropdown(&options, current);
    dd.connect_selected_notify(move |d| {
        if let Some((id, _)) = options.get(d.selected() as usize) {
            on_change(id.clone());
        }
    });
    let r = row(title, desc, Some(dd.upcast_ref()));
    (r, dd)
}

pub fn button_row(title: &str, desc: &str, label: &str, on_click: impl Fn(&gtk::Button) + 'static) -> (gtk::Box, gtk::Button) {
    let b = gtk::Button::with_label(label);
    b.connect_clicked(on_click);
    let r = row(title, desc, Some(b.upcast_ref()));
    (r, b)
}

pub fn hbox(spacing: i32) -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Horizontal, spacing)
}

pub fn vbox(spacing: i32) -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Vertical, spacing)
}

pub fn label(text: &str, class: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    if !class.is_empty() {
        l.add_css_class(class);
    }
    l.set_xalign(0.0);
    l
}

/// A button that needs two clicks: the first arms it ("Click again to …"),
/// the second acts. It disarms itself after a few seconds.
pub fn two_click(label_text: &str, armed_text: &str, act: impl Fn() + 'static) -> gtk::Button {
    let b = gtk::Button::with_label(label_text);
    b.add_css_class("destructive-action");
    let armed = Rc::new(std::cell::Cell::new(false));
    let (idle, armed_label) = (label_text.to_string(), armed_text.to_string());
    b.connect_clicked(move |b| {
        if armed.get() {
            armed.set(false);
            b.set_label(&idle);
            act();
            return;
        }
        armed.set(true);
        b.set_label(&armed_label);
        let (b2, armed2, idle2) = (b.clone(), armed.clone(), idle.clone());
        gtk::glib::timeout_add_local_once(std::time::Duration::from_secs(4), move || {
            if armed2.get() {
                armed2.set(false);
                b2.set_label(&idle2);
            }
        });
    });
    b
}

/// A modal card dialog in the app's style. Returns the window and its content box.
pub fn dialog(title: &str, width: i32) -> (gtk::Window, gtk::Box) {
    let dialog = gtk::Window::builder().modal(true).title(title).default_width(width).build();
    if let Some(parent) = crate::window::window() {
        dialog.set_transient_for(Some(&parent));
    }
    dialog.add_css_class("pdf-window");
    dialog.set_titlebar(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
    let card = vbox(12);
    card.add_css_class("dialog-card");
    card.append(&label(title, "section-title"));
    dialog.set_child(Some(&card));
    let keys = gtk::EventControllerKey::new();
    let d = dialog.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            d.close();
            return gtk::glib::Propagation::Stop;
        }
        gtk::glib::Propagation::Proceed
    });
    dialog.add_controller(keys);
    (dialog, card)
}

/// Buttons joined into one control; exactly one is selected.
pub fn segmented(options: &[(String, String)], current: &str, on_change: impl Fn(String) + 'static) -> gtk::Box {
    let bx = hbox(0);
    bx.add_css_class("segmented");
    bx.set_valign(gtk::Align::Center);
    let mut first: Option<gtk::ToggleButton> = None;
    let on_change = Rc::new(on_change);
    for (id, label) in options {
        let b = gtk::ToggleButton::with_label(label);
        b.add_css_class("segment");
        if let Some(f) = &first {
            b.set_group(Some(f));
        } else {
            first = Some(b.clone());
        }
        b.set_active(id == current);
        let id = id.clone();
        let cb = on_change.clone();
        b.connect_toggled(move |b| {
            if b.is_active() {
                cb(id.clone());
            }
        });
        bx.append(&b);
    }
    bx
}

pub fn segmented_row(
    title: &str,
    desc: &str,
    options: Vec<(String, String)>,
    current: &str,
    on_change: impl Fn(String) + 'static,
) -> gtk::Box {
    let seg = segmented(&options, current, on_change);
    row(title, desc, Some(seg.upcast_ref()))
}

/// A button with an icon and a label.
pub fn labeled_button(icon: &str, text: &str) -> gtk::Button {
    let b = gtk::Button::new();
    let c = hbox(8);
    c.append(&gtk::Image::from_icon_name(icon));
    c.append(&gtk::Label::new(Some(text)));
    b.set_child(Some(&c));
    b
}

/// A centred message for empty views, with an optional action.
pub type Action<'a> = Option<(&'a str, Box<dyn Fn()>)>;

pub fn empty_state(icon: &str, title: &str, desc: &str, action: Action<'_>) -> gtk::Box {
    let b = vbox(8);
    b.add_css_class("empty-state");
    b.set_valign(gtk::Align::Center);
    b.set_halign(gtk::Align::Center);
    b.set_vexpand(true);
    let i = gtk::Image::from_icon_name(icon);
    i.set_pixel_size(48);
    i.add_css_class("dim");
    b.append(&i);
    let t = gtk::Label::new(Some(title));
    t.add_css_class("empty-title");
    b.append(&t);
    if !desc.is_empty() {
        let d = gtk::Label::new(None);
        d.set_markup(desc);
        d.add_css_class("dim");
        d.set_wrap(true);
        d.set_justify(gtk::Justification::Center);
        d.set_max_width_chars(52);
        b.append(&d);
    }
    if let Some((label, act)) = action {
        let btn = gtk::Button::with_label(label);
        btn.add_css_class("suggested-action");
        btn.set_halign(gtk::Align::Center);
        btn.set_margin_top(8);
        btn.connect_clicked(move |_| act());
        b.append(&btn);
    }
    b
}

/// Ask for a line of text in a small dialog ("Not now" first, then the action).
pub fn ask_text(title: &str, desc: &str, initial: &str, confirm: &str, on_ok: impl Fn(String) + 'static) -> gtk::Box {
    let (dialog, card) = dialog(title, 420);
    if !desc.is_empty() {
        let d = label(desc, "dim");
        d.set_wrap(true);
        card.append(&d);
    }
    let entry = gtk::Entry::new();
    entry.set_text(initial);
    entry.set_activates_default(false);
    card.append(&entry);
    let extra = vbox(6);
    card.append(&extra);
    let buttons = hbox(8);
    buttons.set_halign(gtk::Align::End);
    buttons.set_margin_top(6);
    let cancel = gtk::Button::with_label("Not now");
    let ok = gtk::Button::with_label(confirm);
    ok.add_css_class("suggested-action");
    buttons.append(&cancel);
    buttons.append(&ok);
    card.append(&buttons);
    let d = dialog.clone();
    cancel.connect_clicked(move |_| d.close());
    let on_ok = Rc::new(on_ok);
    let submit = {
        let (d, e) = (dialog.clone(), entry.clone());
        move || {
            let text = e.text().trim().to_string();
            if text.is_empty() {
                e.add_css_class("error");
                return;
            }
            d.close();
            on_ok(text);
        }
    };
    let s2 = submit.clone();
    ok.connect_clicked(move |_| s2());
    entry.connect_activate(move |_| submit());
    dialog.present();
    entry.grab_focus();
    entry.select_region(0, -1);
    extra
}

/// Rows of key caps for the keyboard help.
pub fn key_caps(keys: &[&str]) -> gtk::Box {
    let caps = hbox(4);
    for (i, k) in keys.iter().enumerate() {
        if i > 0 {
            caps.append(&label("+", "dim"));
        }
        caps.append(&label(k, "key-cap"));
    }
    caps
}

/// Pack a flow box's children tightly: as many columns as fit `scroll`'s width, each
/// `cell_w` pixels wide, left-aligned, re-flowed whenever the window is resized.
pub fn pack_flow(flow: &gtk::FlowBox, scroll: &gtk::ScrolledWindow, cell_w: i32) {
    flow.set_halign(gtk::Align::Start);
    flow.set_homogeneous(true);
    let fit = {
        let flow = flow.clone();
        move |width: f64| {
            let cols = ((width / f64::from(cell_w)).floor() as u32).clamp(1, 24);
            flow.set_min_children_per_line(cols);
            flow.set_max_children_per_line(cols);
        }
    };
    fit(900.0);
    let f = fit.clone();
    scroll.hadjustment().connect_page_size_notify(move |a| {
        if a.page_size() > 1.0 {
            f(a.page_size());
        }
    });
}
