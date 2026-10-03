//! The open document. Every edit writes a new "generation" file in a working
//! folder (`gen-0.pdf` is the original), which gives undo and redo for free and
//! keeps the user's file untouched until they save. Poppler reads whichever
//! generation is current; lopdf writes the next one.

pub mod annots;
pub mod geom;
pub mod links;
pub mod ops;
pub mod render;
pub mod text;

use crate::{cmd, paths};
use anyhow::{Context, Result, bail};
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Change {
    /// A different file is open now.
    Opened,
    /// The file was closed.
    Closed,
    /// Page contents changed (markup, form values): re-render.
    Content,
    /// The number or order of pages changed: rebuild.
    Structure,
    /// Saved, or the unsaved marker moved.
    Dirty,
    /// The current page moved.
    Page,
}

type Listener = Rc<dyn Fn(Change)>;

thread_local! {
    static CURRENT: RefCell<Option<Rc<Doc>>> = const { RefCell::new(None) };
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
}

pub fn current() -> Option<Rc<Doc>> {
    CURRENT.with(|c| c.borrow().clone())
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
        };
        doc.read_sizes();
        Ok(Rc::new(doc))
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
        let before = self.n_pages();
        self.read_sizes();
        let structural = structural || self.n_pages() != before;
        if self.page.get() >= self.n_pages() {
            self.page.set(self.n_pages().saturating_sub(1));
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
        self.saved_gen.set(self.gen_no.get());
        emit(Change::Dirty);
        Ok(())
    }

    /// Save under a new name; the document is now that file.
    pub fn save_as(&self, path: &Path) -> Result<()> {
        self.write_copy(path)?;
        *self.path.borrow_mut() = path.to_path_buf();
        self.saved_gen.set(self.gen_no.get());
        crate::recent::record(path, self.n_pages(), self.page.get());
        emit(Change::Dirty);
        Ok(())
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
        let _ = std::fs::remove_dir_all(&self.work);
    }
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
    match Doc::open(&path, password) {
        Ok(doc) => {
            let start = page.unwrap_or_else(|| if crate::prefs::get().remember_page { crate::recent::last_page(&path) } else { 0 });
            doc.page.set(start.min(doc.n_pages() - 1));
            crate::recent::record(&path, doc.n_pages(), doc.page.get());
            let old = CURRENT.with(|c| c.replace(Some(doc.clone())));
            drop(old);
            emit(Change::Opened);
            emit(Change::Dirty);
            crate::window::navigate("document");
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

/// Open a file, asking first when the current document has unsaved changes.
pub fn request_open(path: PathBuf, page: Option<usize>) {
    guard_unsaved(move || open(&path, page));
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

pub fn close() {
    let old = CURRENT.with(|c| c.replace(None));
    if old.is_some() {
        drop(old);
        emit(Change::Closed);
    }
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
    let dialog = gtk::FileDialog::builder().title("Open a PDF").modal(true).build();
    let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&pdf_filter());
    dialog.set_filters(Some(&filters));
    let start = current().and_then(|d| d.path().parent().map(|p| p.to_path_buf())).unwrap_or_else(paths::documents_dir);
    if start.is_dir() {
        dialog.set_initial_folder(Some(&gtk::gio::File::for_path(start)));
    }
    dialog.open(crate::window::window().as_ref(), gtk::gio::Cancellable::NONE, |res| {
        let Ok(file) = res else { return };
        if let Some(path) = file.path() {
            request_open(path, None);
        }
    });
}

/// Run `f` on a worker thread and hand the result back on the main one.
pub fn background<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static, done: impl FnOnce(T) + 'static) {
    cmd::background(work, done);
}
