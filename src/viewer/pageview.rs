//! One page on screen: its picture, the marks drawn over it (selection, search
//! hits, the stroke being drawn) and the gestures that feed the tools.

use super::{View, colour, forms, tools};
use crate::doc::geom::Rect;
use crate::doc::links::{self, Link};
use crate::doc::{Doc, render};
use crate::theme;
use gtk::prelude::*;
use gtk::gdk;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The longest side, in pixels, of a whole-page picture. Past this (deep zoom) the
/// page is drawn at this size and a sharp picture of just the visible part goes on top.
const MAX_SIDE: f64 = 4096.0;

/// Half the size of the resize handle on a selected annotation, in pixels.
pub const HANDLE: f64 = 5.0;

type Key = (usize, i64, u32);

#[derive(Default)]
pub struct PageState {
    /// Search hits on this page, in points.
    pub hits: Vec<Rect>,
    /// Which of `hits` is the current one.
    pub current_hit: Option<usize>,
    /// The stroke being drawn, in points.
    pub ink: Vec<(f64, f64)>,
    /// A rubber band being dragged.
    pub band: Option<Rect>,
    /// Highlights from the text selection drag in progress, drawn live.
    pub preview: Vec<Rect>,
}

pub struct PageView {
    pub index: usize,
    pub frame: gtk::Overlay,
    pub picture: gtk::Picture,
    pub marks: gtk::DrawingArea,
    pub forms: gtk::Fixed,
    /// The sharp picture of the visible part, at deep zoom.
    detail_layer: gtk::Fixed,
    detail: gtk::Picture,
    detail_ticket: RefCell<Option<render::Ticket>>,
    /// What the detail picture shows: the render key and the area in points.
    detail_shown: Cell<Option<(Key, Rect)>>,
    detail_pending: Cell<Option<(Key, Rect)>>,
    pub form_items: RefCell<Vec<(gtk::Widget, Rect)>>,
    pub st: RefCell<PageState>,
    zoom: Rc<Cell<f64>>,
    ticket: RefCell<Option<render::Ticket>>,
    shown: Cell<Option<Key>>,
    pending: Cell<Option<Key>>,
    pub size: Cell<(f64, f64)>,
    forms_ready: Cell<bool>,
    links: RefCell<Option<(usize, Rc<Vec<Link>>)>>,
    drag_origin: Cell<(f64, f64)>,
}

fn rgb(hex: &str) -> (f64, f64, f64) {
    colour::parse(hex).map(|c| (f64::from(c[0]), f64::from(c[1]), f64::from(c[2]))).unwrap_or((0.5, 0.5, 0.5))
}

impl PageView {
    pub fn new(index: usize, zoom: Rc<Cell<f64>>) -> Rc<PageView> {
        let frame = gtk::Overlay::new();
        frame.add_css_class("page-card");
        frame.add_css_class("loading");
        frame.set_halign(gtk::Align::Center);
        frame.set_valign(gtk::Align::Start);
        frame.set_overflow(gtk::Overflow::Hidden);
        let picture = gtk::Picture::new();
        picture.set_can_shrink(true);
        picture.set_content_fit(gtk::ContentFit::Fill);
        frame.set_child(Some(&picture));
        let detail_layer = gtk::Fixed::new();
        detail_layer.set_can_target(false);
        let detail = gtk::Picture::new();
        detail.set_can_shrink(true);
        detail.set_content_fit(gtk::ContentFit::Fill);
        detail.set_visible(false);
        detail_layer.put(&detail, 0.0, 0.0);
        frame.add_overlay(&detail_layer);
        let marks = gtk::DrawingArea::new();
        marks.set_can_target(false);
        frame.add_overlay(&marks);
        let forms = gtk::Fixed::new();
        frame.add_overlay(&forms);

        let pv = Rc::new(PageView {
            index,
            frame,
            picture,
            marks,
            forms,
            detail_layer,
            detail,
            detail_ticket: RefCell::new(None),
            detail_shown: Cell::new(None),
            detail_pending: Cell::new(None),
            form_items: RefCell::new(Vec::new()),
            st: RefCell::new(PageState::default()),
            zoom,
            ticket: RefCell::new(None),
            shown: Cell::new(None),
            pending: Cell::new(None),
            size: Cell::new((612.0, 792.0)),
            forms_ready: Cell::new(false),
            links: RefCell::new(None),
            drag_origin: Cell::new((0.0, 0.0)),
        });
        pv.wire();
        pv
    }

    fn wire(self: &Rc<Self>) {
        let me = Rc::downgrade(self);
        self.marks.set_draw_func(move |_, cr, _, _| {
            if let Some(pv) = me.upgrade() {
                pv.draw(cr);
            }
        });

        let drag = gtk::GestureDrag::new();
        drag.set_button(gdk::BUTTON_PRIMARY);
        let me = Rc::downgrade(self);
        drag.connect_drag_begin(move |_, x, y| {
            if let Some(pv) = me.upgrade() {
                let z = pv.zoom.get();
                pv.drag_origin.set((x / z, y / z));
                tools::drag_begin(&pv, x / z, y / z);
            }
        });
        let me = Rc::downgrade(self);
        drag.connect_drag_update(move |_, dx, dy| {
            if let Some(pv) = me.upgrade() {
                let z = pv.zoom.get();
                let (ox, oy) = pv.drag_origin.get();
                tools::drag_update(&pv, ox + dx / z, oy + dy / z);
            }
        });
        let me = Rc::downgrade(self);
        drag.connect_drag_end(move |_, dx, dy| {
            if let Some(pv) = me.upgrade() {
                let z = pv.zoom.get();
                let (ox, oy) = pv.drag_origin.get();
                tools::drag_end(&pv, ox + dx / z, oy + dy / z, dx.hypot(dy) < 4.0);
            }
        });
        self.frame.add_controller(drag);

        let motion = gtk::EventControllerMotion::new();
        let me = Rc::downgrade(self);
        motion.connect_motion(move |_, x, y| {
            if let Some(pv) = me.upgrade() {
                let z = pv.zoom.get();
                tools::hover(&pv, x / z, y / z);
            }
        });
        self.frame.add_controller(motion);
        if let Some(v) = super::view() {
            self.set_cursor_for(v.tool());
        }
    }

    pub fn set_cursor_for(&self, tool: super::Tool) {
        self.frame.set_cursor_from_name(Some(tools::cursor(tool)));
    }

    pub fn set_hover_cursor(&self, name: &str) {
        self.frame.set_cursor_from_name(Some(name));
    }

    pub fn set_size(self: &Rc<Self>, w: f64, h: f64, z: f64) {
        self.size.set((w, h));
        // The detail picture is only right for the zoom it was drawn at.
        if let Some((_, area)) = self.detail_shown.get() {
            self.detail_layer.move_(&self.detail, area.x0 * z, area.y0 * z);
            self.detail.set_size_request((area.width() * z).round() as i32, (area.height() * z).round() as i32);
        }
        self.frame.set_size_request((w * z).round() as i32, (h * z).round() as i32);
        for (widget, r) in self.form_items.borrow().iter() {
            self.forms.move_(widget, r.x0 * z, r.y0 * z);
            widget.set_size_request((r.width() * z).round() as i32, (r.height() * z).round() as i32);
        }
        self.marks.queue_draw();
    }

    /// Forget what was drawn, so the next pass draws it again (the old picture stays until then).
    pub fn invalidate(&self) {
        self.shown.set(None);
        self.pending.set(None);
        if let Some(t) = self.ticket.take() {
            t.cancel();
        }
        *self.links.borrow_mut() = None;
        self.drop_detail();
    }

    fn drop_detail(&self) {
        if let Some(t) = self.detail_ticket.take() {
            t.cancel();
        }
        self.detail.set_visible(false);
        self.detail.set_paintable(None::<&gdk::Paintable>);
        self.detail_shown.set(None);
        self.detail_pending.set(None);
    }

    /// The full-page scale for the current zoom, capped so the picture stays a sane size.
    fn scales(&self, v: &View) -> (f64, f64) {
        let want = (v.zoom.get() * f64::from(self.frame.scale_factor())).max(0.05);
        let (w, h) = self.size.get();
        let cap = MAX_SIDE / w.max(h).max(1.0);
        (want, want.min(cap))
    }

    /// At deep zoom, draw the part of the page in `visible` (points) sharply.
    pub fn ensure_detail(self: &Rc<Self>, v: &View, d: &Doc, visible: Rect) {
        let (want, base) = self.scales(v);
        if base >= want || visible.width() <= 0.0 || visible.height() <= 0.0 {
            if self.detail_shown.get().is_some() || self.detail_pending.get().is_some() {
                self.drop_detail();
            }
            return;
        }
        let key = (d.generation(), (want * 50.0).round() as i64, v.epoch.get());
        let covers = |held: Option<(Key, Rect)>| held.is_some_and(|(k, a)| k == key && a.contains(visible.x0, visible.y0) && a.contains(visible.x1, visible.y1));
        if covers(self.detail_shown.get()) || covers(self.detail_pending.get()) {
            return;
        }
        // Draw a margin around what's visible, so small scrolls don't need a new picture.
        let (pw, ph) = self.size.get();
        let (mx, my) = (visible.width() * 0.35, visible.height() * 0.35);
        let area = Rect::new((visible.x0 - mx).max(0.0), (visible.y0 - my).max(0.0), (visible.x1 + mx).min(pw), (visible.y1 + my).min(ph));
        if let Some(t) = self.detail_ticket.take() {
            t.cancel();
        }
        self.detail_pending.set(Some((key, area)));
        let me = Rc::downgrade(self);
        let clip = [area.x0, area.y0, area.width(), area.height()];
        let ticket = render::request_area(d.gen_path(), d.password(), self.index, want, super::page_colors(), 2, Some(clip), move |tex| {
            let Some(pv) = me.upgrade() else { return };
            let z = pv.zoom.get();
            pv.detail.set_paintable(Some(&tex));
            pv.detail_layer.move_(&pv.detail, area.x0 * z, area.y0 * z);
            pv.detail.set_size_request((area.width() * z).round() as i32, (area.height() * z).round() as i32);
            pv.detail.set_visible(true);
            pv.detail_shown.set(Some((key, area)));
            pv.detail_pending.set(None);
        });
        *self.detail_ticket.borrow_mut() = Some(ticket);
    }

    pub fn ensure_rendered(self: &Rc<Self>, v: &View, d: &Doc) {
        if !self.forms_ready.replace(true) {
            forms::build(self, d);
        }
        let (_, scale) = self.scales(v);
        let key = (d.generation(), (scale * 50.0).round() as i64, v.epoch.get());
        if self.shown.get() == Some(key) || self.pending.get() == Some(key) {
            return;
        }
        if let Some(t) = self.ticket.take() {
            t.cancel();
        }
        self.pending.set(Some(key));
        let me = Rc::downgrade(self);
        let ticket = render::request(d.gen_path(), d.password(), self.index, scale, super::page_colors(), 1, move |tex| {
            if let Some(pv) = me.upgrade() {
                pv.picture.set_paintable(Some(&tex));
                pv.frame.remove_css_class("loading");
                pv.shown.set(Some(key));
                pv.pending.set(None);
            }
        });
        *self.ticket.borrow_mut() = Some(ticket);
    }

    /// Free the picture of a page that's far off screen.
    pub fn release(&self) {
        if self.shown.get().is_none() && self.pending.get().is_none() {
            return;
        }
        if let Some(t) = self.ticket.take() {
            t.cancel();
        }
        self.picture.set_paintable(None::<&gdk::Paintable>);
        self.frame.add_css_class("loading");
        self.shown.set(None);
        self.pending.set(None);
        self.drop_detail();
    }

    /// The links on this page (cached for the current generation).
    pub fn links(&self, d: &Doc) -> Rc<Vec<Link>> {
        if let Some((g, l)) = self.links.borrow().as_ref()
            && *g == d.generation()
        {
            return l.clone();
        }
        let pdf = d.pdf();
        let found = pdf.page(self.index as i32).map(|p| links::links(&pdf, &p)).unwrap_or_default();
        let found = Rc::new(found);
        *self.links.borrow_mut() = Some((d.generation(), found.clone()));
        found
    }

    fn draw(&self, cr: &gtk::cairo::Context) {
        let z = self.zoom.get();
        let Some(v) = super::view() else { return };
        let pal = theme::palette();
        let (ar, ag, ab) = rgb(&pal.accent);
        cr.scale(z, z);
        let st = self.st.borrow();
        let rect = |r: &Rect| cr.rectangle(r.x0, r.y0, r.width(), r.height());

        // Search hits.
        for (i, r) in st.hits.iter().enumerate() {
            if st.current_hit == Some(i) {
                cr.set_source_rgba(ar, ag, ab, 0.14);
                rect(&r.inflate(2.0));
                let _ = cr.fill();
                cr.set_source_rgba(ar, ag, ab, 0.38);
                cr.set_line_width(3.0 / z);
                rect(&r.inflate(1.5));
                let _ = cr.stroke();
                cr.set_source_rgba(ar, ag, ab, 0.9);
                cr.set_line_width(1.2 / z);
                rect(&r.inflate(1.0));
                let _ = cr.stroke();
            } else {
                cr.set_source_rgba(ar, ag, ab, 0.28);
                rect(r);
                let _ = cr.fill();
            }
        }

        // Text selection (and the live preview of a drag).
        let selection = v.selection.borrow();
        let selected: &[Rect] = match selection.as_ref() {
            Some(s) if s.page == self.index => &s.rects,
            _ => &[],
        };
        cr.set_source_rgba(ar, ag, ab, 0.32);
        for r in selected.iter().chain(st.preview.iter()) {
            rect(r);
            let _ = cr.fill();
        }

        // The stroke being drawn.
        if st.ink.len() > 1 {
            let (r, g, b) = rgb(&v.colour());
            cr.set_source_rgba(r, g, b, 1.0);
            cr.set_line_width(crate::prefs::get().ink_width);
            cr.set_line_cap(gtk::cairo::LineCap::Round);
            cr.set_line_join(gtk::cairo::LineJoin::Round);
            cr.move_to(st.ink[0].0, st.ink[0].1);
            for p in &st.ink[1..] {
                cr.line_to(p.0, p.1);
            }
            let _ = cr.stroke();
        }

        // A rubber band.
        if let Some(b) = &st.band {
            cr.set_source_rgba(ar, ag, ab, 0.12);
            rect(b);
            let _ = cr.fill();
            cr.set_source_rgba(ar, ag, ab, 0.9);
            cr.set_line_width(1.2 / z);
            cr.set_dash(&[4.0 / z, 3.0 / z], 0.0);
            rect(b);
            let _ = cr.stroke();
            cr.set_dash(&[], 0.0);
        }

        // The selected annotation.
        let sel = *v.annot_sel.borrow();
        if let Some(sel) = sel
            && sel.page == self.index
            && let Some(r) = tools::annot_rect(&v, sel)
        {
            let r = r.inflate(2.0);
            cr.set_source_rgba(ar, ag, ab, 0.38);
            cr.set_line_width(4.0 / z);
            rect(&r);
            let _ = cr.stroke();
            cr.set_source_rgba(ar, ag, ab, 0.95);
            cr.set_line_width(1.5 / z);
            rect(&r);
            let _ = cr.stroke();
            // A handle to resize it by, at the bottom-right corner.
            if tools::resizable(&v, sel) {
                let s = HANDLE / z;
                cr.rectangle(r.x1 - s, r.y1 - s, 2.0 * s, 2.0 * s);
                cr.set_source_rgba(ar, ag, ab, 1.0);
                let _ = cr.fill_preserve();
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.9);
                cr.set_line_width(1.0 / z);
                let _ = cr.stroke();
            }
        }
    }
}

/// Where in `widget`'s own coordinates a rectangle (in points) sits, as pixels.
pub fn px_rect(pv: &PageView, r: &Rect) -> gtk::gdk::Rectangle {
    let z = pv.zoom.get();
    gtk::gdk::Rectangle::new((r.x0 * z) as i32, (r.y0 * z) as i32, (r.width() * z).ceil() as i32, (r.height() * z).ceil() as i32)
}

pub fn clear_preview(pv: &PageView) {
    let mut st = pv.st.borrow_mut();
    st.preview.clear();
    st.ink.clear();
    st.band = None;
    drop(st);
    pv.marks.queue_draw();
}

pub fn queue_all(v: &View) {
    for p in v.pages.borrow().iter() {
        p.marks.queue_draw();
    }
}

