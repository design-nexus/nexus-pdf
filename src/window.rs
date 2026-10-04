//! The main window: a navigation sidebar with search, and a stack of pages built
//! the first time they're shown.

use crate::sections::{self, Section};
use crate::{doc, prefs, theme, viewer, widgets};
use gtk::prelude::*;
use gtk::{gdk, glib};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

struct Ui {
    window: gtk::ApplicationWindow,
    stack: gtk::Stack,
    nav: gtk::Box,
    nav_items: HashMap<String, gtk::Button>,
    pages: HashMap<String, gtk::Widget>,
    sections: Vec<Section>,
    current: String,
    overlay: gtk::Overlay,
}

thread_local! {
    static UI: RefCell<Option<Rc<RefCell<Ui>>>> = const { RefCell::new(None) };
    static NARROW: Cell<bool> = const { Cell::new(false) };
    /// Set once the user has decided what to do about unsaved changes, so closing goes through.
    static FORCE_CLOSE: Cell<bool> = const { Cell::new(false) };
}

fn ui() -> Option<Rc<RefCell<Ui>>> {
    UI.with(|u| u.borrow().clone())
}

pub fn present(app: &gtk::Application, section: Option<&str>) {
    if let Some(ui) = ui() {
        let window = ui.borrow().window.clone();
        if let Some(s) = section {
            navigate(s);
        }
        window.present();
        return;
    }
    theme::install();
    install_icons();
    build(app);
    let start = section.map(String::from).unwrap_or_else(|| "library".to_string());
    navigate(&start);
    // Developer aid: NPDF_SNAPSHOT=/path.png renders the window to a PNG
    // (invisibly) and quits, so layouts can be checked without a visible window.
    if let Some(out) = std::env::var_os("NPDF_SNAPSHOT") {
        viewer::run_script_from_env();
        snapshot_and_quit(app, std::path::PathBuf::from(out));
        return;
    }
    if let Some(ui) = ui() {
        ui.borrow().window.present();
    }
    viewer::run_script_from_env();
    doc::offer_recovery();
    if prefs::take_broken() {
        toast("Your settings file couldn't be read, so defaults are in use. The old file is kept as settings.toml.bak.");
    }
}

/// Our own symbolic icons, for things the icon theme has no glyph for.
/// They're written to the cache once and added to the icon search path.
fn install_icons() {
    macro_rules! icons {
        ($($name:literal),* $(,)?) => {
            &[$(($name, include_str!(concat!("../data/icons/npdf-", $name, "-symbolic.svg")))),*]
        };
    }
    const ICONS: &[(&str, &str)] = icons![
        "library", "document", "pages", "select", "highlight", "underline", "strike", "ink", "note", "textbox", "edit-text",
        "sign", "fit-width", "fit-page", "thumbs", "outline", "rotate-left", "rotate-right", "layout",
    ];
    let dir = crate::paths::cache_dir().join("icons");
    for (name, svg) in ICONS {
        let file = format!("npdf-{name}-symbolic.svg");
        let path = dir.join(&file);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(*svg) {
            let _ = crate::cmd::atomic_write(&path, svg);
        }
    }
    if let Some(display) = gdk::Display::default() {
        gtk::IconTheme::for_display(&display).add_search_path(&dir);
    }
}

fn nav_button(icon: &str, title: &str, tooltip: &str) -> (gtk::Button, gtk::Label) {
    let button = gtk::Button::new();
    button.add_css_class("nav-item");
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    content.append(&gtk::Image::from_icon_name(icon));
    let l = widgets::label(title, "nav-label");
    l.set_hexpand(true);
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    content.append(&l);
    button.set_child(Some(&content));
    button.set_tooltip_text(Some(tooltip));
    (button, l)
}

fn build(app: &gtk::Application) {
    let window = gtk::ApplicationWindow::builder().application(app).title("Nexus PDF").default_width(1180).default_height(820).build();
    window.add_css_class("pdf-window");
    // No client-side titlebar: Hyprland manages the window.
    window.set_titlebar(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
    window.set_icon_name(Some(crate::APP_ID));

    let sections = sections::all();

    // ----- Sidebar -----
    let nav = gtk::Box::new(gtk::Orientation::Vertical, 0);
    nav.add_css_class("settings-navigation");
    nav.set_hexpand(false);
    let head_row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    head_row.add_css_class("nav-head");
    let heading = widgets::label("NEXUS PDF", "menu-heading");
    heading.add_css_class("compact-hide");
    heading.set_hexpand(true);
    head_row.append(&heading);
    let collapse = gtk::Button::from_icon_name("sidebar-show-symbolic");
    collapse.add_css_class("nav-collapse");
    collapse.set_tooltip_text(Some("Collapse or expand the sidebar (Ctrl+B)"));
    collapse.set_valign(gtk::Align::Center);
    collapse.connect_clicked(|_| toggle_sidebar());
    head_row.append(&collapse);
    nav.append(&head_row);

    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Search files"));
    search.add_css_class("settings-search");
    search.add_css_class("compact-hide");
    nav.append(&search);

    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let mut nav_items = HashMap::new();
    let mut last_group = "";
    for s in sections.iter() {
        if s.group != last_group {
            let g = widgets::label(&s.group.to_uppercase(), "nav-group");
            g.add_css_class("compact-hide");
            list.append(&g);
            last_group = s.group;
        }
        let (button, label) = nav_button(s.icon, s.title, s.description);
        label.add_css_class("compact-hide");
        let id = s.id;
        button.connect_clicked(move |_| navigate(id));
        list.append(&button);
        nav_items.insert(s.id.to_string(), button);
    }
    let nav_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        // Scrolls with wheel, trackpad and keyboard; no visible scrollbar.
        .vscrollbar_policy(gtk::PolicyType::External)
        .vexpand(true)
        .child(&list)
        .build();
    nav.append(&nav_scroll);

    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    footer.add_css_class("nav-footer");
    footer.add_css_class("compact-hide");
    let version = widgets::label(concat!("Nexus PDF ", env!("CARGO_PKG_VERSION")), "dim");
    version.set_hexpand(true);
    footer.append(&version);
    nav.append(&footer);

    // ----- Content -----
    let stack = gtk::Stack::new();
    stack.add_css_class("settings-content");
    stack.set_hexpand(true);
    stack.set_vexpand(true);
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_transition_duration(if prefs::get().reduce_motion { 0 } else { 160 });

    let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    body.append(&nav);
    body.append(&stack);

    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&body));
    window.set_child(Some(&overlay));

    install_keys(&window, &search);
    window.connect_fullscreened_notify(|w| apply_fullscreen(w.is_fullscreen()));
    window.connect_close_request(|w| {
        if FORCE_CLOSE.with(|f| f.get()) || !doc::all().iter().any(|d| d.dirty()) {
            return glib::Propagation::Proceed;
        }
        let w = w.clone();
        doc::guard_all(move || {
            FORCE_CLOSE.with(|f| f.set(true));
            w.close();
        });
        glib::Propagation::Stop
    });

    // Drop PDFs anywhere on the window to open them.
    let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
    drop.connect_drop(|_, value, _, _| {
        let Ok(list) = value.get::<gdk::FileList>() else { return false };
        let mut any = false;
        for f in list.files() {
            if let Some(p) = f.path() {
                let pdf = p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
                if pdf {
                    doc::request_open(p, None);
                    any = true;
                }
            }
        }
        if !any {
            toast("Only PDF files can be opened.");
        }
        any
    });
    window.add_controller(drop);
    search.connect_search_changed(|e| on_search(&e.text()));
    search.connect_stop_search(|e| e.set_text(""));

    // Narrow windows (a tiled half-screen) get an icon-only sidebar.
    let apply_width = {
        let nav = nav.clone();
        move |w: &gtk::ApplicationWindow| {
            let width = if w.width() > 0 { w.width() } else { w.default_width() };
            let narrow = width > 0 && width < 980;
            if narrow == NARROW.with(|n| n.get()) && nav.has_css_class("sized") {
                return;
            }
            nav.add_css_class("sized");
            set_narrow(narrow);
            refresh_compact();
        }
    };
    let aw = apply_width.clone();
    window.connect_default_width_notify(move |w| aw(w));
    let aw = apply_width.clone();
    window.connect_realize(move |w| aw(w));
    // Tiled windows are resized by the compositor; watch the real size too.
    let w2 = window.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(400), move || {
        apply_width(&w2);
        glib::ControlFlow::Continue
    });

    let ui = Ui { window: window.clone(), stack, nav, nav_items, pages: HashMap::new(), sections, current: String::new(), overlay };
    UI.with(|u| *u.borrow_mut() = Some(Rc::new(RefCell::new(ui))));

    // The window title follows the open document.
    let w = window.clone();
    doc::subscribe(&window, move |c| {
        if matches!(c, doc::Change::Opened | doc::Change::Closed | doc::Change::Dirty) {
            w.set_title(Some(&match doc::current() {
                Some(d) => format!("{}{} — Nexus PDF", if d.dirty() { "• " } else { "" }, d.title()),
                None => "Nexus PDF".to_string(),
            }));
        }
    });
}

/// While reading, the sidebar has its own collapsed state (icons only, by default),
/// so the page gets the room; elsewhere it follows `sidebar_collapsed`.
fn collapsed_here() -> bool {
    let p = prefs::get();
    if current() == "document" { p.sidebar_reading_collapsed } else { p.sidebar_collapsed }
}

/// The sidebar shows only icons when the window is narrow or the user collapsed it.
fn refresh_compact() {
    let Some(ui) = ui() else { return };
    let nav = ui.borrow().nav.clone();
    let compact = narrow() || collapsed_here();
    if compact {
        nav.add_css_class("compact");
    } else {
        nav.remove_css_class("compact");
    }
    set_compact_hidden(&nav, compact);
}

pub fn toggle_sidebar() {
    let reading = current() == "document";
    prefs::update(|p| {
        if reading {
            p.sidebar_reading_collapsed = !p.sidebar_reading_collapsed;
        } else {
            p.sidebar_collapsed = !p.sidebar_collapsed;
        }
    });
    refresh_compact();
}

/// Hide everything marked `compact-hide` in the sidebar when it's icon-only.
fn set_compact_hidden(root: &gtk::Box, compact: bool) {
    fn walk(w: &gtk::Widget, compact: bool) {
        if w.has_css_class("compact-hide") {
            w.set_visible(!compact);
        }
        // Icon-only: centre the icon in its pill, and the toggle in the column.
        if w.has_css_class("nav-item")
            && let Some(content) = w.downcast_ref::<gtk::Button>().and_then(|b| b.child())
        {
            content.set_halign(if compact { gtk::Align::Center } else { gtk::Align::Fill });
        }
        if w.has_css_class("nav-collapse") {
            w.set_halign(if compact { gtk::Align::Center } else { gtk::Align::End });
            w.set_hexpand(compact);
        }
        let mut child = w.first_child();
        while let Some(c) = child {
            walk(&c, compact);
            child = c.next_sibling();
        }
    }
    walk(root.upcast_ref(), compact);
}

fn install_keys(window: &gtk::ApplicationWindow, search: &gtk::SearchEntry) {
    // Capture phase: these work wherever focus is, except while typing.
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let s2 = search.clone();
    let w2 = window.clone();
    keys.connect_key_pressed(move |_, key, _, mods| {
        let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
        let shift = mods.contains(gdk::ModifierType::SHIFT_MASK);
        let alt = mods.contains(gdk::ModifierType::ALT_MASK);
        let typing = gtk::prelude::GtkWindowExt::focus(&w2).is_some_and(|f| {
            f.is::<gtk::Text>() || f.ancestor(gtk::Entry::static_type()).is_some() || f.is::<gtk::SearchEntry>() || f.is::<gtk::TextView>()
        });
        let stop = glib::Propagation::Stop;
        let k = key.to_lower();
        let in_doc = current() == "document" && doc::current().is_some();
        // Presenting: arrows, space and the page keys move through the pages; Esc leaves.
        if viewer::is_presenting() {
            match key {
                gdk::Key::Right | gdk::Key::Down | gdk::Key::space | gdk::Key::Page_Down | gdk::Key::Return | gdk::Key::n => viewer::step_page(1),
                gdk::Key::Left | gdk::Key::Up | gdk::Key::BackSpace | gdk::Key::Page_Up | gdk::Key::p => viewer::step_page(-1),
                gdk::Key::Home => viewer::goto_page(0),
                gdk::Key::End => viewer::goto_page(usize::MAX),
                gdk::Key::Escape | gdk::Key::F5 | gdk::Key::q => viewer::toggle_presenting(),
                _ => return glib::Propagation::Proceed,
            }
            return stop;
        }
        if ctrl {
            return match k {
                gdk::Key::f => {
                    if in_doc {
                        viewer::focus_search();
                    } else {
                        navigate("library");
                        s2.grab_focus();
                    }
                    stop
                }
                gdk::Key::b => {
                    toggle_sidebar();
                    stop
                }
                gdk::Key::o => {
                    doc::open_dialog();
                    stop
                }
                gdk::Key::p => {
                    doc::print();
                    stop
                }
                gdk::Key::s if shift => {
                    doc::save_as_dialog();
                    stop
                }
                gdk::Key::s => {
                    doc::save();
                    stop
                }
                gdk::Key::z if !typing && shift => {
                    doc::redo();
                    stop
                }
                gdk::Key::z if !typing => {
                    doc::undo();
                    stop
                }
                gdk::Key::y if !typing => {
                    doc::redo();
                    stop
                }
                gdk::Key::c if !typing && current() == "document" && viewer::copy_selection() => stop,
                gdk::Key::Tab | gdk::Key::ISO_Left_Tab => {
                    doc::cycle(if shift || key == gdk::Key::ISO_Left_Tab { -1 } else { 1 });
                    stop
                }
                gdk::Key::Page_Down => {
                    doc::cycle(1);
                    stop
                }
                gdk::Key::Page_Up => {
                    doc::cycle(-1);
                    stop
                }
                gdk::Key::w if doc::current().is_some() => {
                    viewer::close_file();
                    stop
                }
                gdk::Key::q | gdk::Key::w => {
                    w2.close();
                    stop
                }
                gdk::Key::plus | gdk::Key::equal | gdk::Key::KP_Add => {
                    viewer::zoom_by(1.2);
                    stop
                }
                gdk::Key::minus | gdk::Key::KP_Subtract => {
                    viewer::zoom_by(1.0 / 1.2);
                    stop
                }
                gdk::Key::_0 | gdk::Key::KP_0 => {
                    viewer::zoom_fit("fit-width");
                    stop
                }
                gdk::Key::_9 | gdk::Key::KP_9 => {
                    viewer::zoom_fit("fit-page");
                    stop
                }
                _ => glib::Propagation::Proceed,
            };
        }
        if alt && in_doc {
            match key {
                gdk::Key::Left => viewer::go_back(),
                gdk::Key::Right => viewer::go_forward(),
                _ => return glib::Propagation::Proceed,
            }
            return stop;
        }
        match key {
            gdk::Key::F9 => viewer::toggle_panel(),
            gdk::Key::F11 => toggle_fullscreen(),
            gdk::Key::F5 if in_doc => viewer::toggle_presenting(),
            gdk::Key::Escape if w2.is_fullscreen() => set_fullscreen(false),
            gdk::Key::Escape if !s2.text().is_empty() => s2.set_text(""),
            gdk::Key::Escape if !typing && current() == "document" => viewer::escape(),
            gdk::Key::Delete | gdk::Key::BackSpace if !typing && current() == "document" && viewer::delete_selected() => {}
            _ if typing || alt || !in_doc => return glib::Propagation::Proceed,
            gdk::Key::Page_Down | gdk::Key::space if !shift => viewer::step_page(1),
            gdk::Key::Page_Up => viewer::step_page(-1),
            gdk::Key::space => viewer::step_page(-1),
            gdk::Key::Home => viewer::jump(0),
            gdk::Key::End => viewer::jump(usize::MAX),
            _ => match k {
                gdk::Key::v => viewer::set_tool(viewer::Tool::Select),
                gdk::Key::h => viewer::set_tool(viewer::Tool::Highlight),
                gdk::Key::u => viewer::set_tool(viewer::Tool::Underline),
                gdk::Key::x => viewer::set_tool(viewer::Tool::Strike),
                gdk::Key::d => viewer::set_tool(viewer::Tool::Ink),
                gdk::Key::n => viewer::set_tool(viewer::Tool::Note),
                gdk::Key::t => viewer::set_tool(viewer::Tool::TextBox),
                gdk::Key::e => viewer::set_tool(viewer::Tool::EditText),
                gdk::Key::s => viewer::set_tool(viewer::Tool::Sign),
                gdk::Key::j => viewer::step_page(1),
                gdk::Key::k => viewer::step_page(-1),
                _ => return glib::Propagation::Proceed,
            },
        }
        stop
    });
    window.add_controller(keys);
}

pub fn toggle_fullscreen() {
    let on = window().is_some_and(|w| w.is_fullscreen());
    set_fullscreen(!on);
}

pub fn set_fullscreen(on: bool) {
    let Some(w) = window() else { return };
    if on {
        w.fullscreen();
    } else {
        w.unfullscreen();
    }
    apply_fullscreen(on);
}

/// Fullscreen hides the sidebar.
fn apply_fullscreen(on: bool) {
    let Some(ui) = ui() else { return };
    {
        let u = ui.borrow();
        u.nav.set_visible(!on);
        if on {
            u.window.add_css_class("fullscreen");
        } else {
            u.window.remove_css_class("fullscreen");
        }
    }
    if !on {
        viewer::on_unfullscreen();
    }
}

fn on_search(text: &str) {
    let Some(ui) = ui() else { return };
    let q = text.trim().to_string();
    if !q.is_empty() && ui.borrow().current != "library" {
        navigate("library");
    }
    sections::library::set_query(&q);
}

fn set_narrow(narrow: bool) {
    NARROW.with(|n| n.set(narrow));
    let Some(ui) = ui() else { return };
    for page in ui.borrow().pages.values() {
        mark_page(page, narrow);
    }
    viewer::set_narrow(narrow);
}

pub fn narrow() -> bool {
    NARROW.with(|n| n.get())
}

fn mark_page(page: &gtk::Widget, narrow: bool) {
    let body = page
        .downcast_ref::<gtk::ScrolledWindow>()
        .and_then(|s| s.child())
        .and_then(|v| v.first_child())
        .unwrap_or_else(|| page.clone());
    if narrow {
        body.add_css_class("narrow");
    } else {
        body.remove_css_class("narrow");
    }
}

fn ensure_built(id: &str) -> bool {
    let Some(ui) = ui() else { return false };
    if ui.borrow().pages.contains_key(id) {
        return true;
    }
    let section = {
        let u = ui.borrow();
        u.sections.iter().find(|s| s.id == id).map(|s| (s.id, s.title, s.description, (s.files)(), s.build, s.fill, s.bare))
    };
    let Some((sid, title, description, files, build, fill, bare)) = section else { return false };
    let page = widgets::page(sid, title, description, &files);
    if fill {
        page.fill();
    }
    if bare {
        page.bare();
    }
    build(&page);
    let page: gtk::Widget = page.root.upcast();
    mark_page(&page, narrow());
    let stack = ui.borrow().stack.clone();
    stack.add_named(&page, Some(id));
    ui.borrow_mut().pages.insert(id.to_string(), page);
    true
}

pub fn navigate(id: &str) {
    let Some(ui) = ui() else { return };
    let id = if ensure_built(id) { id.to_string() } else { "library".to_string() };
    if !ensure_built(&id) {
        return;
    }
    let mut u = ui.borrow_mut();
    if let Some(prev) = u.nav_items.get(&u.current) {
        prev.remove_css_class("active");
    }
    if let Some(b) = u.nav_items.get(&id) {
        b.add_css_class("active");
    }
    u.stack.set_visible_child_name(&id);
    let changed = u.current != id;
    u.current = id.clone();
    drop(u);
    prefs::update(|p| p.last_section = id);
    if changed {
        refresh_compact();
    }
}

/// Show a short message at the bottom of the window.
pub fn toast(message: &str) {
    let Some(ui) = ui() else {
        eprintln!("pdf: {message}");
        return;
    };
    let overlay = ui.borrow().overlay.clone();
    let label = gtk::Label::new(Some(message));
    label.set_wrap(true);
    label.set_max_width_chars(70);
    let bx = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    bx.add_css_class("toast");
    bx.append(&label);
    bx.set_halign(gtk::Align::Center);
    bx.set_valign(gtk::Align::End);
    overlay.add_overlay(&bx);
    glib::timeout_add_local_once(std::time::Duration::from_millis(3500), move || {
        overlay.remove_overlay(&bx);
    });
}

pub fn current() -> String {
    ui().map(|u| u.borrow().current.clone()).unwrap_or_default()
}

pub fn window() -> Option<gtk::ApplicationWindow> {
    ui().map(|u| u.borrow().window.clone())
}

fn snapshot_and_quit(app: &gtk::Application, out: std::path::PathBuf) {
    let Some(ui) = ui() else { return };
    let window = ui.borrow().window.clone();
    window.set_opacity(0.01);
    // A distinct title lets a window rule float it at a set size for screenshots.
    window.set_title(Some("Nexus PDF snapshot"));
    window.set_default_size(
        std::env::var("NPDF_SNAPSHOT_W").ok().and_then(|v| v.parse().ok()).unwrap_or(1180),
        std::env::var("NPDF_SNAPSHOT_H").ok().and_then(|v| v.parse().ok()).unwrap_or(820),
    );
    window.present();
    let app = app.clone();
    let delay = std::env::var("NPDF_SNAPSHOT_DELAY").ok().and_then(|v| v.parse().ok()).unwrap_or(2000);
    glib::timeout_add_local_once(std::time::Duration::from_millis(delay), move || {
        if let Some(child) = window.child() {
            let paintable = gtk::WidgetPaintable::new(Some(&child));
            let (w, h) = (child.width(), child.height());
            let snapshot = gtk::Snapshot::new();
            snapshot.append_color(&gdk::RGBA::BLACK, &gtk::graphene::Rect::new(0.0, 0.0, w as f32, h as f32));
            paintable.snapshot(&snapshot, w as f64, h as f64);
            if let (Some(node), Some(renderer)) = (snapshot.to_node(), window.renderer()) {
                let texture = renderer.render_texture(node, None);
                match texture.save_to_png(&out) {
                    Ok(()) => println!("snapshot {w}x{h} -> {}", out.display()),
                    Err(e) => eprintln!("snapshot failed: {e}"),
                }
            }
        }
        FORCE_CLOSE.with(|f| f.set(true));
        app.quit();
    });
}
