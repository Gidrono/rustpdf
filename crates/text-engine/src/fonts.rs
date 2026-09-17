//! Font discovery and measurement.

use std::path::{Path, PathBuf};

use doxo_document::TextStyle;

#[derive(Debug, Clone)]
pub struct FontFaceData {
    pub bytes: Vec<u8>,
    pub index: u32,
    pub path: PathBuf,
    pub family: String,
}

/// Built-in PDF standard font names for the style picker (ASCII path).
pub fn standard_font_families() -> &'static [&'static str] {
    &[
        "Helvetica",
        "Helvetica-Bold",
        "Times-Roman",
        "Times-Bold",
        "Courier",
        "Courier-Bold",
        "System", // maps to best available Unicode TTF
    ]
}

pub fn resolve_font_path(family: &str) -> Option<PathBuf> {
    if let Ok(p) = std::env::var("DOXO_FONT_PATH") {
        let pb = PathBuf::from(&p);
        if pb.exists() {
            return Some(pb);
        }
    }
    let candidates = system_font_candidates(family);
    candidates.into_iter().find(|p| p.exists())
}

fn system_font_candidates(family: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let lower = family.to_lowercase();
    // Prefer fonts with broad Unicode coverage for Hebrew/Arabic/CJK-adjacent Latin.
    let preferred = if lower.contains("bold") {
        vec![
            "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
            "/Library/Fonts/Arial Unicode.ttf",
            "/System/Library/Fonts/Supplemental/Arial.ttf",
            "/Library/Fonts/Arial.ttf",
            "/System/Library/Fonts/Helvetica.ttc",
        ]
    } else if lower == "system" || lower == "helvetica" || lower.is_empty() {
        vec![
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
            "/Library/Fonts/Arial Unicode.ttf",
            "/System/Library/Fonts/Supplemental/Arial.ttf",
            "/Library/Fonts/Arial.ttf",
            "/System/Library/Fonts/SFNS.ttf",
            "/System/Library/Fonts/Helvetica.ttc",
            "/System/Library/Fonts/Supplemental/Times New Roman.ttf",
        ]
    } else if lower.contains("times") {
        vec![
            "/System/Library/Fonts/Supplemental/Times New Roman.ttf",
            "/Library/Fonts/Times New Roman.ttf",
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        ]
    } else if lower.contains("courier") {
        vec![
            "/System/Library/Fonts/Supplemental/Courier New.ttf",
            "/Library/Fonts/Courier New.ttf",
            "/System/Library/Fonts/Supplemental/Arial.ttf",
        ]
    } else {
        vec![
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
            "/System/Library/Fonts/Supplemental/Arial.ttf",
            "/Library/Fonts/Arial.ttf",
        ]
    };
    for p in preferred {
        out.push(PathBuf::from(p));
    }
    out
}

pub fn load_font_bytes(family: &str) -> Option<FontFaceData> {
    let path = resolve_font_path(family)?;
    let bytes = std::fs::read(&path).ok()?;
    // For .ttc, try face 0; callers can retry.
    Some(FontFaceData {
        bytes,
        index: 0,
        path,
        family: family.to_string(),
    })
}

/// Pick a font that can cover the majority of codepoints in `text`.
pub fn find_font_for_text(text: &str, preferred_family: &str) -> Option<FontFaceData> {
    let mut candidates = Vec::new();
    if let Some(f) = load_font_bytes(preferred_family) {
        candidates.push(f);
    }
    if preferred_family != "System" {
        if let Some(f) = load_font_bytes("System") {
            candidates.push(f);
        }
    }
    // Last-resort paths
    for p in [
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        "/System/Library/Fonts/Supplemental/Arial.ttf",
        "/Library/Fonts/Arial.ttf",
    ] {
        let path = Path::new(p);
        if path.exists() {
            if let Ok(bytes) = std::fs::read(path) {
                candidates.push(FontFaceData {
                    bytes,
                    index: 0,
                    path: path.to_path_buf(),
                    family: "fallback".into(),
                });
            }
        }
    }

    let needed: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
    if needed.is_empty() {
        return candidates.into_iter().next();
    }

    let mut best: Option<(usize, FontFaceData)> = None;
    for face in candidates {
        let covered = count_covered(&face, &needed);
        if best.as_ref().map(|(c, _)| covered > *c).unwrap_or(true) {
            best = Some((covered, face));
            if covered == needed.len() {
                break;
            }
        }
    }
    best.map(|(_, f)| f)
}

fn count_covered(face: &FontFaceData, chars: &[char]) -> usize {
    let Ok(parsed) = ttf_parser::Face::parse(&face.bytes, face.index) else {
        return 0;
    };
    chars
        .iter()
        .filter(|&&ch| parsed.glyph_index(ch).is_some())
        .count()
}

pub fn measure_text(text: &str, style: &TextStyle) -> (f32, f32) {
    if let Some(face) = find_font_for_text(text, &style.font_family) {
        if let Some((w, h)) = shape_width(&face, text, style.font_size) {
            return (w, h);
        }
    }
    let avg = style.font_size * 0.5;
    let width = text.chars().count() as f32 * avg;
    (width, style.font_size * 1.2)
}

fn shape_width(face: &FontFaceData, text: &str, font_size: f32) -> Option<(f32, f32)> {
    let rb = rustybuzz::Face::from_slice(&face.bytes, face.index)?;
    let mut buf = rustybuzz::UnicodeBuffer::new();
    buf.push_str(text);
    // Hint RTL for Hebrew/Arabic ranges
    if text.chars().any(|c| matches!(c, '\u{0590}'..='\u{05FF}' | '\u{0600}'..='\u{06FF}')) {
        buf.set_direction(rustybuzz::Direction::RightToLeft);
    }
    let glyphs = rustybuzz::shape(&rb, &[], buf);
    let upem = rb.units_per_em() as f32;
    let scale = font_size / upem.max(1.0);
    let width: f32 = glyphs
        .glyph_positions()
        .iter()
        .map(|pos| pos.x_advance as f32 * scale)
        .sum();
    Some((width.abs(), font_size * 1.2))
}
