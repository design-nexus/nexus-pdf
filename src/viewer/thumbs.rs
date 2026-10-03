//! Small page pictures for the side panel and the Pages section, shared so each
//! page of each generation is drawn once.

use crate::doc::{self, render};
use gtk::gdk;
use std::cell::RefCell;
use std::collections::HashMap;

type Key = (usize, usize, u32);

thread_local! {
    static CACHE: RefCell<HashMap<Key, gdk::Texture>> = RefCell::new(HashMap::new());
}

/// A picture of page `page` about `width` pixels wide. `done` runs now when it's
/// cached, otherwise when it's been drawn.
pub fn get(page: usize, width: u32, done: impl FnOnce(gdk::Texture) + 'static) -> Option<render::Ticket> {
    let doc = doc::current()?;
    let key = (doc.generation(), page, width);
    if let Some(t) = CACHE.with(|c| c.borrow().get(&key).cloned()) {
        done(t);
        return None;
    }
    let (w, _) = doc.page_size(page);
    let scale = f64::from(width) / w.max(1.0);
    let colors = super::page_colors();
    let epoch = super::view().map(|v| v.epoch.get());
    Some(render::request(doc.gen_path(), doc.password(), page, scale, colors, 0, move |t| {
        // The palette changed while this was being drawn: it's the wrong colour now.
        if super::view().map(|v| v.epoch.get()) != epoch {
            return;
        }
        CACHE.with(|c| {
            let mut c = c.borrow_mut();
            // Old generations are never asked for again.
            c.retain(|k, _| k.0 == key.0);
            c.insert(key, t.clone());
        });
        done(t);
    }))
}

pub fn clear() {
    CACHE.with(|c| c.borrow_mut().clear());
}
