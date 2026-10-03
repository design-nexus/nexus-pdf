//! Looking at existing text before it's replaced: its font, size and colour, and the
//! paper colour around it, so the replacement blends in.

use super::annots::Face;
use super::geom::Rect;
use super::links;
use gtk::cairo;
use std::collections::HashMap;

pub struct Look {
    /// The box the replacement fills, on the displayed page.
    pub rect: Rect,
    pub face: Face,
    pub size: f64,
    pub fg: [f32; 3],
    pub bg: [f32; 3],
}

/// Judge `region` (on the displayed page, y down) of page `index`.
pub fn analyse(pdf: &poppler::Document, index: usize, region: &Rect) -> Look {
    let page = pdf.page(index as i32);
    let looks = page.as_ref().and_then(|p| links::text_look(p, region));
    let (face, size, fg) = match &looks {
        Some(l) if l.size > 1.0 => (Face::from_name(&l.font), l.size, l.colour),
        _ => (Face::default(), (region.height() * 0.78).clamp(6.0, 72.0), [0.0, 0.0, 0.0]),
    };
    let rect = region.inflate(0.6);
    let bg = page.as_ref().and_then(|p| paper_colour(p, &rect)).unwrap_or([1.0, 1.0, 1.0]);
    Look { rect, face, size, fg, bg }
}

/// The most common colour just outside `rect`: the paper the text sits on.
fn paper_colour(page: &poppler::Page, rect: &Rect) -> Option<[f32; 3]> {
    const S: f64 = 2.0;
    let ring = rect.inflate(3.0);
    let (w, h) = ((ring.width() * S).ceil() as i32, (ring.height() * S).ceil() as i32);
    if w < 4 || h < 4 || w > 4000 || h > 4000 {
        return None;
    }
    let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, w, h).ok()?;
    {
        let cr = cairo::Context::new(&surface).ok()?;
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.paint().ok()?;
        cr.scale(S, S);
        cr.translate(-ring.x0, -ring.y0);
        page.render(&cr);
    }
    surface.flush();
    let stride = surface.stride() as usize;
    let data = surface.data().ok()?;
    let inner = ((rect.x0 - ring.x0) * S, (rect.y0 - ring.y0) * S, (rect.x1 - ring.x0) * S, (rect.y1 - ring.y0) * S);
    let mut counts: HashMap<[u8; 3], u32> = HashMap::new();
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (f64::from(x), f64::from(y));
            // Only the ring between the text box and the sampling frame.
            if fx >= inner.0 - S && fx <= inner.2 + S && fy >= inner.1 - S && fy <= inner.3 + S {
                continue;
            }
            let o = y as usize * stride + x as usize * 4;
            *counts.entry([data[o + 2], data[o + 1], data[o]]).or_default() += 1;
        }
    }
    let best = counts.into_iter().max_by_key(|(_, n)| *n)?.0;
    Some([f32::from(best[0]) / 255.0, f32::from(best[1]) / 255.0, f32::from(best[2]) / 255.0])
}
