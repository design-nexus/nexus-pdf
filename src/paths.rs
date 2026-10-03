//! Well-known locations. Every path honours the XDG overrides so the whole app
//! can be pointed at a scratch copy of `~/.config` for testing.

use std::path::PathBuf;

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home().join(fallback))
}

pub fn config_home() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config")
}

pub fn state_home() -> PathBuf {
    xdg("XDG_STATE_HOME", ".local/state")
}

/// `~/.cache/nexus-pdf`: covers, rendered icons and the working copies of open documents.
pub fn cache_dir() -> PathBuf {
    xdg("XDG_CACHE_HOME", ".cache").join("nexus-pdf")
}

/// `~/.local/share/nexus-pdf`: the recent-files database and saved signatures.
pub fn data_dir() -> PathBuf {
    xdg("XDG_DATA_HOME", ".local/share").join("nexus-pdf")
}

/// `~/.config/nexus-pdf`: everything the user can edit lives here.
pub fn app_dir() -> PathBuf {
    config_home().join("nexus-pdf")
}

pub fn prefs_file() -> PathBuf {
    app_dir().join("settings.toml")
}

pub fn custom_themes_dir() -> PathBuf {
    app_dir().join("themes")
}

pub fn library_db() -> PathBuf {
    data_dir().join("library.db")
}

pub fn signatures_dir() -> PathBuf {
    data_dir().join("signatures")
}

/// Page-one pictures for the library.
pub fn covers_dir() -> PathBuf {
    cache_dir().join("covers")
}

/// Working copies of open documents (one folder per open file).
pub fn work_dir() -> PathBuf {
    cache_dir().join("work")
}

pub fn documents_dir() -> PathBuf {
    let dirs = config_home().join("user-dirs.dirs");
    if let Ok(text) = std::fs::read_to_string(dirs) {
        for line in text.lines() {
            if let Some(v) = line.trim().strip_prefix("XDG_DOCUMENTS_DIR=") {
                let v = v.trim_matches('"').replace("$HOME", &home().to_string_lossy());
                if !v.is_empty() {
                    return PathBuf::from(v);
                }
            }
        }
    }
    home().join("Documents")
}

pub fn omarchy_theme_dir() -> PathBuf {
    state_home().join("omarchy/current/theme")
}

pub fn omarchy_colors() -> PathBuf {
    omarchy_theme_dir().join("colors.toml")
}

/// Replace `$HOME` with `~` for display.
pub fn pretty(path: &std::path::Path) -> String {
    let home = home();
    match path.strip_prefix(&home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}
