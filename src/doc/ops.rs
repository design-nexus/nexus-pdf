//! Changes to the page structure: reorder, rotate, delete, insert from another file,
//! extract, combine. All of them work on a lopdf document.

use super::annots::page_ids;
use super::geom::inherited;
use anyhow::{Context, Result, bail};
use lopdf::{Document, Object, ObjectId};
use std::path::Path;

/// Copy the attributes pages inherit from their ancestors onto the pages themselves,
/// so a page can move without losing its size, rotation or resources.
fn flatten(doc: &mut Document) -> Result<()> {
    for page in page_ids(doc) {
        for key in [&b"MediaBox"[..], b"CropBox", b"Rotate", b"Resources"] {
            let has = doc.get_dictionary(page)?.has(key);
            if !has && let Some(v) = inherited(doc, page, key).cloned() {
                doc.get_dictionary_mut(page)?.set(key.to_vec(), v);
            }
        }
    }
    Ok(())
}

fn root_pages(doc: &Document) -> Result<ObjectId> {
    let catalog = doc.catalog()?;
    catalog.get(b"Pages")?.as_reference().context("the page tree is damaged")
}

/// Make the page tree one flat list in this order.
fn set_order(doc: &mut Document, order: &[ObjectId]) -> Result<()> {
    flatten(doc)?;
    let root = root_pages(doc)?;
    for id in order {
        doc.get_dictionary_mut(*id)?.set("Parent", root);
    }
    let d = doc.get_dictionary_mut(root)?;
    d.set("Kids", order.iter().map(|id| Object::Reference(*id)).collect::<Vec<_>>());
    d.set("Count", order.len() as i64);
    Ok(())
}

/// Put the pages in a new order: `order[i]` is the old index of the page that goes at `i`.
pub fn reorder(doc: &mut Document, order: &[usize]) -> Result<()> {
    let ids = page_ids(doc);
    if order.len() != ids.len() || order.iter().any(|&i| i >= ids.len()) {
        bail!("that isn't a valid page order");
    }
    let new: Vec<ObjectId> = order.iter().map(|&i| ids[i]).collect();
    set_order(doc, &new)
}

/// Turn pages `pages` (zero-based) by `degrees` (90, -90 or 180).
pub fn rotate(doc: &mut Document, pages: &[usize], degrees: i32) -> Result<()> {
    let ids = page_ids(doc);
    for &i in pages {
        let id = *ids.get(i).context("no such page")?;
        let current = inherited(doc, id, b"Rotate")
            .and_then(|o| match o {
                Object::Integer(v) => Some(*v),
                Object::Real(v) => Some(*v as i64),
                _ => None,
            })
            .unwrap_or(0);
        let next = (current + i64::from(degrees)).rem_euclid(360);
        doc.get_dictionary_mut(id)?.set("Rotate", next);
    }
    Ok(())
}

/// Remove pages (zero-based). A file can't lose all its pages.
pub fn delete(doc: &mut Document, pages: &[usize]) -> Result<()> {
    let ids = page_ids(doc);
    let keep: Vec<ObjectId> = ids.iter().enumerate().filter(|(i, _)| !pages.contains(i)).map(|(_, id)| *id).collect();
    if keep.is_empty() {
        bail!("A file needs at least one page, so they can't all be deleted.");
    }
    set_order(doc, &keep)?;
    for (i, id) in ids.iter().enumerate() {
        if pages.contains(&i) {
            doc.delete_object(*id);
        }
    }
    doc.prune_objects();
    Ok(())
}

/// Bring all of `other`'s pages into `doc`, placed before page `at` (zero-based; the
/// page count appends them). Returns how many pages were added.
pub fn insert_from(doc: &mut Document, other: &mut Document, at: usize) -> Result<usize> {
    if other.is_encrypted() {
        bail!("that file is encrypted");
    }
    flatten(doc)?;
    flatten(other)?;
    other.renumber_objects_with(doc.max_id + 1);
    let new_pages = page_ids(other);
    if new_pages.is_empty() {
        bail!("that file has no pages");
    }
    let (objects, max_id) = (std::mem::take(&mut other.objects), other.max_id);
    // Their page tree and catalog stay behind; only the pages and what they use come over.
    let skip_root = root_pages(other).ok();
    let skip_catalog = other.trailer.get(b"Root").ok().and_then(|o| o.as_reference().ok());
    for (id, object) in objects {
        if Some(id) == skip_root || Some(id) == skip_catalog {
            continue;
        }
        doc.objects.insert(id, object);
    }
    doc.max_id = doc.max_id.max(max_id);
    let mut order = page_ids(doc);
    let at = at.min(order.len());
    for (k, id) in new_pages.iter().enumerate() {
        order.insert(at + k, *id);
    }
    set_order(doc, &order)?;
    Ok(new_pages.len())
}

/// Write only `pages` (zero-based, in the order given) of `source` to `target`.
pub fn extract(source: &Path, pages: &[usize], target: &Path) -> Result<()> {
    let mut doc = Document::load(source).context("couldn't read the file")?;
    let ids = page_ids(&doc);
    let chosen: Vec<ObjectId> = pages.iter().filter_map(|&i| ids.get(i).copied()).collect();
    if chosen.is_empty() {
        bail!("no pages were chosen");
    }
    set_order(&mut doc, &chosen)?;
    for id in ids.iter().filter(|id| !chosen.contains(id)) {
        doc.delete_object(*id);
    }
    doc.prune_objects();
    doc.compress();
    doc.save(target).with_context(|| format!("couldn't write {}", target.display()))?;
    Ok(())
}

/// Join whole files, in order, into `target`.
pub fn combine(sources: &[std::path::PathBuf], target: &Path) -> Result<()> {
    let mut iter = sources.iter();
    let first = iter.next().context("no files were chosen")?;
    let mut doc = Document::load(first).with_context(|| format!("couldn't read {}", first.display()))?;
    if doc.is_encrypted() {
        bail!("{} is encrypted", first.display());
    }
    for path in iter {
        let mut other = Document::load(path).with_context(|| format!("couldn't read {}", path.display()))?;
        let n = page_ids(&doc).len();
        insert_from(&mut doc, &mut other, n).with_context(|| format!("couldn't add {}", path.display()))?;
    }
    doc.compress();
    doc.save(target).with_context(|| format!("couldn't write {}", target.display()))?;
    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::testpdf;

    fn texts(path: &Path) -> Vec<String> {
        use gtk::gio::prelude::FileExt;
        let pdf = poppler::Document::from_file(&gtk::gio::File::for_path(path).uri(), None).unwrap();
        (0..pdf.n_pages()).map(|i| pdf.page(i).unwrap().text().map(|t| t.trim().to_string()).unwrap_or_default()).collect()
    }

    fn make(name: &str, words: &[&str]) -> (std::path::PathBuf, Document) {
        let dir = testpdf::scratch(name);
        (dir, testpdf::simple(words, None))
    }

    #[test]
    fn reorders_pages() {
        let (dir, mut doc) = make("reorder", &["one", "two", "three"]);
        reorder(&mut doc, &[2, 0, 1]).unwrap();
        let f = dir.join("o.pdf");
        doc.save(&f).unwrap();
        assert_eq!(texts(&f), vec!["three", "one", "two"]);
    }

    #[test]
    fn rotates_and_rotates_back() {
        let (dir, mut doc) = make("rotate", &["a", "b"]);
        rotate(&mut doc, &[1], 90).unwrap();
        let f = dir.join("o.pdf");
        doc.save(&f).unwrap();
        use gtk::gio::prelude::FileExt;
        let pdf = poppler::Document::from_file(&gtk::gio::File::for_path(&f).uri(), None).unwrap();
        assert_eq!(pdf.page(0).unwrap().size(), (612.0, 792.0));
        assert_eq!(pdf.page(1).unwrap().size(), (792.0, 612.0));
        rotate(&mut doc, &[1], -90).unwrap();
        doc.save(&f).unwrap();
        let pdf = poppler::Document::from_file(&gtk::gio::File::for_path(&f).uri(), None).unwrap();
        assert_eq!(pdf.page(1).unwrap().size(), (612.0, 792.0));
    }

    #[test]
    fn deletes_pages_but_not_all() {
        let (dir, mut doc) = make("delete", &["one", "two", "three"]);
        delete(&mut doc, &[1]).unwrap();
        let f = dir.join("o.pdf");
        doc.save(&f).unwrap();
        assert_eq!(texts(&f), vec!["one", "three"]);
        assert!(delete(&mut doc, &[0, 1]).is_err());
    }

    #[test]
    fn inserts_another_file() {
        let (dir, mut doc) = make("insert", &["one", "two"]);
        let mut other = testpdf::simple(&["x", "y"], None);
        assert_eq!(insert_from(&mut doc, &mut other, 1).unwrap(), 2);
        let f = dir.join("o.pdf");
        doc.save(&f).unwrap();
        assert_eq!(texts(&f), vec!["one", "x", "y", "two"]);
    }

    #[test]
    fn extracts_and_combines() {
        let (dir, mut doc) = make("extract", &["one", "two", "three", "four"]);
        let src = dir.join("src.pdf");
        doc.save(&src).unwrap();
        let out = dir.join("out.pdf");
        extract(&src, &[3, 1], &out).unwrap();
        assert_eq!(texts(&out), vec!["four", "two"]);
        let both = dir.join("both.pdf");
        combine(&[src.clone(), out.clone()], &both).unwrap();
        assert_eq!(texts(&both), vec!["one", "two", "three", "four", "four", "two"]);
    }

    #[test]
    fn reorder_keeps_annotations_with_their_pages() {
        let (dir, mut doc) = make("annot-move", &["one", "two"]);
        let ids = page_ids(&doc);
        super::super::annots::add_note(&mut doc, ids[0], 40.0, 40.0, "on page one", [1.0, 1.0, 0.0], "").unwrap();
        reorder(&mut doc, &[1, 0]).unwrap();
        let all = super::super::annots::read_all(&doc);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].0, 1, "the note should now be on the second page");
        let f = dir.join("o.pdf");
        doc.save(&f).unwrap();
        assert_eq!(texts(&f), vec!["two", "one"]);
    }
}
