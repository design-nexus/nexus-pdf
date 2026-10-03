//! Organise pages: a grid of every page. Select some, then rotate, delete, move,
//! add pages from another file, or save them as a new file.

use crate::doc::{self, Change, ops};
use crate::viewer::thumbs;
use crate::widgets::{self, Page};
use crate::{fmt, paths, window};
use anyhow::Context;
use gtk::prelude::*;
use gtk::{gdk, gio};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

const TILE_W: i32 = 170;

struct State {
    stack: gtk::Stack,
    grid: gtk::FlowBox,
    count: gtk::Label,
    tiles: RefCell<Vec<gtk::Button>>,
    selected: RefCell<Vec<usize>>,
    /// The last tile clicked, for shift-click ranges.
    anchor: Cell<Option<usize>>,
    toolbar: gtk::Box,
}

thread_local! {
    static STATE: RefCell<Option<Rc<State>>> = const { RefCell::new(None) };
}

fn state() -> Option<Rc<State>> {
    STATE.with(|s| s.borrow().clone())
}

pub fn build(page: &Page) {
    let stack = gtk::Stack::new();
    stack.set_vexpand(true);
    stack.add_named(
        &widgets::empty_state(
            "npdf-pages-symbolic",
            "No file open",
            "Open a PDF to reorder, rotate, delete and add its pages.",
            Some(("Open a file", Box::new(doc::open_dialog))),
        ),
        Some("empty"),
    );

    let root = widgets::vbox(0);
    let toolbar = widgets::hbox(8);
    toolbar.add_css_class("pages-toolbar");
    toolbar.set_halign(gtk::Align::Start);
    root.append(&toolbar);
    let grid = gtk::FlowBox::new();
    grid.add_css_class("pages-grid");
    grid.set_selection_mode(gtk::SelectionMode::None);
    grid.set_valign(gtk::Align::Start);
    grid.set_row_spacing(4);
    grid.set_column_spacing(4);
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .vexpand(true)
        .child(&grid)
        .build();
    widgets::pack_flow(&grid, &scroll, TILE_W + 36);
    root.append(&scroll);
    stack.add_named(&root, Some("grid"));
    page.body.append(&stack);

    let count = widgets::label("", "dim");
    count.set_valign(gtk::Align::Center);
    let st = Rc::new(State {
        stack,
        grid,
        count,
        tiles: RefCell::new(Vec::new()),
        selected: RefCell::new(Vec::new()),
        anchor: Cell::new(None),
        toolbar,
    });
    STATE.with(|s| *s.borrow_mut() = Some(st.clone()));
    build_toolbar(&st);
    rebuild();
    doc::subscribe(&page.root, |c| {
        if matches!(c, Change::Opened | Change::Closed | Change::Structure) {
            rebuild();
        }
    });
}

fn chip(icon: &str, text: &str, tip: &str) -> gtk::Button {
    let b = widgets::labeled_button(icon, text);
    b.set_tooltip_text(Some(tip));
    b
}

fn build_toolbar(st: &Rc<State>) {
    let all = gtk::Button::with_label("Select all");
    all.connect_clicked(|_| {
        if let (Some(st), Some(d)) = (state(), doc::current()) {
            *st.selected.borrow_mut() = (0..d.n_pages()).collect();
            refresh_selection(&st);
        }
    });
    let none = gtk::Button::with_label("Clear");
    none.connect_clicked(|_| {
        if let Some(st) = state() {
            st.selected.borrow_mut().clear();
            refresh_selection(&st);
        }
    });
    let left = chip("npdf-rotate-left-symbolic", "Left", "Turn the selected pages a quarter turn left");
    left.connect_clicked(|_| rotate(-90));
    let right = chip("npdf-rotate-right-symbolic", "Right", "Turn the selected pages a quarter turn right");
    right.connect_clicked(|_| rotate(90));
    let earlier = chip("go-previous-symbolic", "Earlier", "Move the selected pages one place earlier");
    earlier.connect_clicked(|_| nudge(-1));
    let later = chip("go-next-symbolic", "Later", "Move the selected pages one place later");
    later.connect_clicked(|_| nudge(1));
    let delete = widgets::two_click("Delete", "Click again to delete", delete_selected);
    delete.set_tooltip_text(Some("Remove the selected pages (undo brings them back)"));
    let insert = chip("list-add-symbolic", "Add pages", "Add every page of another PDF after the selection");
    insert.connect_clicked(|_| insert_dialog());
    let extract = chip("document-save-as-symbolic", "Save as new file", "Save the selected pages (or all of them) as a new PDF");
    extract.connect_clicked(|_| extract_dialog());
    for w in [&all, &none] {
        st.toolbar.append(w);
    }
    st.toolbar.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    for w in [&left, &right, &earlier, &later] {
        st.toolbar.append(w);
    }
    st.toolbar.append(&delete);
    st.toolbar.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    st.toolbar.append(&insert);
    st.toolbar.append(&extract);
    st.toolbar.append(&st.count);
}

fn rebuild() {
    let Some(st) = state() else { return };
    while let Some(c) = st.grid.first_child() {
        st.grid.remove(&c);
    }
    st.tiles.borrow_mut().clear();
    st.selected.borrow_mut().clear();
    st.anchor.set(None);
    let Some(d) = doc::current() else {
        st.stack.set_visible_child_name("empty");
        return;
    };
    st.stack.set_visible_child_name("grid");
    let scale = st.grid.scale_factor().max(1);
    for i in 0..d.n_pages() {
        let (w, h) = d.page_size(i);
        let button = gtk::Button::new();
        button.add_css_class("page-tile");
        let card = widgets::vbox(4);
        let picture = gtk::Picture::new();
        picture.set_can_shrink(true);
        picture.set_content_fit(gtk::ContentFit::Fill);
        picture.set_size_request(TILE_W, (f64::from(TILE_W) * h / w.max(1.0)).round() as i32);
        let num = widgets::label(&(i + 1).to_string(), "mono");
        num.add_css_class("thumb-number");
        num.set_halign(gtk::Align::Center);
        card.append(&picture);
        card.append(&num);
        button.set_child(Some(&card));
        let pic = picture.clone();
        let ticket = thumbs::get(i, (TILE_W * scale) as u32, move |t| pic.set_paintable(Some(&t)));
        std::mem::forget(ticket);
        wire_tile(&button, i);
        let child = gtk::FlowBoxChild::new();
        child.set_focusable(false);
        child.set_halign(gtk::Align::Start);
        child.set_child(Some(&button));
        st.grid.append(&child);
        st.tiles.borrow_mut().push(button);
    }
    refresh_selection(&st);
}

fn wire_tile(button: &gtk::Button, i: usize) {
    // Click toggles; Shift-click selects a range.
    let click = gtk::GestureClick::new();
    click.set_button(gdk::BUTTON_PRIMARY);
    click.connect_released(move |g, _, _, _| {
        let Some(st) = state() else { return };
        let shift = g.current_event_state().contains(gdk::ModifierType::SHIFT_MASK);
        let mut sel = st.selected.borrow_mut();
        match (shift, st.anchor.get()) {
            (true, Some(a)) => {
                let (lo, hi) = (a.min(i), a.max(i));
                for p in lo..=hi {
                    if !sel.contains(&p) {
                        sel.push(p);
                    }
                }
            }
            _ => {
                if let Some(pos) = sel.iter().position(|&p| p == i) {
                    sel.remove(pos);
                } else {
                    sel.push(i);
                }
                st.anchor.set(Some(i));
            }
        }
        drop(sel);
        refresh_selection(&st);
    });
    button.add_controller(click);
    // Double-click opens the page.
    let dbl = gtk::GestureClick::new();
    dbl.connect_pressed(move |_, n, _, _| {
        if n == 2
            && let Some(d) = doc::current()
        {
            crate::viewer::show_page(&d, i);
        }
    });
    button.add_controller(dbl);

    // Drag the selection (or this page alone) to a new place.
    let source = gtk::DragSource::new();
    source.set_actions(gdk::DragAction::MOVE);
    source.connect_prepare(move |_, _, _| {
        let st = state()?;
        let mut moving = st.selected.borrow().clone();
        if !moving.contains(&i) {
            moving = vec![i];
        }
        moving.sort_unstable();
        let text = moving.iter().map(usize::to_string).collect::<Vec<_>>().join(",");
        Some(gdk::ContentProvider::for_value(&text.to_value()))
    });
    button.add_controller(source);
    let target = gtk::DropTarget::new(String::static_type(), gdk::DragAction::MOVE);
    let b = button.clone();
    target.connect_enter(move |_, _, _| {
        b.add_css_class("drop-before");
        gdk::DragAction::MOVE
    });
    let b = button.clone();
    target.connect_leave(move |_| b.remove_css_class("drop-before"));
    let b = button.clone();
    target.connect_drop(move |_, value, _, _| {
        b.remove_css_class("drop-before");
        let Ok(text) = value.get::<String>() else { return false };
        let moving: Vec<usize> = text.split(',').filter_map(|s| s.parse().ok()).collect();
        move_before(&moving, i)
    });
    button.add_controller(target);
}

/// Move `moving` so they sit just before page `before`. True if anything moved.
fn move_before(moving: &[usize], before: usize) -> bool {
    let Some(d) = doc::current() else { return false };
    let n = d.n_pages();
    let mut rest: Vec<usize> = (0..n).filter(|p| !moving.contains(p)).collect();
    let at = rest.iter().position(|&p| p >= before).unwrap_or(rest.len());
    for (k, &m) in moving.iter().enumerate() {
        rest.insert(at + k, m);
    }
    if rest.iter().copied().eq(0..n) {
        return false;
    }
    doc::try_edit(true, move |lo| ops::reorder(lo, &rest))
}

fn refresh_selection(st: &State) {
    let sel = st.selected.borrow();
    for (i, b) in st.tiles.borrow().iter().enumerate() {
        if sel.contains(&i) {
            b.add_css_class("selected");
        } else {
            b.remove_css_class("selected");
        }
    }
    st.count.set_text(&if sel.is_empty() { String::new() } else { format!("{} selected", fmt::count(sel.len(), "page", "pages")) });
}

/// The selected pages, sorted; every page if none is selected and `all` is set.
fn chosen(all: bool) -> Vec<usize> {
    let Some(st) = state() else { return Vec::new() };
    let mut sel = st.selected.borrow().clone();
    sel.sort_unstable();
    if sel.is_empty() && all {
        return doc::current().map(|d| (0..d.n_pages()).collect()).unwrap_or_default();
    }
    sel
}

fn need_selection(sel: &[usize]) -> bool {
    if sel.is_empty() {
        window::toast("Select some pages first.");
        return false;
    }
    true
}

fn rotate(degrees: i32) {
    let sel = chosen(false);
    if !need_selection(&sel) {
        return;
    }
    let keep = sel.clone();
    if doc::try_edit(true, move |lo| ops::rotate(lo, &sel, degrees))
        && let Some(st) = state()
    {
        *st.selected.borrow_mut() = keep;
        refresh_selection(&st);
    }
}

fn nudge(by: i32) {
    let sel = chosen(false);
    if !need_selection(&sel) {
        return;
    }
    let Some(d) = doc::current() else { return };
    let n = d.n_pages();
    let mut order: Vec<usize> = (0..n).collect();
    let ok = if by < 0 {
        sel.first().is_some_and(|&f| f > 0) && {
            for &p in &sel {
                order.swap(p, p - 1);
            }
            true
        }
    } else {
        sel.last().is_some_and(|&l| l + 1 < n) && {
            for &p in sel.iter().rev() {
                order.swap(p, p + 1);
            }
            true
        }
    };
    if !ok {
        return;
    }
    let moved: Vec<usize> = sel.iter().map(|&p| (p as i64 + i64::from(by)) as usize).collect();
    if doc::try_edit(true, move |lo| ops::reorder(lo, &order))
        && let Some(st) = state()
    {
        *st.selected.borrow_mut() = moved;
        refresh_selection(&st);
    }
}

fn delete_selected() {
    let sel = chosen(false);
    if !need_selection(&sel) {
        return;
    }
    doc::try_edit(true, move |lo| ops::delete(lo, &sel));
}

fn pdf_dialog_filters() -> gio::ListStore {
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&doc::pdf_filter());
    filters
}

fn insert_dialog() {
    let Some(d) = doc::current() else { return };
    let at = chosen(false).last().map(|&l| l + 1).unwrap_or(d.n_pages());
    let dialog = gtk::FileDialog::builder().title("Add pages from a PDF").modal(true).build();
    dialog.set_filters(Some(&pdf_dialog_filters()));
    dialog.open(window::window().as_ref(), gio::Cancellable::NONE, move |res| {
        let Some(path) = res.ok().and_then(|f| f.path()) else { return };
        let mut added = 0;
        let ok = doc::try_edit(true, |lo| {
            let mut other = lopdf::Document::load(&path).with_context(|| format!("couldn't read {}", paths::pretty(&path)))?;
            added = ops::insert_from(lo, &mut other, at)?;
            Ok(())
        });
        if ok {
            window::toast(&format!("Added {}.", fmt::count(added, "page", "pages")));
        }
    });
}

fn extract_dialog() {
    let Some(d) = doc::current() else { return };
    let pages = chosen(true);
    if pages.is_empty() {
        return;
    }
    let stem = d.path().file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "pages".into());
    let dialog = gtk::FileDialog::builder().title("Save pages as a new file").modal(true).initial_name(format!("{stem}-pages.pdf")).build();
    dialog.set_filters(Some(&pdf_dialog_filters()));
    if let Some(dir) = d.path().parent() {
        dialog.set_initial_folder(Some(&gio::File::for_path(dir)));
    }
    let source = d.gen_path();
    dialog.save(window::window().as_ref(), gio::Cancellable::NONE, move |res| {
        let Some(mut target) = res.ok().and_then(|f| f.path()) else { return };
        if target.extension().is_none() {
            target.set_extension("pdf");
        }
        let n = pages.len();
        let t2 = target.clone();
        doc::background(
            move || ops::extract(&source, &pages, &target),
            move |r| match r {
                Ok(()) => window::toast(&format!("Saved {} to {}.", fmt::count(n, "page", "pages"), paths::pretty(&t2))),
                Err(e) => window::toast(&format!("Couldn't save the pages: {e:#}")),
            },
        );
    });
}

/// Join several PDFs into one new file.
pub fn merge_dialog() {
    let (dialog, card) = widgets::dialog("Combine PDFs", 520);
    let d = widgets::label("Choose the files in the order they should appear.", "dim");
    d.set_wrap(true);
    card.append(&d);
    let files: Rc<RefCell<Vec<PathBuf>>> = Rc::new(RefCell::new(Vec::new()));
    let list = widgets::vbox(6);
    card.append(&list);
    let combine = gtk::Button::with_label("Combine");
    combine.add_css_class("suggested-action");
    combine.set_sensitive(false);

    type Render = Rc<RefCell<Option<Box<dyn Fn()>>>>;
    let render: Render = Rc::new(RefCell::new(None));
    {
        let (files, list, combine, render2) = (files.clone(), list.clone(), combine.clone(), render.clone());
        *render.borrow_mut() = Some(Box::new(move || {
            while let Some(c) = list.first_child() {
                list.remove(&c);
            }
            let n = files.borrow().len();
            combine.set_sensitive(n >= 2);
            for (i, p) in files.borrow().iter().enumerate() {
                let row = widgets::hbox(6);
                row.add_css_class("settings-option");
                let name = widgets::label(&paths::pretty(p), "mono");
                name.set_hexpand(true);
                name.set_ellipsize(gtk::pango::EllipsizeMode::Start);
                row.append(&name);
                for (icon, delta, tip) in [("go-up-symbolic", -1i64, "Earlier"), ("go-down-symbolic", 1, "Later")] {
                    let b = gtk::Button::from_icon_name(icon);
                    b.add_css_class("flat");
                    b.set_tooltip_text(Some(tip));
                    let j = i as i64 + delta;
                    b.set_sensitive(j >= 0 && (j as usize) < n);
                    let (files, render2) = (files.clone(), render2.clone());
                    b.connect_clicked(move |_| {
                        files.borrow_mut().swap(i, j as usize);
                        if let Some(r) = render2.borrow().as_ref() {
                            r();
                        }
                    });
                    row.append(&b);
                }
                let rm = gtk::Button::from_icon_name("window-close-symbolic");
                rm.add_css_class("flat");
                rm.set_tooltip_text(Some("Remove"));
                let (files, render2) = (files.clone(), render2.clone());
                rm.connect_clicked(move |_| {
                    files.borrow_mut().remove(i);
                    if let Some(r) = render2.borrow().as_ref() {
                        r();
                    }
                });
                row.append(&rm);
                list.append(&row);
            }
        }));
    }
    let rerender = {
        let render = render.clone();
        move || {
            if let Some(r) = render.borrow().as_ref() {
                r();
            }
        }
    };

    let buttons = widgets::hbox(8);
    buttons.set_halign(gtk::Align::End);
    buttons.set_margin_top(6);
    let cancel = gtk::Button::with_label("Not now");
    let add = gtk::Button::with_label("Add files…");
    buttons.append(&cancel);
    buttons.append(&add);
    buttons.append(&combine);
    card.append(&buttons);
    {
        let dd = dialog.clone();
        cancel.connect_clicked(move |_| dd.close());
    }
    {
        let (files, rerender) = (files.clone(), rerender.clone());
        add.connect_clicked(move |_| {
            let fd = gtk::FileDialog::builder().title("Choose PDFs").modal(true).build();
            fd.set_filters(Some(&pdf_dialog_filters()));
            let (files, rerender) = (files.clone(), rerender.clone());
            fd.open_multiple(window::window().as_ref(), gio::Cancellable::NONE, move |res| {
                let Ok(list) = res else { return };
                for i in 0..list.n_items() {
                    if let Some(p) = list.item(i).and_downcast::<gio::File>().and_then(|f| f.path()) {
                        files.borrow_mut().push(p);
                    }
                }
                rerender();
            });
        });
    }
    {
        let (files, dd) = (files.clone(), dialog.clone());
        combine.connect_clicked(move |_| {
            let sources = files.borrow().clone();
            let fd = gtk::FileDialog::builder().title("Save the combined file").modal(true).initial_name("combined.pdf").build();
            fd.set_filters(Some(&pdf_dialog_filters()));
            if let Some(dir) = sources.first().and_then(|p| p.parent()) {
                fd.set_initial_folder(Some(&gio::File::for_path(dir)));
            }
            let dd = dd.clone();
            fd.save(window::window().as_ref(), gio::Cancellable::NONE, move |res| {
                let Some(mut target) = res.ok().and_then(|f| f.path()) else { return };
                if target.extension().is_none() {
                    target.set_extension("pdf");
                }
                dd.close();
                let t2 = target.clone();
                doc::background(
                    move || ops::combine(&sources, &target),
                    move |r| match r {
                        Ok(()) => {
                            window::toast(&format!("Saved {}.", paths::pretty(&t2)));
                            doc::request_open(t2.clone(), None);
                        }
                        Err(e) => window::toast(&format!("Couldn't combine them: {e:#}")),
                    },
                );
            });
        });
    }
    rerender();
    dialog.present();
}

