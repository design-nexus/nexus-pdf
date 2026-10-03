//! Colour choices: a row of round swatches (STYLE §7) with a hex field.

use crate::widgets;
use gtk::prelude::*;
use std::rc::Rc;

pub const PRESETS: &[(&str, &str)] = &[
    ("#000000", "Black"),
    ("#ffffff", "White"),
    ("#ff3b30", "Red"),
    ("#ff8c1a", "Orange"),
    ("#ffd60a", "Yellow"),
    ("#34c759", "Green"),
    ("#32d7e6", "Cyan"),
    ("#0a84ff", "Blue"),
    ("#8e5cf7", "Purple"),
    ("#ff4fa3", "Pink"),
];

pub fn parse(hex: &str) -> Option<[f32; 3]> {
    let h = hex.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some([((v >> 16) & 255) as f32 / 255.0, ((v >> 8) & 255) as f32 / 255.0, (v & 255) as f32 / 255.0])
}

pub fn parse_u8(hex: &str) -> Option<[u8; 3]> {
    parse(hex).map(|c| [(c[0] * 255.0).round() as u8, (c[1] * 255.0).round() as u8, (c[2] * 255.0).round() as u8])
}

fn dot_css(dot: &gtk::Box, colour: &str) {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(&format!("box.dot {{ background: {colour}; }}"));
    #[allow(deprecated)]
    dot.style_context().add_provider(&provider, gtk::STYLE_PROVIDER_PRIORITY_USER);
}

/// Swatches plus a hex field. `on_change` gets `#rrggbb`.
pub fn row(current: &str, on_change: impl Fn(String) + 'static) -> gtk::Box {
    let bx = widgets::hbox(6);
    bx.add_css_class("colour-row");
    let on_change = Rc::new(on_change);
    let entry = gtk::Entry::new();
    entry.add_css_class("mono");
    entry.set_width_chars(8);
    entry.set_max_width_chars(8);
    entry.set_text(current);
    let buttons: Rc<Vec<(gtk::Button, &'static str)>> = Rc::new(
        PRESETS
            .iter()
            .map(|(hex, name)| {
                let b = gtk::Button::new();
                b.add_css_class("swatch-button");
                b.set_tooltip_text(Some(name));
                let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                dot.add_css_class("dot");
                b.set_child(Some(&dot));
                dot_css(&dot, hex);
                (b, *hex)
            })
            .collect(),
    );
    let select = {
        let buttons = buttons.clone();
        move |hex: &str| {
            for (b, h) in buttons.iter() {
                if h.eq_ignore_ascii_case(hex) {
                    b.add_css_class("selected");
                } else {
                    b.remove_css_class("selected");
                }
            }
        }
    };
    select(current);
    for (b, hex) in buttons.iter() {
        let (cb, entry, select, hex) = (on_change.clone(), entry.clone(), select.clone(), hex.to_string());
        b.connect_clicked(move |_| {
            entry.remove_css_class("error");
            entry.set_text(&hex);
            select(&hex);
            cb(hex.clone());
        });
        bx.append(b);
    }
    {
        let (cb, select) = (on_change.clone(), select.clone());
        entry.connect_changed(move |e| {
            let text = e.text().to_string();
            let hex = if text.starts_with('#') { text } else { format!("#{text}") };
            if parse(&hex).is_some() {
                e.remove_css_class("error");
                select(&hex);
                cb(hex.to_lowercase());
            } else {
                e.add_css_class("error");
            }
        });
    }
    bx.append(&entry);
    bx
}

/// The highlight colour in Settings.
pub fn pref_row() -> gtk::Box {
    row(&crate::prefs::get().highlight_color, |hex| {
        crate::prefs::update(|p| p.highlight_color = hex.clone());
        if let Some(v) = super::view() {
            v.set_highlight_colour(&hex);
        }
    })
}
