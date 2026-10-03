# Nexus PDF

A PDF viewer and editor for [Omarchy](https://omarchy.org). It opens a file quickly,
remembers the page you were on, and lets you mark it up, fill it in, sign it and
rearrange its pages. It takes its colours from your Omarchy theme and fits a
half-screen tile.

## What it does

- **Reading:** continuous or single-page scrolling, fit width, fit page or any zoom
  (<kbd>Ctrl</kbd>+wheel or pinch), and links that work. Select text and copy it.
  - **Side panel:** page thumbnails, the file's outline, and a list of your markup.
  - **Search** (<kbd>Ctrl</kbd>+<kbd>F</kbd>): every match is marked on the page, with
    <kbd>Enter</kbd> and <kbd>Shift</kbd>+<kbd>Enter</kbd> to step through them.
  - **Page colours:** invert the pages, or tint them with your theme, for reading at
    night.
  - **Library:** your recent files as cards with the first page and where you left off.
- **Markup:** highlight, underline and strike out text, draw freehand, add notes and
  text boxes. They're saved as ordinary PDF annotations, so other viewers show them.
  Click one to change its colour or text, move it, or delete it.
- **Forms:** fill in text fields, check boxes and drop-downs.
- **Signatures:** draw a signature, or use a picture of one, and place it anywhere. Saved
  signatures are kept for next time.
- **Edit text:** drag over words and type their replacement. The new text is written over
  the old words on a patch of the page's colour, in the closest standard font. The
  original text stays in the file underneath.
- **Pages:** reorder by dragging, rotate, delete, add the pages of another PDF, save some
  pages as a new file, or combine several PDFs into one.
- **Undo and redo** for every edit, and your file stays untouched until you save. A copy
  can be saved under another name.

Files that are encrypted can be read (it asks for the password) but not edited.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/design-nexus/nexus-pdf/main/install.sh | bash
```

This installs GTK 4 and poppler-glib if they're missing, builds with Cargo, and installs
`pdf` to `~/.local/bin` along with a launcher entry. Add `-s -- --default` after `bash`
to also make it the default app for PDF files.

To remove it, run the same line with `uninstall.sh` in place of `install.sh`. Add
`-s -- --purge` to also remove its settings, recent files list and saved signatures. Your
PDF files are never touched.

## Usage

```
pdf [OPTIONS] [FILE]
```

| Option | What |
| --- | --- |
| `FILE` | Open this PDF |
| `--page N` | Open `FILE` on page `N` (1 is the first) |
| `--section ID` | Open a page: `library`, `document`, `pages`, `settings` |
| `--toggle` | Close the window if it's open, otherwise open it |

Launching it again while it's running opens the file in the existing window.

| Key | What |
| --- | --- |
| <kbd>Ctrl</kbd>+<kbd>O</kbd> | Open a file |
| <kbd>Ctrl</kbd>+<kbd>S</kbd> / <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>S</kbd> | Save / save a copy as… |
| <kbd>Ctrl</kbd>+<kbd>Z</kbd> / <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>Z</kbd> | Undo / redo |
| <kbd>Ctrl</kbd>+<kbd>F</kbd> | Search the document |
| <kbd>Ctrl</kbd>+<kbd>C</kbd> | Copy the selected text |
| <kbd>Ctrl</kbd>+<kbd>+</kbd> / <kbd>Ctrl</kbd>+<kbd>−</kbd> / <kbd>Ctrl</kbd>+<kbd>0</kbd> | Zoom in / out / fit width |
| <kbd>PgUp</kbd> / <kbd>PgDn</kbd>, <kbd>Home</kbd> / <kbd>End</kbd> | Previous / next page, first / last page |
| <kbd>V</kbd> <kbd>H</kbd> <kbd>D</kbd> <kbd>N</kbd> <kbd>T</kbd> <kbd>E</kbd> | Select, highlight, draw, note, text box, edit text |
| <kbd>Delete</kbd> | Delete the selected annotation |
| <kbd>F9</kbd> / <kbd>F11</kbd> | Side panel / fullscreen |
| <kbd>Esc</kbd> | Close the search, clear the selection, or go back to Select |
| <kbd>Ctrl</kbd>+<kbd>Q</kbd> | Close |

## Files

| Path | What |
| --- | --- |
| `~/.config/nexus-pdf/settings.toml` | Settings |
| `~/.config/nexus-pdf/themes/*.toml` | Your own themes |
| `~/.local/share/nexus-pdf/library.db` | Recent files and the page each was left on |
| `~/.local/share/nexus-pdf/signatures/` | Saved signatures |
| `~/.cache/nexus-pdf/` | Page-one pictures and working copies of open files |

## Building

```sh
cargo build --release
```

Needs GTK 4 and poppler-glib (`pacman -S gtk4 poppler-glib`).
