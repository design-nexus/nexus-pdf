//! Form fields: real entries, check boxes and drop-downs laid over the page where
//! the PDF's fields are. Edits go through poppler and become a new generation.

use super::pageview::PageView;
use crate::doc::links::{self, Field};
use crate::doc::{self, Doc};
use gtk::prelude::*;
use poppler::{FormButtonType, FormFieldType};
use std::cell::RefCell;
use std::rc::Rc;

/// Put this page's fields over it.
pub fn build(pv: &Rc<PageView>, d: &Doc) {
    let pdf = d.pdf();
    let Some(page) = pdf.page(pv.index as i32) else { return };
    let z = pv.size.get();
    let _ = z;
    for f in links::fields(&page) {
        if f.field.is_read_only() {
            continue;
        }
        let Some(widget) = widget_for(&f) else { continue };
        widget.add_css_class("form-field");
        let zoom = super::view().map(|v| v.zoom.get()).unwrap_or(1.0);
        widget.set_size_request((f.area.width() * zoom).round() as i32, (f.area.height() * zoom).round() as i32);
        widget.set_halign(gtk::Align::Start);
        widget.set_valign(gtk::Align::Start);
        pv.forms.put(&widget, f.area.x0 * zoom, f.area.y0 * zoom);
        pv.form_items.borrow_mut().push((widget, f.area));
    }
}

fn commit(label: &str, change: impl FnOnce(&poppler::Document) + 'static) {
    if let Some(d) = doc::current()
        && let Err(e) = d.edit_forms(change)
    {
        crate::window::toast(&format!("Couldn't change {label}: {e:#}"));
    }
}

fn widget_for(f: &Field) -> Option<gtk::Widget> {
    let id = f.field.id();
    let name = f.field.partial_name().map(|n| n.to_string()).unwrap_or_else(|| "that field".into());
    match f.field.field_type() {
        FormFieldType::Text => {
            let initial = f.field.text_get_text().map(|t| t.to_string()).unwrap_or_default();
            let entry = gtk::Entry::new();
            entry.set_text(&initial);
            entry.set_has_frame(false);
            entry.set_tooltip_text(f.field.alternate_ui_name().as_deref());
            if f.field.text_is_password() {
                entry.set_visibility(false);
            }
            let max = f.field.text_get_max_len();
            if max > 0 {
                entry.set_max_length(max);
            }
            let last = Rc::new(RefCell::new(initial));
            let apply = {
                let (last, name) = (last.clone(), name.clone());
                move |e: &gtk::Entry| {
                    let text = e.text().to_string();
                    if *last.borrow() == text {
                        return;
                    }
                    *last.borrow_mut() = text.clone();
                    commit(&name, move |pdf| {
                        if let Some(field) = pdf.form_field(id) {
                            field.text_set_text(&text);
                        }
                    });
                }
            };
            let a = apply.clone();
            entry.connect_activate(move |e| a(e));
            let focus = gtk::EventControllerFocus::new();
            let e2 = entry.clone();
            focus.connect_leave(move |_| apply(&e2));
            entry.add_controller(focus);
            Some(entry.upcast())
        }
        FormFieldType::Button => match f.field.button_get_button_type() {
            FormButtonType::Check | FormButtonType::Radio => {
                let b = gtk::CheckButton::new();
                b.set_active(f.field.button_get_state());
                if f.field.button_get_button_type() == FormButtonType::Radio {
                    b.add_css_class("form-radio");
                }
                let name = name.clone();
                b.connect_toggled(move |b| {
                    let on = b.is_active();
                    commit(&name, move |pdf| {
                        if let Some(field) = pdf.form_field(id) {
                            field.button_set_state(on);
                        }
                    });
                });
                Some(b.upcast())
            }
            _ => None,
        },
        FormFieldType::Choice => {
            let n = f.field.choice_get_n_items();
            let items: Vec<String> = (0..n).map(|i| f.field.choice_get_item(i).map(|s| s.to_string()).unwrap_or_default()).collect();
            let strs: Vec<&str> = items.iter().map(String::as_str).collect();
            let dd = gtk::DropDown::from_strings(&strs);
            let selected = (0..n).find(|&i| f.field.choice_is_item_selected(i));
            dd.set_selected(selected.map_or(gtk::INVALID_LIST_POSITION, |i| i as u32));
            let name = name.clone();
            dd.connect_selected_notify(move |d| {
                let i = d.selected();
                if i == gtk::INVALID_LIST_POSITION {
                    return;
                }
                commit(&name, move |pdf| {
                    if let Some(field) = pdf.form_field(id) {
                        field.choice_unselect_all();
                        field.choice_select_item(i as i32);
                    }
                });
            });
            Some(dd.upcast())
        }
        _ => None,
    }
}
