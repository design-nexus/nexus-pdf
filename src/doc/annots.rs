//! Annotations, written as real PDF annotations with their own appearance streams
//! so every viewer shows them. poppler-rs can't create highlights, ink, notes or
//! stamps, so lopdf does it.
//!
//! Positions come in on the displayed page (points from the top-left); `PageGeom`
//! turns them into PDF space, including for rotated pages.

use super::geom::{PageGeom, Rect, get_dict, page_geom};
use anyhow::{Context, Result, bail};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, dictionary};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Highlight,
    Underline,
    StrikeOut,
    Ink,
    Note,
    FreeText,
    /// Replacement text written over the original (a FreeText with a solid background).
    Edit,
    Stamp,
    Other,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Highlight => "Highlight",
            Kind::Underline => "Underline",
            Kind::StrikeOut => "Strike out",
            Kind::Ink => "Drawing",
            Kind::Note => "Note",
            Kind::FreeText => "Text box",
            Kind::Edit => "Edited text",
            Kind::Stamp => "Signature",
            Kind::Other => "Annotation",
        }
    }

    /// Can be moved by dragging.
    pub fn movable(self) -> bool {
        matches!(self, Kind::Note | Kind::FreeText | Kind::Edit | Kind::Stamp)
    }

    /// Has text the user can change.
    pub fn has_text(self) -> bool {
        matches!(self, Kind::Note | Kind::FreeText | Kind::Edit)
    }

    /// Its colour can be changed (its appearance is drawn from the colour).
    pub fn recolourable(self) -> bool {
        !matches!(self, Kind::Stamp | Kind::Other | Kind::Edit)
    }
}

#[derive(Clone, Debug)]
pub struct AnnotInfo {
    pub id: ObjectId,
    pub kind: Kind,
    /// On the displayed page, y down.
    pub rect: Rect,
    pub contents: String,
    pub colour: Option<[f32; 3]>,
}

/// Page object ids in page order.
pub fn page_ids(doc: &Document) -> Vec<ObjectId> {
    doc.get_pages().into_values().collect()
}

fn num(o: &Object) -> Option<f64> {
    match o {
        Object::Integer(i) => Some(*i as f64),
        Object::Real(r) => Some(f64::from(*r)),
        _ => None,
    }
}

fn nums(doc: &Document, o: &Object) -> Vec<f64> {
    let o = match o {
        Object::Reference(id) => match doc.get_object(*id) {
            Ok(o) => o,
            Err(_) => return Vec::new(),
        },
        o => o,
    };
    o.as_array().map(|a| a.iter().filter_map(num).collect()).unwrap_or_default()
}

fn real(v: f64) -> Object {
    Object::Real(v as f32)
}

fn reals(v: &[f64]) -> Object {
    Object::Array(v.iter().map(|x| real(*x)).collect())
}

/// A PDF text string: PDFDocEncoding (Latin-1 here) or UTF-16BE when it needs more.
pub fn text_object(s: &str) -> Object {
    if s.chars().all(|c| (' '..='~').contains(&c) || c == '\n' || c == '\r') {
        return Object::String(s.as_bytes().to_vec(), lopdf::StringFormat::Literal);
    }
    let mut bytes = vec![0xFE, 0xFF];
    for u in s.encode_utf16() {
        bytes.extend_from_slice(&u.to_be_bytes());
    }
    Object::String(bytes, lopdf::StringFormat::Hexadecimal)
}

pub fn text_of(o: &Object) -> String {
    match o {
        Object::String(b, _) => {
            if b.starts_with(&[0xFE, 0xFF]) {
                let units: Vec<u16> = b[2..].as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
                String::from_utf16_lossy(&units)
            } else {
                b.iter().map(|&c| c as char).collect()
            }
        }
        _ => String::new(),
    }
}

/// The date in PDF's `D:YYYYMMDDHHmmSSZ` form (UTC).
fn pdf_date() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil date from days since 1970 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("D:{y:04}{m:02}{d:02}{:02}{:02}{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

fn unique_name() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("nexus-{t:x}-{}", N.fetch_add(1, Ordering::Relaxed))
}

// ---------- Fonts ----------

/// Helvetica advance widths for ' '..='~' in 1/1000 em.
const HELV: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556, 556, 556,
    556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778,
    722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222,
    500, 222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
];

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Face {
    pub serif: bool,
    pub mono: bool,
    pub bold: bool,
    pub italic: bool,
}

impl Face {
    /// The standard-14 font this face maps to.
    pub fn base_font(self) -> &'static str {
        match (self.mono, self.serif, self.bold, self.italic) {
            (true, _, false, false) => "Courier",
            (true, _, true, false) => "Courier-Bold",
            (true, _, false, true) => "Courier-Oblique",
            (true, _, true, true) => "Courier-BoldOblique",
            (false, true, false, false) => "Times-Roman",
            (false, true, true, false) => "Times-Bold",
            (false, true, false, true) => "Times-Italic",
            (false, true, true, true) => "Times-BoldItalic",
            (false, false, false, false) => "Helvetica",
            (false, false, true, false) => "Helvetica-Bold",
            (false, false, false, true) => "Helvetica-Oblique",
            (false, false, true, true) => "Helvetica-BoldOblique",
        }
    }

    /// Guess a face from a font's name ("ArialMT", "TimesNewRoman-Bold", "CourierNew").
    pub fn from_name(name: &str) -> Face {
        let n = name.to_lowercase();
        let n = n.rsplit('+').next().unwrap_or(&n).to_string();
        Face {
            mono: ["courier", "mono", "consolas", "menlo", "typewriter", "code"].iter().any(|k| n.contains(k)),
            serif: ["times", "serif", "georgia", "garamond", "palatino", "minion", "cambria", "book", "roman"]
                .iter()
                .any(|k| n.contains(k))
                && !n.contains("sans"),
            bold: n.contains("bold") || n.contains("black") || n.contains("heavy"),
            italic: n.contains("italic") || n.contains("oblique"),
        }
    }

    fn advance(self, c: char) -> f64 {
        if self.mono {
            return 0.6;
        }
        let w = match c {
            ' '..='~' => f64::from(HELV[c as usize - 32]) / 1000.0,
            _ => 0.556,
        };
        let w = if self.serif { w * 0.93 } else { w };
        if self.bold { w * 1.04 } else { w }
    }
}

/// Width of `text` in points.
pub fn text_width(face: Face, size: f64, text: &str) -> f64 {
    text.chars().map(|c| face.advance(c)).sum::<f64>() * size
}

/// Break `text` into lines no wider than `width` points (hard line breaks kept).
pub fn wrap(face: Face, size: f64, width: f64, text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split(' ') {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if !line.is_empty() && text_width(face, size, &candidate) > width {
                lines.push(std::mem::take(&mut line));
                line = word.to_string();
            } else {
                line = candidate;
            }
        }
        lines.push(line);
    }
    lines
}

/// Text as WinAnsi bytes, for a PDF string.
fn winansi(s: &str) -> Vec<u8> {
    s.chars()
        .map(|c| match c {
            '\u{2019}' | '\u{2018}' => 0x27,
            '\u{201C}' => 0x93,
            '\u{201D}' => 0x94,
            '\u{2013}' => 0x96,
            '\u{2014}' => 0x97,
            '\u{2022}' => 0x95,
            '\u{2026}' => 0x85,
            '\u{20AC}' => 0x80,
            c if (c as u32) < 256 && c as u32 >= 32 => c as u32 as u8,
            _ => b'?',
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

fn font_dict(face: Face) -> Dictionary {
    dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => face.base_font(),
        "Encoding" => "WinAnsiEncoding",
    }
}

// ---------- Reading ----------

fn annots_of(doc: &Document, page: ObjectId) -> Vec<ObjectId> {
    let Ok(d) = doc.get_dictionary(page) else { return Vec::new() };
    let Ok(a) = d.get(b"Annots") else { return Vec::new() };
    let arr = match a {
        Object::Reference(r) => doc.get_object(*r).and_then(|o| o.as_array()).ok().cloned(),
        Object::Array(a) => Some(a.clone()),
        _ => None,
    };
    arr.unwrap_or_default().iter().filter_map(|o| o.as_reference().ok()).collect()
}

fn kind_of(d: &Dictionary) -> Kind {
    match d.get(b"Subtype").and_then(|s| s.as_name()).unwrap_or(b"") {
        b"Highlight" => Kind::Highlight,
        b"Underline" => Kind::Underline,
        b"StrikeOut" => Kind::StrikeOut,
        b"Ink" => Kind::Ink,
        b"Text" => Kind::Note,
        b"FreeText" if d.has(b"NexusEdit") => Kind::Edit,
        b"FreeText" => Kind::FreeText,
        b"Stamp" => Kind::Stamp,
        _ => Kind::Other,
    }
}

pub fn read_page(doc: &Document, page: ObjectId) -> Vec<AnnotInfo> {
    let geom = page_geom(doc, page);
    let mut out = Vec::new();
    for id in annots_of(doc, page) {
        let Ok(d) = doc.get_dictionary(id) else { continue };
        let subtype = d.get(b"Subtype").and_then(|s| s.as_name()).unwrap_or(b"");
        if matches!(subtype, b"Link" | b"Widget" | b"Popup") {
            continue;
        }
        let Some(r) = d.get(b"Rect").ok().map(|o| nums(doc, o)).filter(|r| r.len() >= 4) else { continue };
        let rect = geom.rect_to_view(&Rect::new(r[0], r[1], r[2], r[3]));
        let colour = d.get(b"C").ok().map(|o| nums(doc, o)).filter(|c| c.len() == 3).map(|c| [c[0] as f32, c[1] as f32, c[2] as f32]);
        out.push(AnnotInfo { id, kind: kind_of(d), rect, contents: d.get(b"Contents").map(text_of).unwrap_or_default(), colour });
    }
    out
}

pub fn exists(doc: &Document, id: ObjectId) -> bool {
    doc.get_dictionary(id).is_ok()
}

/// Every annotation in the document with its zero-based page.
pub fn read_all(doc: &Document) -> Vec<(usize, AnnotInfo)> {
    page_ids(doc).into_iter().enumerate().flat_map(|(i, p)| read_page(doc, p).into_iter().map(move |a| (i, a))).collect()
}

// ---------- Writing ----------

fn push_annot(doc: &mut Document, page: ObjectId, annot: ObjectId) -> Result<()> {
    let existing = doc.get_dictionary(page)?.get(b"Annots").ok().cloned();
    match existing {
        Some(Object::Reference(r)) => {
            doc.get_object_mut(r)?.as_array_mut()?.push(Object::Reference(annot));
        }
        Some(Object::Array(_)) => {
            if let Ok(Object::Array(a)) = doc.get_dictionary_mut(page)?.get_mut(b"Annots") {
                a.push(Object::Reference(annot));
            }
        }
        _ => doc.get_dictionary_mut(page)?.set("Annots", vec![Object::Reference(annot)]),
    }
    Ok(())
}

/// Take an annotation off its page and delete it.
pub fn delete(doc: &mut Document, page: ObjectId, id: ObjectId) -> Result<()> {
    let existing = doc.get_dictionary(page)?.get(b"Annots").ok().cloned();
    let drop_from = |a: &mut Vec<Object>| a.retain(|o| o.as_reference().ok() != Some(id));
    match existing {
        Some(Object::Reference(r)) => drop_from(doc.get_object_mut(r)?.as_array_mut()?),
        Some(Object::Array(_)) => {
            if let Ok(Object::Array(a)) = doc.get_dictionary_mut(page)?.get_mut(b"Annots") {
                drop_from(a);
            }
        }
        _ => {}
    }
    doc.delete_object(id);
    Ok(())
}

fn colour_array(c: [f32; 3]) -> Object {
    Object::Array(c.iter().map(|v| Object::Real(*v)).collect())
}

fn c3(c: [f32; 3]) -> String {
    format!("{:.3} {:.3} {:.3}", c[0], c[1], c[2])
}

fn base_dict(subtype: &str, rect: Rect, colour: [f32; 3], contents: &str, author: &str) -> Dictionary {
    let mut d = dictionary! {
        "Type" => "Annot",
        "Subtype" => subtype,
        "Rect" => reals(&[rect.x0, rect.y0, rect.x1, rect.y1]),
        "C" => colour_array(colour),
        "F" => 4,
        "NM" => text_object(&unique_name()),
        "M" => text_object(&pdf_date()),
    };
    if !contents.is_empty() {
        d.set("Contents", text_object(contents));
    }
    if !author.is_empty() {
        d.set("T", text_object(author));
    }
    d
}

/// A form XObject with an upright local frame `w`×`h`, rotated to match the page.
fn upright_form(doc: &mut Document, geom: &PageGeom, w: f64, h: f64, content: String, resources: Dictionary) -> ObjectId {
    let matrix: [f64; 6] = match geom.rotate {
        90 => [0.0, 1.0, -1.0, 0.0, 0.0, 0.0],
        180 => [-1.0, 0.0, 0.0, -1.0, 0.0, 0.0],
        270 => [0.0, -1.0, 1.0, 0.0, 0.0, 0.0],
        _ => [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
    };
    let dict = dictionary! {
        "Type" => "XObject",
        "Subtype" => "Form",
        "BBox" => reals(&[0.0, 0.0, w, h]),
        "Matrix" => reals(&matrix),
        "Resources" => resources,
    };
    doc.add_object(Stream::new(dict, content.into_bytes()))
}

/// A form XObject drawn directly in PDF space (the BBox is the annotation rectangle).
fn flat_form(doc: &mut Document, bbox: Rect, content: String, resources: Dictionary) -> ObjectId {
    let dict = dictionary! {
        "Type" => "XObject",
        "Subtype" => "Form",
        "BBox" => reals(&[bbox.x0, bbox.y0, bbox.x1, bbox.y1]),
        "Resources" => resources,
    };
    doc.add_object(Stream::new(dict, content.into_bytes()))
}

fn set_ap(doc: &mut Document, annot: ObjectId, form: ObjectId) -> Result<()> {
    doc.get_dictionary_mut(annot)?.set("AP", dictionary! { "N" => form });
    Ok(())
}

/// Corner points of a view rectangle in PDF space: top-left, top-right, bottom-left, bottom-right.
fn quad(geom: &PageGeom, r: &Rect) -> [(f64, f64); 4] {
    [geom.to_pdf(r.x0, r.y0), geom.to_pdf(r.x1, r.y0), geom.to_pdf(r.x0, r.y1), geom.to_pdf(r.x1, r.y1)]
}

fn bounds(points: impl Iterator<Item = (f64, f64)>) -> Rect {
    let mut r = Rect { x0: f64::MAX, y0: f64::MAX, x1: f64::MIN, y1: f64::MIN };
    for (x, y) in points {
        r.x0 = r.x0.min(x);
        r.y0 = r.y0.min(y);
        r.x1 = r.x1.max(x);
        r.y1 = r.y1.max(y);
    }
    r
}

/// Highlight, underline or strike-out over the given text rectangles.
pub fn add_markup(doc: &mut Document, page: ObjectId, kind: Kind, rects: &[Rect], colour: [f32; 3], author: &str) -> Result<ObjectId> {
    let subtype = match kind {
        Kind::Highlight => "Highlight",
        Kind::Underline => "Underline",
        Kind::StrikeOut => "StrikeOut",
        _ => bail!("not a text markup"),
    };
    if rects.is_empty() {
        bail!("nothing is selected");
    }
    let geom = page_geom(doc, page);
    let quads: Vec<[(f64, f64); 4]> = rects.iter().map(|r| quad(&geom, r)).collect();
    let bb = bounds(quads.iter().flatten().copied()).inflate(1.0);
    let mut points = Vec::new();
    for q in &quads {
        for (x, y) in q {
            points.push(*x);
            points.push(*y);
        }
    }
    let mut d = base_dict(subtype, bb, colour, "", author);
    d.set("QuadPoints", reals(&points));
    let id = doc.add_object(d);
    rebuild_ap(doc, page, id)?;
    push_annot(doc, page, id)?;
    Ok(id)
}

/// A freehand drawing: one or more strokes, in points on the displayed page.
pub fn add_ink(doc: &mut Document, page: ObjectId, strokes: &[Vec<(f64, f64)>], colour: [f32; 3], width: f64, author: &str) -> Result<ObjectId> {
    let geom = page_geom(doc, page);
    let list: Vec<Vec<(f64, f64)>> = strokes.iter().filter(|s| s.len() > 1).map(|s| s.iter().map(|&(x, y)| geom.to_pdf(x, y)).collect()).collect();
    if list.is_empty() {
        bail!("nothing was drawn");
    }
    let bb = bounds(list.iter().flatten().copied()).inflate(width);
    let mut d = base_dict("Ink", bb, colour, "", author);
    d.set(
        "InkList",
        Object::Array(list.iter().map(|s| reals(&s.iter().flat_map(|&(x, y)| [x, y]).collect::<Vec<_>>())).collect()),
    );
    d.set("BS", dictionary! { "W" => real(width) });
    let id = doc.add_object(d);
    rebuild_ap(doc, page, id)?;
    push_annot(doc, page, id)?;
    Ok(id)
}

pub const NOTE_SIZE: f64 = 22.0;

/// A sticky note whose icon sits at (`x`, `y`) on the displayed page.
pub fn add_note(doc: &mut Document, page: ObjectId, x: f64, y: f64, text: &str, colour: [f32; 3], author: &str) -> Result<ObjectId> {
    let geom = page_geom(doc, page);
    let view = Rect::new(x, y, x + NOTE_SIZE, y + NOTE_SIZE);
    let rect = geom.rect_to_pdf(&view);
    let mut d = base_dict("Text", rect, colour, text, author);
    d.set("Name", "Note");
    d.set("Open", false);
    d.set("F", 4 | 8 | 16);
    let id = doc.add_object(d);
    rebuild_ap(doc, page, id)?;
    push_annot(doc, page, id)?;
    Ok(id)
}

/// A text box over `view` (on the displayed page).
pub fn add_text_box(doc: &mut Document, page: ObjectId, view: Rect, text: &str, colour: [f32; 3], size: f64, author: &str) -> Result<ObjectId> {
    let geom = page_geom(doc, page);
    let rect = geom.rect_to_pdf(&view);
    let mut d = base_dict("FreeText", rect, colour, text, author);
    d.set("DA", Object::string_literal(format!("/Helv {size:.1} Tf {} rg", c3(colour))));
    d.set("NexusSize", real(size));
    let id = doc.add_object(d);
    rebuild_ap(doc, page, id)?;
    push_annot(doc, page, id)?;
    Ok(id)
}

/// Replacement text over `view`: an opaque `background`, then `text` in `colour` using `face`.
#[allow(clippy::too_many_arguments)]
pub fn add_edit(
    doc: &mut Document,
    page: ObjectId,
    view: Rect,
    text: &str,
    face: Face,
    size: f64,
    colour: [f32; 3],
    background: [f32; 3],
    author: &str,
) -> Result<ObjectId> {
    let geom = page_geom(doc, page);
    let rect = geom.rect_to_pdf(&view);
    let mut d = base_dict("FreeText", rect, colour, text, author);
    d.set("DA", Object::string_literal(format!("/Helv {size:.1} Tf {} rg", c3(colour))));
    d.set("NexusSize", real(size));
    d.set("NexusEdit", true);
    d.set("NexusBg", colour_array(background));
    d.set("NexusFont", Object::Name(face.base_font().as_bytes().to_vec()));
    let id = doc.add_object(d);
    rebuild_ap(doc, page, id)?;
    push_annot(doc, page, id)?;
    Ok(id)
}

/// A picture (a signature) over `view`.
pub struct Picture {
    pub width: usize,
    pub height: usize,
    /// RGB, 8 bits a channel.
    pub rgb: Vec<u8>,
    /// Alpha, 8 bits.
    pub alpha: Vec<u8>,
}

pub fn add_stamp(doc: &mut Document, page: ObjectId, view: Rect, pic: &Picture, author: &str) -> Result<ObjectId> {
    let geom = page_geom(doc, page);
    let rect = geom.rect_to_pdf(&view);
    let mut smask = Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => pic.width as i64, "Height" => pic.height as i64,
            "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
        },
        pic.alpha.clone(),
    );
    let _ = smask.compress();
    let smask_id = doc.add_object(smask);
    let mut image = Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => pic.width as i64, "Height" => pic.height as i64,
            "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8, "SMask" => smask_id,
        },
        pic.rgb.clone(),
    );
    let _ = image.compress();
    let image_id = doc.add_object(image);
    let (w, h) = (view.width(), view.height());
    let form = upright_form(
        doc,
        &geom,
        w,
        h,
        format!("q {w:.3} 0 0 {h:.3} 0 0 cm /Im0 Do Q"),
        dictionary! { "XObject" => dictionary! { "Im0" => image_id } },
    );
    let mut d = base_dict("Stamp", rect, [0.0, 0.0, 0.0], "", author);
    d.set("Subj", text_object("Signature"));
    d.set("F", 4 | 128);
    let id = doc.add_object(d);
    set_ap(doc, id, form)?;
    push_annot(doc, page, id)?;
    Ok(id)
}

/// Rebuild an annotation's appearance from the values in its dictionary.
pub fn rebuild_ap(doc: &mut Document, page: ObjectId, id: ObjectId) -> Result<()> {
    let geom = page_geom(doc, page);
    let d = doc.get_dictionary(id).context("annotation is missing")?.clone();
    let subtype = d.get(b"Subtype").and_then(|s| s.as_name()).unwrap_or(b"").to_vec();
    let rect = d
        .get(b"Rect")
        .ok()
        .map(|o| nums(doc, o))
        .filter(|r| r.len() >= 4)
        .map(|r| Rect::new(r[0], r[1], r[2], r[3]))
        .context("annotation has no rectangle")?;
    let colour = d.get(b"C").ok().map(|o| nums(doc, o)).filter(|c| c.len() == 3).map(|c| [c[0] as f32, c[1] as f32, c[2] as f32]).unwrap_or([1.0, 0.84, 0.04]);
    let form = match subtype.as_slice() {
        b"Highlight" | b"Underline" | b"StrikeOut" => {
            let pts = d.get(b"QuadPoints").ok().map(|o| nums(doc, o)).unwrap_or_default();
            let mut c = String::new();
            let mut resources = Dictionary::new();
            if subtype == b"Highlight" {
                c.push_str(&format!("/GS gs {} rg\n", c3(colour)));
                resources.set("ExtGState", dictionary! { "GS" => dictionary! { "Type" => "ExtGState", "BM" => "Multiply" } });
            }
            for q in pts.as_chunks::<8>().0 {
                // Points are top-left, top-right, bottom-left, bottom-right.
                let (tl, tr, bl, br) = ((q[0], q[1]), (q[2], q[3]), (q[4], q[5]), (q[6], q[7]));
                match subtype.as_slice() {
                    b"Highlight" => c.push_str(&format!(
                        "{:.3} {:.3} m {:.3} {:.3} l {:.3} {:.3} l {:.3} {:.3} l h f\n",
                        tl.0, tl.1, tr.0, tr.1, br.0, br.1, bl.0, bl.1
                    )),
                    b"Underline" => {
                        let thick = ((tl.0 - bl.0).hypot(tl.1 - bl.1) * 0.07).max(0.8);
                        c.push_str(&format!(
                            "{} RG {thick:.3} w {:.3} {:.3} m {:.3} {:.3} l S\n",
                            c3(colour),
                            bl.0,
                            bl.1,
                            br.0,
                            br.1
                        ));
                    }
                    _ => {
                        let thick = ((tl.0 - bl.0).hypot(tl.1 - bl.1) * 0.07).max(0.8);
                        c.push_str(&format!(
                            "{} RG {thick:.3} w {:.3} {:.3} m {:.3} {:.3} l S\n",
                            c3(colour),
                            (tl.0 + bl.0) / 2.0,
                            (tl.1 + bl.1) / 2.0,
                            (tr.0 + br.0) / 2.0,
                            (tr.1 + br.1) / 2.0
                        ));
                    }
                }
            }
            flat_form(doc, rect, c, resources)
        }
        b"Ink" => {
            let width = d
                .get(b"BS")
                .ok()
                .and_then(|o| get_dict(doc, o))
                .and_then(|b| b.get(b"W").ok())
                .and_then(num)
                .unwrap_or(2.0);
            let mut c = format!("{} RG {width:.3} w 1 J 1 j\n", c3(colour));
            if let Ok(list) = d.get(b"InkList").and_then(|l| l.as_array()) {
                for stroke in list {
                    let p = nums(doc, stroke);
                    if p.len() >= 4 {
                        c.push_str(&format!("{:.3} {:.3} m\n", p[0], p[1]));
                        for xy in p[2..].as_chunks::<2>().0 {
                            c.push_str(&format!("{:.3} {:.3} l\n", xy[0], xy[1]));
                        }
                        c.push_str("S\n");
                    }
                }
            }
            flat_form(doc, rect, c, Dictionary::new())
        }
        b"Text" => {
            // A note icon: a folded page in the chosen colour.
            let (w, h) = (rect.width(), rect.height());
            let (m, f) = (w * 0.12, w * 0.28);
            let c = format!(
                "{} rg 0.25 0.25 0.25 RG 0.8 w\n{m:.2} {m:.2} m {:.2} {m:.2} l {:.2} {:.2} l {:.2} {:.2} l {m:.2} {:.2} l h B\n0.25 0.25 0.25 RG 0.7 w\n{:.2} {:.2} m {:.2} {:.2} l S {:.2} {:.2} m {:.2} {:.2} l S {:.2} {:.2} m {:.2} {:.2} l S\n",
                c3(colour),
                w - m,
                w - m,
                h - m - f,
                w - m - f,
                h - m,
                h - m,
                w * 0.25,
                h * 0.62,
                w * 0.75,
                h * 0.62,
                w * 0.25,
                h * 0.45,
                w * 0.75,
                h * 0.45,
                w * 0.25,
                h * 0.28,
                w * 0.55,
                h * 0.28,
            );
            upright_form(doc, &geom, w, h, c, Dictionary::new())
        }
        b"FreeText" => {
            let size = d.get(b"NexusSize").ok().and_then(num).unwrap_or(12.0);
            let is_edit = d.has(b"NexusEdit");
            let face = if is_edit {
                let name = d.get(b"NexusFont").and_then(|n| n.as_name()).map(|n| String::from_utf8_lossy(n).into_owned()).unwrap_or_default();
                Face::from_name(&name)
            } else {
                Face::default()
            };
            let bg = d.get(b"NexusBg").ok().map(|o| nums(doc, o)).filter(|c| c.len() == 3).map(|c| [c[0] as f32, c[1] as f32, c[2] as f32]);
            let text = d.get(b"Contents").map(text_of).unwrap_or_default();
            // The local frame is upright: width and height as displayed.
            let (w, h) = {
                let v = geom.rect_to_view(&rect);
                (v.width(), v.height())
            };
            let pad = if is_edit { 0.5 } else { 3.0 };
            let lines = if is_edit { vec![text.replace('\n', " ")] } else { wrap(face, size, (w - 2.0 * pad).max(size), &text) };
            let mut c = String::new();
            if let Some(bg) = bg {
                c.push_str(&format!("{} rg 0 0 {w:.3} {h:.3} re f\n", c3(bg)));
            }
            c.push_str(&format!("BT /F1 {size:.2} Tf {} rg\n", c3(colour)));
            let lead = size * 1.2;
            let first = if is_edit { (h - size) / 2.0 + size * 0.22 } else { h - pad - size * 0.9 };
            for (i, l) in lines.iter().enumerate() {
                c.push_str(&format!("1 0 0 1 {pad:.3} {:.3} Tm <{}> Tj\n", first - lead * i as f64, hex(&winansi(l))));
            }
            c.push_str("ET\n");
            upright_form(doc, &geom, w, h, c, dictionary! { "Font" => dictionary! { "F1" => font_dict(face) } })
        }
        // Stamps and anything else keep the appearance they have.
        _ => return Ok(()),
    };
    set_ap(doc, id, form)
}

/// Change a text-bearing annotation's contents (and its appearance).
pub fn set_contents(doc: &mut Document, page: ObjectId, id: ObjectId, text: &str) -> Result<()> {
    {
        let d = doc.get_dictionary_mut(id)?;
        d.set("Contents", text_object(text));
        d.set("M", text_object(&pdf_date()));
    }
    // An edit that grows past its box grows the box.
    grow_edit(doc, page, id)?;
    rebuild_ap(doc, page, id)
}

fn grow_edit(doc: &mut Document, page: ObjectId, id: ObjectId) -> Result<()> {
    let d = doc.get_dictionary(id)?;
    if !d.has(b"NexusEdit") {
        return Ok(());
    }
    let geom = page_geom(doc, page);
    let size = d.get(b"NexusSize").ok().and_then(num).unwrap_or(12.0);
    let name = d.get(b"NexusFont").and_then(|n| n.as_name()).map(|n| String::from_utf8_lossy(n).into_owned()).unwrap_or_default();
    let text = d.get(b"Contents").map(text_of).unwrap_or_default();
    let r = d.get(b"Rect").ok().map(|o| nums(doc, o)).filter(|r| r.len() >= 4).context("no rectangle")?;
    let view = geom.rect_to_view(&Rect::new(r[0], r[1], r[2], r[3]));
    let wanted = text_width(Face::from_name(&name), size, &text.replace('\n', " ")) + 2.0;
    if wanted > view.width() {
        let bigger = Rect::new(view.x0, view.y0, view.x0 + wanted, view.y1);
        let pdf = geom.rect_to_pdf(&bigger);
        doc.get_dictionary_mut(id)?.set("Rect", reals(&[pdf.x0, pdf.y0, pdf.x1, pdf.y1]));
    }
    Ok(())
}

pub fn set_colour(doc: &mut Document, page: ObjectId, id: ObjectId, colour: [f32; 3]) -> Result<()> {
    {
        let d = doc.get_dictionary_mut(id)?;
        d.set("C", colour_array(colour));
        if d.get(b"Subtype").and_then(|s| s.as_name()).unwrap_or(b"") == b"FreeText" {
            let size = d.get(b"NexusSize").ok().and_then(num).unwrap_or(12.0);
            d.set("DA", Object::string_literal(format!("/Helv {size:.1} Tf {} rg", c3(colour))));
        }
    }
    rebuild_ap(doc, page, id)
}

/// Slide an annotation by (`dx`, `dy`) points on the displayed page.
pub fn move_by(doc: &mut Document, page: ObjectId, id: ObjectId, dx: f64, dy: f64) -> Result<()> {
    let geom = page_geom(doc, page);
    let r = doc
        .get_dictionary(id)?
        .get(b"Rect")
        .ok()
        .map(|o| nums(doc, o))
        .filter(|r| r.len() >= 4)
        .context("annotation has no rectangle")?;
    let view = geom.rect_to_view(&Rect::new(r[0], r[1], r[2], r[3]));
    let moved = Rect::new(view.x0 + dx, view.y0 + dy, view.x1 + dx, view.y1 + dy);
    let pdf = geom.rect_to_pdf(&moved);
    doc.get_dictionary_mut(id)?.set("Rect", reals(&[pdf.x0, pdf.y0, pdf.x1, pdf.y1]));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testpdf;

    fn open(path: &std::path::Path) -> poppler::Document {
        use gtk::gio::prelude::FileExt;
        poppler::Document::from_file(&gtk::gio::File::for_path(path).uri(), None).unwrap()
    }

    /// Render page 0 at 1 pixel per point and report the pixel at (x, y) as (r, g, b).
    fn pixel(path: &std::path::Path, x: i32, y: i32) -> (u8, u8, u8) {
        let pdf = open(path);
        let page = pdf.page(0).unwrap();
        let (w, h) = page.size();
        let mut s = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, w as i32, h as i32).unwrap();
        {
            let cr = gtk::cairo::Context::new(&s).unwrap();
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.paint().unwrap();
            page.render(&cr);
        }
        s.flush();
        let stride = s.stride() as usize;
        let data = s.data().unwrap();
        let o = y as usize * stride + x as usize * 4;
        (data[o + 2], data[o + 1], data[o])
    }

    fn setup(name: &str, rotate: Option<i64>) -> (std::path::PathBuf, Document, ObjectId) {
        let dir = testpdf::scratch(name);
        let doc = testpdf::simple(&["Hello world"], rotate);
        let page = page_ids(&doc)[0];
        (dir, doc, page)
    }

    #[test]
    fn highlight_is_drawn_and_listed() {
        let (dir, mut doc, page) = setup("hl", None);
        // Over "Hello" on the displayed page (text is at y-down 75..97, x 100..155).
        let id = add_markup(&mut doc, page, Kind::Highlight, &[Rect::new(100.0, 75.0, 155.0, 97.0)], [1.0, 0.0, 0.0], "Ken").unwrap();
        let file = dir.join("out.pdf");
        doc.save(&file).unwrap();
        let loaded = Document::load(&file).unwrap();
        let found = read_all(&loaded);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].1.kind, Kind::Highlight);
        assert!((found[0].1.rect.x0 - 99.0).abs() < 1.5 && (found[0].1.rect.y0 - 74.0).abs() < 1.5, "{:?}", found[0].1.rect);
        let _ = id;
        // Red multiplied over white paper is red in the highlighted box and white outside.
        let (r, g, b) = pixel(&file, 130, 95);
        assert!(r > 200 && g < 80 && b < 80, "inside: {r},{g},{b}");
        let (r, g, b) = pixel(&file, 400, 400);
        assert_eq!((r, g, b), (255, 255, 255));
    }

    #[test]
    fn highlight_on_rotated_page_lands_on_the_text() {
        let (dir, mut doc, page) = setup("hl90", Some(90));
        // On the 90° page the displayed text runs down the page at x 695..717.
        add_markup(&mut doc, page, Kind::Highlight, &[Rect::new(695.0, 100.0, 717.0, 155.0)], [0.0, 0.0, 1.0], "").unwrap();
        let file = dir.join("out.pdf");
        doc.save(&file).unwrap();
        let (r, g, b) = pixel(&file, 697, 128);
        assert!(b > 200 && r < 120, "{r},{g},{b}");
        let (r, g, b) = pixel(&file, 300, 400);
        assert_eq!((r, g, b), (255, 255, 255));
    }

    #[test]
    fn ink_note_textbox_and_edit_are_drawn() {
        let (dir, mut doc, page) = setup("mix", None);
        add_ink(&mut doc, page, &[vec![(300.0, 300.0), (400.0, 300.0), (400.0, 350.0)]], [0.0, 0.5, 0.0], 3.0, "").unwrap();
        add_note(&mut doc, page, 50.0, 400.0, "Remember this", [1.0, 0.84, 0.04], "").unwrap();
        add_text_box(&mut doc, page, Rect::new(300.0, 500.0, 500.0, 560.0), "A text box that wraps onto more than one line", [0.0, 0.0, 0.0], 14.0, "").unwrap();
        add_edit(&mut doc, page, Rect::new(100.0, 75.0, 190.0, 98.0), "Howdy", Face::default(), 24.0, [1.0, 0.0, 0.0], [1.0, 1.0, 1.0], "").unwrap();
        let file = dir.join("out.pdf");
        doc.save(&file).unwrap();
        assert_eq!(read_all(&Document::load(&file).unwrap()).iter().map(|(_, a)| a.kind).collect::<Vec<_>>(), vec![Kind::Ink, Kind::Note, Kind::FreeText, Kind::Edit]);
        let (r, g, b) = pixel(&file, 350, 300);
        assert!(g > 100 && r < 60 && b < 60, "ink {r},{g},{b}");
        let (r, g, _) = pixel(&file, 54, 404);
        assert!(r > 200 && g > 150, "note {r},{g}");
        // The edit covers "Hello" (no black left there) and writes "Howdy" in red.
        let (mut red, mut black) = (0, 0);
        for y in 75..98 {
            for x in 100..150 {
                let (r, g, b) = pixel(&file, x, y);
                if r > 200 && g < 80 && b < 80 {
                    red += 1;
                }
                if r < 60 && g < 60 && b < 60 {
                    black += 1;
                }
            }
        }
        assert!(red > 20, "red {red}");
        assert_eq!(black, 0, "old text still shows");
    }

    #[test]
    fn stamps_and_text_boxes_stay_upright_on_rotated_pages() {
        for rotate in [0i64, 90, 180, 270] {
            let dir = testpdf::scratch(&format!("stamp{rotate}"));
            let mut doc = testpdf::simple(&["x"], if rotate == 0 { None } else { Some(rotate) });
            let page = page_ids(&doc)[0];
            // A 2×1 picture: red on the left, blue on the right.
            let pic = Picture { width: 2, height: 1, rgb: vec![255, 0, 0, 0, 0, 255], alpha: vec![255, 255] };
            // On the displayed page, 100 × 50 at (200, 300).
            add_stamp(&mut doc, page, Rect::new(200.0, 300.0, 300.0, 350.0), &pic, "").unwrap();
            let file = dir.join("o.pdf");
            doc.save(&file).unwrap();
            let (r, g, b) = pixel(&file, 225, 325);
            assert!(r > 200 && g < 60 && b < 60, "rotate {rotate}: left should be red, got {r},{g},{b}");
            let (r, g, b) = pixel(&file, 275, 325);
            assert!(b > 200 && r < 60 && g < 60, "rotate {rotate}: right should be blue, got {r},{g},{b}");
            let found = read_all(&Document::load(&file).unwrap());
            assert_eq!(found[0].1.kind, Kind::Stamp);
            let r = found[0].1.rect;
            assert!((r.x0 - 200.0).abs() < 0.6 && (r.y1 - 350.0).abs() < 0.6, "rotate {rotate}: {r:?}");
        }
    }

    #[test]
    fn delete_removes_the_annotation() {
        let (dir, mut doc, page) = setup("del", None);
        let id = add_note(&mut doc, page, 50.0, 50.0, "x", [1.0, 1.0, 0.0], "").unwrap();
        assert_eq!(read_all(&doc).len(), 1);
        delete(&mut doc, page, id).unwrap();
        assert!(read_all(&doc).is_empty());
        let file = dir.join("out.pdf");
        doc.save(&file).unwrap();
        assert!(read_all(&Document::load(&file).unwrap()).is_empty());
    }

    #[test]
    fn contents_survive_unicode() {
        let (_dir, mut doc, page) = setup("uni", None);
        let id = add_note(&mut doc, page, 50.0, 50.0, "café — ✓", [1.0, 1.0, 0.0], "").unwrap();
        assert_eq!(read_all(&doc)[0].1.contents, "café — ✓");
        set_contents(&mut doc, page, id, "plain").unwrap();
        assert_eq!(read_all(&doc)[0].1.contents, "plain");
    }

    #[test]
    fn wrapping_respects_width() {
        let lines = wrap(Face::default(), 12.0, 60.0, "one two three four five six");
        assert!(lines.len() > 1);
        for l in &lines {
            assert!(text_width(Face::default(), 12.0, l) <= 60.0 + 20.0, "{l}");
        }
        assert_eq!(wrap(Face::default(), 12.0, 500.0, "a\nb"), vec!["a", "b"]);
    }

    #[test]
    fn dates_look_like_pdf_dates() {
        let d = pdf_date();
        assert!(d.starts_with("D:20") && d.ends_with('Z') && d.len() == 17, "{d}");
    }
}
