//! Page geometry: the bridge between what's on screen (points from the top-left
//! corner of the displayed page, which is what poppler reports) and PDF user space
//! (points from the bottom-left of the crop box, before rotation).

use lopdf::{Dictionary, Document, Object, ObjectId};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageGeom {
    /// Crop box (falls back to the media box): x0, y0, x1, y1 in PDF space.
    pub bx: [f64; 4],
    /// 0, 90, 180 or 270, clockwise.
    pub rotate: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl Rect {
    pub fn new(ax: f64, ay: f64, bx: f64, by: f64) -> Rect {
        Rect { x0: ax.min(bx), y0: ay.min(by), x1: ax.max(bx), y1: ay.max(by) }
    }

    pub fn width(&self) -> f64 {
        self.x1 - self.x0
    }

    pub fn height(&self) -> f64 {
        self.y1 - self.y0
    }

    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x0 && x <= self.x1 && y >= self.y0 && y <= self.y1
    }

    pub fn inflate(&self, by: f64) -> Rect {
        Rect { x0: self.x0 - by, y0: self.y0 - by, x1: self.x1 + by, y1: self.y1 + by }
    }

    pub fn union(&self, o: &Rect) -> Rect {
        Rect { x0: self.x0.min(o.x0), y0: self.y0.min(o.y0), x1: self.x1.max(o.x1), y1: self.y1.max(o.y1) }
    }
}

fn num(o: &Object) -> Option<f64> {
    match o {
        Object::Integer(i) => Some(*i as f64),
        Object::Real(r) => Some(*r as f64),
        _ => None,
    }
}

pub fn rect_from(doc: &Document, o: &Object) -> Option<[f64; 4]> {
    let o = match o {
        Object::Reference(id) => doc.get_object(*id).ok()?,
        o => o,
    };
    let a = o.as_array().ok()?;
    if a.len() < 4 {
        return None;
    }
    let mut v = [0.0; 4];
    for (i, x) in a.iter().take(4).enumerate() {
        let x = match x {
            Object::Reference(id) => doc.get_object(*id).ok()?,
            x => x,
        };
        v[i] = num(x)?;
    }
    Some([v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])])
}

/// A page attribute, looked up the page tree (`MediaBox`, `CropBox`, `Rotate` and
/// `Resources` are inheritable).
pub fn inherited<'a>(doc: &'a Document, page: ObjectId, key: &[u8]) -> Option<&'a Object> {
    let mut id = page;
    for _ in 0..64 {
        let d = doc.get_object(id).ok()?.as_dict().ok()?;
        if let Ok(v) = d.get(key) {
            return Some(match v {
                Object::Reference(r) => doc.get_object(*r).ok()?,
                v => v,
            });
        }
        id = d.get(b"Parent").ok()?.as_reference().ok()?;
    }
    None
}

pub fn page_geom(doc: &Document, page: ObjectId) -> PageGeom {
    let media = inherited(doc, page, b"MediaBox").and_then(|o| rect_from(doc, o)).unwrap_or([0.0, 0.0, 612.0, 792.0]);
    let bx = inherited(doc, page, b"CropBox").and_then(|o| rect_from(doc, o)).map(|c| intersect(c, media)).unwrap_or(media);
    let rotate = inherited(doc, page, b"Rotate").and_then(num).map(|r| (r as i32).rem_euclid(360) / 90 * 90).unwrap_or(0);
    PageGeom { bx, rotate }
}

fn intersect(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    let r = [a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].min(b[3])];
    if r[2] > r[0] && r[3] > r[1] { r } else { b }
}

impl PageGeom {
    fn w(&self) -> f64 {
        self.bx[2] - self.bx[0]
    }

    fn h(&self) -> f64 {
        self.bx[3] - self.bx[1]
    }

    /// A point on the displayed page → PDF user space.
    pub fn to_pdf(self, vx: f64, vy: f64) -> (f64, f64) {
        let (w, h) = (self.w(), self.h());
        let (ux, uy) = match self.rotate {
            90 => (vy, h - vx),
            180 => (w - vx, h - vy),
            270 => (w - vy, vx),
            _ => (vx, vy),
        };
        (self.bx[0] + ux, self.bx[3] - uy)
    }

    /// A point in PDF user space → the displayed page.
    pub fn to_view(self, px: f64, py: f64) -> (f64, f64) {
        let (w, h) = (self.w(), self.h());
        let (ux, uy) = (px - self.bx[0], self.bx[3] - py);
        match self.rotate {
            90 => (h - uy, ux),
            180 => (w - ux, h - uy),
            270 => (uy, w - ux),
            _ => (ux, uy),
        }
    }

    /// A rectangle on the displayed page → the bounding rectangle in PDF space.
    pub fn rect_to_pdf(&self, r: &Rect) -> Rect {
        let (ax, ay) = self.to_pdf(r.x0, r.y0);
        let (bx, by) = self.to_pdf(r.x1, r.y1);
        Rect::new(ax, ay, bx, by)
    }

    pub fn rect_to_view(&self, r: &Rect) -> Rect {
        let (ax, ay) = self.to_view(r.x0, r.y0);
        let (bx, by) = self.to_view(r.x1, r.y1);
        Rect::new(ax, ay, bx, by)
    }
}

pub fn get_dict<'a>(doc: &'a Document, o: &'a Object) -> Option<&'a Dictionary> {
    match o {
        Object::Dictionary(d) => Some(d),
        Object::Reference(id) => doc.get_object(*id).ok()?.as_dict().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9
    }

    #[test]
    fn round_trips_every_rotation() {
        for rotate in [0, 90, 180, 270] {
            let g = PageGeom { bx: [10.0, 20.0, 310.0, 420.0], rotate };
            let (w, h) = if rotate % 180 == 0 { (300.0, 400.0) } else { (400.0, 300.0) };
            for (x, y) in [(0.0, 0.0), (w, h), (12.5, 77.0), (w / 2.0, h / 3.0)] {
                let (px, py) = g.to_pdf(x, y);
                assert!(close(g.to_view(px, py), (x, y)), "rotate {rotate} at {x},{y}");
            }
        }
    }

    #[test]
    fn top_left_is_top_of_the_crop_box() {
        let g = PageGeom { bx: [0.0, 0.0, 200.0, 100.0], rotate: 0 };
        assert!(close(g.to_pdf(0.0, 0.0), (0.0, 100.0)));
        let g = PageGeom { bx: [0.0, 0.0, 200.0, 100.0], rotate: 90 };
        // Displayed 100 wide and 200 tall; its top-left is the old bottom-left corner.
        assert!(close(g.to_pdf(0.0, 0.0), (0.0, 0.0)));
    }
}
