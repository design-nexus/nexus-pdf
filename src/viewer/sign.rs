//! Signatures: draw one, or pick a picture, keep it in `~/.local/share/nexus-pdf/signatures`.

use crate::doc::annots::Picture;
use crate::{paths, widgets, window};
use anyhow::{Context, Result, bail};
use gtk::prelude::*;
use gtk::{gdk, gio};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Pen colour of a drawn signature: dark blue ink, whatever the theme.
const INK: (f64, f64, f64) = (0.07, 0.10, 0.30);

fn saved() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(paths::signatures_dir())
        .map(|d| d.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect())
        .unwrap_or_default();
    v.sort();
    v.reverse();
    v
}

/// Ask which signature to use. `done` gets the file, or `None` when the user gave up.
pub fn choose(done: impl Fn(Option<PathBuf>) + 'static) {
    let done = Rc::new(done);
    let chosen = Rc::new(Cell::new(false));
    let (dialog, card) = widgets::dialog("Signature", 480);
    let list = saved();
    if list.is_empty() {
        let d = widgets::label("Draw your signature with the mouse or a touchpad, or use a picture of it.", "dim");
        d.set_wrap(true);
        card.append(&d);
    } else {
        let flow = gtk::FlowBox::new();
        flow.set_selection_mode(gtk::SelectionMode::None);
        flow.set_max_children_per_line(3);
        flow.set_column_spacing(8);
        flow.set_row_spacing(8);
        for path in list.iter().take(9) {
            let b = gtk::Button::new();
            b.add_css_class("signature-choice");
            let pic = gtk::Picture::for_filename(path);
            pic.set_can_shrink(true);
            pic.set_content_fit(gtk::ContentFit::Contain);
            pic.set_size_request(130, 52);
            b.set_child(Some(&pic));
            let (d, done, chosen, path) = (dialog.clone(), done.clone(), chosen.clone(), path.clone());
            b.connect_clicked(move |_| {
                chosen.set(true);
                d.close();
                done(Some(path.clone()));
            });
            flow.append(&b);
        }
        card.append(&flow);
    }
    let buttons = widgets::hbox(8);
    buttons.set_halign(gtk::Align::End);
    buttons.set_margin_top(6);
    let cancel = gtk::Button::with_label("Not now");
    let pick = gtk::Button::with_label("Use a picture…");
    let draw = gtk::Button::with_label("Draw new");
    draw.add_css_class("suggested-action");
    buttons.append(&cancel);
    buttons.append(&pick);
    buttons.append(&draw);
    card.append(&buttons);
    {
        let d = dialog.clone();
        cancel.connect_clicked(move |_| d.close());
    }
    {
        let (d, done, chosen) = (dialog.clone(), done.clone(), chosen.clone());
        draw.connect_clicked(move |_| {
            chosen.set(true);
            d.close();
            draw_dialog(done.clone());
        });
    }
    {
        let (d, done, chosen) = (dialog.clone(), done.clone(), chosen.clone());
        pick.connect_clicked(move |_| {
            chosen.set(true);
            d.close();
            pick_picture(done.clone());
        });
    }
    {
        let (done, chosen) = (done.clone(), chosen.clone());
        dialog.connect_close_request(move |_| {
            if !chosen.get() {
                done(None);
            }
            gtk::glib::Propagation::Proceed
        });
    }
    dialog.present();
}

fn stamp_name(ext: &str) -> PathBuf {
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    paths::signatures_dir().join(format!("signature-{ms}.{ext}"))
}

fn pick_picture(done: Rc<dyn Fn(Option<PathBuf>)>) {
    let dialog = gtk::FileDialog::builder().title("Choose a picture of your signature").modal(true).build();
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Pictures"));
    filter.add_mime_type("image/*");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    dialog.set_filters(Some(&filters));
    dialog.open(window::window().as_ref(), gio::Cancellable::NONE, move |res| {
        let Some(path) = res.ok().and_then(|f| f.path()) else {
            done(None);
            return;
        };
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_else(|| "png".into());
        let target = stamp_name(&ext);
        let copied = std::fs::create_dir_all(paths::signatures_dir()).and_then(|_| std::fs::copy(&path, &target));
        match (copied, load_picture(&target)) {
            (Ok(_), Ok(_)) => done(Some(target)),
            (_, Err(e)) => {
                let _ = std::fs::remove_file(&target);
                window::toast(&format!("Couldn't use that picture: {e}"));
                done(None);
            }
            (Err(e), _) => {
                window::toast(&format!("Couldn't keep that picture: {e}"));
                done(None);
            }
        }
    });
}

fn draw_dialog(done: Rc<dyn Fn(Option<PathBuf>)>) {
    let (dialog, card) = widgets::dialog("Draw your signature", 520);
    type Strokes = Rc<RefCell<Vec<Vec<(f64, f64)>>>>;
    let strokes: Strokes = Rc::new(RefCell::new(Vec::new()));
    let area = gtk::DrawingArea::new();
    area.set_content_width(480);
    area.set_content_height(180);
    area.add_css_class("signature-pad");
    {
        let strokes = strokes.clone();
        area.set_draw_func(move |_, cr, w, h| {
            // The pad is paper: the ink is dark, so the background is light, whatever the theme.
            cr.set_source_rgb(1.0, 1.0, 1.0);
            let _ = cr.paint();
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.18);
            cr.set_line_width(1.0);
            cr.move_to(24.0, f64::from(h) - 44.0);
            cr.line_to(f64::from(w) - 24.0, f64::from(h) - 44.0);
            let _ = cr.stroke();
            cr.set_source_rgb(INK.0, INK.1, INK.2);
            cr.set_line_width(2.4);
            cr.set_line_cap(gtk::cairo::LineCap::Round);
            cr.set_line_join(gtk::cairo::LineJoin::Round);
            for s in strokes.borrow().iter() {
                if let Some(first) = s.first() {
                    cr.move_to(first.0, first.1);
                    if s.len() == 1 {
                        cr.line_to(first.0 + 0.1, first.1);
                    }
                    for p in &s[1..] {
                        cr.line_to(p.0, p.1);
                    }
                    let _ = cr.stroke();
                }
            }
        });
    }
    let drag = gtk::GestureDrag::new();
    let origin = Rc::new(Cell::new((0.0, 0.0)));
    {
        let (strokes, area, origin) = (strokes.clone(), area.clone(), origin.clone());
        drag.connect_drag_begin(move |_, x, y| {
            origin.set((x, y));
            strokes.borrow_mut().push(vec![(x, y)]);
            area.queue_draw();
        });
    }
    {
        let (strokes, area, origin) = (strokes.clone(), area.clone(), origin.clone());
        drag.connect_drag_update(move |_, dx, dy| {
            let (ox, oy) = origin.get();
            if let Some(s) = strokes.borrow_mut().last_mut() {
                s.push((ox + dx, oy + dy));
            }
            area.queue_draw();
        });
    }
    area.add_controller(drag);
    area.add_css_class("card-frame");
    card.append(&area);
    let buttons = widgets::hbox(8);
    buttons.set_halign(gtk::Align::End);
    buttons.set_margin_top(6);
    let cancel = gtk::Button::with_label("Not now");
    let clear = gtk::Button::with_label("Clear");
    let save = gtk::Button::with_label("Use this signature");
    save.add_css_class("suggested-action");
    buttons.append(&cancel);
    buttons.append(&clear);
    buttons.append(&save);
    card.append(&buttons);
    {
        let (d, done) = (dialog.clone(), done.clone());
        cancel.connect_clicked(move |_| {
            d.close();
            done(None);
        });
    }
    {
        let (strokes, area) = (strokes.clone(), area.clone());
        clear.connect_clicked(move |_| {
            strokes.borrow_mut().clear();
            area.queue_draw();
        });
    }
    {
        let (d, done, strokes) = (dialog.clone(), done.clone(), strokes.clone());
        save.connect_clicked(move |_| match render_strokes(&strokes.borrow()) {
            Ok(path) => {
                d.close();
                done(Some(path));
            }
            Err(e) => window::toast(&format!("{e}")),
        });
    }
    {
        // Closing the window any other way counts as giving up (after the save path closed it, `done` already ran).
        let done = done.clone();
        let finished = Rc::new(Cell::new(false));
        let f2 = finished.clone();
        save.connect_clicked(move |_| f2.set(true));
        let f3 = finished.clone();
        cancel.connect_clicked(move |_| f3.set(true));
        dialog.connect_close_request(move |_| {
            if !finished.replace(true) {
                done(None);
            }
            gtk::glib::Propagation::Proceed
        });
    }
    dialog.present();
}

/// Draw the strokes onto a transparent picture, cropped to the ink, and keep it.
fn render_strokes(strokes: &[Vec<(f64, f64)>]) -> Result<PathBuf> {
    let pts = strokes.iter().flatten().copied().collect::<Vec<_>>();
    if pts.is_empty() {
        bail!("Draw your signature first.");
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for (x, y) in &pts {
        x0 = x0.min(*x);
        y0 = y0.min(*y);
        x1 = x1.max(*x);
        y1 = y1.max(*y);
    }
    let margin = 6.0;
    let scale = 3.0;
    let (w, h) = (((x1 - x0 + 2.0 * margin) * scale).ceil() as i32, ((y1 - y0 + 2.0 * margin) * scale).ceil() as i32);
    let surface = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, w.max(8), h.max(8))?;
    {
        let cr = gtk::cairo::Context::new(&surface)?;
        cr.scale(scale, scale);
        cr.translate(margin - x0, margin - y0);
        cr.set_source_rgb(INK.0, INK.1, INK.2);
        cr.set_line_width(2.4);
        cr.set_line_cap(gtk::cairo::LineCap::Round);
        cr.set_line_join(gtk::cairo::LineJoin::Round);
        for s in strokes {
            if let Some(first) = s.first() {
                cr.move_to(first.0, first.1);
                if s.len() == 1 {
                    cr.line_to(first.0 + 0.1, first.1);
                }
                for p in &s[1..] {
                    cr.line_to(p.0, p.1);
                }
                cr.stroke()?;
            }
        }
    }
    std::fs::create_dir_all(paths::signatures_dir())?;
    let path = stamp_name("png");
    let mut file = std::fs::File::create(&path)?;
    surface.write_to_png(&mut file).context("couldn't write the signature")?;
    Ok(path)
}

/// Read a signature picture as RGB plus alpha, trimmed to its ink and capped in size.
/// A picture with no transparency (a photo of a signature) has its paper made transparent.
pub fn load_picture(path: &Path) -> Result<Picture> {
    let tex = gdk::Texture::from_filename(path).map_err(|e| anyhow::anyhow!("{}", e.message()))?;
    let (w, h) = (tex.width() as usize, tex.height() as usize);
    if w == 0 || h == 0 {
        bail!("the picture is empty");
    }
    let mut dl = gdk::TextureDownloader::new(&tex);
    dl.set_format(gdk::MemoryFormat::R8g8b8a8);
    let (bytes, stride) = dl.download_bytes();
    let px = |x: usize, y: usize| -> [u8; 4] {
        let o = y * stride + x * 4;
        [bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]
    };
    let opaque = (0..h).step_by(3).all(|y| (0..w).step_by(3).all(|x| px(x, y)[3] > 250));
    let mut rgba = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let [r, g, b, a] = px(x, y);
            let a = if opaque {
                let lum = (u32::from(r) + u32::from(g) + u32::from(b)) / 3;
                if lum >= 225 { 0 } else { ((225 - lum) * 255 / 225).min(255) as u8 }
            } else {
                a
            };
            rgba.push([r, g, b, a]);
        }
    }
    // Trim to the ink.
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if rgba[y * w + x][3] > 12 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    if x1 < x0 || y1 < y0 {
        bail!("there's no signature in that picture");
    }
    let (cw, ch) = (x1 - x0 + 1, y1 - y0 + 1);
    // Box-average down so the file stays small.
    let f = (cw.max(ch * 3) / 900).max(1);
    let (ow, oh) = (cw.div_ceil(f), ch.div_ceil(f));
    let mut rgb = Vec::with_capacity(ow * oh * 3);
    let mut alpha = Vec::with_capacity(ow * oh);
    for oy in 0..oh {
        for ox in 0..ow {
            let (mut r, mut g, mut b, mut a, mut n) = (0u32, 0u32, 0u32, 0u32, 0u32);
            for sy in 0..f {
                for sx in 0..f {
                    let (x, y) = (x0 + ox * f + sx, y0 + oy * f + sy);
                    if x <= x1 && y <= y1 {
                        let p = rgba[y * w + x];
                        // Weight colour by alpha so transparent pixels don't wash it out.
                        let wgt = u32::from(p[3]);
                        r += u32::from(p[0]) * wgt;
                        g += u32::from(p[1]) * wgt;
                        b += u32::from(p[2]) * wgt;
                        a += wgt;
                        n += 1;
                    }
                }
            }
            rgb.extend_from_slice(&[r.checked_div(a).unwrap_or(0) as u8, g.checked_div(a).unwrap_or(0) as u8, b.checked_div(a).unwrap_or(0) as u8]);
            alpha.push((a / n.max(1)) as u8);
        }
    }
    Ok(Picture { width: ow, height: oh, rgb, alpha })
}
