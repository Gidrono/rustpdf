//! Level-2: rewrite PDF content-stream text operators (Tj/TJ/'/") when possible.

use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RewriteError {
    #[error("lopdf: {0}")]
    Lopdf(#[from] lopdf::Error),
    #[error("no matching text operator for {0:?}")]
    NotFound(String),
    #[error("{0}")]
    Msg(String),
}

/// Try to replace the first occurrence of `old` (as a PDF literal string) with `new`
/// inside the page's content streams. Returns Ok(true) if a rewrite landed.
pub fn try_rewrite_page_literal(
    doc: &mut Document,
    page_id: ObjectId,
    old: &str,
    new: &str,
) -> Result<bool, RewriteError> {
    if old.is_empty() || old == new {
        return Ok(false);
    }
    let content_ids = collect_content_ids(doc, page_id)?;
    for cid in content_ids {
        let rewritten = {
            let obj = doc.get_object(cid)?;
            let Object::Stream(stream) = obj else {
                continue;
            };
            let data = stream.content.clone();
            // Decompress if needed — lopdf often stores decoded in .content after load
            let text = String::from_utf8_lossy(&data);
            if !text.contains(&escape_for_search(old)) && !text.contains(old) {
                // Also try raw latin-1-ish
                if !bytes_contains_pdf_string(&data, old) {
                    continue;
                }
            }
            rewrite_stream_bytes(&data, old, new)
        };
        if let Some(new_bytes) = rewritten {
            let stream = {
                let obj = doc.get_object_mut(cid)?;
                let Object::Stream(stream) = obj else {
                    continue;
                };
                stream.content = new_bytes;
                // Drop filter so we don't keep claiming FlateDecode on raw bytes
                stream.dict.remove(b"Filter");
                stream.dict.set("Length", Object::Integer(stream.content.len() as i64));
                true
            };
            if stream {
                return Ok(true);
            }
        }
    }
    Err(RewriteError::NotFound(old.to_string()))
}

fn collect_content_ids(doc: &Document, page_id: ObjectId) -> Result<Vec<ObjectId>, RewriteError> {
    let page = doc.get_object(page_id)?.as_dict()?;
    let mut ids = Vec::new();
    match page.get(b"Contents") {
        Ok(Object::Reference(r)) => ids.push(*r),
        Ok(Object::Array(arr)) => {
            for o in arr {
                if let Object::Reference(r) = o {
                    ids.push(*r);
                }
            }
        }
        _ => {}
    }
    Ok(ids)
}

fn escape_for_search(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '(' | ')' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

fn bytes_contains_pdf_string(data: &[u8], s: &str) -> bool {
    let lit = format!("({})", escape_for_search(s));
    data.windows(lit.len()).any(|w| w == lit.as_bytes())
}

/// Rewrite `(old) Tj` / `(old) '` style operators. Handles basic escaped literals.
fn rewrite_stream_bytes(data: &[u8], old: &str, new: &str) -> Option<Vec<u8>> {
    let old_lit = format!("({})", escape_pdf_literal(old));
    let new_lit = format!("({})", escape_pdf_literal(new));
    let hay = String::from_utf8_lossy(data);
    if let Some(idx) = hay.find(&old_lit) {
        let mut out = Vec::with_capacity(data.len() + new_lit.len());
        out.extend_from_slice(data.get(..idx)?);
        out.extend_from_slice(new_lit.as_bytes());
        out.extend_from_slice(data.get(idx + old_lit.len()..)?);
        return Some(out);
    }
    // Hex string form <...>
    if old.is_ascii() && new.is_ascii() {
        let old_hex = format!("<{}>", to_hex(old.as_bytes()));
        let new_hex = format!("<{}>", to_hex(new.as_bytes()));
        if let Some(idx) = hay.find(&old_hex) {
            let mut out = Vec::with_capacity(data.len() + new_hex.len());
            out.extend_from_slice(data.get(..idx)?);
            out.extend_from_slice(new_hex.as_bytes());
            out.extend_from_slice(data.get(idx + old_hex.len()..)?);
            return Some(out);
        }
    }
    None
}

fn escape_pdf_literal(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '(' => out.push_str("\\("),
            ')' => out.push_str("\\)"),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if c.is_ascii() => out.push(c),
            // Non-ASCII won't match Type1 literals; Level-2 only for ASCII streams
            _ => out.push('?'),
        }
    }
    out
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

/// Reconstruct a line: replace `selected` within `line` with `replacement`.
pub fn reconstruct_line(line: &str, selected: &str, replacement: &str) -> Option<String> {
    if let Some(idx) = line.find(selected) {
        let mut s = String::new();
        s.push_str(&line[..idx]);
        s.push_str(replacement);
        s.push_str(&line[idx + selected.len()..]);
        Some(s)
    } else {
        None
    }
}

#[allow(dead_code)]
fn _dict_unused(_: &Dictionary) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_simple_tj() {
        let data = b"BT /F1 12 Tf 100 700 Td (Hello World) Tj ET";
        let out = rewrite_stream_bytes(data, "Hello World", "Hi World").unwrap();
        let s = String::from_utf8_lossy(&out);
        assert!(s.contains("(Hi World)"));
        assert!(!s.contains("(Hello World)"));
    }

    #[test]
    fn reconstruct_line_works() {
        assert_eq!(
            reconstruct_line("abc def ghi", "def", "XYZ").as_deref(),
            Some("abc XYZ ghi")
        );
    }
}
