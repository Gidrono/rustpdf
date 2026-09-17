//! Text engine — shaping, font discovery, and PDF subset preparation.

mod embed;
mod fonts;

pub use embed::{prepare_embedded_run, EmbeddedRun, GlyphId};
pub use fonts::{
    find_font_for_text, load_font_bytes, measure_text, resolve_font_path, standard_font_families,
    FontFaceData,
};

use doxo_document::{PageId, TextStyle};

/// Shaped run ready for overlay paint / PDF emit.
#[derive(Debug, Clone)]
pub struct ShapedRun {
    pub page: PageId,
    pub text: String,
    pub style: TextStyle,
    pub width_pts: f32,
    pub height_pts: f32,
}

/// Shape text for insertion.
pub fn shape_text(page: PageId, text: &str, style: TextStyle) -> ShapedRun {
    let (width_pts, height_pts) = measure_text(text, &style);
    ShapedRun {
        page,
        text: text.to_string(),
        style,
        width_pts,
        height_pts,
    }
}

pub fn hit_test_stub(_page: PageId, _x: f32, _y: f32) -> Option<()> {
    None
}
