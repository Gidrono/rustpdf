use std::path::Path;

use lopdf::Document;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PdfInspectError {
    #[error("failed to open PDF: {0}")]
    Open(#[from] lopdf::Error),
    #[error("PDF has no pages")]
    NoPages,
}

#[derive(Debug, Clone)]
pub struct PageBox {
    pub width_pts: f32,
    pub height_pts: f32,
}

#[derive(Debug, Clone)]
pub struct PdfInspection {
    pub page_count: u32,
    pub pages: Vec<PageBox>,
}

/// Best-effort page geometry without PDFium.
pub fn inspect_pdf(path: &Path) -> Result<PdfInspection, PdfInspectError> {
    let doc = Document::load(path)?;
    let pages = doc.get_pages();
    if pages.is_empty() {
        return Err(PdfInspectError::NoPages);
    }

    let mut boxes = Vec::with_capacity(pages.len());
    for (_num, id) in pages.iter() {
        let (w, h) = page_size(&doc, *id).unwrap_or((612.0, 792.0));
        boxes.push(PageBox {
            width_pts: w,
            height_pts: h,
        });
    }

    Ok(PdfInspection {
        page_count: boxes.len() as u32,
        pages: boxes,
    })
}

fn page_size(doc: &Document, page_id: lopdf::ObjectId) -> Option<(f32, f32)> {
    let page = doc.get_object(page_id).ok()?.as_dict().ok()?;
    let media = page
        .get(b"MediaBox")
        .ok()
        .and_then(|o| o.as_array().ok())
        .or_else(|| {
            // Inherit from Pages tree — lopdf may already resolve; fall back Letter.
            None
        })?;

    if media.len() < 4 {
        return None;
    }
    let x0 = object_as_f32(&media[0])?;
    let y0 = object_as_f32(&media[1])?;
    let x1 = object_as_f32(&media[2])?;
    let y1 = object_as_f32(&media[3])?;
    Some(((x1 - x0).abs(), (y1 - y0).abs()))
}

fn object_as_f32(obj: &lopdf::Object) -> Option<f32> {
    match obj {
        lopdf::Object::Integer(i) => Some(*i as f32),
        lopdf::Object::Real(r) => Some(*r),
        _ => None,
    }
}
