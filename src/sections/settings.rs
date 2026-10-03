use crate::widgets::{self, Page};
use crate::{cmd, fmt, paths, prefs, theme};
use gtk::glib;
use gtk::prelude::*;

fn dir_size(dir: &std::path::Path) -> u64 {
    let mut total = 0;
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    for e in rd.flatten() {
        let Ok(m) = e.metadata() else { continue };
        total += if m.is_dir() { dir_size(&e.path()) } else { m.len() };
    }
    total
}

pub fn build(page: &Page) {
    let p = prefs::get();

    // ----- Viewing -----
    let g = page.group("Viewing");
    g.add(&widgets::segmented_row(
        "Zoom when a file opens",
        "Fit width keeps the text a comfortable size; fit page shows a whole page.",
        widgets::opts(&[("fit-width", "Fit width"), ("fit-page", "Fit page"), ("100", "100%")]),
        &p.default_zoom,
        |v| prefs::update(|p| p.default_zoom = v),
    ));
    g.add(&widgets::segmented_row(
        "Page colours",
        "Recolour pages to the theme for reading at night. Pictures are recoloured too.",
        widgets::opts(&[("off", "Off"), ("invert", "Invert"), ("tint", "Theme tint")]),
        &p.page_colors,
        |v| {
            prefs::update(|p| p.page_colors = v);
            theme::apply();
        },
    ));
    let (r, _) = widgets::switch_row("Continuous scrolling", "Pages run one after another. Off shows one page at a time.", p.continuous, |on| {
        prefs::update(|p| p.continuous = on);
        crate::viewer::relayout();
    });
    g.add(&r);
    let (r, _) = widgets::switch_row("Remember the page", "Open each file on the page you left it on.", p.remember_page, |on| {
        prefs::update(|p| p.remember_page = on)
    });
    g.add(&r);

    // ----- Editing -----
    let g = page.group("Editing");
    let author = gtk::Entry::new();
    author.set_text(&p.author);
    author.set_placeholder_text(Some("Your name"));
    author.set_width_chars(22);
    author.connect_changed(|e| {
        let text = e.text().to_string();
        prefs::update(|p| p.author = text);
    });
    g.add(&widgets::row("Author name", "Stored on the notes and markup you add.", Some(author.upcast_ref())));
    g.add(&widgets::row("Highlight colour", "The colour new highlights start with.", Some(crate::viewer::colour::pref_row().upcast_ref())));
    let sigs = paths::signatures_dir();
    let n = std::fs::read_dir(&sigs).map(|d| d.flatten().count()).unwrap_or(0);
    let desc = format!("{} saved. They live in <tt>{}</tt>.", fmt::count(n, "signature", "signatures"), glib::markup_escape_text(&paths::pretty(&sigs)));
    let (r, _) = widgets::button_row("Signatures", &desc, "Open folder", move |_| {
        let _ = std::fs::create_dir_all(&sigs);
        cmd::spawn(&["xdg-open", &sigs.to_string_lossy()]);
    });
    g.add(&r);

    // ----- Files -----
    let g = page.group("Files");
    let cache = paths::cache_dir();
    let size = dir_size(&cache);
    let (r, _) = widgets::button_row(
        "Clear cache",
        &format!("Covers and working copies of open files, {} now.", fmt::size(size)),
        "Clear",
        |_| {
            crate::sections::library::clear_covers();
            crate::window::toast("Cleared the cover cache.");
        },
    );
    g.add(&r);

    // ----- This window -----
    let g = page.group("This window");
    let app_themes = theme::all();
    let options: Vec<(String, String)> = app_themes.iter().map(|t| (t.id.clone(), t.name.clone())).collect();
    let (theme_row, theme_dd) = widgets::choice_row(
        "Theme",
        "Dracula, Catppuccin, Tokyo Night, One Dark Pro and more. Add your own in <tt>~/.config/nexus-pdf/themes</tt>.",
        options,
        &p.theme,
        |id| {
            prefs::update(|p| {
                p.theme = id;
                p.mode = prefs::ThemeMode::Theme;
            });
            theme::apply();
        },
    );
    theme_dd.set_sensitive(p.mode == prefs::ThemeMode::Theme || !theme::omarchy_available());

    if theme::omarchy_available() {
        let dd = theme_dd.clone();
        let (r, _) = widgets::switch_row(
            "Follow Omarchy theme",
            "Match the desktop's colours and update live whenever the Omarchy theme changes.",
            p.mode == prefs::ThemeMode::Omarchy,
            move |on| {
                prefs::update(|p| p.mode = if on { prefs::ThemeMode::Omarchy } else { prefs::ThemeMode::Theme });
                dd.set_sensitive(!on);
                theme::apply();
            },
        );
        g.add(&r);
    }
    g.add(&theme_row);

    let swatches = widgets::hbox(4);
    let refresh_swatches = {
        let swatches = swatches.clone();
        move || {
            while let Some(c) = swatches.first_child() {
                swatches.remove(&c);
            }
            let pal = theme::current_palette();
            for c in [&pal.bg, &pal.surface, &pal.muted, &pal.text, &pal.accent, &pal.danger] {
                let s = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                s.add_css_class("swatch");
                let provider = gtk::CssProvider::new();
                provider.load_from_string(&format!("box {{ background: {c}; }}"));
                #[allow(deprecated)]
                s.style_context().add_provider(&provider, gtk::STYLE_PROVIDER_PRIORITY_USER);
                swatches.append(&s);
            }
        }
    };
    refresh_swatches();
    let last = std::cell::RefCell::new(theme::current_palette());
    let weak = swatches.downgrade();
    glib::timeout_add_seconds_local(1, move || {
        if weak.upgrade().is_none() {
            return glib::ControlFlow::Break;
        }
        let now = theme::current_palette();
        if *last.borrow() != now {
            *last.borrow_mut() = now;
            refresh_swatches();
        }
        glib::ControlFlow::Continue
    });
    g.add(&widgets::row("Current colours", "", Some(swatches.upcast_ref())));

    let (r, _) = widgets::switch_row("Glow", "Soft accent glow around focused and selected elements.", p.glow, |on| {
        prefs::update(|p| p.glow = on);
        theme::apply();
    });
    g.add(&r);
    let (r, _) =
        widgets::switch_row("Reduce motion", "Turn off transitions and animations in this window.", p.reduce_motion, |on| {
            prefs::update(|p| p.reduce_motion = on);
            theme::apply();
        });
    g.add(&r);

    // ----- Keyboard -----
    let g = page.group("Keyboard");
    for (keys, what) in [
        (&["Ctrl", "O"][..], "Open a file"),
        (&["Ctrl", "S"][..], "Save"),
        (&["Ctrl", "Shift", "S"][..], "Save a copy as…"),
        (&["Ctrl", "Z"][..], "Undo"),
        (&["Ctrl", "Shift", "Z"][..], "Redo"),
        (&["Ctrl", "F"][..], "Search the document"),
        (&["Ctrl", "+"][..], "Zoom in"),
        (&["Ctrl", "−"][..], "Zoom out"),
        (&["Ctrl", "0"][..], "Fit width"),
        (&["PgUp"][..], "Previous page"),
        (&["PgDn"][..], "Next page"),
        (&["F9"][..], "Show or hide the side panel"),
        (&["F11"][..], "Fullscreen"),
        (&["V"][..], "Select tool"),
        (&["H"][..], "Highlight tool"),
        (&["D"][..], "Draw tool"),
        (&["N"][..], "Note tool"),
        (&["T"][..], "Text box tool"),
        (&["E"][..], "Edit text tool"),
        (&["Esc"][..], "Back to Select, or clear the search"),
        (&["Ctrl", "Q"][..], "Close"),
    ] {
        g.add(&widgets::row(what, "", Some(widgets::key_caps(keys).upcast_ref())));
    }
    g.note("Open a file from a terminal or a binding with <tt>pdf FILE</tt>, and jump to a page with <tt>pdf FILE --page 12</tt>.");
}
