//! Searching the open document: a bar above the pages, hits drawn on them.

use super::{PAD_Y, view};
use crate::doc::geom::Rect;
use crate::doc::{self, Change};
use crate::widgets;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};

#[derive(Clone, Copy, Debug)]
struct Hit {
    page: usize,
    rect: Rect,
}

pub struct Search {
    pub bar: gtk::Revealer,
    entry: gtk::SearchEntry,
    readout: gtk::Label,
    hits: RefCell<Vec<Hit>>,
    cur: Cell<Option<usize>>,
    token: Cell<u32>,
}

impl Search {
    pub fn new() -> Search {
        let bar = gtk::Revealer::new();
        bar.set_transition_duration(if crate::prefs::get().reduce_motion { 0 } else { 150 });
        let row = widgets::hbox(8);
        row.add_css_class("search-bar");
        let entry = gtk::SearchEntry::new();
        entry.set_placeholder_text(Some("Search this document"));
        entry.add_css_class("settings-search");
        entry.set_hexpand(true);
        let readout = widgets::label("", "mono");
        readout.add_css_class("dim");
        let prev = gtk::Button::from_icon_name("go-up-symbolic");
        prev.set_tooltip_text(Some("Previous match (Shift+Enter)"));
        let next = gtk::Button::from_icon_name("go-down-symbolic");
        next.set_tooltip_text(Some("Next match (Enter)"));
        let close = gtk::Button::from_icon_name("window-close-symbolic");
        close.add_css_class("flat");
        close.set_tooltip_text(Some("Close search (Esc)"));
        for b in [&prev, &next] {
            b.add_css_class("flat");
        }
        row.append(&entry);
        row.append(&readout);
        row.append(&prev);
        row.append(&next);
        row.append(&close);
        bar.set_child(Some(&row));

        entry.connect_search_changed(|e| {
            if let Some(v) = view() {
                v.search.run(&e.text());
            }
        });
        entry.connect_activate(|_| step(1));
        entry.connect_next_match(|_| step(1));
        entry.connect_previous_match(|_| step(-1));
        entry.connect_stop_search(|_| {
            if let Some(v) = view() {
                v.search.close();
            }
        });
        prev.connect_clicked(|_| step(-1));
        next.connect_clicked(|_| step(1));
        close.connect_clicked(|_| {
            if let Some(v) = view() {
                v.search.close();
            }
        });
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(|_, key, _, mods| {
            if (key == gtk::gdk::Key::Return || key == gtk::gdk::Key::KP_Enter) && mods.contains(gtk::gdk::ModifierType::SHIFT_MASK) {
                step(-1);
                return gtk::glib::Propagation::Stop;
            }
            gtk::glib::Propagation::Proceed
        });
        entry.add_controller(keys);

        Search { bar, entry, readout, hits: RefCell::new(Vec::new()), cur: Cell::new(None), token: Cell::new(0) }
    }

    pub fn is_open(&self) -> bool {
        self.bar.reveals_child()
    }

    pub fn open(&self) {
        self.bar.set_reveal_child(true);
        self.entry.grab_focus();
        self.entry.select_region(0, -1);
    }

    pub fn close(&self) {
        self.bar.set_reveal_child(false);
        self.clear();
        if let Some(v) = view() {
            v.scroller.grab_focus();
        }
    }

    /// A different document: forget everything and hide.
    pub fn reset(&self) {
        self.bar.set_reveal_child(false);
        self.entry.set_text("");
        self.clear();
    }

    fn clear(&self) {
        self.token.set(self.token.get() + 1);
        self.hits.borrow_mut().clear();
        self.cur.set(None);
        self.readout.set_text("");
        self.paint();
    }

    pub fn rerun(&self) {
        if self.is_open() {
            let q = self.entry.text();
            self.run(&q);
        }
    }

    pub fn run(&self, query: &str) {
        let q = query.trim().to_string();
        let token = self.token.get() + 1;
        self.token.set(token);
        let Some(d) = doc::current() else { return };
        if q.is_empty() {
            self.hits.borrow_mut().clear();
            self.cur.set(None);
            self.readout.set_text("");
            self.paint();
            return;
        }
        let (path, pw, start) = (d.gen_path(), d.password(), d.page());
        doc::background(
            move || find_all(&path, pw.as_deref(), &q),
            move |hits| {
                let Some(v) = view() else { return };
                let s = &v.search;
                if s.token.get() != token {
                    return;
                }
                // Start at the first hit on or after the page being read.
                let first = hits.iter().position(|h| h.page >= start).or(if hits.is_empty() { None } else { Some(0) });
                *s.hits.borrow_mut() = hits;
                s.cur.set(first);
                s.paint();
                s.show_current(true);
            },
        );
    }

    fn paint(&self) {
        let Some(v) = view() else { return };
        let hits = self.hits.borrow();
        let cur = self.cur.get();
        for pv in v.pages.borrow().iter() {
            let mut st = pv.st.borrow_mut();
            st.hits.clear();
            st.current_hit = None;
            for (i, h) in hits.iter().enumerate() {
                if h.page == pv.index {
                    if cur == Some(i) {
                        st.current_hit = Some(st.hits.len());
                    }
                    st.hits.push(h.rect);
                }
            }
            drop(st);
            pv.marks.queue_draw();
        }
    }

    fn show_current(&self, scroll: bool) {
        let n = self.hits.borrow().len();
        match self.cur.get() {
            Some(i) if n > 0 => {
                self.readout.set_text(&format!("{} / {n}", i + 1));
                if scroll && let Some(v) = view() {
                    let h = self.hits.borrow()[i];
                    reveal(&v, h);
                }
            }
            _ => self.readout.set_text(if self.entry.text().trim().is_empty() { "" } else { "No matches" }),
        }
    }
}

/// Scroll so a hit is on screen.
fn reveal(v: &std::rc::Rc<super::View>, h: Hit) {
    let Some(d) = doc::current() else { return };
    d.set_page(h.page);
    if !crate::prefs::get().continuous {
        v.goto_page(h.page);
    }
    let adj = v.scroller.vadjustment();
    let z = v.zoom.get();
    let v2 = v.clone();
    super::after_layout(&v.column, move || {
        let y = v2.page_top(h.page) + h.rect.y0 * z;
        let (top, vh) = (adj.value(), adj.page_size());
        if y < top + 40.0 || y + h.rect.height() * z > top + vh - 40.0 {
            v2.programmatic.set(true);
            adj.set_value((y - vh / 3.0).max(0.0).min((adj.upper() - vh).max(0.0)));
            v2.programmatic.set(false);
        }
        v2.queue_visible();
        let _ = PAD_Y;
    });
}

fn step(dir: i32) {
    let Some(v) = view() else { return };
    let s = &v.search;
    let n = s.hits.borrow().len();
    if n == 0 {
        return;
    }
    let cur = s.cur.get().unwrap_or(0) as i64;
    s.cur.set(Some((cur + i64::from(dir)).rem_euclid(n as i64) as usize));
    s.paint();
    s.show_current(true);
}

/// Every match of `query` in the file, with its place on the displayed page.
fn find_all(path: &std::path::Path, password: Option<&str>, query: &str) -> Vec<Hit> {
    use gtk::gio::prelude::FileExt;
    let uri = gtk::gio::File::for_path(path).uri();
    let Ok(pdf) = poppler::Document::from_file(&uri, password) else { return Vec::new() };
    let mut out = Vec::new();
    for i in 0..pdf.n_pages() {
        let Some(page) = pdf.page(i) else { continue };
        let (_, ph) = page.size();
        for r in page.find_text(query) {
            // Poppler gives these with y up.
            out.push(Hit { page: i as usize, rect: Rect::new(r.x1(), ph - r.y2(), r.x2(), ph - r.y1()) });
        }
    }
    out
}

impl Search {
    pub fn on_change(&self, c: Change) {
        if c == Change::Structure {
            self.rerun();
        }
    }
}
