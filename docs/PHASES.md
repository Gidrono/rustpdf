# doxo phases — V1 status

## Phase 1 — Viewer ✅
## Phase 2 — Editor canvas ✅ (snap guides included)
## Phase 3 — New text ✅ (Unicode subset embed on flatten)
## Phase 4 — Images ✅ (crop + rotate controls)
## Phase 5 — Ink / shapes ✅
## Phase 6 — Existing text ✅

- [x] Level-1 visual replacement (cover + redraw)
- [x] Level-2 content-stream literal rewrite when `(line) Tj` matches
- [x] Fallback to Level-1 when rewrite fails / non-ASCII streams
- [ ] Full operator graph reconstruct for TJ arrays / positioned glyphs (still imperfect)

## Phase 7 — Page editing ✅

- [x] Delete / rotate / duplicate / reorder / blank
- [x] **Extract** current page to a new PDF file

## Phase 8 — Reliability ✅

- [x] Atomic save, autosave journal, export flattened

## Remaining imperfections

- Level-2 only rewrites simple PDF string literals; compressed/encoded or TJ-array text often falls back to visual replace
- Subset embedding needs a system TTF (Arial Unicode recommended); CFF2 fonts unsupported by `subsetter`
- Image crop is nudge-based (not freehand marquee); rotate is 90° toolbar steps (arbitrary angle stored on object)
- Extract is single current page (not multi-select sidebar yet)
- Snap threshold fixed at 6pt; no column guides
