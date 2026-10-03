//! Recent files and the page each was left on (`~/.local/share/nexus-pdf/library.db`).

use crate::paths;
use rusqlite::{Connection, params};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Recent {
    pub path: PathBuf,
    pub pages: usize,
    /// Zero-based page last viewed.
    pub page: usize,
}

fn open() -> rusqlite::Result<Connection> {
    let file = paths::library_db();
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let c = Connection::open(file)?;
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS recent (
            path TEXT PRIMARY KEY,
            pages INTEGER NOT NULL DEFAULT 0,
            page INTEGER NOT NULL DEFAULT 0,
            opened INTEGER NOT NULL DEFAULT 0
        );",
    )?;
    Ok(c)
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn key(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Note that a file was opened (or saved under a new name).
pub fn record(path: &Path, pages: usize, page: usize) {
    if let Ok(c) = open() {
        let _ = c.execute(
            "INSERT INTO recent (path, pages, page, opened) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(path) DO UPDATE SET pages = ?2, page = ?3, opened = ?4",
            params![key(path), pages as i64, page as i64, now()],
        );
    }
}

pub fn set_page(path: &Path, page: usize) {
    if let Ok(c) = open() {
        let _ = c.execute("UPDATE recent SET page = ?2 WHERE path = ?1", params![key(path), page as i64]);
    }
}

pub fn last_page(path: &Path) -> usize {
    open()
        .and_then(|c| c.query_row("SELECT page FROM recent WHERE path = ?1", params![key(path)], |r| r.get::<_, i64>(0)))
        .map(|p| p.max(0) as usize)
        .unwrap_or(0)
}

pub fn forget(path: &Path) {
    if let Ok(c) = open() {
        let _ = c.execute("DELETE FROM recent WHERE path = ?1", params![key(path)]);
    }
}

/// Newest first.
pub fn list() -> Vec<Recent> {
    let Ok(c) = open() else { return Vec::new() };
    let Ok(mut stmt) = c.prepare("SELECT path, pages, page FROM recent ORDER BY opened DESC LIMIT 200") else {
        return Vec::new();
    };
    stmt.query_map([], |r| {
        Ok(Recent {
            path: PathBuf::from(r.get::<_, String>(0)?),
            pages: r.get::<_, i64>(1)?.max(0) as usize,
            page: r.get::<_, i64>(2)?.max(0) as usize,
        })
    })
    .map(|rows| rows.flatten().collect())
    .unwrap_or_default()
}
