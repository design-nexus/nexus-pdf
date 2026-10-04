//! The open documents. Every edit writes a new "generation" file in a working
//! folder (`gen-0.pdf` is the original), which gives undo and redo for free and
//! keeps the user's file untouched until they save. Poppler reads whichever
//! generation is current; lopdf writes the next one. Several files can be open;
//! one of them is current, and the viewer shows that one.

pub mod annots;
pub mod geom;
pub mod links;
pub mod ops;
pub mod render;
pub mod text;

use crate::{cmd, paths};
use anyhow::{Context, Result, bail};
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::time::SystemTime;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Change {
    /// A different file is open now.
    Opened,
    /// The last open file was closed.
    Closed,
    /// Page contents changed (markup, form values): re-render.
    Content,
    /// The number or order of pages changed: rebuild.
    Structure,
    /// Saved, the unsaved marker moved, or the file changed on disk.
    Dirty,
    /// The current page moved.
    Page,
    /// A file was opened or closed but the current one stayed (the tab list changed).
    Files,
}

type Listener = Rc<dyn Fn(Change)>;

thread_local! {
    static CURRENT: RefCell<Option<Rc<Doc>>> = const { RefCell::new(None) };
    /// Every open file, in tab order.
    static OPEN: RefCell<Vec<Rc<Doc>>> = const { RefCell::new(Vec::new()) };
    static SUBS: RefCell<Vec<(glib::WeakRef<gtk::Widget>, Listener)>> = const { RefCell::new(Vec::new()) };
    static COUNTER: Cell<u32> = const { Cell::new(0) };
}

pub struct Doc {
    path: RefCell<PathBuf>,
    work: PathBuf,
    gen_no: Cell<usize>,
    max_gen: Cell<usize>,
    saved_gen: Cell<usize>,
    pdf: RefCell<poppler::Document>,
    password: Option<String>,
    sizes: RefCell<Vec<(f64, f64)>>,
    editable: bool,
    page: Cell<usize>,
    /// Pages to go back and forward to after following links.
    back: RefCell<Vec<usize>>,
    forward: RefCell<Vec<usize>>,
    /// The file's modification time and size when it was last read or written.
    stamp: Cell<Option<(SystemTime, u64)>>,
    monitor: RefCell<Option<gio::FileMonitor>>,
    settle: Cell<Option<glib::SourceId>>,
    /// The file on disk changed while there were unsaved edits.
    changed_on_disk: Cell<bool>,
}

pub fn current() -> Option<Rc<Doc>> {
    CURRENT.with(|c| c.borrow().clone())
}

/// Every open file, in tab order.
pub fn all() -> Vec<Rc<Doc>> {
    OPEN.with(|o| o.borrow().clone())
}

/// Make an open file the current one.
pub fn switch_to(doc: &Rc<Doc>) {
    if current().is_some_and(|c| Rc::ptr_eq(&c, doc)) {
        return;
    }
    CURRENT.with(|c| *c.borrow_mut() = Some(doc.clone()));
    emit(Change::Opened);
    emit(Change::Dirty);
}

/// Step through the open files (Ctrl+Tab).
pub fn cycle(by: i64) {
    let open = all();
    let Some(cur) = current() else { return };
    let Some(i) = open.iter().position(|d| Rc::ptr_eq(d, &cur)) else { return };
    let next = (i as i64 + by).rem_euclid(open.len() as i64) as usize;
    switch_to(&open[next]);
}

/// Call `f` on every change, for as long as `owner` lives.
pub fn subscribe(owner: &impl IsA<gtk::Widget>, f: impl Fn(Change) + 'static) {
    SUBS.with(|s| s.borrow_mut().push((owner.upcast_ref::<gtk::Widget>().downgrade(), Rc::new(f))));
}

pub fn emit(change: Change) {
    let subs: Vec<Listener> = SUBS.with(|s| {
        s.borrow_mut().retain(|(w, _)| w.upgrade().is_some());
        s.borrow().iter().map(|(_, f)| f.clone()).collect()
    });
    for f in subs {
        f(change);
    }
}

fn uri(path: &Path) -> String {
    gtk::gio::File::for_path(path).uri().to_string()
}

fn open_poppler(path: &Path, password: Option<&str>) -> Result<poppler::Document> {
    poppler::Document::from_file(&uri(path), password).map_err(|e| anyhow::anyhow!("{}", e.message()))
}

/// True when the error means a password is needed.
pub fn needs_password(err: &anyhow::Error) -> bool {
    let m = err.to_string().to_lowercase();
    m.contains("encrypted") || m.contains("password")
}

impl Doc {
    pub fn open(path: &Path, password: Option<String>) -> Result<Rc<Doc>> {
        let n = COUNTER.with(|c| c.replace(c.get() + 1));
        let work = paths::work_dir().join(format!("{}-{n}", std::process::id()));
        std::fs::create_dir_all(&work).with_context(|| format!("couldn't make {}", work.display()))?;
        let first = work.join("gen-0.pdf");
        std::fs::copy(path, &first).with_context(|| format!("couldn't read {}", path.display()))?;
        let pdf = match open_poppler(&first, password.as_deref()) {
            Ok(p) => p,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&work);
                return Err(e);
            }
        };
        if pdf.n_pages() < 1 {
            let _ = std::fs::remove_dir_all(&work);
            bail!("This file has no pages.");
        }
        let editable = password.is_none() && lopdf::Document::load(&first).is_ok_and(|d| !d.is_encrypted());
        let doc = Doc {
            path: RefCell::new(path.to_path_buf()),
            work,
            gen_no: Cell::new(0),
            max_gen: Cell::new(0),
            saved_gen: Cell::new(0),
            pdf: RefCell::new(pdf),
            password,
            sizes: RefCell::new(Vec::new()),
            editable,
            page: Cell::new(0),
            back: RefCell::new(Vec::new()),
            forward: RefCell::new(Vec::new()),
            stamp: Cell::new(file_stamp(path)),
            monitor: RefCell::new(None),
            settle: Cell::new(None),
            changed_on_disk: Cell::new(false),
        };
        doc.read_sizes();
        doc.write_state();
        let doc = Rc::new(doc);
        doc.watch();
        Ok(doc)
    }

    /// Record where this copy stands, so a crash can be recovered from (see `recoverable`).
    fn write_state(&self) {
        let text = format!("{} {}\n{}\n", self.gen_no.get(), self.saved_gen.get(), self.path().display());
        let _ = std::fs::write(self.work.join("state"), text);
    }

    /// Notice when another program changes the file.
    fn watch(self: &Rc<Self>) {
        let file = gio::File::for_path(self.path());
        let Ok(monitor) = file.monitor_file(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE) else { return };
        let me: Weak<Doc> = Rc::downgrade(self);
        monitor.connect_changed(move |_, _, _, _| {
            let Some(d) = me.upgrade() else { return };
            // Editors and compilers write in bursts; look once they've settled.
            if let Some(id) = d.settle.take() {
                id.remove();
            }
            let me = me.clone();
            let id = glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
                if let Some(d) = me.upgrade() {
                    d.settle.set(None);
                    d.check_disk();
                }
            });
            d.settle.set(Some(id));
        });
        *self.monitor.borrow_mut() = Some(monitor);
    }

    fn check_disk(self: &Rc<Self>) {
        let path = self.path();
        let now = file_stamp(&path);
        if now.is_none() || now == self.stamp.get() {
            return;
        }
        self.stamp.set(now);
        if self.dirty() {
            self.changed_on_disk.set(true);
            emit(Change::Dirty);
        } else {
            reload(self);
        }
    }

    /// The file on disk changed while this copy had unsaved edits.
    pub fn changed_on_disk(&self) -> bool {
        self.changed_on_disk.get()
    }

    /// Keep the unsaved edits and stop warning about the change on disk.
    pub fn ignore_disk_change(&self) {
        self.changed_on_disk.set(false);
        emit(Change::Dirty);
    }

    /// Note the page being left before a jump, so Back returns to it.
    pub fn remember(&self) {
        let here = self.page.get();
        let mut back = self.back.borrow_mut();
        if back.last() != Some(&here) {
            back.push(here);
            if back.len() > 100 {
                back.remove(0);
            }
        }
        self.forward.borrow_mut().clear();
    }

    pub fn can_go_back(&self) -> bool {
        !self.back.borrow().is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward.borrow().is_empty()
    }

    /// The page to go back to, if any (the current one becomes "forward").
    pub fn take_back(&self) -> Option<usize> {
        let p = self.back.borrow_mut().pop()?;
        self.forward.borrow_mut().push(self.page.get());
        Some(p)
    }

    pub fn take_forward(&self) -> Option<usize> {
        let p = self.forward.borrow_mut().pop()?;
        self.back.borrow_mut().push(self.page.get());
        Some(p)
    }

    fn read_sizes(&self) {
        let pdf = self.pdf.borrow();
        let sizes = (0..pdf.n_pages()).map(|i| pdf.page(i).map(|p| p.size()).unwrap_or((612.0, 792.0))).collect();
        *self.sizes.borrow_mut() = sizes;
    }

    pub fn path(&self) -> PathBuf {
        self.path.borrow().clone()
    }

    pub fn gen_path(&self) -> PathBuf {
        self.path_of(self.gen_no.get())
    }

    fn path_of(&self, gen_no: usize) -> PathBuf {
        self.work.join(format!("gen-{gen_no}.pdf"))
    }

    /// Bumped by every edit; renders are keyed by it.
    pub fn generation(&self) -> usize {
        self.gen_no.get()
    }

    pub fn pdf(&self) -> poppler::Document {
        self.pdf.borrow().clone()
    }

    pub fn n_pages(&self) -> usize {
        self.sizes.borrow().len()
    }

    /// Page size in points, as displayed (rotation applied).
    pub fn page_size(&self, i: usize) -> (f64, f64) {
        self.sizes.borrow().get(i).copied().unwrap_or((612.0, 792.0))
    }

    pub fn password(&self) -> Option<String> {
        self.password.clone()
    }

    pub fn editable(&self) -> bool {
        self.editable
    }

    pub fn title(&self) -> String {
        let t = self.pdf.borrow().title().map(|t| t.to_string()).unwrap_or_default();
        if t.trim().is_empty() { self.file_name() } else { t.trim().to_string() }
    }

    pub fn file_name(&self) -> String {
        self.path.borrow().file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }

    pub fn dirty(&self) -> bool {
        self.gen_no.get() != self.saved_gen.get()
    }

    pub fn can_undo(&self) -> bool {
        self.gen_no.get() > 0
    }

    pub fn can_redo(&self) -> bool {
        self.gen_no.get() < self.max_gen.get()
    }

    pub fn page(&self) -> usize {
        self.page.get()
    }

    pub fn set_page(&self, i: usize) {
        let i = i.min(self.n_pages().saturating_sub(1));
        if self.page.replace(i) != i {
            crate::recent::set_page(&self.path(), i);
            emit(Change::Page);
        }
    }

    /// Run `f` on the current generation with lopdf, write the result as the next
    /// generation, and show it. On any error nothing changes.
    pub fn edit(&self, structural: bool, f: impl FnOnce(&mut lopdf::Document) -> Result<()>) -> Result<()> {
        if !self.editable {
            bail!("This file can't be edited: it's encrypted or damaged.");
        }
        let mut lo = lopdf::Document::load(self.gen_path()).context("couldn't read the file to edit it")?;
        f(&mut lo)?;
        let next = self.gen_no.get() + 1;
        let target = self.path_of(next);
        lo.save(&target).with_context(|| format!("couldn't write {}", target.display()))?;
        self.adopt(next, structural)
    }

    /// Change form fields through poppler, then keep the result as the next generation.
    pub fn edit_forms(&self, f: impl FnOnce(&poppler::Document)) -> Result<()> {
        let next = self.gen_no.get() + 1;
        let target = self.path_of(next);
        f(&self.pdf.borrow());
        self.pdf.borrow().save(&uri(&target)).map_err(|e| anyhow::anyhow!("{}", e.message()))?;
        self.adopt(next, false)
    }

    /// Make generation `next` (already written) the current one.
    fn adopt(&self, next: usize, structural: bool) -> Result<()> {
        let target = self.path_of(next);
        let pdf = match open_poppler(&target, self.password.as_deref()) {
            Ok(p) => p,
            Err(e) => {
                let _ = std::fs::remove_file(&target);
                return Err(e.context("the edited file wouldn't open, so the change was dropped"));
            }
        };
        *self.pdf.borrow_mut() = pdf;
        self.gen_no.set(next);
        self.max_gen.set(next);
        self.after_move(structural);
        Ok(())
    }

    fn after_move(&self, structural: bool) {
        self.write_state();
        let before = self.n_pages();
        self.read_sizes();
        let structural = structural || self.n_pages() != before;
        if self.page.get() >= self.n_pages() {
            self.page.set(self.n_pages().saturating_sub(1));
        }
        if structural {
            // Page numbers may mean different pages now.
            self.back.borrow_mut().clear();
            self.forward.borrow_mut().clear();
        }
        emit(if structural { Change::Structure } else { Change::Content });
        emit(Change::Dirty);
    }

    fn goto_gen(&self, g: usize) -> Result<()> {
        let pdf = open_poppler(&self.path_of(g), self.password.as_deref())?;
        *self.pdf.borrow_mut() = pdf;
        self.gen_no.set(g);
        // Undo and redo may add or remove pages; always rebuild.
        self.after_move(true);
        Ok(())
    }

    pub fn undo(&self) -> Result<()> {
        if self.can_undo() { self.goto_gen(self.gen_no.get() - 1) } else { Ok(()) }
    }

    pub fn redo(&self) -> Result<()> {
        if self.can_redo() { self.goto_gen(self.gen_no.get() + 1) } else { Ok(()) }
    }

    /// Write the current generation to the file the user opened.
    pub fn save(&self) -> Result<()> {
        let path = self.path();
        self.write_copy(&path)?;
        self.stamp.set(file_stamp(&path));
        self.changed_on_disk.set(false);
        self.saved_gen.set(self.gen_no.get());
        self.write_state();
        emit(Change::Dirty);
        Ok(())
    }

    /// Save under a new name; the document is now that file.
    pub fn save_as(self: &Rc<Self>, path: &Path) -> Result<()> {
        self.write_copy(path)?;
        *self.path.borrow_mut() = path.to_path_buf();
        self.stamp.set(file_stamp(path));
        self.changed_on_disk.set(false);
        self.saved_gen.set(self.gen_no.get());
        self.write_state();
        self.watch();
        crate::recent::record(path, self.n_pages(), self.page.get());
        crate::recent::set_title(path, &self.title());
        emit(Change::Dirty);
        Ok(())
    }

    /// Take a file written elsewhere (a recovered generation) as the next edit.
    fn adopt_file(&self, file: &Path) -> Result<()> {
        let next = self.gen_no.get() + 1;
        std::fs::copy(file, self.path_of(next)).with_context(|| format!("couldn't read {}", file.display()))?;
        self.adopt(next, true)
    }

    /// Copy the current generation to `path` atomically (temporary file, then rename).
    pub fn write_copy(&self, path: &Path) -> Result<()> {
        let dir = path.parent().context("that path has no folder")?;
        let tmp = dir.join(format!(".{}.tmp-{}", path.file_name().unwrap_or_default().to_string_lossy(), std::process::id()));
        std::fs::copy(self.gen_path(), &tmp).with_context(|| format!("couldn't write in {}", dir.display()))?;
        std::fs::rename(&tmp, path).with_context(|| format!("couldn't write {}", path.display()))?;
        Ok(())
    }
}

impl Drop for Doc {
    fn drop(&mut self) {
        if let Some(id) = self.settle.take() {
            id.remove();
        }
        if let Some(m) = self.monitor.take() {
            m.cancel();
        }
        let _ = std::fs::remove_dir_all(&self.work);
    }
}

fn file_stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.modified().ok()?, m.len()))
}

/// Read a file again after another program changed it, keeping the page.
fn reload(old: &Rc<Doc>) {
    let path = old.path();
    let Ok(doc) = Doc::open(&path, old.password()) else { return };
    doc.page.set(old.page().min(doc.n_pages() - 1));
    let was_current = current().is_some_and(|c| Rc::ptr_eq(&c, old));
    OPEN.with(|o| {
        let mut o = o.borrow_mut();
        if let Some(slot) = o.iter_mut().find(|d| Rc::ptr_eq(d, old)) {
            *slot = doc.clone();
        }
    });
    if was_current {
        CURRENT.with(|c| *c.borrow_mut() = Some(doc.clone()));
        emit(Change::Opened);
        emit(Change::Dirty);
        crate::window::toast(&format!("{} changed on disk, so it was read again.", doc.file_name()));
    } else {
        emit(Change::Files);
    }
}

/// Throw away the unsaved edits of `doc` and read its file again.
pub fn reload_discarding(doc: &Rc<Doc>) {
    reload(doc);
}

/// Open `path`, replacing the current document. The caller has already dealt
/// with unsaved changes (see `request_open`).
pub fn open(path: &Path, page: Option<usize>) {
    open_with(path, page, None);
}

fn open_with(path: &Path, page: Option<usize>, password: Option<String>) {
    let path = path.to_path_buf();
    if !path.is_file() {
        crate::window::toast(&format!("{} doesn't exist.", paths::pretty(&path)));
        return;
    }
    // Already open: show that tab.
    let same = |d: &Rc<Doc>| d.path() == path || std::fs::canonicalize(d.path()).ok() == std::fs::canonicalize(&path).ok();
    if let Some(doc) = all().into_iter().find(same) {
        switch_to(&doc);
        if let Some(p) = page {
            doc.set_page(p);
            crate::viewer::goto_page(p);
        }
        show_document();
        return;
    }
    match Doc::open(&path, password) {
        Ok(doc) => {
            let start = page.unwrap_or_else(|| if crate::prefs::get().remember_page { crate::recent::last_page(&path) } else { 0 });
            doc.page.set(start.min(doc.n_pages() - 1));
            crate::recent::record(&path, doc.n_pages(), doc.page.get());
            crate::recent::set_title(&path, &doc.title());
            // A new tab goes after the current one.
            OPEN.with(|o| {
                let mut o = o.borrow_mut();
                let at = current().and_then(|c| o.iter().position(|d| Rc::ptr_eq(d, &c))).map_or(o.len(), |i| i + 1);
                o.insert(at, doc.clone());
            });
            CURRENT.with(|c| *c.borrow_mut() = Some(doc.clone()));
            emit(Change::Opened);
            emit(Change::Dirty);
            show_document();
            if !doc.editable() {
                crate::window::toast("Opened for reading only: this file can't be edited.");
            }
        }
        Err(e) if needs_password(&e) => ask_password(path, page),
        Err(e) => crate::window::toast(&format!("Couldn't open {}: {e}", crate::paths::pretty(&path))),
    }
}

fn ask_password(path: PathBuf, page: Option<usize>) {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let extra = crate::widgets::ask_text(
        "Password needed",
        &format!("{name} is protected. Enter its password to read it."),
        "",
        "Open",
        move |pw| open_with(&path, page, Some(pw)),
    );
    drop(extra);
}

/// Show the document page, unless the user is organising pages, where a newly
/// opened file can be organised just as well.
fn show_document() {
    if crate::window::current() != "pages" {
        crate::window::navigate("document");
    }
}

/// Open a file in a new tab (or show its tab if it's open already).
pub fn request_open(path: PathBuf, page: Option<usize>) {
    open(&path, page);
}

/// Run `then` now, or after the user decides what to do about unsaved changes.
pub fn guard_unsaved(then: impl FnOnce() + 'static) {
    let Some(doc) = current().filter(|d| d.dirty()) else {
        then();
        return;
    };
    let (dialog, card) = crate::widgets::dialog("Save your changes?", 440);
    let d = crate::widgets::label(&format!("{} has changes that aren't saved yet.", doc.file_name()), "dim");
    d.set_wrap(true);
    card.append(&d);
    let buttons = crate::widgets::hbox(8);
    buttons.set_halign(gtk::Align::End);
    buttons.set_margin_top(6);
    let keep = gtk::Button::with_label("Keep editing");
    let discard = gtk::Button::with_label("Discard");
    discard.add_css_class("destructive-action");
    let save = gtk::Button::with_label("Save");
    save.add_css_class("suggested-action");
    buttons.append(&keep);
    buttons.append(&discard);
    buttons.append(&save);
    card.append(&buttons);
    let then = Rc::new(RefCell::new(Some(then)));
    {
        let d = dialog.clone();
        keep.connect_clicked(move |_| d.close());
    }
    {
        let (d, then) = (dialog.clone(), then.clone());
        discard.connect_clicked(move |_| {
            d.close();
            if let Some(f) = then.borrow_mut().take() {
                f();
            }
        });
    }
    {
        let (d, then, doc) = (dialog.clone(), then.clone(), doc.clone());
        save.connect_clicked(move |_| match doc.save() {
            Ok(()) => {
                d.close();
                if let Some(f) = then.borrow_mut().take() {
                    f();
                }
            }
            Err(e) => crate::window::toast(&format!("Couldn't save: {e}")),
        });
    }
    dialog.present();
}

/// Close the current file; the neighbouring tab becomes current.
pub fn close() {
    let Some(old) = current() else { return };
    let next = OPEN.with(|o| {
        let mut o = o.borrow_mut();
        let i = o.iter().position(|d| Rc::ptr_eq(d, &old))?;
        o.remove(i);
        o.get(i).or_else(|| o.last()).cloned()
    });
    CURRENT.with(|c| *c.borrow_mut() = next.clone());
    drop(old);
    if next.is_some() {
        emit(Change::Opened);
        emit(Change::Dirty);
    } else {
        emit(Change::Closed);
    }
}

/// Close every file (on quit).
pub fn close_all() {
    CURRENT.with(|c| c.borrow_mut().take());
    OPEN.with(|o| o.borrow_mut().clear());
}

/// Run `then` once every file with unsaved changes has been saved or let go of.
/// Each one is shown and asked about in turn; "Discard" closes it.
pub fn guard_all(then: impl FnOnce() + 'static) {
    let Some(doc) = all().into_iter().find(|d| d.dirty()) else {
        then();
        return;
    };
    switch_to(&doc);
    crate::window::navigate("document");
    guard_unsaved(move || {
        if doc.dirty() {
            // Discarded: let it go so the next one is asked about.
            OPEN.with(|o| o.borrow_mut().retain(|d| !Rc::ptr_eq(d, &doc)));
            if current().is_some_and(|c| Rc::ptr_eq(&c, &doc)) {
                CURRENT.with(|c| *c.borrow_mut() = all().first().cloned());
            }
        }
        guard_all(then);
    });
}

// ---------- Recovering from a crash ----------

/// A working folder left behind by a run that ended without closing its files.
pub struct Leftover {
    dir: PathBuf,
    /// The file that was being edited.
    pub source: PathBuf,
    /// The generation that was showing.
    latest: PathBuf,
}

/// True when the process that owns `dir` (named `<pid>-<n>`) is still running this app.
fn owner_alive(dir: &Path) -> bool {
    let Some(pid) = dir.file_name().and_then(|n| n.to_str()).and_then(|n| n.split('-').next()).and_then(|p| p.parse::<u32>().ok()) else {
        return false;
    };
    if pid == std::process::id() {
        return true;
    }
    let exe = std::env::current_exe().ok().and_then(|e| e.file_name().map(|n| n.to_owned()));
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok();
    match (comm, exe) {
        (Some(c), Some(e)) => c.trim() == e.to_string_lossy().chars().take(15).collect::<String>(),
        (Some(_), None) => true,
        _ => false,
    }
}

/// Working folders whose owner has gone. Those with unsaved edits are returned;
/// the rest are deleted.
pub fn leftovers() -> Vec<Leftover> {
    leftovers_in(&paths::work_dir())
}

fn leftovers_in(work: &Path) -> Vec<Leftover> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(work) else { return out };
    for e in rd.flatten() {
        let dir = e.path();
        if !dir.is_dir() || owner_alive(&dir) {
            continue;
        }
        let state = std::fs::read_to_string(dir.join("state")).unwrap_or_default();
        let mut lines = state.lines();
        let gens: Vec<usize> = lines.next().unwrap_or("").split_whitespace().filter_map(|n| n.parse().ok()).collect();
        let source = lines.next().map(PathBuf::from);
        match (gens.as_slice(), source) {
            (&[gen_no, saved], Some(source)) if gen_no != saved && source.is_file() && dir.join(format!("gen-{gen_no}.pdf")).is_file() => {
                out.push(Leftover { latest: dir.join(format!("gen-{gen_no}.pdf")), dir, source });
            }
            _ => {
                let _ = std::fs::remove_dir_all(&dir);
            }
        }
    }
    out
}

/// Delete every working folder whose owner has gone, unsaved edits and all.
/// Returns how many were removed.
pub fn clear_leftovers() -> usize {
    let mut n = 0;
    if let Ok(rd) = std::fs::read_dir(paths::work_dir()) {
        for e in rd.flatten() {
            let dir = e.path();
            if dir.is_dir() && !owner_alive(&dir) && std::fs::remove_dir_all(&dir).is_ok() {
                n += 1;
            }
        }
    }
    n
}

/// Ask about each file that had unsaved edits when the app last stopped.
pub fn offer_recovery() {
    let mut list = leftovers();
    let Some(next) = list.pop() else { return };
    let name = next.source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let (dialog, card) = crate::widgets::dialog("Recover your changes?", 460);
    let d = crate::widgets::label(
        &format!("{name} had changes that weren't saved when Nexus PDF last closed. Recover them to keep working, then save."),
        "dim",
    );
    d.set_wrap(true);
    card.append(&d);
    let buttons = crate::widgets::hbox(8);
    buttons.set_halign(gtk::Align::End);
    buttons.set_margin_top(6);
    let discard = gtk::Button::with_label("Discard");
    discard.add_css_class("destructive-action");
    let recover = gtk::Button::with_label("Recover");
    recover.add_css_class("suggested-action");
    buttons.append(&discard);
    buttons.append(&recover);
    card.append(&buttons);
    let next = Rc::new(next);
    {
        let (dl, next) = (dialog.clone(), next.clone());
        discard.connect_clicked(move |_| {
            let _ = std::fs::remove_dir_all(&next.dir);
            dl.close();
            offer_recovery();
        });
    }
    {
        let (dl, next) = (dialog.clone(), next.clone());
        recover.connect_clicked(move |_| {
            dl.close();
            open(&next.source, None);
            let opened = current().filter(|d| d.path() == next.source);
            match opened.map(|d| d.adopt_file(&next.latest)) {
                Some(Ok(())) => {
                    let _ = std::fs::remove_dir_all(&next.dir);
                    crate::window::toast("Recovered. Undo goes back to the file as it's saved.");
                }
                Some(Err(e)) => crate::window::toast(&format!("Couldn't recover the changes: {e:#}")),
                None => {}
            }
            offer_recovery();
        });
    }
    dialog.present();
}

/// Run a fallible edit and report a failure as a toast. True on success.
pub fn try_edit(structural: bool, f: impl FnOnce(&mut lopdf::Document) -> Result<()>) -> bool {
    let Some(doc) = current() else { return false };
    match doc.edit(structural, f) {
        Ok(()) => true,
        Err(e) => {
            crate::window::toast(&format!("{e:#}"));
            false
        }
    }
}

pub fn undo() {
    if let Some(d) = current()
        && let Err(e) = d.undo()
    {
        crate::window::toast(&format!("Couldn't undo: {e:#}"));
    }
}

pub fn redo() {
    if let Some(d) = current()
        && let Err(e) = d.redo()
    {
        crate::window::toast(&format!("Couldn't redo: {e:#}"));
    }
}

pub fn save() {
    let Some(doc) = current() else { return };
    match doc.save() {
        Ok(()) => crate::window::toast(&format!("Saved {}.", doc.file_name())),
        Err(e) => crate::window::toast(&format!("Couldn't save: {e:#}")),
    }
}

/// Print the current file (as it is now, markup included).
pub fn print() {
    print_to(None);
}

/// Print, or with `export` write what would be printed to that PDF instead (for checking).
pub fn print_to(export: Option<&Path>) {
    let Some(doc) = current() else { return };
    let op = gtk::PrintOperation::new();
    op.set_n_pages(doc.n_pages() as i32);
    op.set_job_name(&doc.title());
    op.set_use_full_page(true);
    op.set_embed_page_setup(true);
    let pdf = doc.pdf();
    op.connect_draw_page(move |_, ctx, n| {
        let Some(page) = pdf.page(n) else { return };
        let (pw, ph) = page.size();
        let (w, h) = (ctx.width(), ctx.height());
        let cr = ctx.cairo_context();
        // Fit the page on the paper, turning landscape pages to match landscape paper.
        let turn = (pw > ph) != (w > h);
        let (fw, fh) = if turn { (ph, pw) } else { (pw, ph) };
        let s = (w / fw).min(h / fh);
        cr.translate((w - fw * s) / 2.0, (h - fh * s) / 2.0);
        cr.scale(s, s);
        if turn {
            cr.translate(ph, 0.0);
            cr.rotate(std::f64::consts::FRAC_PI_2);
        }
        page.render_for_printing(&cr);
    });
    let parent = crate::window::window();
    let action = match export {
        Some(path) => {
            op.set_export_filename(path);
            gtk::PrintOperationAction::Export
        }
        None => {
            // Don't hold up the window while the dialog is open.
            op.set_allow_async(true);
            gtk::PrintOperationAction::PrintDialog
        }
    };
    op.connect_done(|_, result| {
        if result == gtk::PrintOperationResult::Error {
            crate::window::toast("Couldn't print.");
        }
    });
    if let Err(e) = op.run(action, parent.as_ref()) {
        crate::window::toast(&format!("Couldn't print: {e}"));
    }
}

pub fn save_as_dialog() {
    let Some(doc) = current() else { return };
    let dialog = gtk::FileDialog::builder().title("Save a copy").modal(true).initial_name(doc.file_name()).build();
    if let Some(dir) = doc.path().parent() {
        dialog.set_initial_folder(Some(&gtk::gio::File::for_path(dir)));
    }
    dialog.save(crate::window::window().as_ref(), gtk::gio::Cancellable::NONE, move |res| {
        let Ok(file) = res else { return };
        let Some(mut path) = file.path() else { return };
        if path.extension().is_none() {
            path.set_extension("pdf");
        }
        match doc.save_as(&path) {
            Ok(()) => crate::window::toast(&format!("Saved {}.", paths::pretty(&path))),
            Err(e) => crate::window::toast(&format!("Couldn't save: {e:#}")),
        }
    });
}

pub fn pdf_filter() -> gtk::FileFilter {
    let f = gtk::FileFilter::new();
    f.set_name(Some("PDF files"));
    f.add_mime_type("application/pdf");
    f.add_suffix("pdf");
    f
}

pub fn open_dialog() {
    let dialog = gtk::FileDialog::builder().title("Open PDFs").modal(true).build();
    let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&pdf_filter());
    dialog.set_filters(Some(&filters));
    let start = current().and_then(|d| d.path().parent().map(|p| p.to_path_buf())).unwrap_or_else(paths::documents_dir);
    if start.is_dir() {
        dialog.set_initial_folder(Some(&gtk::gio::File::for_path(start)));
    }
    dialog.open_multiple(crate::window::window().as_ref(), gtk::gio::Cancellable::NONE, |res| {
        let Ok(list) = res else { return };
        for i in 0..list.n_items() {
            if let Some(path) = list.item(i).and_downcast::<gtk::gio::File>().and_then(|f| f.path()) {
                request_open(path, None);
            }
        }
    });
}

/// Run `f` on a worker thread and hand the result back on the main one.
pub fn background<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static, done: impl FnOnce(T) + 'static) {
    cmd::background(work, done);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leftover(work: &Path, name: &str, state: &str, gens: &[usize]) -> PathBuf {
        let dir = work.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        for g in gens {
            std::fs::write(dir.join(format!("gen-{g}.pdf")), b"%PDF").unwrap();
        }
        std::fs::write(dir.join("state"), state).unwrap();
        dir
    }

    #[test]
    fn finds_unsaved_leftovers_and_clears_the_rest() {
        let work = std::env::temp_dir().join(format!("npdf-leftovers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&work);
        let source = work.join("source.pdf");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(&source, b"%PDF").unwrap();
        // pid 1 is never this app, so these count as abandoned.
        let unsaved = leftover(&work, "1-1", &format!("2 0\n{}\n", source.display()), &[0, 1, 2]);
        let saved = leftover(&work, "1-2", &format!("1 1\n{}\n", source.display()), &[0, 1]);
        let gone = leftover(&work, "1-3", "1 0\n/nowhere/at/all.pdf\n", &[0, 1]);
        let ours = leftover(&work, &format!("{}-9", std::process::id()), &format!("1 0\n{}\n", source.display()), &[0, 1]);
        let found = leftovers_in(&work);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].source, source);
        assert_eq!(found[0].latest, unsaved.join("gen-2.pdf"));
        assert!(!saved.exists() && !gone.exists(), "nothing to recover, so deleted");
        assert!(ours.exists(), "a running copy's folder is left alone");
        let _ = std::fs::remove_dir_all(&work);
    }
}
