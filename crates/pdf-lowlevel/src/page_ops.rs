use std::path::Path;

use lopdf::{Dictionary, Document, Object, ObjectId};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PageOpError {
    #[error("lopdf: {0}")]
    Lopdf(#[from] lopdf::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Msg(String),
}

#[derive(Debug, Clone)]
pub enum PageOp {
    Delete { index: u32 },
    Rotate { index: u32, degrees: i32 },
    Reorder { from: u32, to: u32 },
    InsertBlank { at: u32 },
    Duplicate { index: u32 },
}

/// Extract the given 0-based page indices into a new PDF (deep-enough copy of page refs).
pub fn extract_pages(pdf_bytes: &[u8], indices: &[u32]) -> Result<Vec<u8>, PageOpError> {
    if indices.is_empty() {
        return Err(PageOpError::Msg("no pages selected for extract".into()));
    }
    let mut doc = Document::load_mem(pdf_bytes)?;
    let pages = doc.get_pages();
    let mut ordered: Vec<(u32, ObjectId)> = pages.into_iter().collect();
    ordered.sort_by_key(|(n, _)| *n);
    let all_ids: Vec<ObjectId> = ordered.iter().map(|(_, id)| *id).collect();

    let mut keep = Vec::new();
    for &idx in indices {
        let i = idx as usize;
        if i >= all_ids.len() {
            return Err(PageOpError::Msg(format!("page index {idx} out of range")));
        }
        keep.push(all_ids[i]);
    }
    rewrite_kids(&mut doc, &keep)?;
    let mut out = Vec::new();
    doc.save_to(&mut out)?;
    Ok(out)
}

/// Apply a page-structure operation via lopdf; returns new PDF bytes.
pub fn apply_page_op(pdf_bytes: &[u8], op: PageOp) -> Result<Vec<u8>, PageOpError> {
    let mut doc = Document::load_mem(pdf_bytes)?;
    let pages = doc.get_pages();
    let mut page_ids: Vec<ObjectId> = pages.values().copied().collect();
    // lopdf BTreeMap is ordered by page number keys
    let mut ordered: Vec<(u32, ObjectId)> = pages.into_iter().collect();
    ordered.sort_by_key(|(n, _)| *n);
    page_ids = ordered.iter().map(|(_, id)| *id).collect();

    match op {
        PageOp::Delete { index } => {
            let idx = index as usize;
            if idx >= page_ids.len() {
                return Err(PageOpError::Msg("page index out of range".into()));
            }
            if page_ids.len() == 1 {
                return Err(PageOpError::Msg("cannot delete the last page".into()));
            }
            let victim = page_ids[idx];
            // Remove from Pages Kids
            remove_page_from_tree(&mut doc, victim)?;
        }
        PageOp::Rotate { index, degrees } => {
            let idx = index as usize;
            if idx >= page_ids.len() {
                return Err(PageOpError::Msg("page index out of range".into()));
            }
            let id = page_ids[idx];
            let page = doc.get_object_mut(id)?.as_dict_mut()?;
            let current = page
                .get(b"Rotate")
                .ok()
                .and_then(|o| o.as_i64().ok())
                .unwrap_or(0) as i32;
            let new_rot = ((current + degrees) % 360 + 360) % 360;
            page.set("Rotate", Object::Integer(new_rot as i64));
        }
        PageOp::Reorder { from, to } => {
            let from = from as usize;
            let to = to as usize;
            if from >= page_ids.len() || to >= page_ids.len() {
                return Err(PageOpError::Msg("page index out of range".into()));
            }
            let id = page_ids.remove(from);
            page_ids.insert(to, id);
            rewrite_kids(&mut doc, &page_ids)?;
        }
        PageOp::InsertBlank { at } => {
            let at = (at as usize).min(page_ids.len());
            let blank = create_blank_page(&mut doc)?;
            page_ids.insert(at, blank);
            rewrite_kids(&mut doc, &page_ids)?;
        }
        PageOp::Duplicate { index } => {
            let idx = index as usize;
            if idx >= page_ids.len() {
                return Err(PageOpError::Msg("page index out of range".into()));
            }
            // Shallow duplicate: reference same page object (content shared).
            // Good enough for V1; deep copy can come later.
            let id = page_ids[idx];
            page_ids.insert(idx + 1, id);
            rewrite_kids(&mut doc, &page_ids)?;
        }
    }

    let mut out = Vec::new();
    doc.save_to(&mut out)?;
    Ok(out)
}

fn create_blank_page(doc: &mut Document) -> Result<ObjectId, PageOpError> {
    let mut dict = Dictionary::new();
    dict.set("Type", Object::Name(b"Page".to_vec()));
    dict.set(
        "MediaBox",
        Object::Array(vec![
            Object::Integer(0),
            Object::Integer(0),
            Object::Integer(612),
            Object::Integer(792),
        ]),
    );
    dict.set("Resources", Object::Dictionary(Dictionary::new()));
    dict.set("Contents", Object::Array(vec![]));
    // Parent set by rewrite_kids
    let id = doc.add_object(Object::Dictionary(dict));
    Ok(id)
}

fn pages_root_id(doc: &Document) -> Result<ObjectId, PageOpError> {
    let catalog = doc
        .trailer
        .get(b"Root")
        .map_err(|_| PageOpError::Msg("missing Root".into()))?
        .as_reference()
        .map_err(|_| PageOpError::Msg("Root not a reference".into()))?;
    let pages = doc
        .get_object(catalog)?
        .as_dict()?
        .get(b"Pages")?
        .as_reference()?;
    Ok(pages)
}

fn rewrite_kids(doc: &mut Document, page_ids: &[ObjectId]) -> Result<(), PageOpError> {
    let pages_id = pages_root_id(doc)?;
    let kids: Vec<Object> = page_ids.iter().map(|id| Object::Reference(*id)).collect();
    {
        let pages = doc.get_object_mut(pages_id)?.as_dict_mut()?;
        pages.set("Kids", Object::Array(kids));
        pages.set("Count", Object::Integer(page_ids.len() as i64));
    }
    for &pid in page_ids {
        if let Ok(page) = doc.get_object_mut(pid).and_then(|o| o.as_dict_mut()) {
            page.set("Parent", Object::Reference(pages_id));
        }
    }
    Ok(())
}

fn remove_page_from_tree(doc: &mut Document, victim: ObjectId) -> Result<(), PageOpError> {
    let pages = doc.get_pages();
    let mut ordered: Vec<(u32, ObjectId)> = pages.into_iter().collect();
    ordered.sort_by_key(|(n, _)| *n);
    let page_ids: Vec<ObjectId> = ordered
        .into_iter()
        .map(|(_, id)| id)
        .filter(|id| *id != victim)
        .collect();
    rewrite_kids(doc, &page_ids)
}

/// Convenience: load path, apply op, return bytes.
pub fn apply_page_op_file(path: &Path, op: PageOp) -> Result<Vec<u8>, PageOpError> {
    let bytes = std::fs::read(path)?;
    apply_page_op(&bytes, op)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_keeps_requested_count() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/large-sample.pdf");
        if !fixture.exists() {
            return;
        }
        let bytes = std::fs::read(&fixture).unwrap();
        let out = extract_pages(&bytes, &[0, 2, 5]).unwrap();
        let doc = Document::load_mem(&out).unwrap();
        assert_eq!(doc.get_pages().len(), 3);
    }
}
