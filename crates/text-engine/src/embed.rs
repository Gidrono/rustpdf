//! Prepare subsetted font bytes + CID stream for PDF embedding.

use subsetter::GlyphRemapper;
use ttf_parser::Face;

use crate::fonts::{find_font_for_text, FontFaceData};
use doxo_document::TextStyle;

pub type GlyphId = u16;

#[derive(Debug, Clone)]
pub struct EmbeddedRun {
    /// Subsetted OpenType bytes (FontFile2).
    pub font_bytes: Vec<u8>,
    /// Original font family hint.
    pub family: String,
    /// Units per em.
    pub units_per_em: u16,
    /// Remapped glyph IDs in visual order for the run (Identity-H CIDs).
    pub cids: Vec<u16>,
    /// Advances in font units (for /W), parallel to cids.
    pub advances: Vec<u16>,
    /// ToUnicode: remapped gid → char (BMP).
    pub to_unicode: Vec<(u16, char)>,
    /// Total width in PDF points at `font_size`.
    pub width_pts: f32,
    pub font_size: f32,
    /// True when text needs RTL positioning (draw from right edge).
    pub rtl: bool,
}

/// Build an embeddable subset for `text` using the best available system font.
pub fn prepare_embedded_run(text: &str, style: &TextStyle) -> Option<EmbeddedRun> {
    if text.is_empty() {
        return None;
    }
    // Pure ASCII without needing subset — caller may still use Type1 Helvetica.
    // We always prefer subset when any non-latin1 or when System font requested.
    let needs_subset = text.chars().any(|c| !c.is_ascii())
        || style.font_family.eq_ignore_ascii_case("System")
        || std::env::var("DOXO_FORCE_SUBSET").is_ok();

    let face = find_font_for_text(text, &style.font_family)?;
    if !needs_subset && text.is_ascii() {
        // Still produce subset for consistency when a TTF is available —
        // enables correct metrics. Skip only if we have no TTF.
        let _ = &face;
    }

    prepare_from_face(&face, text, style.font_size)
}

pub fn prepare_from_face(face: &FontFaceData, text: &str, font_size: f32) -> Option<EmbeddedRun> {
    let parsed = Face::parse(&face.bytes, face.index).ok()?;
    let upem = parsed.units_per_em();

    let rb = rustybuzz::Face::from_slice(&face.bytes, face.index)?;
    let mut buf = rustybuzz::UnicodeBuffer::new();
    buf.push_str(text);
    let rtl = text
        .chars()
        .any(|c| matches!(c, '\u{0590}'..='\u{05FF}' | '\u{0600}'..='\u{06FF}'));
    if rtl {
        buf.set_direction(rustybuzz::Direction::RightToLeft);
    }
    let shaped = rustybuzz::shape(&rb, &[], buf);
    let infos = shaped.glyph_infos();
    let positions = shaped.glyph_positions();

    let mut remapper = GlyphRemapper::new();
    remapper.remap(0); // .notdef
    let mut pairs: Vec<(u16, char, i32)> = Vec::new();
    for (info, pos) in infos.iter().zip(positions.iter()) {
        let gid = info.glyph_id as u16;
        remapper.remap(gid);
        // cluster → approximate char
        let ch = text
            .chars()
            .nth(info.cluster as usize)
            .unwrap_or('\u{FFFD}');
        pairs.push((gid, ch, pos.x_advance));
    }

    let subset_bytes = subsetter::subset(&face.bytes, face.index, &remapper).ok()?;

    let mut cids = Vec::new();
    let mut advances = Vec::new();
    let mut to_unicode = Vec::new();
    let mut total_adv = 0i32;
    for (gid, ch, adv) in &pairs {
        let new_gid = remapper.get(*gid).unwrap_or(0);
        cids.push(new_gid);
        let a = (*adv).max(0) as u16;
        advances.push(a);
        total_adv += *adv;
        to_unicode.push((new_gid, *ch));
    }

    let scale = font_size / upem as f32;
    Some(EmbeddedRun {
        font_bytes: subset_bytes,
        family: face.family.clone(),
        units_per_em: upem,
        cids,
        advances,
        to_unicode,
        width_pts: (total_adv as f32 * scale).abs(),
        font_size,
        rtl,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use doxo_document::TextStyle;

    #[test]
    fn subsets_hebrew_when_font_present() {
        let style = TextStyle {
            font_family: "System".into(),
            font_size: 14.0,
            ..Default::default()
        };
        let text = "שלום";
        // Skip if no system font (CI without macOS fonts).
        if crate::fonts::resolve_font_path("System").is_none() {
            return;
        }
        let run = prepare_embedded_run(text, &style);
        assert!(run.is_some(), "expected subset for Hebrew");
        let run = run.unwrap();
        assert!(!run.cids.is_empty());
        assert!(!run.font_bytes.is_empty());
    }
}
