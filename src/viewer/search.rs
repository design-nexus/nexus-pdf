//! Searching the open document: a bar above the pages, hits drawn on them.

use super::view;
use crate::doc::geom::Rect;
use crate::doc::{self, Change, annots};
use crate::widgets;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug)]
struct Hit {
    page: usize,
    rect: Rect,
}

pub struct Search {
    pub bar: gtk::Revealer,
    entry: gtk::SearchEntry,
    readout: gtk::Label,
    case: gtk::ToggleButton,
    words: gtk::ToggleButton,
    hits: RefCell<Vec<Hit>>,
    cur: Cell<Option<usize>>,
    token: Cell<u32>,
    /// Stops the search running in the background.
    cancel: RefCell<Arc<AtomicBool>>,
    /// Still looking (the readout says how far it's got).
    busy: Cell<bool>,
    /// The page being read when the search started: the first match on or after it comes first.
    start: Cell<usize>,
}

/// What the background search sends back.
enum Found {
    /// The matches on one page.
    Page(usize, Vec<Hit>),
    Done,
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
        let case = gtk::ToggleButton::with_label("Aa");
        case.set_tooltip_text(Some("Match case"));
        let words = gtk::ToggleButton::with_label("Word");
        words.set_tooltip_text(Some("Whole words only"));
        for b in [&case, &words] {
            b.add_css_class("search-option");
            b.set_valign(gtk::Align::Center);
            b.connect_toggled(|_| {
                if let Some(v) = view() {
                    v.search.rerun();
                }
            });
        }
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
        row.append(&case);
        row.append(&words);
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

        Search {
            bar,
            entry,
            readout,
            case,
            words,
            hits: RefCell::new(Vec::new()),
            cur: Cell::new(None),
            token: Cell::new(0),
            cancel: RefCell::new(Arc::new(AtomicBool::new(false))),
            busy: Cell::new(false),
            start: Cell::new(0),
        }
    }

    /// What the readout says and how many matches there are (for the developer script).
    pub fn describe(&self) -> String {
        format!("{:?}, {} hits, busy {}", self.readout.text(), self.hits.borrow().len(), self.busy.get())
    }

    /// Type a query into the box, as the user would (the developer script).
    pub fn type_query(&self, q: &str) {
        self.entry.set_text(q);
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
        self.cancel.borrow().store(true, Ordering::Relaxed);
        self.busy.set(false);
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
        self.cancel.borrow().store(true, Ordering::Relaxed);
        self.hits.borrow_mut().clear();
        self.cur.set(None);
        self.busy.set(false);
        let Some(d) = doc::current() else { return };
        if q.is_empty() {
            self.readout.set_text("");
            self.paint();
            return;
        }
        let mut flags = poppler::FindFlags::DEFAULT;
        if self.case.is_active() {
            flags |= poppler::FindFlags::CASE_SENSITIVE;
        }
        if self.words.is_active() {
            flags |= poppler::FindFlags::WHOLE_WORDS_ONLY;
        }
        // Notes and text boxes are searched too: their text isn't on the page itself.
        let notes = annotation_hits(&q, self.case.is_active());
        let cancel = Arc::new(AtomicBool::new(false));
        *self.cancel.borrow_mut() = cancel.clone();
        self.busy.set(true);
        self.start.set(d.page());
        self.paint();
        self.show_progress(0, d.n_pages());
        let (path, pw, n) = (d.gen_path(), d.password(), d.n_pages());
        let (tx, rx) = async_channel::unbounded::<Found>();
        std::thread::spawn(move || find_all(&path, pw.as_deref(), &q, flags, notes, &cancel, &tx));
        gtk::glib::spawn_future_local(async move {
            while let Ok(found) = rx.recv().await {
                let Some(v) = view() else { return };
                let s = &v.search;
                if s.token.get() != token {
                    return;
                }
                match found {
                    Found::Page(page, hits) => s.add(page, hits, n),
                    Found::Done => {
                        s.busy.set(false);
                        if s.cur.get().is_none() && !s.hits.borrow().is_empty() {
                            // Nothing after the page being read: wrap round to the first match.
                            s.cur.set(Some(0));
                            s.paint();
                            s.show_current(true);
                        } else {
                            s.show_current(false);
                        }
                        return;
                    }
                }
            }
        });
    }

    /// Matches from one more page arrived (pages come in order).
    fn add(&self, page: usize, hits: Vec<Hit>, n_pages: usize) {
        if !hits.is_empty() {
            let first_new = self.hits.borrow().len();
            self.hits.borrow_mut().extend(hits);
            let jump = self.cur.get().is_none() && page >= self.start.get();
            if jump {
                self.cur.set(Some(first_new));
            }
            self.paint();
            if jump {
                self.show_current(true);
            }
        }
        if self.busy.get() {
            self.show_progress(page + 1, n_pages);
        }
    }

    fn show_progress(&self, done: usize, n_pages: usize) {
        let n = self.hits.borrow().len();
        let pct = done * 100 / n_pages.max(1);
        let found = match self.cur.get() {
            Some(i) => format!("{} / {n}", i + 1),
            None if n > 0 => format!("{n} found"),
            None => "Searching".to_string(),
        };
        self.readout.set_text(&format!("{found} · {pct}%"));
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
                let more = if self.busy.get() { "+" } else { "" };
                self.readout.set_text(&format!("{} / {n}{more}", i + 1));
                if scroll && let Some(v) = view() {
                    let h = self.hits.borrow()[i];
                    reveal(&v, h);
                }
            }
            _ if self.busy.get() => {}
            _ => self.readout.set_text(if self.entry.text().trim().is_empty() { "" } else { "No matches" }),
        }
    }
}

/// Scroll so a hit is on screen.
fn reveal(v: &std::rc::Rc<super::View>, h: Hit) {
    let Some(d) = doc::current() else { return };
    d.set_page(h.page);
    if !v.continuous() {
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

/// Notes and text boxes whose text contains `query`, as hits on their pages.
fn annotation_hits(query: &str, case: bool) -> Vec<Hit> {
    let Some(lo) = view().and_then(|v| v.lo()) else { return Vec::new() };
    let fold = |s: &str| if case { s.to_string() } else { s.to_lowercase() };
    let q = fold(query);
    annots::read_all(&lo)
        .into_iter()
        .filter(|(_, a)| a.kind.has_text() && a.kind != annots::Kind::Edit && fold(&a.contents).contains(&q))
        .map(|(page, a)| Hit { page, rect: a.rect })
        .collect()
}

/// Every match of `query` in the file, page by page, with its place on the displayed
/// page. Runs on its own thread; `notes` are merged in on their pages.
fn find_all(
    path: &std::path::Path,
    password: Option<&str>,
    query: &str,
    flags: poppler::FindFlags,
    notes: Vec<Hit>,
    cancel: &AtomicBool,
    tx: &async_channel::Sender<Found>,
) {
    use gtk::gio::prelude::FileExt;
    let uri = gtk::gio::File::for_path(path).uri();
    if let Ok(pdf) = poppler::Document::from_file(&uri, password) {
        for i in 0..pdf.n_pages() {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let Some(page) = pdf.page(i) else { continue };
            let (_, ph) = page.size();
            // Poppler gives these with y up.
            let mut hits: Vec<Hit> = page
                .find_text_with_options(query, flags)
                .into_iter()
                .map(|r| Hit { page: i as usize, rect: Rect::new(r.x1(), ph - r.y2(), r.x2(), ph - r.y1()) })
                .collect();
            hits.extend(notes.iter().filter(|h| h.page == i as usize).copied());
            if tx.send_blocking(Found::Page(i as usize, hits)).is_err() {
                return;
            }
        }
    }
    let _ = tx.send_blocking(Found::Done);
}

impl Search {
    pub fn on_change(&self, c: Change) {
        if c == Change::Structure {
            self.rerun();
        }
    }
}
