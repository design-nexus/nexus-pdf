//! Page pictures. One worker thread owns its own copy of the document (poppler
//! objects can't cross threads), renders what the window asks for, newest request
//! first, and sends the pixels back for the main thread to turn into textures.

use gtk::prelude::*;
use gtk::{gdk, glib};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

/// How page pixels are recoloured.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Colors {
    Off,
    Invert,
    /// White becomes `.0`, black becomes `.1` (both as 0..255 RGB).
    Tint([u8; 3], [u8; 3]),
}

struct Job {
    id: u64,
    priority: u8,
    cancel: Arc<AtomicBool>,
    path: PathBuf,
    password: Option<String>,
    page: usize,
    /// Pixels per point.
    scale: f64,
    colors: Colors,
}

struct Done {
    id: u64,
    width: i32,
    height: i32,
    stride: usize,
    data: Vec<u8>,
}

struct Queue {
    jobs: Mutex<Vec<Job>>,
    wake: Condvar,
}

type Callback = Box<dyn FnOnce(gdk::Texture)>;

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

struct State {
    queue: Arc<Queue>,
    next: u64,
    waiting: HashMap<u64, Callback>,
}

/// A pending render; dropping or cancelling it stops the work if it hasn't started.
pub struct Ticket {
    cancel: Arc<AtomicBool>,
    id: u64,
}

impl Ticket {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
        STATE.with(|s| {
            if let Some(st) = s.borrow_mut().as_mut() {
                st.waiting.remove(&self.id);
            }
        });
    }
}

fn start() -> State {
    let queue = Arc::new(Queue { jobs: Mutex::new(Vec::new()), wake: Condvar::new() });
    let (tx, rx) = async_channel::unbounded::<Done>();
    let q = queue.clone();
    std::thread::spawn(move || worker(q, tx));
    glib::spawn_future_local(async move {
        while let Ok(done) = rx.recv().await {
            let texture = gdk::MemoryTexture::new(
                done.width,
                done.height,
                gdk::MemoryFormat::B8g8r8a8Premultiplied,
                &glib::Bytes::from_owned(done.data),
                done.stride,
            );
            let cb = STATE.with(|s| s.borrow_mut().as_mut().and_then(|st| st.waiting.remove(&done.id)));
            if let Some(cb) = cb {
                cb(texture.upcast());
            }
        }
    });
    State { queue, next: 1, waiting: HashMap::new() }
}

/// Ask for page `page` of the file at `path` at `scale` pixels per point. Higher `priority` renders first.
/// `done` runs on the main thread with the picture, unless the ticket is cancelled first.
pub fn request(
    path: PathBuf,
    password: Option<String>,
    page: usize,
    scale: f64,
    colors: Colors,
    priority: u8,
    done: impl FnOnce(gdk::Texture) + 'static,
) -> Ticket {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let st = s.get_or_insert_with(start);
        let id = st.next;
        st.next += 1;
        st.waiting.insert(id, Box::new(done));
        let cancel = Arc::new(AtomicBool::new(false));
        st.queue.jobs.lock().unwrap().push(Job { id, priority, cancel: cancel.clone(), path, password, page, scale, colors });
        st.queue.wake.notify_one();
        Ticket { cancel, id }
    })
}

fn worker(queue: Arc<Queue>, tx: async_channel::Sender<Done>) {
    let mut open: Option<(PathBuf, poppler::Document)> = None;
    loop {
        let job = {
            let mut jobs = queue.jobs.lock().unwrap();
            loop {
                jobs.retain(|j| !j.cancel.load(Ordering::Relaxed));
                // Highest priority first; among equals, the newest request.
                if let Some(best) = (0..jobs.len()).max_by_key(|&i| (jobs[i].priority, jobs[i].id)) {
                    break jobs.swap_remove(best);
                }
                jobs = queue.wake.wait(jobs).unwrap();
            }
        };
        if open.as_ref().map(|(p, _)| p) != Some(&job.path) {
            let uri = gtk::gio::File::for_path(&job.path).uri();
            open = poppler::Document::from_file(&uri, job.password.as_deref()).ok().map(|d| (job.path.clone(), d));
        }
        let Some((_, doc)) = &open else { continue };
        let Some(page) = doc.page(job.page as i32) else { continue };
        if let Some(done) = draw(job.id, &page, job.scale, job.colors)
            && tx.send_blocking(done).is_err()
        {
            return;
        }
    }
}

fn draw(id: u64, page: &poppler::Page, scale: f64, colors: Colors) -> Option<Done> {
    let (w, h) = page.size();
    let (pw, ph) = (((w * scale).ceil() as i32).max(1), ((h * scale).ceil() as i32).max(1));
    let mut surface = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, pw, ph).ok()?;
    {
        let cr = gtk::cairo::Context::new(&surface).ok()?;
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.paint().ok()?;
        cr.scale(pw as f64 / w, ph as f64 / h);
        page.render(&cr);
    }
    surface.flush();
    let stride = surface.stride() as usize;
    let mut data = surface.data().ok()?.to_vec();
    recolor(&mut data, colors);
    Some(Done { id, width: pw, height: ph, stride, data })
}

/// Recolour opaque BGRA pixels in place.
pub fn recolor(data: &mut [u8], colors: Colors) {
    match colors {
        Colors::Off => {}
        Colors::Invert => {
            for px in data.as_chunks_mut::<4>().0 {
                px[0] = 255 - px[0];
                px[1] = 255 - px[1];
                px[2] = 255 - px[2];
            }
        }
        Colors::Tint(paper, ink) => {
            // Per channel: white maps to the paper colour, black to the ink colour.
            for px in data.as_chunks_mut::<4>().0 {
                for (i, c) in [2usize, 1, 0].into_iter().enumerate() {
                    let (lo, hi) = (ink[i] as f32, paper[i] as f32);
                    px[c] = (lo + (hi - lo) * (px[c] as f32 / 255.0)).round() as u8;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tint_maps_white_and_black() {
        let mut px = [255, 255, 255, 255, 0, 0, 0, 255];
        recolor(&mut px, Colors::Tint([10, 20, 30], [200, 210, 220]));
        assert_eq!(&px[0..3], &[30, 20, 10]);
        assert_eq!(&px[4..7], &[220, 210, 200]);
    }

    #[test]
    fn invert_flips() {
        let mut px = [0, 100, 255, 255];
        recolor(&mut px, Colors::Invert);
        assert_eq!(&px[0..3], &[255, 155, 0]);
    }
}
