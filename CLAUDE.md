# Nexus PDF — notes for working on this repo

- The command is `pdf`; everything else (crate, config/data/cache folders) is `nexus-pdf`.
  App id `io.github.design_nexus.Pdf`.

- GTK4 (gtk4-rs 0.11) + Rust, no libadwaita. Follows `~/Projects/STYLE.md`; the shell
  (`theme.rs`, `prefs.rs`, `widgets.rs`, `window.rs`, `style.css`, installers) started as
  copies of Nexus Media Player (`~/Projects/nexus-media-player`). Every colour is a
  `@theme_*` token. The only fixed colours are page-content ones: form-field paper and
  ink, the signature pad, and the dark-blue signature pen.
- **poppler-rs** renders and reads (text, search, outline, links, form fields); **lopdf**
  writes. poppler-rs can't create highlights, ink, notes or stamps and hides the fields of
  link, outline and form-field mappings, so `doc/annots.rs` writes annotations (with their
  own appearance streams) and `doc/links.rs` reads the mappings through `poppler::ffi`
  (the only unsafe code).
- **Open files** (`doc/mod.rs`): `OPEN` holds every open `Doc` (one tab each) and
  `CURRENT` the one showing; `switch_to` makes another current and emits `Opened`.
  Each `Doc` watches its file (reloads it, or flags `changed_on_disk` when there are
  unsaved edits) and keeps its own Back/Forward pages. Its working folder has a `state`
  file (`gen saved` and the path) so `offer_recovery` can bring back unsaved edits
  after a crash.
- **Generations** (`doc/mod.rs`): the open file is copied to
  `~/.cache/nexus-pdf/work/<pid>-<n>/gen-0.pdf`. Every edit loads the current generation
  with lopdf, changes it, writes `gen-N+1.pdf` and reopens it in poppler. Undo and redo move
  between generations; Save copies the current one over the original atomically. Form
  edits go through poppler (`Doc::edit_forms`) instead, then the same.
- **Coordinates** (`doc/geom.rs`): everything the UI handles is in points from the
  top-left of the *displayed* page (rotation and crop box applied), which is what
  `selected_region` uses. `find_text`, link, form-field and annotation mappings are y-up in
  that same rotated frame and are flipped (`page_h - y`). `PageGeom` converts to and from
  PDF space, including `/Rotate` and the crop box. Annotations that draw text or pictures
  use an upright local frame plus a `/Matrix` so they stay upright on rotated pages.
- **Edit text** is a FreeText annotation (`NexusEdit`) with an opaque background sampled
  from the page, not a patch to the content stream, so it stays removable and re-editable.
- **Rendering** (`doc/render.rs`): one worker thread owns its own poppler document and
  renders the newest highest-priority request first (detail 2, pages 1, thumbnails 0);
  pixels come back over a channel and become `gdk::MemoryTexture`s. A whole page is at
  most 4096 px on its longest side; past that `PageView::ensure_detail` overlays a
  picture of just the visible part (`render::request_area`). Page colours (invert / theme tint)
  are applied to the pixels in the worker. `View::epoch` is bumped when the palette
  changes so stale pictures are dropped.
- `viewer/` is the document page: `mod.rs` (state, tabs, zoom, scroll, layouts and
  presenting, tool strip and menus), `pageview.rs`
  (one page and its marks), `tools.rs` (gestures → edits), `panel.rs`, `search.rs`,
  `forms.rs`, `sign.rs`. `sections/` has the other pages. Nothing holds a `with(...)`
  borrow while emitting; `doc::emit` clones its listeners first. Page positions in the
  scrolled area come from `View::bounds_of`, which measures against the page column:
  measuring against the viewport mixes in the scroll offset, depending on whether GTK
  has laid out since the last scroll.
- Checks: `cargo clippy --all-targets -- -D warnings`, `cargo test` (tests render real PDFs
  made in code: `src/testpdf.rs`). Layout check:
  `NPDF_SNAPSHOT=/tmp/x.png pdf FILE` (renders offscreen and quits; the compositor decides
  the size, so on a busy workspace the window can be a small tile; launch it floating
  with `hyprctl dispatch "hl.dsp.exec_cmd('[float; size 1280 860] CMD')"`, which runs
  without your shell's environment, so put the variables in a script). Hidden
  workspaces aren't drawn. `NPDF_SNAPSHOT_DELAY=ms` waits longer. `NPDF_SCRIPT="step; step"`
  drives the window first (see `viewer/script.rs`: `tool highlight; drag 2 97 105 253 137; …`;
  `status` and `windows` print state to stdout). Point
  `XDG_CONFIG_HOME`/`XDG_DATA_HOME`/`XDG_CACHE_HOME` at scratch dirs so tests don't touch
  real settings. `cargo test -- --ignored write_form_sample` writes a form PDF to
  `$NPDF_SAMPLE_DIR`.
