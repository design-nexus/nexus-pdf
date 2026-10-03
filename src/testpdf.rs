//! PDFs made in code for the tests, so no binary files live in the repo.

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, Stream, dictionary};
use std::path::Path;

/// A document with `texts.len()` pages, each with its string drawn at (100, 700) in Helvetica 24.
pub fn simple(texts: &[&str], rotate: Option<i64>) -> Document {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
    let resources = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font } });
    let mut kids = Vec::new();
    for text in texts {
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 24.into()]),
                Operation::new("Td", vec![100.into(), 700.into()]),
                Operation::new("Tj", vec![Object::string_literal(*text)]),
                Operation::new("ET", vec![]),
            ],
        };
        let content_id = doc.add_object(Stream::new(Dictionary::new(), content.encode().unwrap()));
        let mut page = dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Resources" => resources,
        };
        if let Some(r) = rotate {
            page.set("Rotate", r);
        }
        kids.push(Object::Reference(doc.add_object(page)));
    }
    let count = kids.len() as i64;
    doc.objects.insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }));
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    doc
}

/// One page with a text field, a check box and a drop-down, as an AcroForm.
pub fn with_form() -> Document {
    let mut doc = simple(&["Application form"], None);
    let page = *doc.get_pages().values().next().unwrap();
    let helv = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding" });
    let text = doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx", "T" => Object::string_literal("name"),
        "Rect" => vec![100.into(), 600.into(), 400.into(), 624.into()], "F" => 4,
        "DA" => Object::string_literal("/Helv 12 Tf 0 g"), "P" => page,
        "MK" => dictionary! { "BC" => vec![0.into()] },
    });
    let check = doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Widget", "FT" => "Btn", "T" => Object::string_literal("agree"),
        "Rect" => vec![100.into(), 550.into(), 120.into(), 570.into()], "F" => 4, "V" => "Off", "AS" => "Off", "P" => page,
        "MK" => dictionary! { "BC" => vec![0.into()], "CA" => Object::string_literal("4") },
        "DA" => Object::string_literal("/ZaDb 0 Tf 0 g"),
    });
    let choice = doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Widget", "FT" => "Ch", "Ff" => 1 << 17, "T" => Object::string_literal("colour"),
        "Rect" => vec![100.into(), 500.into(), 300.into(), 522.into()], "F" => 4, "P" => page,
        "Opt" => vec![Object::string_literal("Red"), Object::string_literal("Green"), Object::string_literal("Blue")],
        "DA" => Object::string_literal("/Helv 12 Tf 0 g"), "MK" => dictionary! { "BC" => vec![0.into()] },
    });
    doc.get_dictionary_mut(page).unwrap().set("Annots", vec![text.into(), check.into(), choice.into()]);
    let form = doc.add_object(dictionary! {
        "Fields" => vec![text.into(), check.into(), choice.into()],
        "NeedAppearances" => true,
        "DA" => Object::string_literal("/Helv 0 Tf 0 g"),
        "DR" => dictionary! { "Font" => dictionary! { "Helv" => helv } },
    });
    let catalog = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
    doc.get_dictionary_mut(catalog).unwrap().set("AcroForm", form);
    doc
}

pub fn write(doc: &mut Document, path: &Path) {
    doc.save(path).unwrap();
}

/// A scratch folder for one test.
pub fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("nexus-pdf-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(test)]
mod probe {
    use super::*;

    use gtk::gio::prelude::FileExt;

    fn open(path: &Path) -> poppler::Document {
        poppler::Document::from_file(&gtk::gio::File::for_path(path).uri(), None).unwrap()
    }

    #[test]
    fn coordinate_probe() {
        let dir = scratch("probe");
        let file = dir.join("a.pdf");
        write(&mut simple(&["Hello world"], None), &file);
        let pdf = open(&file);
        let page = pdf.page(0).unwrap();
        println!("size {:?}", page.size());
        for r in page.find_text("Hello") {
            println!("find_text {} {} {} {}", r.x1(), r.y1(), r.x2(), r.y2());
        }
        let mut sel = poppler::Rectangle::new();
        sel.set_x1(110.0);
        sel.set_y1(70.0);
        sel.set_x2(112.0);
        sel.set_y2(72.0);
        if let Some(region) = page.selected_region(1.0, poppler::SelectionStyle::Word, &mut sel) {
            for i in 0..region.num_rectangles() {
                let r = region.rectangle(i);
                println!("word region (y-down click at 110,70) {} {} {} {}", r.x(), r.y(), r.width(), r.height());
            }
        }
        let mut sel = poppler::Rectangle::new();
        sel.set_x1(90.0);
        sel.set_y1(60.0);
        sel.set_x2(300.0);
        sel.set_y2(110.0);
        println!("selected_text (y-down 60..110) = {:?}", page.selected_text(poppler::SelectionStyle::Glyph, &mut sel));
        let mut sel = poppler::Rectangle::new();
        sel.set_x1(90.0);
        sel.set_y1(680.0);
        sel.set_x2(300.0);
        sel.set_y2(730.0);
        println!("selected_text (y-up 680..730) = {:?}", page.selected_text(poppler::SelectionStyle::Glyph, &mut sel));
    }

    #[test]
    fn rotated_probe() {
        let dir = scratch("rot");
        let file = dir.join("r.pdf");
        write(&mut simple(&["Hello world"], Some(90)), &file);
        let pdf = open(&file);
        let page = pdf.page(0).unwrap();
        println!("rot90 size {:?}", page.size());
        for r in page.find_text("Hello") {
            println!("rot90 find_text {} {} {} {}", r.x1(), r.y1(), r.x2(), r.y2());
        }
        let mut sel = poppler::Rectangle::new();
        sel.set_x1(0.0);
        sel.set_y1(0.0);
        sel.set_x2(792.0);
        sel.set_y2(612.0);
        if let Some(region) = page.selected_region(1.0, poppler::SelectionStyle::Line, &mut sel) {
            for i in 0..region.num_rectangles() {
                let r = region.rectangle(i);
                println!("rot90 region {} {} {} {}", r.x(), r.y(), r.width(), r.height());
            }
        }
    }

    #[test]
    fn text_look_probe() {
        let dir = scratch("look");
        let file = dir.join("l.pdf");
        write(&mut simple(&["Hello world"], None), &file);
        let pdf = open(&file);
        let page = pdf.page(0).unwrap();
        // The text is at y-down 75..97 (y-up 695..717).
        let down = crate::doc::geom::Rect::new(100.0, 75.0, 154.0, 97.0);
        let up = crate::doc::geom::Rect::new(100.0, 695.0, 154.0, 717.0);
        println!("look y-down: {:?}", crate::doc::links::text_look(&page, &down));
        println!("look y-up: {:?}", crate::doc::links::text_look(&page, &up));
    }

    #[test]
    fn form_fields_are_found_and_editable() {
        let dir = scratch("form");
        let file = dir.join("f.pdf");
        write(&mut with_form(), &file);
        let pdf = open(&file);
        let page = pdf.page(0).unwrap();
        let fields = crate::doc::links::fields(&page);
        assert_eq!(fields.len(), 3, "text, check box and drop-down");
        // Areas come back in the displayed frame (y down): the text field is at pdf y 600..624.
        let text = fields.iter().find(|f| f.field.field_type() == poppler::FormFieldType::Text).unwrap();
        assert!((text.area.y0 - (792.0 - 624.0)).abs() < 1.0, "{:?}", text.area);
        // Writing through poppler and saving keeps the value.
        text.field.text_set_text("Ken Smith");
        let out = dir.join("g.pdf");
        pdf.save(&gtk::gio::File::for_path(&out).uri()).unwrap();
        let again = open(&out);
        let f2 = crate::doc::links::fields(&again.page(0).unwrap());
        let t2 = f2.iter().find(|f| f.field.field_type() == poppler::FormFieldType::Text).unwrap();
        assert_eq!(t2.field.text_get_text().unwrap().as_str(), "Ken Smith");
    }
}
#[cfg(test)]
mod write_samples {
    /// `cargo test -- --ignored write_form_sample` leaves a form PDF at $NPDF_SAMPLE_DIR.
    #[test]
    #[ignore]
    fn write_form_sample() {
        let dir = std::env::var("NPDF_SAMPLE_DIR").unwrap();
        super::write(&mut super::with_form(), &std::path::Path::new(&dir).join("form.pdf"));
    }
}
