# doxo

Fast, lightweight macOS PDF viewer/editor (Rust). **V1 essentials** plus Level-2 text rewrite, Unicode font subset embedding, image crop/rotate, snap guides, and page extract.

## Features

| Area | Status |
|------|--------|
| Open / Save / Save As / Export Flattened | ✅ atomic write |
| Viewer, zoom, scroll, thumbnails | ✅ |
| Select, move/resize, **snap-to-guides** | ✅ |
| Undo / redo | ✅ |
| Add text + fonts/sizes/colors | ✅ |
| Existing text select + **Level-2 rewrite** (visual fallback) | ✅ |
| **Unicode subset embed** (Type0/CID Identity-H) | ✅ when system TTF found |
| Images: paste, move/resize, **crop, rotate** | ✅ |
| Ink / highlight / shapes / arrows | ✅ |
| Copy/paste | ✅ |
| Search (⌘F) | ✅ |
| Page delete / rotate / duplicate / reorder / blank / **Extract** | ✅ |
| Crash recovery autosave | ✅ |

## Run

```bash
./scripts/fetch-pdfium.sh
cargo run -p doxo-app -- fixtures/large-sample.pdf
```

Optional:

```bash
export DOXO_FONT_PATH="/System/Library/Fonts/Supplemental/Arial Unicode.ttf"
export DOXO_FORCE_SUBSET=1   # force subset even for ASCII
```

## Shortcuts / tools

| Action | |
|--------|--|
| Open / Save / Save As | ⌘O / ⌘S / ⌘⇧S |
| Find | ⌘F |
| Undo / Redo | ⌘Z / ⌘⇧Z |
| Tools | V select · T text · P pencil · H highlight |
| Image | toolbar ⟲/⟳ rotate · Crop mode nudges L/R/T/B |
| Pages | Extract… writes current page to a new PDF |

## Architecture

**User Action → EditCommand → EditorScene → PdfWorker flatten/page-ops.**  
Overlays never re-rasterize PDF while dragging. PDFium for render/extract; lopdf for surgery + flatten; `subsetter` + rustybuzz for Unicode fonts.

See [docs/PHASES.md](docs/PHASES.md).

## License

MIT — see [LICENSE](LICENSE).
