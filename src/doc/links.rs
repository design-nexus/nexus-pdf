//! What poppler-glib knows about links, the outline and form fields, but poppler-rs
//! doesn't expose (their fields are only in the C structs), read through the raw
//! bindings. Everything unsafe about it stays in this file.

use super::geom::Rect;
use gtk::glib;
use gtk::glib::translate::*;
use poppler::ffi;
use std::ffi::CStr;

#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// Zero-based page.
    Page(usize),
    Uri(String),
}

#[derive(Debug, Clone)]
pub struct Link {
    /// On the displayed page, y down.
    pub area: Rect,
    pub target: Target,
}

#[derive(Debug, Clone)]
pub struct Outline {
    pub title: String,
    pub page: Option<usize>,
    pub open: bool,
    pub children: Vec<Outline>,
}

pub struct Field {
    pub area: Rect,
    pub field: poppler::FormField,
}

/// Poppler reports these areas with y up; the viewer works with y down.
fn flip(a: &ffi::PopplerRectangle, page_h: f64) -> Rect {
    Rect::new(a.x1, page_h - a.y2, a.x2, page_h - a.y1)
}

/// Run `f` over each element of a `GList`.
unsafe fn each<T>(mut list: *mut glib::ffi::GList, mut f: impl FnMut(*mut T)) {
    while !list.is_null() {
        // SAFETY: the caller guarantees the list holds `T` pointers.
        unsafe {
            f((*list).data as *mut T);
            list = (*list).next;
        }
    }
}

unsafe fn dest_page(doc: *mut ffi::PopplerDocument, dest: *mut ffi::PopplerDest) -> Option<usize> {
    if dest.is_null() {
        return None;
    }
    // SAFETY: `dest` is a live PopplerDest owned by the action being read.
    unsafe {
        let d = &*dest;
        let page_num = if d.type_ == ffi::POPPLER_DEST_NAMED {
            let found = ffi::poppler_document_find_dest(doc, d.named_dest);
            if found.is_null() {
                return None;
            }
            let n = (*found).page_num;
            ffi::poppler_dest_free(found);
            n
        } else {
            d.page_num
        };
        (page_num >= 1).then(|| page_num as usize - 1)
    }
}

unsafe fn text(p: *const std::os::raw::c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: poppler hands out NUL-terminated strings.
    unsafe { CStr::from_ptr(p).to_string_lossy().into_owned() }
}

/// The links on a page.
pub fn links(pdf: &poppler::Document, page: &poppler::Page) -> Vec<Link> {
    let (_, page_h) = page.size();
    let mut out = Vec::new();
    // SAFETY: the list and its mappings are owned by us until freed below.
    unsafe {
        let doc: *mut ffi::PopplerDocument = pdf.to_glib_none().0;
        let list = ffi::poppler_page_get_link_mapping(page.to_glib_none().0);
        each::<ffi::PopplerLinkMapping>(list, |m| {
            let m = &*m;
            if m.action.is_null() {
                return;
            }
            let target = match m.action.cast::<ffi::PopplerActionAny>().read().type_ {
                ffi::POPPLER_ACTION_GOTO_DEST => {
                    let a = &(*m.action).goto_dest;
                    dest_page(doc, a.dest).map(Target::Page)
                }
                ffi::POPPLER_ACTION_URI => {
                    let uri = text((*m.action).uri.uri);
                    (!uri.is_empty()).then_some(Target::Uri(uri))
                }
                _ => None,
            };
            if let Some(target) = target {
                out.push(Link { area: flip(&m.area, page_h), target });
            }
        });
        ffi::poppler_page_free_link_mapping(list);
    }
    out
}

/// The table of contents, nested.
pub fn outline(pdf: &poppler::Document) -> Vec<Outline> {
    // SAFETY: iterators are freed on every path; actions are owned by the caller.
    unsafe fn walk(doc: *mut ffi::PopplerDocument, iter: *mut ffi::PopplerIndexIter) -> Vec<Outline> {
        let mut items = Vec::new();
        unsafe {
            loop {
                let action = ffi::poppler_index_iter_get_action(iter);
                if !action.is_null() {
                    let title = text(action.cast::<ffi::PopplerActionAny>().read().title);
                    let page = if action.cast::<ffi::PopplerActionAny>().read().type_ == ffi::POPPLER_ACTION_GOTO_DEST {
                        dest_page(doc, (*action).goto_dest.dest)
                    } else {
                        None
                    };
                    let open = ffi::poppler_index_iter_is_open(iter) != 0;
                    let child = ffi::poppler_index_iter_get_child(iter);
                    let children = if child.is_null() {
                        Vec::new()
                    } else {
                        let c = walk(doc, child);
                        ffi::poppler_index_iter_free(child);
                        c
                    };
                    ffi::poppler_action_free(action);
                    if !title.trim().is_empty() {
                        items.push(Outline { title: title.trim().to_string(), page, open, children });
                    }
                }
                if ffi::poppler_index_iter_next(iter) == 0 {
                    break;
                }
            }
        }
        items
    }
    unsafe {
        let doc: *mut ffi::PopplerDocument = pdf.to_glib_none().0;
        let iter = ffi::poppler_index_iter_new(doc);
        if iter.is_null() {
            return Vec::new();
        }
        let items = walk(doc, iter);
        ffi::poppler_index_iter_free(iter);
        items
    }
}

/// The form fields on a page.
pub fn fields(page: &poppler::Page) -> Vec<Field> {
    let (_, page_h) = page.size();
    let mut out = Vec::new();
    // SAFETY: each field is copied (refcounted) before the list is freed.
    unsafe {
        let list = ffi::poppler_page_get_form_field_mapping(page.to_glib_none().0);
        each::<ffi::PopplerFormFieldMapping>(list, |m| {
            let m = &*m;
            if !m.field.is_null() {
                out.push(Field { area: flip(&m.area, page_h), field: from_glib_none(m.field) });
            }
        });
        ffi::poppler_page_free_form_field_mapping(list);
    }
    out
}

/// How a run of text looks: its font name, size in points and colour (0..1 RGB).
#[derive(Debug, Clone)]
pub struct TextLook {
    pub font: String,
    pub size: f64,
    pub colour: [f32; 3],
}

/// The font, size and colour of the text inside `area` (on the displayed page, y down).
pub fn text_look(page: &poppler::Page, area: &Rect) -> Option<TextLook> {
    let mut r = poppler::Rectangle::new();
    r.set_x1(area.x0);
    r.set_y1(area.y0);
    r.set_x2(area.x1);
    r.set_y2(area.y1);
    let attrs = page.text_attributes_for_area(&mut r);
    let first = attrs.first()?;
    // SAFETY: `first` wraps a live PopplerTextAttributes.
    unsafe {
        let ptr: *const ffi::PopplerTextAttributes = first.to_glib_none().0;
        let a = &*ptr;
        Some(TextLook {
            font: text(a.font_name),
            size: a.font_size,
            colour: [f32::from(a.color.red) / 65535.0, f32::from(a.color.green) / 65535.0, f32::from(a.color.blue) / 65535.0],
        })
    }
}
