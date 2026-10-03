//! Recent files: a grid of cards with a picture of page one and where you left off.

use crate::widgets::{self, Page};
use crate::{doc, fmt, paths, recent};
use gtk::prelude::*;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

const COVER_W: i32 = 190;

thread_local! {
    static GRID: RefCell<Option<gtk::FlowBox>> = const { RefCell::new(None) };
    static QUERY: RefCell<String> = const { RefCell::new(String::new()) };
    static PAGE: RefCell<Option<gtk::Stack>> = const { RefCell::new(None) };
}

pub fn set_query(q: &str) {
    QUERY.with(|s| *s.borrow_mut() = q.to_lowercase());
    refresh();
}

/// Delete the cached covers.
pub fn clear_covers() {
    let dir = paths::covers_dir();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let _ = std::fs::remove_file(e.path());
        }
    }
    refresh();
}

pub fn build(page: &Page) {
    let top = widgets::hbox(8);
    top.add_css_class("pages-toolbar");
    let open = widgets::labeled_button("document-open-symbolic", "Open a file");
    open.add_css_class("suggested-action");
    open.connect_clicked(|_| doc::open_dialog());
    let merge = widgets::labeled_button("list-add-symbolic", "Combine PDFs");
    merge.set_tooltip_text(Some("Join several PDFs into a new file"));
    merge.connect_clicked(|_| crate::sections::pages::merge_dialog());
    top.append(&open);
    top.append(&merge);
    page.body.append(&top);

    let stack = gtk::Stack::new();
    stack.set_vexpand(true);
    let flow = gtk::FlowBox::new();
    flow.add_css_class("library-grid");
    flow.set_selection_mode(gtk::SelectionMode::None);
    flow.set_valign(gtk::Align::Start);
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .vexpand(true)
        .child(&flow)
        .build();
    widgets::pack_flow(&flow, &scroll, COVER_W + 28);
    stack.add_named(&scroll, Some("grid"));
    stack.add_named(
        &widgets::empty_state(
            "npdf-library-symbolic",
            "No recent files",
            "PDFs you open show up here, with the page you were on.",
            Some(("Open a file", Box::new(doc::open_dialog))),
        ),
        Some("empty"),
    );
    stack.add_named(&widgets::empty_state("system-search-symbolic", "No matching files", "Nothing in your recent files matches that search.", None), Some("none"));
    page.body.append(&stack);
    GRID.with(|g| *g.borrow_mut() = Some(flow));
    PAGE.with(|p| *p.borrow_mut() = Some(stack));
    refresh();
    // The list changes when files open, close or save under a new name.
    doc::subscribe(&page.root, |c| {
        if matches!(c, doc::Change::Opened | doc::Change::Closed | doc::Change::Dirty | doc::Change::Page) {
            glib_idle_refresh();
        }
    });
}

fn glib_idle_refresh() {
    gtk::glib::idle_add_local_once(refresh);
}

fn cover_file(path: &Path, mtime: u64) -> PathBuf {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&(path, mtime), &mut h);
    paths::covers_dir().join(format!("{:016x}.png", std::hash::Hasher::finish(&h)))
}

fn mtime(path: &Path) -> u64 {
    std::fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
}

fn refresh() {
    let (Some(flow), Some(stack)) = (GRID.with(|g| g.borrow().clone()), PAGE.with(|p| p.borrow().clone())) else { return };
    while let Some(c) = flow.first_child() {
        flow.remove(&c);
    }
    let all = recent::list();
    let q = QUERY.with(|q| q.borrow().clone());
    let shown: Vec<_> = all
        .iter()
        .filter(|r| q.is_empty() || r.path.to_string_lossy().to_lowercase().contains(&q))
        .cloned()
        .collect();
    stack.set_visible_child_name(if all.is_empty() { "empty" } else if shown.is_empty() { "none" } else { "grid" });
    for r in shown {
        let child = gtk::FlowBoxChild::new();
        child.set_focusable(false);
        child.set_halign(gtk::Align::Start);
        child.set_child(Some(&card(r)));
        flow.append(&child);
    }
}

fn card(r: recent::Recent) -> gtk::Button {
    let exists = r.path.is_file();
    let button = gtk::Button::new();
    button.add_css_class("pdf-card");
    let body = widgets::vbox(0);
    let cover = gtk::Picture::new();
    cover.add_css_class("cover");
    cover.set_can_shrink(true);
    cover.set_content_fit(gtk::ContentFit::Cover);
    cover.set_size_request(COVER_W, (f64::from(COVER_W) * 1.3) as i32);
    body.append(&cover);
    let info = widgets::vbox(2);
    info.add_css_class("card-body");
    let name = r.path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let title = widgets::label(&name, "card-title");
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.set_max_width_chars(20);
    info.append(&title);
    let dir = r.path.parent().map(paths::pretty).unwrap_or_default();
    let place = widgets::label(&dir, "card-meta");
    place.add_css_class("mono");
    place.set_ellipsize(gtk::pango::EllipsizeMode::Start);
    place.set_max_width_chars(24);
    info.append(&place);
    let meta = if !exists {
        "File not found".to_string()
    } else if r.pages > 0 {
        format!("Page {} of {}", (r.page + 1).min(r.pages), r.pages)
    } else {
        fmt::count(r.pages, "page", "pages")
    };
    info.append(&widgets::label(&meta, "card-meta"));
    if exists && r.pages > 1 {
        let bar = gtk::ProgressBar::new();
        bar.add_css_class("card-progress");
        bar.set_fraction(((r.page + 1) as f64 / r.pages as f64).min(1.0));
        bar.set_margin_top(4);
        info.append(&bar);
    }
    body.append(&info);
    button.set_child(Some(&body));
    button.set_tooltip_text(Some(&paths::pretty(&r.path)));
    if exists {
        load_cover(&cover, &r.path);
    } else {
        cover.add_css_class("dim");
    }
    let path = r.path.clone();
    button.connect_clicked(move |_| {
        if path.is_file() {
            doc::request_open(path.clone(), None);
        } else {
            crate::window::toast("That file isn't there any more.");
        }
    });

    // Right-click or long press: take it off the list.
    let menu = gtk::Popover::new();
    menu.set_parent(&button);
    menu.add_css_class("menu-popover");
    let forget = widgets::two_click("Remove from list", "Click again to remove", {
        let (path, menu) = (r.path.clone(), menu.clone());
        move || {
            recent::forget(&path);
            menu.popdown();
            refresh();
        }
    });
    let holder = widgets::vbox(4);
    holder.append(&forget);
    holder.append(&widgets::label("The file itself isn't touched.", "dim"));
    menu.set_child(Some(&holder));
    let click = gtk::GestureClick::new();
    click.set_button(gtk::gdk::BUTTON_SECONDARY);
    let m2 = menu.clone();
    click.connect_pressed(move |_, _, _, _| m2.popup());
    button.add_controller(click);
    let m3 = menu.clone();
    button.connect_destroy(move |_| m3.unparent());
    button
}

/// Show page one: from the cache when it's there, otherwise draw it and keep it.
fn load_cover(picture: &gtk::Picture, path: &Path) {
    let file = cover_file(path, mtime(path));
    if file.is_file() {
        picture.set_filename(Some(&file));
        return;
    }
    let (path, pic, out) = (path.to_path_buf(), picture.clone(), file);
    let weak = pic.downgrade();
    doc::background(
        move || {
            use gtk::gio::prelude::FileExt;
            let uri = gtk::gio::File::for_path(&path).uri();
            let pdf = poppler::Document::from_file(&uri, None).ok()?;
            let page = pdf.page(0)?;
            let (w, h) = page.size();
            let scale = f64::from(COVER_W * 2) / w.max(1.0);
            let (pw, ph) = ((w * scale).ceil() as i32, (h * scale).ceil() as i32);
            let surface = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, pw, ph).ok()?;
            {
                let cr = gtk::cairo::Context::new(&surface).ok()?;
                cr.set_source_rgb(1.0, 1.0, 1.0);
                cr.paint().ok()?;
                cr.scale(scale, scale);
                page.render(&cr);
            }
            std::fs::create_dir_all(out.parent()?).ok()?;
            let tmp = out.with_extension("tmp");
            let mut f = std::fs::File::create(&tmp).ok()?;
            surface.write_to_png(&mut f).ok()?;
            std::fs::rename(&tmp, &out).ok()?;
            Some(out)
        },
        move |res| {
            if let (Some(file), Some(p)) = (res, weak.upgrade()) {
                p.set_filename(Some(&file));
            }
        },
    );
    let _ = Rc::new(());
}
