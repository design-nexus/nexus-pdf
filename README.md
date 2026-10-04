# Nexus PDF

A PDF viewer and editor for [Omarchy](https://omarchy.org). It opens a file quickly,
remembers the page you were on, and lets you mark it up, fill it in, sign it and
rearrange its pages. It takes its colours from your Omarchy theme and fits a
half-screen tile.

## What it does

- **Reading:** continuous or one page at a time, single pages, two side by side, or
  like a book; fit width, fit page or any zoom (<kbd>Ctrl</kbd>+wheel or pinch, around
  the pointer), and links that work, with Back and Forward to return from them. Select
  text and copy it. Pages stay sharp at any zoom.
  - **Tabs:** open several files at once and switch with <kbd>Ctrl</kbd>+<kbd>Tab</kbd>.
    Drop PDFs on the window to open them.
  - **Side panel:** page thumbnails, the file's outline, and a list of your markup with
    the words it covers. Drag its edge to resize it; on a narrow window it floats over
    the pages.
  - **Search** (<kbd>Ctrl</kbd>+<kbd>F</kbd>): every match is marked on the page as it's
    found, with <kbd>Enter</kbd> and <kbd>Shift</kbd>+<kbd>Enter</kbd> to step through
    them. Match case and whole words are options, and notes and text boxes are searched
    too.
  - **Page colours:** invert the pages, or tint them with your theme, for reading at
    night.
  - **Present** (<kbd>F5</kbd>): full screen, one page at a time; click or press
    <kbd>→</kbd> to go on.
  - **Print** (<kbd>Ctrl</kbd>+<kbd>P</kbd>), markup included.
  - **Library:** your recent files as cards with the first page and where you left off,
    by date or by name.
- **Markup:** highlight, underline and strike out text, draw freehand, add notes and
  text boxes. They're saved as ordinary PDF annotations, so other viewers show them.
  Click one to change its colour or text, move it, resize it by its corner, or delete
  it. The colour button sets each tool's colour, highlight opacity, pen width and text
  size.
- **Forms:** fill in text fields (one line or several), check boxes and drop-downs, or
  clear the whole form.
- **Signatures:** draw a signature, or use a picture of one, and place it anywhere. Saved
  signatures are kept for next time.
- **Edit text:** drag over words and type their replacement. The new text is written over
  the old words on a patch of the page's colour, in the closest standard font. The
  original text stays in the file underneath.
- **Pages:** reorder by dragging, rotate, delete, add the pages of another PDF (or drop
  PDFs between the pages), save some pages as a new file, or combine several PDFs into
  one.
- **Undo and redo** for every edit, and your file stays untouched until you save. A copy
  can be saved under another name. If the app stops with changes unsaved, it offers them
  back next time. If another program changes the file, it's read again (or, with
  unsaved changes, you're asked).

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
pdf [OPTIONS] [FILE…]
```

| Option | What |
| --- | --- |
| `FILE…` | Open these PDFs, each in a tab |
| `--page N` | Open the last `FILE` on page `N` (1 is the first) |
| `--section ID` | Open a page: `library`, `document`, `pages`, `settings` |
| `--toggle` | Close the window if it's open, otherwise open it |

Launching it again while it's running opens the file in a new tab of the existing window.

| Key | What |
| --- | --- |
| <kbd>Ctrl</kbd>+<kbd>O</kbd> | Open files |
| <kbd>Ctrl</kbd>+<kbd>S</kbd> / <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>S</kbd> | Save / save a copy as… |
| <kbd>Ctrl</kbd>+<kbd>P</kbd> | Print |
| <kbd>Ctrl</kbd>+<kbd>W</kbd> | Close the file |
| <kbd>Ctrl</kbd>+<kbd>Tab</kbd> / <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>Tab</kbd> | Next / previous file |
| <kbd>Ctrl</kbd>+<kbd>Z</kbd> / <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>Z</kbd> | Undo / redo |
| <kbd>Ctrl</kbd>+<kbd>F</kbd> | Search the document |
| <kbd>Ctrl</kbd>+<kbd>C</kbd> | Copy the selected text |
| <kbd>Ctrl</kbd>+<kbd>+</kbd> / <kbd>Ctrl</kbd>+<kbd>−</kbd> | Zoom in / out |
| <kbd>Ctrl</kbd>+<kbd>0</kbd> / <kbd>Ctrl</kbd>+<kbd>9</kbd> | Fit width / fit page |
| <kbd>PgUp</kbd> / <kbd>PgDn</kbd>, <kbd>Home</kbd> / <kbd>End</kbd> | Previous / next page, first / last page |
| <kbd>Alt</kbd>+<kbd>←</kbd> / <kbd>Alt</kbd>+<kbd>→</kbd> | Back / forward (after following a link) |
| <kbd>V</kbd> <kbd>H</kbd> <kbd>U</kbd> <kbd>X</kbd> <kbd>D</kbd> <kbd>N</kbd> <kbd>T</kbd> <kbd>E</kbd> <kbd>S</kbd> | Select, highlight, underline, strike out, draw, note, text box, edit text, sign |
| <kbd>Delete</kbd> | Delete the selected annotation |
| <kbd>F5</kbd> | Present |
| <kbd>F9</kbd> / <kbd>F11</kbd> | Side panel / fullscreen |
| <kbd>Ctrl</kbd>+<kbd>B</kbd> | Collapse or expand the sidebar (it's collapsed while reading, by default) |
| <kbd>Esc</kbd> | Stop presenting, close the search, clear the selection, or go back to Select |
| <kbd>Ctrl</kbd>+<kbd>Q</kbd> | Quit |

## Files

| Path | What |
| --- | --- |
| `~/.config/nexus-pdf/settings.toml` | Settings |
| `~/.config/nexus-pdf/themes/*.toml` | Your own themes |
| `~/.local/share/nexus-pdf/library.db` | Recent files and the page each was left on |
| `~/.local/share/nexus-pdf/signatures/` | Saved signatures |
| `~/.cache/nexus-pdf/` | Page-one pictures and working copies of open files (and of files with unsaved changes when the app last stopped) |

## Building

```sh
cargo build --release
```

Needs GTK 4 and poppler-glib (`pacman -S gtk4 poppler-glib`).
