//! Every page in the sidebar, in order.

use crate::paths;
use crate::widgets::Page;
use std::path::PathBuf;

pub mod document;
pub mod library;
pub mod pages;
pub mod settings;

pub struct Section {
    pub id: &'static str,
    pub title: &'static str,
    pub icon: &'static str,
    pub group: &'static str,
    pub description: &'static str,
    /// Files the "Open config" button offers.
    pub files: fn() -> Vec<PathBuf>,
    pub build: fn(&Page),
    /// The page manages its own scrolling (the viewer, the page grid).
    pub fill: bool,
    /// No page header (the viewer has its own).
    pub bare: bool,
}

fn none() -> Vec<PathBuf> {
    Vec::new()
}

fn settings_files() -> Vec<PathBuf> {
    vec![paths::prefs_file()]
}

pub fn all() -> Vec<Section> {
    let s = |id, title, icon, group, description, build, fill, bare| Section {
        id,
        title,
        icon,
        group,
        description,
        files: none,
        build,
        fill,
        bare,
    };
    vec![
        s("library", "Library", "npdf-library-symbolic", "Files", "Your recent PDFs, with the page you were on.", library::build, true, false),
        s("document", "Document", "npdf-document-symbolic", "Open file", "Read, mark up, fill in and sign the open PDF.", document::build, true, true),
        s("pages", "Pages", "npdf-pages-symbolic", "Open file", "Reorder, rotate, delete, add and extract pages.", pages::build, true, false),
        Section {
            files: settings_files,
            ..s(
                "settings",
                "Settings",
                "emblem-system-symbolic",
                "App",
                "Viewing, editing and this window.",
                settings::build,
                false,
                false,
            )
        },
    ]
}
