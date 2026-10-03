//! The document viewer and editor: a column of pages with a side panel and a
//! tool strip. This file holds the shared state, zoom, scrolling and the strip;
//! `pageview` draws and handles one page, `tools` turns gestures into edits.

pub mod colour;
mod forms;
mod pageview;
mod panel;
mod script;
mod search;
mod sign;
pub mod thumbs;
mod tools;

use crate::doc::{self, Change, Doc, geom::Rect, render};
use crate::widgets::{self, Page};
use crate::{prefs, theme, window};
use gtk::prelude::*;
use gtk::{gdk, glib};
use pageview::PageView;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

pub use tools::Tool;

/// Space around and between pages, in pixels.
const PAD_X: f64 = 28.0;
const PAD_Y: f64 = 20.0;
const GAP: f64 = 14.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fit {
    None,
    Width,
    Page,
}

/// A text selection on one page.
pub struct Sel {
    pub page: usize,
    pub rects: Vec<Rect>,
    pub text: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AnnotSel {
    pub page: usize,
    pub id: lopdf::ObjectId,
}

pub struct View {
    pub stack: gtk::Stack,
    pub scroller: gtk::ScrolledWindow,
    pub column: gtk::Box,
    pub pages: RefCell<Vec<Rc<PageView>>>,
    pub zoom: Rc<Cell<f64>>,
    fit: Cell<Fit>,
    tool: Cell<Tool>,
    colour: RefCell<String>,
    tool_colours: RefCell<HashMap<Tool, String>>,
    tool_buttons: RefCell<Vec<(Tool, gtk::ToggleButton)>>,
    panel: panel::Panel,
    panel_toggle: gtk::ToggleButton,
    page_entry: gtk::Entry,
    total: gtk::Label,
    zoom_label: gtk::Label,
    undo_btn: gtk::Button,
    redo_btn: gtk::Button,
    save_btn: gtk::Button,
    title: gtk::Label,
    subtitle: gtk::Label,
    banner: gtk::Box,
    strip: gtk::Box,
    right: gtk::Box,
    pub search: search::Search,
    programmatic: Cell<bool>,
    visible_queued: Cell<bool>,
    lo_cache: RefCell<Option<(usize, Rc<lopdf::Document>)>>,
    pub selection: RefCell<Option<Sel>>,
    pub annot_sel: RefCell<Option<AnnotSel>>,
    /// The signature picture waiting to be placed.
    pub signature: RefCell<Option<PathBuf>>,
    narrow: Cell<bool>,
    panel_wanted: Cell<bool>,
    /// Bumped when the page colours change, so pictures are redrawn.
    pub epoch: Cell<u32>,
    settling: Cell<bool>,
}

thread_local! {
    static VIEW: RefCell<Option<Rc<View>>> = const { RefCell::new(None) };
}

pub fn view() -> Option<Rc<View>> {
    VIEW.with(|v| v.borrow().clone())
}

/// How pages are recoloured, from the preference and the theme.
pub fn page_colors() -> render::Colors {
    match prefs::get().page_colors.as_str() {
        "invert" => render::Colors::Invert,
        "tint" => {
            let p = theme::palette();
            match (colour::parse_u8(&p.bg), colour::parse_u8(&p.text)) {
                (Some(paper), Some(ink)) => render::Colors::Tint(paper, ink),
                _ => render::Colors::Off,
            }
        }
        _ => render::Colors::Off,
    }
}

/// Run `f` after GTK has laid out the next frames, when sizes are real.
pub fn after_layout(widget: &impl IsA<gtk::Widget>, f: impl FnOnce() + 'static) {
    let f = RefCell::new(Some(f));
    let ticks = Cell::new(0);
    widget.add_tick_callback(move |_, _| {
        ticks.set(ticks.get() + 1);
        if ticks.get() < 3 {
            return glib::ControlFlow::Continue;
        }
        if let Some(f) = f.borrow_mut().take() {
            f();
        }
        glib::ControlFlow::Break
    });
}

fn icon_toggle(icon: &str, tooltip: &str) -> gtk::ToggleButton {
    let b = gtk::ToggleButton::new();
    b.set_child(Some(&gtk::Image::from_icon_name(icon)));
    b.set_tooltip_text(Some(tooltip));
    b
}

fn icon_btn(icon: &str, tooltip: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.set_tooltip_text(Some(tooltip));
    b
}

pub fn build(page: &Page) {
    let v = View::new();
    page.body.append(&v.stack);
    VIEW.with(|c| *c.borrow_mut() = Some(v.clone()));
    v.wire();
    let c = v.colour();
    v.show_colour(&c);
    v.mark_page_colours();
    // Show whatever is already open (a file given on the command line).
    if doc::current().is_some() {
        v.rebuild();
        v.refresh_chrome();
    }
}

impl View {
    fn new() -> Rc<View> {
        let stack = gtk::Stack::new();
        stack.set_vexpand(true);
        stack.set_hexpand(true);
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        stack.set_transition_duration(if prefs::get().reduce_motion { 0 } else { 160 });

        stack.add_named(
            &widgets::empty_state(
                "npdf-document-symbolic",
                "No file open",
                "Open a PDF to read it, mark it up, fill it in and sign it.",
                Some(("Open a file", Box::new(doc::open_dialog))),
            ),
            Some("empty"),
        );

        let root = widgets::vbox(0);
        root.add_css_class("viewer");

        // ----- Header -----
        let header = widgets::hbox(12);
        header.add_css_class("viewer-header");
        let text = widgets::vbox(0);
        text.set_hexpand(true);
        let title = widgets::label("", "viewer-title");
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let subtitle = widgets::label("", "viewer-subtitle");
        subtitle.add_css_class("mono");
        subtitle.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        text.append(&title);
        text.append(&subtitle);
        header.append(&text);
        let open_btn = widgets::labeled_button("document-open-symbolic", "Open");
        open_btn.connect_clicked(|_| doc::open_dialog());
        open_btn.add_css_class("flat");
        let save_as = widgets::labeled_button("document-save-as-symbolic", "Save as");
        save_as.connect_clicked(|_| doc::save_as_dialog());
        save_as.set_tooltip_text(Some("Save a copy under another name (Ctrl+Shift+S)"));
        save_as.add_css_class("flat");
        let save_btn = widgets::labeled_button("document-save-symbolic", "Save");
        save_btn.set_tooltip_text(Some("Save (Ctrl+S)"));
        save_btn.connect_clicked(|_| doc::save());
        let close_btn = widgets::labeled_button("window-close-symbolic", "Close");
        close_btn.add_css_class("flat");
        close_btn.set_tooltip_text(Some("Close this file and go back to the library"));
        close_btn.connect_clicked(|_| close_file());
        for b in [&open_btn, &save_as, &close_btn, &save_btn] {
            b.set_valign(gtk::Align::Center);
            header.append(b);
        }
        root.append(&header);

        // ----- Toolbar -----
        // One bar: tools on the left, view and history on the right. When the window is
        // narrow the two halves stack (see `apply_strip`).
        let strip = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        strip.add_css_class("tool-strip");
        let (left, right) = (widgets::hbox(2), widgets::hbox(2));
        left.set_hexpand(true);
        right.set_halign(gtk::Align::End);
        strip.append(&left);
        strip.append(&right);
        let sep = || {
            let s = gtk::Separator::new(gtk::Orientation::Vertical);
            s.add_css_class("tb-sep");
            s
        };

        let panel_toggle = icon_toggle("npdf-thumbs-symbolic", "Show or hide the side panel (F9)");
        panel_toggle.set_active(true);
        panel_toggle.add_css_class("tb-btn");
        left.append(&panel_toggle);
        left.append(&sep());

        // Select | text markup | drawing and notes | edit and sign.
        let mut tool_buttons = Vec::new();
        let mut first: Option<gtk::ToggleButton> = None;
        for (i, t) in Tool::ALL.into_iter().enumerate() {
            if matches!(i, 1 | 4 | 7) {
                left.append(&sep());
            }
            let b = icon_toggle(t.icon(), &t.tooltip());
            b.add_css_class("tb-btn");
            if let Some(f) = &first {
                b.set_group(Some(f));
            } else {
                first = Some(b.clone());
            }
            b.set_active(t == Tool::Select);
            b.connect_toggled(move |b| {
                if b.is_active() {
                    set_tool(t);
                }
            });
            left.append(&b);
            tool_buttons.push((t, b));
        }
        left.append(&sep());

        let colour_btn = gtk::MenuButton::new();
        colour_btn.set_tooltip_text(Some("Colour for new markup"));
        colour_btn.add_css_class("colour-button");
        colour_btn.set_valign(gtk::Align::Center);
        let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        dot.add_css_class("dot");
        dot.set_halign(gtk::Align::Center);
        dot.set_valign(gtk::Align::Center);
        colour_btn.set_child(Some(&dot));
        let pop = gtk::Popover::new();
        let initial = prefs::get().highlight_color;
        pop.set_child(Some(&colour::row(&initial, |hex| {
            if let Some(v) = view() {
                v.set_colour(&hex);
            }
        })));
        colour_btn.set_popover(Some(&pop));
        left.append(&colour_btn);
        COLOUR_DOT.with(|c| *c.borrow_mut() = Some(dot));

        let zout = icon_btn("zoom-out-symbolic", "Zoom out (Ctrl+−)");
        let zin = icon_btn("zoom-in-symbolic", "Zoom in (Ctrl++)");
        let zwidth = icon_btn("npdf-fit-width-symbolic", "Fit width (Ctrl+0)");
        let zpage = icon_btn("npdf-fit-page-symbolic", "Fit page");
        for b in [&zout, &zin, &zwidth, &zpage] {
            b.add_css_class("tb-btn");
        }
        zout.connect_clicked(|_| zoom_by(1.0 / 1.2));
        zin.connect_clicked(|_| zoom_by(1.2));
        zwidth.connect_clicked(|_| zoom_fit("fit-width"));
        zpage.connect_clicked(|_| zoom_fit("fit-page"));
        let zoom_label = widgets::label("100%", "mono");
        zoom_label.add_css_class("zoom-readout");
        zoom_label.set_width_chars(5);
        zoom_label.set_xalign(0.5);
        right.append(&zout);
        right.append(&zoom_label);
        right.append(&zin);
        right.append(&zwidth);
        right.append(&zpage);
        right.append(&sep());

        let nav = widgets::hbox(6);
        nav.add_css_class("page-nav");
        let page_entry = gtk::Entry::new();
        page_entry.add_css_class("mono");
        page_entry.add_css_class("page-entry");
        page_entry.set_width_chars(3);
        page_entry.set_max_width_chars(5);
        EditableExt::set_alignment(&page_entry, 0.5);
        page_entry.set_tooltip_text(Some("Go to page"));
        let total = widgets::label("/ 1", "mono");
        total.add_css_class("dim");
        nav.append(&page_entry);
        nav.append(&total);
        nav.set_valign(gtk::Align::Center);
        right.append(&nav);
        right.append(&sep());

        let undo_btn = icon_btn("edit-undo-symbolic", "Undo (Ctrl+Z)");
        let redo_btn = icon_btn("edit-redo-symbolic", "Redo (Ctrl+Shift+Z)");
        for b in [&undo_btn, &redo_btn] {
            b.add_css_class("tb-btn");
            right.append(b);
        }
        undo_btn.connect_clicked(|_| doc::undo());
        redo_btn.connect_clicked(|_| doc::redo());
        root.append(&strip);

        // ----- Notices -----
        let banner = widgets::banner("This file is open for reading only: it's encrypted or damaged, so it can't be edited.", true);
        banner.set_visible(false);
        banner.add_css_class("viewer-banner");
        root.append(&banner);

        let search = search::Search::new();
        root.append(&search.bar);

        // ----- Panel and canvas -----
        let split = widgets::hbox(0);
        split.set_vexpand(true);
        let panel = panel::Panel::new();
        split.append(&panel.root);

        let column = widgets::vbox(GAP as i32);
        column.add_css_class("page-column");
        column.set_halign(gtk::Align::Center);
        column.set_margin_top(PAD_Y as i32);
        column.set_margin_bottom(PAD_Y as i32);
        column.set_margin_start(PAD_X as i32);
        column.set_margin_end(PAD_X as i32);
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::External)
            .vscrollbar_policy(gtk::PolicyType::External)
            .hexpand(true)
            .vexpand(true)
            .child(&column)
            .build();
        scroller.add_css_class("canvas");
        split.append(&scroller);
        root.append(&split);
        stack.add_named(&root, Some("viewer"));
        stack.set_visible_child_name("empty");

        Rc::new(View {
            stack,
            scroller,
            column,
            pages: RefCell::new(Vec::new()),
            zoom: Rc::new(Cell::new(1.0)),
            fit: Cell::new(Fit::Width),
            tool: Cell::new(Tool::Select),
            colour: RefCell::new(initial),
            tool_colours: RefCell::new(HashMap::new()),
            tool_buttons: RefCell::new(tool_buttons),
            panel,
            panel_toggle,
            page_entry,
            total,
            zoom_label,
            undo_btn,
            redo_btn,
            save_btn,
            title,
            subtitle,
            banner,
            strip,
            right,
            search,
            programmatic: Cell::new(false),
            visible_queued: Cell::new(false),
            lo_cache: RefCell::new(None),
            selection: RefCell::new(None),
            annot_sel: RefCell::new(None),
            signature: RefCell::new(None),
            narrow: Cell::new(false),
            panel_wanted: Cell::new(true),
            epoch: Cell::new(0),
            settling: Cell::new(false),
        })
    }

    fn wire(self: &Rc<Self>) {
        let v = self.clone();
        doc::subscribe(&self.stack, move |c| v.on_change(c));

        let v = self.clone();
        self.scroller.vadjustment().connect_value_changed(move |_| v.queue_visible());
        // Fit modes follow the window's size.
        let v = self.clone();
        self.scroller.hadjustment().connect_page_size_notify(move |_| v.on_resize());
        let v = self.clone();
        self.scroller.vadjustment().connect_page_size_notify(move |_| v.on_resize());

        // Ctrl+wheel zooms; a plain wheel scrolls (and in single-page mode turns pages at the ends).
        let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        wheel.set_propagation_phase(gtk::PropagationPhase::Capture);
        wheel.connect_scroll(|c, _, dy| {
            if c.current_event_state().contains(gdk::ModifierType::CONTROL_MASK) {
                zoom_by(if dy < 0.0 { 1.1 } else { 1.0 / 1.1 });
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.scroller.add_controller(wheel);

        let pinch = gtk::GestureZoom::new();
        let base = Rc::new(Cell::new(1.0));
        let b = base.clone();
        pinch.connect_begin(move |_, _| {
            if let Some(v) = view() {
                b.set(v.zoom.get());
            }
        });
        let b = base.clone();
        pinch.connect_scale_changed(move |_, s| {
            if let Some(v) = view() {
                v.set_fit_zoom(b.get() * s, Fit::None);
            }
        });
        self.scroller.add_controller(pinch);

        let v = self.clone();
        self.panel_toggle.connect_toggled(move |b| {
            v.panel_wanted.set(b.is_active());
            v.apply_panel();
        });
        let v = self.clone();
        self.page_entry.connect_activate(move |e| {
            if let Ok(n) = e.text().trim().parse::<usize>() {
                v.goto_page(n.saturating_sub(1));
            }
            if let Some(d) = doc::current() {
                e.set_text(&(d.page() + 1).to_string());
            }
        });
        let v = self.clone();
        theme::subscribe(&self.stack, move || {
            v.epoch.set(v.epoch.get() + 1);
            v.mark_page_colours();
            thumbs::clear();
            v.queue_visible();
            for p in v.pages.borrow().iter() {
                p.marks.queue_draw();
            }
            v.panel.refresh_thumbs();
        });
    }

    fn on_change(self: &Rc<Self>, c: Change) {
        match c {
            Change::Opened => {
                self.fit.set(match prefs::get().default_zoom.as_str() {
                    "fit-page" => Fit::Page,
                    "100" => Fit::None,
                    _ => Fit::Width,
                });
                if self.fit.get() == Fit::None {
                    self.zoom.set(1.0);
                }
                *self.selection.borrow_mut() = None;
                *self.annot_sel.borrow_mut() = None;
                self.search.reset();
                thumbs::clear();
                self.rebuild();
                self.refresh_chrome();
            }
            Change::Closed => {
                self.clear_pages();
                self.stack.set_visible_child_name("empty");
                self.refresh_chrome();
            }
            Change::Structure => {
                *self.selection.borrow_mut() = None;
                *self.annot_sel.borrow_mut() = None;
                thumbs::clear();
                self.rebuild();
                self.refresh_chrome();
                self.search.on_change(c);
            }
            Change::Content => {
                *self.selection.borrow_mut() = None;
                // The selected annotation may be gone or changed; keep it only if it still exists.
                self.validate_annot_sel();
                thumbs::clear();
                for p in self.pages.borrow().iter() {
                    p.invalidate();
                }
                self.queue_visible();
                self.panel.refresh_annots();
                self.panel.refresh_thumbs();
            }
            Change::Dirty => self.refresh_chrome(),
            Change::Page => self.refresh_readout(),
        }
    }

    fn clear_pages(&self) {
        while let Some(c) = self.column.first_child() {
            self.column.remove(&c);
        }
        for p in self.pages.borrow().iter() {
            p.release();
        }
        self.pages.borrow_mut().clear();
    }

    /// Build a frame for every page.
    pub fn rebuild(self: &Rc<Self>) {
        self.clear_pages();
        let Some(d) = doc::current() else { return };
        self.stack.set_visible_child_name("viewer");
        *self.lo_cache.borrow_mut() = None;
        let mut pages = Vec::new();
        for i in 0..d.n_pages() {
            let pv = PageView::new(i, self.zoom.clone());
            self.column.append(&pv.frame);
            pages.push(pv);
        }
        *self.pages.borrow_mut() = pages;
        self.apply_fit();
        self.layout_pages();
        self.panel.rebuild();
        let v = self.clone();
        let page = d.page();
        after_layout(&self.column, move || {
            v.apply_strip();
            v.apply_fit();
            v.scroll_to_page(page);
            v.queue_visible();
        });
    }

    pub fn relayout(self: &Rc<Self>) {
        self.layout_pages();
        if let Some(d) = doc::current() {
            self.scroll_to_page(d.page());
        }
        self.queue_visible();
    }

    /// Single-page mode shows just the current page.
    fn layout_pages(&self) {
        let Some(d) = doc::current() else { return };
        let continuous = prefs::get().continuous;
        for (i, p) in self.pages.borrow().iter().enumerate() {
            p.frame.set_visible(continuous || i == d.page());
        }
        self.apply_sizes();
    }

    fn apply_sizes(&self) {
        let Some(d) = doc::current() else { return };
        let z = self.zoom.get();
        for p in self.pages.borrow().iter() {
            let (w, h) = d.page_size(p.index);
            p.set_size(w, h, z);
        }
        self.zoom_label.set_text(&format!("{:.0}%", z * 100.0));
    }

    fn viewport(&self) -> (f64, f64) {
        let w = self.scroller.hadjustment().page_size();
        let h = self.scroller.vadjustment().page_size();
        let w = if w > 1.0 { w } else { f64::from(self.scroller.width()) };
        let h = if h > 1.0 { h } else { f64::from(self.scroller.height()) };
        // Before the first layout, guess from the window.
        let fallback = window::window().map(|w| (f64::from(w.width()) - 520.0, f64::from(w.height()) - 160.0)).unwrap_or((800.0, 600.0));
        (if w > 1.0 { w } else { fallback.0.max(300.0) }, if h > 1.0 { h } else { fallback.1.max(300.0) })
    }

    fn apply_fit(self: &Rc<Self>) {
        let Some(d) = doc::current() else { return };
        let (vw, vh) = self.viewport();
        let avail_w = (vw - 2.0 * PAD_X).max(100.0);
        let z = match self.fit.get() {
            Fit::None => return,
            Fit::Width => {
                let widest = (0..d.n_pages()).map(|i| d.page_size(i).0).fold(1.0, f64::max);
                avail_w / widest
            }
            Fit::Page => {
                let (w, h) = d.page_size(d.page());
                (avail_w / w).min((vh - 2.0 * PAD_Y).max(100.0) / h)
            }
        };
        let z = z.clamp(0.1, 8.0);
        if (z - self.zoom.get()).abs() > 0.0005 {
            self.zoom.set(z);
            self.apply_sizes();
            self.queue_visible();
        }
    }

    /// Side by side when there's room, stacked when there isn't.
    fn apply_strip(&self) {
        let w = self.stack.width();
        let stacked = w > 0 && w < 900;
        let want = if stacked { gtk::Orientation::Vertical } else { gtk::Orientation::Horizontal };
        if self.strip.orientation() != want {
            self.strip.set_orientation(want);
            self.right.set_halign(if stacked { gtk::Align::Start } else { gtk::Align::End });
            self.strip.set_spacing(if stacked { 2 } else { 12 });
        }
    }

    fn on_resize(self: &Rc<Self>) {
        self.apply_strip();
        if self.fit.get() != Fit::None && !self.settling.get() {
            self.apply_fit();
        }
        self.queue_visible();
    }

    /// Set the zoom (and the fit mode it came from), keeping the reader's place.
    pub fn set_fit_zoom(self: &Rc<Self>, z: f64, fit: Fit) {
        let z = z.clamp(0.1, 8.0);
        self.fit.set(fit);
        if fit != Fit::None {
            self.apply_fit();
        } else {
            self.zoom.set(z);
        }
        let adj = self.scroller.vadjustment();
        let ratio = if adj.upper() > 0.0 { adj.value() / adj.upper() } else { 0.0 };
        self.apply_sizes();
        let v = self.clone();
        after_layout(&self.column, move || {
            let adj = v.scroller.vadjustment();
            if prefs::get().continuous {
                v.programmatic.set(true);
                adj.set_value(ratio * adj.upper());
                v.programmatic.set(false);
            }
            v.queue_visible();
        });
        self.queue_visible();
    }

    pub fn queue_visible(self: &Rc<Self>) {
        if self.visible_queued.replace(true) {
            return;
        }
        let v = self.clone();
        glib::idle_add_local_once(move || {
            v.visible_queued.set(false);
            v.update_visible();
        });
    }

    /// Draw what's on screen, drop what's far away, and work out which page the reader is on.
    fn update_visible(self: &Rc<Self>) {
        let Some(d) = doc::current() else { return };
        let adj = self.scroller.vadjustment();
        let (top, vh) = (adj.value(), adj.page_size().max(1.0));
        let continuous = prefs::get().continuous;
        let mut best = (d.page(), f64::MIN);
        let pages = self.pages.borrow().clone();
        for pv in &pages {
            if !pv.frame.is_visible() {
                pv.release();
                continue;
            }
            let Some(b) = pv.frame.compute_bounds(&self.column) else { continue };
            let (y, h) = (f64::from(b.y()), f64::from(b.height()));
            let overlap = (y + h).min(top + vh) - y.max(top);
            if y + h > top - vh && y < top + 2.0 * vh {
                pv.ensure_rendered(self, &d);
            } else if y + h < top - 4.0 * vh || y > top + 5.0 * vh {
                pv.release();
            }
            if overlap > best.1 {
                best = (pv.index, overlap);
            }
        }
        if continuous && !self.programmatic.get() && best.1 > 0.0 {
            d.set_page(best.0);
        }
    }

    fn page_top(&self, i: usize) -> f64 {
        self.pages
            .borrow()
            .get(i)
            .and_then(|p| p.frame.compute_bounds(&self.column))
            .map(|b| f64::from(b.y()))
            .unwrap_or(0.0)
    }

    pub fn scroll_to_page(self: &Rc<Self>, i: usize) {
        let adj = self.scroller.vadjustment();
        self.programmatic.set(true);
        if prefs::get().continuous {
            adj.set_value((self.page_top(i) - 6.0).max(0.0));
        } else {
            adj.set_value(0.0);
        }
        let v = self.clone();
        glib::idle_add_local_once(move || {
            v.programmatic.set(false);
            v.queue_visible();
        });
    }

    pub fn goto_page(self: &Rc<Self>, i: usize) {
        let Some(d) = doc::current() else { return };
        let i = i.min(d.n_pages().saturating_sub(1));
        d.set_page(i);
        if !prefs::get().continuous {
            self.layout_pages();
            if self.fit.get() == Fit::Page {
                self.apply_fit();
            }
        }
        self.scroll_to_page(i);
    }

    pub fn tool(&self) -> Tool {
        self.tool.get()
    }

    pub fn colour(&self) -> String {
        self.colour.borrow().clone()
    }

    /// The user picked a colour for the current tool.
    pub fn set_colour(&self, hex: &str) {
        self.tool_colours.borrow_mut().insert(self.tool.get(), hex.to_string());
        self.show_colour(hex);
    }

    /// Settings changed the highlight colour.
    pub fn set_highlight_colour(&self, hex: &str) {
        self.tool_colours.borrow_mut().insert(Tool::Highlight, hex.to_string());
        if self.tool.get() == Tool::Highlight {
            self.show_colour(hex);
        }
    }

    fn show_colour(&self, hex: &str) {
        *self.colour.borrow_mut() = hex.to_string();
        COLOUR_DOT.with(|c| {
            if let Some(dot) = c.borrow().as_ref() {
                let provider = gtk::CssProvider::new();
                provider.load_from_string(&format!("box.dot {{ background: {hex}; }}"));
                #[allow(deprecated)]
                dot.style_context().add_provider(&provider, gtk::STYLE_PROVIDER_PRIORITY_USER);
            }
        });
    }

    /// The loaded form of the current generation, for reading annotations.
    pub fn lo(&self) -> Option<Rc<lopdf::Document>> {
        let d = doc::current()?;
        let g = d.generation();
        if let Some((cg, lo)) = self.lo_cache.borrow().as_ref()
            && *cg == g
        {
            return Some(lo.clone());
        }
        let lo = Rc::new(lopdf::Document::load(d.gen_path()).ok()?);
        *self.lo_cache.borrow_mut() = Some((g, lo.clone()));
        Some(lo)
    }

    fn validate_annot_sel(&self) {
        let sel = *self.annot_sel.borrow();
        if let Some(s) = sel {
            let alive = self.lo().is_some_and(|lo| doc::annots::exists(&lo, s.id));
            if !alive {
                *self.annot_sel.borrow_mut() = None;
            }
        }
        for p in self.pages.borrow().iter() {
            p.marks.queue_draw();
        }
    }

    /// Tell the stylesheet whether pages are recoloured (form fields follow).
    fn mark_page_colours(&self) {
        if prefs::get().page_colors == "off" {
            self.stack.remove_css_class("pages-themed");
        } else {
            self.stack.add_css_class("pages-themed");
        }
    }

    fn apply_panel(&self) {
        self.panel.root.set_visible(self.panel_wanted.get() && !self.narrow.get());
    }

    /// Everything in the header and strip that follows the document.
    fn refresh_chrome(self: &Rc<Self>) {
        let d = doc::current();
        match &d {
            Some(d) => {
                self.title.set_text(&format!("{}{}", if d.dirty() { "• " } else { "" }, d.title()));
                self.subtitle.set_text(&crate::paths::pretty(&d.path()));
                self.total.set_text(&format!("/ {}", d.n_pages()));
                self.banner.set_visible(!d.editable());
                self.undo_btn.set_sensitive(d.can_undo());
                self.redo_btn.set_sensitive(d.can_redo());
                if d.dirty() {
                    self.save_btn.add_css_class("suggested-action");
                } else {
                    self.save_btn.remove_css_class("suggested-action");
                }
                self.save_btn.set_sensitive(d.dirty());
            }
            None => {
                self.undo_btn.set_sensitive(false);
                self.redo_btn.set_sensitive(false);
            }
        }
        self.refresh_readout();
        self.apply_panel();
    }

    fn refresh_readout(&self) {
        if let Some(d) = doc::current() {
            let text = (d.page() + 1).to_string();
            if self.page_entry.text() != text && !self.page_entry.has_focus() {
                self.page_entry.set_text(&text);
            }
            self.panel.highlight_page(d.page());
        }
    }

    pub fn set_tool(self: &Rc<Self>, t: Tool) {
        if self.tool.replace(t) == t {
            return;
        }
        for (tool, b) in self.tool_buttons.borrow().iter() {
            if *tool == t && !b.is_active() {
                b.set_active(true);
            }
        }
        *self.annot_sel.borrow_mut() = None;
        *self.selection.borrow_mut() = None;
        let colour = self.tool_colours.borrow().get(&t).cloned().unwrap_or_else(|| {
            if t == Tool::Select { prefs::get().highlight_color } else { t.default_colour() }
        });
        self.show_colour(&colour);
        for p in self.pages.borrow().iter() {
            p.set_cursor_for(t);
            p.marks.queue_draw();
        }
        match t {
            Tool::Sign => tools::choose_signature(),
            Tool::EditText if !EDIT_NOTE_SHOWN.with(|s| s.replace(true)) => {
                window::toast("Edit text covers the old words and writes new ones over them. The original stays in the file underneath.");
            }
            _ => {}
        }
        if t != Tool::Select && doc::current().is_some_and(|d| !d.editable()) {
            window::toast("This file is read-only, so edits will be refused.");
        }
    }
}

thread_local! {
    static COLOUR_DOT: RefCell<Option<gtk::Box>> = const { RefCell::new(None) };
    static EDIT_NOTE_SHOWN: Cell<bool> = const { Cell::new(false) };
}

// ----- The functions the window calls -----

/// Run the developer script from `NPDF_SCRIPT`, if there is one.
pub fn run_script_from_env() {
    if let Some(s) = std::env::var_os("NPDF_SCRIPT") {
        script::start(&s.to_string_lossy());
    }
}

/// Close the open file (asking about unsaved changes) and go back to the library.
pub fn close_file() {
    doc::guard_unsaved(|| {
        doc::close();
        window::navigate("library");
    });
}

pub fn focus_search() {
    if let Some(v) = view() {
        v.search.open();
    }
}

pub fn zoom_by(f: f64) {
    if let Some(v) = view() {
        let z = v.zoom.get() * f;
        v.set_fit_zoom(z, Fit::None);
    }
}

pub fn zoom_fit(mode: &str) {
    if let Some(v) = view() {
        let fit = if mode == "fit-page" { Fit::Page } else { Fit::Width };
        v.set_fit_zoom(v.zoom.get(), fit);
    }
}

pub fn toggle_panel() {
    if let Some(v) = view() {
        v.panel_toggle.set_active(!v.panel_toggle.is_active());
    }
}

pub fn escape() {
    let Some(v) = view() else { return };
    if v.search.is_open() {
        v.search.close();
    } else if v.selection.borrow().is_some() || v.annot_sel.borrow().is_some() {
        *v.selection.borrow_mut() = None;
        *v.annot_sel.borrow_mut() = None;
        for p in v.pages.borrow().iter() {
            p.marks.queue_draw();
        }
    } else if v.tool() != Tool::Select {
        v.set_tool(Tool::Select);
    }
}

pub fn step_page(d: i32) {
    if let (Some(v), Some(doc)) = (view(), doc::current()) {
        let next = (doc.page() as i64 + i64::from(d)).clamp(0, doc.n_pages() as i64 - 1) as usize;
        v.goto_page(next);
    }
}

pub fn goto_page(i: usize) {
    if let Some(v) = view() {
        v.goto_page(i);
    }
}

pub fn set_tool(t: Tool) {
    if let Some(v) = view() {
        v.set_tool(t);
    }
}

pub fn set_narrow(narrow: bool) {
    if let Some(v) = view() {
        v.narrow.set(narrow);
        v.apply_panel();
    }
}

pub fn relayout() {
    if let Some(v) = view() {
        v.relayout();
    }
}

/// Copy the selected text, if there is any. True when something was copied.
pub fn copy_selection() -> bool {
    let Some(v) = view() else { return false };
    let text = v.selection.borrow().as_ref().map(|s| s.text.clone()).filter(|t| !t.is_empty());
    match text {
        Some(t) => {
            if let Some(display) = gdk::Display::default() {
                display.clipboard().set_text(&t);
            }
            window::toast("Copied.");
            true
        }
        None => false,
    }
}

pub fn delete_selected() -> bool {
    tools::delete_selected()
}

/// Open the document at a page from elsewhere in the app (library, outline).
pub fn show_page(d: &Doc, page: usize) {
    d.set_page(page);
    window::navigate("document");
    if let Some(v) = view() {
        v.goto_page(page);
    }
}
