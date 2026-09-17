//! Editor scene — overlay objects separate from the PDF raster model.
//!
//! User Action → EditCommand → EditorScene → (on save) PDF Engine flatten.
//! Dragging overlays never invalidates page tiles.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::PageId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ObjectId(pub Uuid);

impl ObjectId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ObjectId {
    fn default() -> Self {
        Self::new()
    }
}

/// Axis-aligned box in PDF page space (origin bottom-left, y-up) — points.
/// UI converts to egui top-left when drawing.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RectPts {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl RectPts {
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.x + self.w && py >= self.y && py <= self.y + self.h
    }

    pub fn translated(self, dx: f32, dy: f32) -> Self {
        Self {
            x: self.x + dx,
            y: self.y + dy,
            ..self
        }
    }

    pub fn expanded(self, pad: f32) -> Self {
        Self {
            x: self.x - pad,
            y: self.y - pad,
            w: self.w + pad * 2.0,
            h: self.h + pad * 2.0,
        }
    }

    pub fn center(self) -> (f32, f32) {
        (self.x + self.w * 0.5, self.y + self.h * 0.5)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RgbaColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl RgbaColor {
    pub const BLACK: Self = Self {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };
    pub const YELLOW_HL: Self = Self {
        r: 255,
        g: 230,
        b: 80,
        a: 100,
    };
    pub const RED: Self = Self {
        r: 220,
        g: 50,
        b: 50,
        a: 255,
    };
    pub const BLUE: Self = Self {
        r: 40,
        g: 100,
        b: 220,
        a: 255,
    };

    pub fn to_egui(self) -> [u8; 4] {
        [self.r, self.g, self.b, self.a]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextStyle {
    pub font_family: String,
    pub font_size: f32,
    pub color: RgbaColor,
    pub bold: bool,
    pub italic: bool,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font_family: "Helvetica".into(),
            font_size: 14.0,
            color: RgbaColor::BLACK,
            bold: false,
            italic: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShapeKind {
    Rectangle,
    Ellipse,
    Line,
    Arrow,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EditorObject {
    /// Newly inserted / overlay text (Phase 3).
    TextBox {
        id: ObjectId,
        page: PageId,
        rect: RectPts,
        text: String,
        style: TextStyle,
        rotation_deg: f32,
    },
    /// Covers existing PDF text + draws replacement (Visual Replacement / Level 1 fallback).
    VisualTextPatch {
        id: ObjectId,
        page: PageId,
        cover: RectPts,
        text: String,
        style: TextStyle,
    },
    /// Level-2: attempt content-stream rewrite of `line_original` → reconstructed line.
    /// Falls back to visual cover + `replacement` draw when rewrite fails at flatten.
    NativeTextReplace {
        id: ObjectId,
        page: PageId,
        cover: RectPts,
        /// Full line (or run) text as extracted.
        line_original: String,
        /// Selected span within the line.
        selected: String,
        replacement: String,
        style: TextStyle,
    },
    Image {
        id: ObjectId,
        page: PageId,
        rect: RectPts,
        /// PNG/JPEG bytes (editor-owned until flatten).
        bytes: Vec<u8>,
        format: ImageFormat,
        rotation_deg: f32,
        opacity: f32,
        /// Crop in normalized 0..1 source space.
        crop: Option<[f32; 4]>,
    },
    InkStroke {
        id: ObjectId,
        page: PageId,
        points: Vec<[f32; 2]>,
        color: RgbaColor,
        width: f32,
    },
    Highlight {
        id: ObjectId,
        page: PageId,
        /// One or more quads (PDF space).
        quads: Vec<RectPts>,
        color: RgbaColor,
        /// True when tied to extracted text ranges.
        semantic: bool,
    },
    Shape {
        id: ObjectId,
        page: PageId,
        kind: ShapeKind,
        rect: RectPts,
        stroke: RgbaColor,
        fill: Option<RgbaColor>,
        stroke_width: f32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageFormat {
    Png,
    Jpeg,
}

impl EditorObject {
    pub fn id(&self) -> ObjectId {
        match self {
            Self::TextBox { id, .. }
            | Self::VisualTextPatch { id, .. }
            | Self::NativeTextReplace { id, .. }
            | Self::Image { id, .. }
            | Self::InkStroke { id, .. }
            | Self::Highlight { id, .. }
            | Self::Shape { id, .. } => *id,
        }
    }

    pub fn page(&self) -> PageId {
        match self {
            Self::TextBox { page, .. }
            | Self::VisualTextPatch { page, .. }
            | Self::NativeTextReplace { page, .. }
            | Self::Image { page, .. }
            | Self::InkStroke { page, .. }
            | Self::Highlight { page, .. }
            | Self::Shape { page, .. } => *page,
        }
    }

    pub fn set_page(&mut self, page: PageId) {
        match self {
            Self::TextBox { page: p, .. }
            | Self::VisualTextPatch { page: p, .. }
            | Self::NativeTextReplace { page: p, .. }
            | Self::Image { page: p, .. }
            | Self::InkStroke { page: p, .. }
            | Self::Highlight { page: p, .. }
            | Self::Shape { page: p, .. } => *p = page,
        }
    }

    pub fn bounds(&self) -> RectPts {
        match self {
            Self::TextBox { rect, .. }
            | Self::VisualTextPatch { cover: rect, .. }
            | Self::NativeTextReplace { cover: rect, .. }
            | Self::Image { rect, .. }
            | Self::Shape { rect, .. } => *rect,
            Self::Highlight { quads, .. } => {
                if quads.is_empty() {
                    return RectPts {
                        x: 0.0,
                        y: 0.0,
                        w: 0.0,
                        h: 0.0,
                    };
                }
                let mut min_x = f32::MAX;
                let mut min_y = f32::MAX;
                let mut max_x = f32::MIN;
                let mut max_y = f32::MIN;
                for q in quads {
                    min_x = min_x.min(q.x);
                    min_y = min_y.min(q.y);
                    max_x = max_x.max(q.x + q.w);
                    max_y = max_y.max(q.y + q.h);
                }
                RectPts {
                    x: min_x,
                    y: min_y,
                    w: max_x - min_x,
                    h: max_y - min_y,
                }
            }
            Self::InkStroke { points, width, .. } => {
                if points.is_empty() {
                    return RectPts {
                        x: 0.0,
                        y: 0.0,
                        w: 0.0,
                        h: 0.0,
                    };
                }
                let mut min_x = f32::MAX;
                let mut min_y = f32::MAX;
                let mut max_x = f32::MIN;
                let mut max_y = f32::MIN;
                for p in points {
                    min_x = min_x.min(p[0]);
                    min_y = min_y.min(p[1]);
                    max_x = max_x.max(p[0]);
                    max_y = max_y.max(p[1]);
                }
                let pad = *width;
                RectPts {
                    x: min_x - pad,
                    y: min_y - pad,
                    w: max_x - min_x + pad * 2.0,
                    h: max_y - min_y + pad * 2.0,
                }
            }
        }
    }

    pub fn hit_test(&self, page: PageId, x: f32, y: f32) -> bool {
        if self.page() != page {
            return false;
        }
        match self {
            Self::InkStroke { points, width, .. } => {
                let thr = (*width * 0.5 + 4.0).max(6.0);
                points.windows(2).any(|w| dist_to_segment(x, y, w[0], w[1]) <= thr)
                    || points.first().is_some_and(|p| {
                        let dx = p[0] - x;
                        let dy = p[1] - y;
                        (dx * dx + dy * dy).sqrt() <= thr
                    })
            }
            Self::Highlight { quads, .. } => quads.iter().any(|q| q.contains(x, y)),
            _ => self.bounds().expanded(2.0).contains(x, y),
        }
    }

    pub fn translate(&mut self, dx: f32, dy: f32) {
        match self {
            Self::TextBox { rect, .. }
            | Self::VisualTextPatch { cover: rect, .. }
            | Self::NativeTextReplace { cover: rect, .. }
            | Self::Image { rect, .. }
            | Self::Shape { rect, .. } => *rect = rect.translated(dx, dy),
            Self::Highlight { quads, .. } => {
                for q in quads {
                    *q = q.translated(dx, dy);
                }
            }
            Self::InkStroke { points, .. } => {
                for p in points {
                    p[0] += dx;
                    p[1] += dy;
                }
            }
        }
    }

    pub fn set_rect(&mut self, new_rect: RectPts) {
        match self {
            Self::TextBox { rect, .. }
            | Self::Image { rect, .. }
            | Self::Shape { rect, .. } => *rect = new_rect,
            Self::VisualTextPatch { cover, .. } | Self::NativeTextReplace { cover, .. } => {
                *cover = new_rect
            }
            _ => {}
        }
    }
}

fn dist_to_segment(px: f32, py: f32, a: [f32; 2], b: [f32; 2]) -> f32 {
    let (ax, ay) = (a[0], a[1]);
    let (bx, by) = (b[0], b[1]);
    let abx = bx - ax;
    let aby = by - ay;
    let len2 = abx * abx + aby * aby;
    if len2 < 1e-6 {
        let dx = px - ax;
        let dy = py - ay;
        return (dx * dx + dy * dy).sqrt();
    }
    let t = ((px - ax) * abx + (py - ay) * aby) / len2;
    let t = t.clamp(0.0, 1.0);
    let qx = ax + t * abx;
    let qy = ay + t * aby;
    let dx = px - qx;
    let dy = py - qy;
    (dx * dx + dy * dy).sqrt()
}

/// Full editor model for one open document.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EditorScene {
    pub objects: Vec<EditorObject>,
    pub dirty: bool,
}

impl EditorScene {
    pub fn objects_on_page(&self, page: PageId) -> impl Iterator<Item = &EditorObject> {
        self.objects.iter().filter(move |o| o.page() == page)
    }

    pub fn find(&self, id: ObjectId) -> Option<&EditorObject> {
        self.objects.iter().find(|o| o.id() == id)
    }

    pub fn find_mut(&mut self, id: ObjectId) -> Option<&mut EditorObject> {
        self.objects.iter_mut().find(|o| o.id() == id)
    }

    pub fn hit_test_top(&self, page: PageId, x: f32, y: f32) -> Option<ObjectId> {
        self.objects
            .iter()
            .rev()
            .find(|o| o.hit_test(page, x, y))
            .map(|o| o.id())
    }

    /// Select-tool hit test: skip highlights so they don't block PDF text underneath.
    pub fn hit_test_top_for_select(&self, page: PageId, x: f32, y: f32) -> Option<ObjectId> {
        self.objects
            .iter()
            .rev()
            .find(|o| {
                !matches!(o, EditorObject::Highlight { .. }) && o.hit_test(page, x, y)
            })
            .map(|o| o.id())
    }

    /// Highlight-only hit (used when no PDF char is near the pointer).
    pub fn hit_test_highlight(&self, page: PageId, x: f32, y: f32) -> Option<ObjectId> {
        self.objects
            .iter()
            .rev()
            .find(|o| matches!(o, EditorObject::Highlight { .. }) && o.hit_test(page, x, y))
            .map(|o| o.id())
    }

    pub fn insert(&mut self, obj: EditorObject) {
        self.objects.push(obj);
        self.dirty = true;
    }

    pub fn remove(&mut self, id: ObjectId) -> Option<EditorObject> {
        if let Some(pos) = self.objects.iter().position(|o| o.id() == id) {
            self.dirty = true;
            Some(self.objects.remove(pos))
        } else {
            None
        }
    }

    pub fn clear(&mut self) {
        self.objects.clear();
        self.dirty = false;
    }
}

/// Extracted PDF text (from PDFium) — not part of overlay scene.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedChar {
    pub ch: char,
    pub rect: RectPts,
    pub font_size: f32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtractedPageText {
    pub page: PageId,
    pub chars: Vec<ExtractedChar>,
    pub plain: String,
}

impl ExtractedPageText {
    pub fn char_index_at(&self, x: f32, y: f32) -> Option<usize> {
        self.chars
            .iter()
            .enumerate()
            .find(|(_, c)| c.rect.expanded(1.0).contains(x, y))
            .map(|(i, _)| i)
    }

    /// Hit-test with a generous nearest-glyph fallback (gaps between PDFium boxes).
    pub fn nearest_char_index(&self, x: f32, y: f32) -> Option<usize> {
        if let Some(i) = self.char_index_at(x, y) {
            return Some(i);
        }
        let mut best: Option<(usize, f32)> = None;
        for (i, c) in self.chars.iter().enumerate() {
            if c.ch.is_whitespace() {
                continue;
            }
            let cx = c.rect.x + c.rect.w * 0.5;
            let cy = c.rect.y + c.rect.h * 0.5;
            let dx = x - cx;
            let dy = y - cy;
            // Prefer horizontal neighbors on the same baseline.
            let dist = (dx * dx + (dy * 2.0) * (dy * 2.0)).sqrt();
            let thr = c.rect.h.max(c.rect.w).max(6.0) * 1.75;
            if dist <= thr && best.map_or(true, |(_, d)| dist < d) {
                best = Some((i, dist));
            }
        }
        best.map(|(i, _)| i)
    }

    /// Expand `index` to a word span `[start, end)` (non-whitespace / non-punct run).
    pub fn word_span_at(&self, index: usize) -> Option<(usize, usize)> {
        if index >= self.chars.len() {
            return None;
        }
        if !is_word_char(self.chars[index].ch) {
            return Some((index, index + 1));
        }
        let mut start = index;
        while start > 0 && is_word_char(self.chars[start - 1].ch) {
            start -= 1;
        }
        let mut end = index + 1;
        while end < self.chars.len() && is_word_char(self.chars[end].ch) {
            end += 1;
        }
        Some((start, end))
    }

    pub fn range_bounds(&self, start: usize, end: usize) -> Vec<RectPts> {
        if start >= end || end > self.chars.len() {
            return Vec::new();
        }
        // Group into line quads by similar baseline.
        let mut quads = Vec::new();
        let slice = &self.chars[start..end];
        let mut line_start = 0;
        while line_start < slice.len() {
            let base_y = slice[line_start].rect.y;
            let mut line_end = line_start + 1;
            while line_end < slice.len() {
                if (slice[line_end].rect.y - base_y).abs() > slice[line_start].rect.h * 0.5 {
                    break;
                }
                line_end += 1;
            }
            let line = &slice[line_start..line_end];
            let min_x = line.iter().map(|c| c.rect.x).fold(f32::MAX, f32::min);
            let max_x = line
                .iter()
                .map(|c| c.rect.x + c.rect.w)
                .fold(f32::MIN, f32::max);
            let min_y = line.iter().map(|c| c.rect.y).fold(f32::MAX, f32::min);
            let max_y = line
                .iter()
                .map(|c| c.rect.y + c.rect.h)
                .fold(f32::MIN, f32::max);
            quads.push(RectPts {
                x: min_x,
                y: min_y,
                w: max_x - min_x,
                h: max_y - min_y,
            });
            line_start = line_end;
        }
        quads
    }

    pub fn text_range(&self, start: usize, end: usize) -> String {
        self.chars[start.min(self.chars.len())..end.min(self.chars.len())]
            .iter()
            .map(|c| c.ch)
            .collect()
    }

    /// Expand a selection to the contiguous line (same baseline) and return
    /// `(line_start, line_end, line_text)`.
    pub fn line_span_containing(&self, start: usize, end: usize) -> Option<(usize, usize, String)> {
        if self.chars.is_empty() || start >= self.chars.len() {
            return None;
        }
        let end = end.min(self.chars.len()).max(start + 1);
        let base_y = self.chars[start].rect.y;
        let thr = self.chars[start].rect.h.max(4.0) * 0.6;
        let mut line_start = start;
        while line_start > 0 {
            let prev = &self.chars[line_start - 1];
            if (prev.rect.y - base_y).abs() > thr || prev.ch == '\n' {
                break;
            }
            line_start -= 1;
        }
        let mut line_end = end;
        while line_end < self.chars.len() {
            let next = &self.chars[line_end];
            if (next.rect.y - base_y).abs() > thr || next.ch == '\n' {
                break;
            }
            line_end += 1;
        }
        let line_text: String = self.chars[line_start..line_end].iter().map(|c| c.ch).collect();
        Some((line_start, line_end, line_text))
    }
}

fn is_word_char(c: char) -> bool {
    !c.is_whitespace()
        && !matches!(
            c,
            '.' | ','
                | ';'
                | ':'
                | '!'
                | '?'
                | '"'
                | '\''
                | '('
                | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '«'
                | '»'
                | '—'
                | '–'
                | '/'
                | '\\'
                | '|'
        )
}

#[cfg(test)]
mod extracted_text_tests {
    use super::*;

    fn sample_text() -> ExtractedPageText {
        let chars: Vec<ExtractedChar> = "Hello, world"
            .chars()
            .enumerate()
            .map(|(i, ch)| ExtractedChar {
                ch,
                rect: RectPts {
                    x: i as f32 * 10.0,
                    y: 100.0,
                    w: 9.0,
                    h: 12.0,
                },
                font_size: 12.0,
            })
            .collect();
        let plain: String = chars.iter().map(|c| c.ch).collect();
        ExtractedPageText {
            page: PageId(0),
            chars,
            plain,
        }
    }

    #[test]
    fn word_span_at_selects_word() {
        let t = sample_text();
        assert_eq!(t.word_span_at(1), Some((0, 5))); // e in Hello
        assert_eq!(t.word_span_at(5), Some((5, 6))); // comma
        assert_eq!(t.word_span_at(8), Some((7, 12))); // o in world
    }

    #[test]
    fn nearest_char_fills_gaps() {
        let t = sample_text();
        // Between char 0 and 1 centers
        let idx = t.nearest_char_index(5.0, 106.0);
        assert!(idx.is_some());
    }
}
