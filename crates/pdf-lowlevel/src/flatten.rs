//! Flatten editor overlays into PDF content streams.
//!
//! - Unicode text → subsetted Type0/CIDFontType2 (Identity-H) via text-engine
//! - ASCII-only may use Helvetica Type1
//! - NativeTextReplace tries content-stream rewrite first; visual fallback otherwise

use std::collections::HashMap;
use std::io::Write;

use doxo_document::{EditorObject, EditorScene, ImageFormat, PageId, RgbaColor, ShapeKind, TextStyle};
use doxo_text_engine::{prepare_embedded_run, EmbeddedRun};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use thiserror::Error;

use crate::text_rewrite::{reconstruct_line, try_rewrite_page_literal};

#[derive(Debug, Error)]
pub enum FlattenError {
    #[error("lopdf: {0}")]
    Lopdf(#[from] lopdf::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Msg(String),
}

struct FontSlot {
    name: String,
    type0_id: ObjectId,
}

/// Produce new PDF bytes with overlays burned into page content.
pub fn flatten_scene_into_pdf(
    pdf_bytes: &[u8],
    scene: &EditorScene,
    page_heights: &HashMap<u32, f32>,
) -> Result<Vec<u8>, FlattenError> {
    let mut doc = Document::load_mem(pdf_bytes)?;
    let pages = doc.get_pages();
    let mut ordered: Vec<(u32, ObjectId)> = pages.into_iter().collect();
    ordered.sort_by_key(|(n, _)| *n);

    // Shared subset fonts keyed by blake-ish content hash of subset bytes
    let mut font_cache: HashMap<u64, FontSlot> = HashMap::new();
    let mut font_seq = 0u32;

    for (page_num, page_id) in &ordered {
        let page_index = (*page_num as u32).saturating_sub(1);
        let page_id_zero = PageId(page_index);
        let objs: Vec<&EditorObject> = scene.objects_on_page(page_id_zero).collect();
        if objs.is_empty() {
            continue;
        }
        let _height = page_heights.get(&page_index).copied().unwrap_or(792.0);

        let mut content = String::new();
        let mut xobjects = Dictionary::new();
        let mut image_count = 0u32;
        let mut page_font_names: Vec<String> = Vec::new();

        for obj in objs {
            match obj {
                EditorObject::NativeTextReplace {
                    cover,
                    line_original,
                    selected,
                    replacement,
                    style,
                    ..
                } => {
                    let reconstructed = reconstruct_line(line_original, selected, replacement)
                        .unwrap_or_else(|| replacement.clone());
                    let rewritten = try_rewrite_page_literal(
                        &mut doc,
                        *page_id,
                        line_original,
                        &reconstructed,
                    )
                    .unwrap_or(false);

                    if !rewritten {
                        // Level-1 visual fallback
                        content.push_str(&format!(
                            "q 1 1 1 rg {:.2} {:.2} {:.2} {:.2} re f Q\n",
                            cover.x, cover.y, cover.w, cover.h
                        ));
                        append_text_draw(
                            &mut doc,
                            &mut content,
                            &mut font_cache,
                            &mut font_seq,
                            &mut page_font_names,
                            cover.x,
                            cover.y + 2.0,
                            replacement,
                            style,
                        )?;
                    }
                }
                EditorObject::VisualTextPatch {
                    cover, text, style, ..
                } => {
                    content.push_str(&format!(
                        "q 1 1 1 rg {:.2} {:.2} {:.2} {:.2} re f Q\n",
                        cover.x, cover.y, cover.w, cover.h
                    ));
                    append_text_draw(
                        &mut doc,
                        &mut content,
                        &mut font_cache,
                        &mut font_seq,
                        &mut page_font_names,
                        cover.x,
                        cover.y + 2.0,
                        text,
                        style,
                    )?;
                }
                EditorObject::TextBox {
                    rect, text, style, ..
                } => {
                    append_text_draw(
                        &mut doc,
                        &mut content,
                        &mut font_cache,
                        &mut font_seq,
                        &mut page_font_names,
                        rect.x,
                        rect.y + 2.0,
                        text,
                        style,
                    )?;
                }
                EditorObject::Highlight { quads, color, .. } => {
                    let (r, g, b, _) = rgba_f(*color);
                    content.push_str("q /GS0 gs ");
                    content.push_str(&format!("{r:.3} {g:.3} {b:.3} rg\n"));
                    for q in quads {
                        content.push_str(&format!(
                            "{:.2} {:.2} {:.2} {:.2} re f\n",
                            q.x, q.y, q.w, q.h
                        ));
                    }
                    content.push_str("Q\n");
                }
                EditorObject::InkStroke {
                    points, color, width, ..
                } => {
                    if points.len() < 2 {
                        continue;
                    }
                    let (r, g, b, _) = rgba_f(*color);
                    content.push_str(&format!(
                        "q {r:.3} {g:.3} {b:.3} RG {:.2} w 1 j 1 J\n",
                        width
                    ));
                    content.push_str(&format!("{:.2} {:.2} m\n", points[0][0], points[0][1]));
                    for p in &points[1..] {
                        content.push_str(&format!("{:.2} {:.2} l\n", p[0], p[1]));
                    }
                    content.push_str("S Q\n");
                }
                EditorObject::Shape {
                    kind,
                    rect,
                    stroke,
                    fill,
                    stroke_width,
                    ..
                } => {
                    append_shape(&mut content, *kind, rect, *stroke, *fill, *stroke_width);
                }
                EditorObject::Image {
                    rect,
                    bytes,
                    format,
                    opacity,
                    rotation_deg,
                    crop,
                    ..
                } => {
                    image_count += 1;
                    let name = format!("Im{image_count}");
                    let cropped = apply_crop_bytes(bytes, *format, *crop)?;
                    let xobj = embed_image(&mut doc, &cropped, *format)?;
                    xobjects.set(name.as_bytes().to_vec(), Object::Reference(xobj));
                    let _ = opacity;
                    let rad = rotation_deg.to_radians();
                    let (cx, cy) = (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5);
                    let cos = rad.cos();
                    let sin = rad.sin();
                    // Translate to center, rotate, scale, draw at -w/2,-h/2
                    content.push_str(&format!(
                        "q {:.4} {:.4} {:.4} {:.4} {:.2} {:.2} cm {:.2} 0 0 {:.2} {:.2} {:.2} cm /{name} Do Q\n",
                        cos,
                        sin,
                        -sin,
                        cos,
                        cx,
                        cy,
                        rect.w,
                        rect.h,
                        -rect.w * 0.5,
                        -rect.h * 0.5,
                    ));
                }
            }
        }

        ensure_font_and_gs(&mut doc, *page_id)?;
        // Register subset fonts on this page
        for slot_name in &page_font_names {
            if let Some(slot) = font_cache.values().find(|s| s.name == *slot_name) {
                register_page_font(&mut doc, *page_id, &slot.name, slot.type0_id)?;
            }
        }

        if !xobjects.is_empty() {
            let page = doc.get_object_mut(*page_id)?.as_dict_mut()?;
            let resources = ensure_resources_dict(page)?;
            let mut xobj_dict = resources
                .get(b"XObject")
                .ok()
                .and_then(|o| o.as_dict().ok().cloned())
                .unwrap_or_default();
            for (k, v) in xobjects {
                xobj_dict.set(k, v);
            }
            resources.set("XObject", Object::Dictionary(xobj_dict));
        }

        if !content.is_empty() {
            append_content_stream(&mut doc, *page_id, content.as_bytes())?;
        }
    }

    let mut out = Vec::new();
    doc.save_to(&mut out)?;
    Ok(out)
}

fn append_text_draw(
    doc: &mut Document,
    content: &mut String,
    font_cache: &mut HashMap<u64, FontSlot>,
    font_seq: &mut u32,
    page_font_names: &mut Vec<String>,
    x: f32,
    y: f32,
    text: &str,
    style: &TextStyle,
) -> Result<(), FlattenError> {
    let (r, g, b) = (
        style.color.r as f32 / 255.0,
        style.color.g as f32 / 255.0,
        style.color.b as f32 / 255.0,
    );

    if let Some(run) = prepare_embedded_run(text, style) {
        let key = simple_hash(&run.font_bytes);
        let slot = if let Some(s) = font_cache.get(&key) {
            s.clone_name()
        } else {
            *font_seq += 1;
            let name = format!("FSub{font_seq}");
            let type0 = embed_type0_font(doc, &run, &name)?;
            font_cache.insert(
                key,
                FontSlot {
                    name: name.clone(),
                    type0_id: type0,
                },
            );
            name
        };
        if !page_font_names.contains(&slot) {
            page_font_names.push(slot.clone());
        }
        let hex = cids_to_hex(&run.cids);
        let draw_x = if run.rtl { x + run.width_pts } else { x };
        content.push_str(&format!(
            "q BT /{slot} {size:.2} Tf {r:.3} {g:.3} {b:.3} rg {x:.2} {y:.2} Td <{hex}> Tj ET Q\n",
            size = style.font_size,
            x = draw_x,
            y = y,
        ));
    } else {
        // Helvetica ASCII fallback
        let escaped = escape_pdf_string_ascii(text);
        content.push_str(&format!(
            "q BT /F1 {size:.2} Tf {r:.3} {g:.3} {b:.3} rg {x:.2} {y:.2} Td ({escaped}) Tj ET Q\n",
            size = style.font_size,
        ));
    }
    Ok(())
}

impl FontSlot {
    fn clone_name(&self) -> String {
        self.name.clone()
    }
}

fn simple_hash(bytes: &[u8]) -> u64 {
    // FNV-1a
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes.iter().take(4096) {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h ^= bytes.len() as u64;
    h
}

fn cids_to_hex(cids: &[u16]) -> String {
    let mut s = String::with_capacity(cids.len() * 4);
    for c in cids {
        s.push_str(&format!("{c:04X}"));
    }
    s
}

fn embed_type0_font(
    doc: &mut Document,
    run: &EmbeddedRun,
    _res_name: &str,
) -> Result<ObjectId, FlattenError> {
    let base = format!("AAAAAA+{}", sanitize_name(&run.family));

    // FontFile2
    let mut ff_dict = Dictionary::new();
    ff_dict.set("Length1", Object::Integer(run.font_bytes.len() as i64));
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    enc.write_all(&run.font_bytes)
        .map_err(|e| FlattenError::Msg(e.to_string()))?;
    let compressed = enc.finish().map_err(|e| FlattenError::Msg(e.to_string()))?;
    ff_dict.set("Filter", Object::Name(b"FlateDecode".to_vec()));
    ff_dict.set("Length", Object::Integer(compressed.len() as i64));
    let ff_id = doc.add_object(Object::Stream(Stream::new(ff_dict, compressed)));

    // FontDescriptor
    let mut fd = Dictionary::new();
    fd.set("Type", Object::Name(b"FontDescriptor".to_vec()));
    fd.set("FontName", Object::Name(base.as_bytes().to_vec()));
    fd.set("Flags", Object::Integer(32));
    fd.set(
        "FontBBox",
        Object::Array(vec![
            Object::Integer(-100),
            Object::Integer(-200),
            Object::Integer(1000),
            Object::Integer(900),
        ]),
    );
    fd.set("ItalicAngle", Object::Integer(0));
    fd.set("Ascent", Object::Integer(800));
    fd.set("Descent", Object::Integer(-200));
    fd.set("CapHeight", Object::Integer(700));
    fd.set("StemV", Object::Integer(80));
    fd.set("FontFile2", Object::Reference(ff_id));
    let fd_id = doc.add_object(Object::Dictionary(fd));

    // CIDSystemInfo
    let mut csi = Dictionary::new();
    csi.set("Registry", Object::String(b"Adobe".to_vec(), lopdf::StringFormat::Literal));
    csi.set("Ordering", Object::String(b"Identity".to_vec(), lopdf::StringFormat::Literal));
    csi.set("Supplement", Object::Integer(0));

    // /W array: [ cid [width] cid [width] ... ] in font units (1000-scale often expected;
    // TrueType uses units_per_em — PDF wants widths in 1/1000 of text space for CID fonts
    // when DW is used. We'll scale advances to 1000 units.
    let scale = 1000.0 / run.units_per_em as f32;
    let mut w_arr = Vec::new();
    for (cid, adv) in run.cids.iter().zip(run.advances.iter()) {
        w_arr.push(Object::Integer(*cid as i64));
        w_arr.push(Object::Array(vec![Object::Integer(
            (*adv as f32 * scale).round() as i64,
        )]));
    }

    let mut cid_font = Dictionary::new();
    cid_font.set("Type", Object::Name(b"Font".to_vec()));
    cid_font.set("Subtype", Object::Name(b"CIDFontType2".to_vec()));
    cid_font.set("BaseFont", Object::Name(base.as_bytes().to_vec()));
    cid_font.set("CIDSystemInfo", Object::Dictionary(csi));
    cid_font.set("FontDescriptor", Object::Reference(fd_id));
    cid_font.set("DW", Object::Integer(1000));
    cid_font.set("W", Object::Array(w_arr));
    cid_font.set("CIDToGIDMap", Object::Name(b"Identity".to_vec()));
    let cid_id = doc.add_object(Object::Dictionary(cid_font));

    // ToUnicode CMap
    let cmap = build_tounicode_cmap(&run.to_unicode);
    let mut cm_dict = Dictionary::new();
    cm_dict.set("Length", Object::Integer(cmap.len() as i64));
    let cm_id = doc.add_object(Object::Stream(Stream::new(cm_dict, cmap)));

    let mut type0 = Dictionary::new();
    type0.set("Type", Object::Name(b"Font".to_vec()));
    type0.set("Subtype", Object::Name(b"Type0".to_vec()));
    type0.set("BaseFont", Object::Name(base.as_bytes().to_vec()));
    type0.set("Encoding", Object::Name(b"Identity-H".to_vec()));
    type0.set(
        "DescendantFonts",
        Object::Array(vec![Object::Reference(cid_id)]),
    );
    type0.set("ToUnicode", Object::Reference(cm_id));
    Ok(doc.add_object(Object::Dictionary(type0)))
}

fn build_tounicode_cmap(pairs: &[(u16, char)]) -> Vec<u8> {
    let mut body = String::new();
    body.push_str("/CIDInit /ProcSet findresource begin\n");
    body.push_str("12 dict begin\nbegincmap\n");
    body.push_str("/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n");
    body.push_str("/CMapName /Adobe-Identity-UCS def\n");
    body.push_str("/CMapType 2 def\n");
    body.push_str("1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n");
    // dedupe by gid
    let mut map = HashMap::new();
    for (gid, ch) in pairs {
        map.insert(*gid, *ch);
    }
    let entries: Vec<_> = map.into_iter().collect();
    body.push_str(&format!("{} beginbfchar\n", entries.len()));
    for (gid, ch) in entries {
        let cp = ch as u32;
        if cp <= 0xFFFF {
            body.push_str(&format!("<{gid:04X}> <{cp:04X}>\n"));
        }
    }
    body.push_str("endbfchar\nendcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    body.into_bytes()
}

fn sanitize_name(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .take(24)
        .collect()
}

fn register_page_font(
    doc: &mut Document,
    page_id: ObjectId,
    name: &str,
    type0: ObjectId,
) -> Result<(), FlattenError> {
    let page = doc.get_object_mut(page_id)?.as_dict_mut()?;
    let resources = ensure_resources_dict(page)?;
    let mut font_dict = resources
        .get(b"Font")
        .ok()
        .and_then(|o| o.as_dict().ok().cloned())
        .unwrap_or_default();
    font_dict.set(name.as_bytes().to_vec(), Object::Reference(type0));
    resources.set("Font", Object::Dictionary(font_dict));
    Ok(())
}

fn append_shape(
    content: &mut String,
    kind: ShapeKind,
    rect: &doxo_document::RectPts,
    stroke: RgbaColor,
    fill: Option<RgbaColor>,
    stroke_width: f32,
) {
    let (sr, sg, sb, _) = rgba_f(stroke);
    content.push_str(&format!(
        "q {sr:.3} {sg:.3} {sb:.3} RG {:.2} w\n",
        stroke_width
    ));
    if let Some(fill_c) = fill {
        let (fr, fg, fb, _) = rgba_f(fill_c);
        content.push_str(&format!("{fr:.3} {fg:.3} {fb:.3} rg\n"));
    }
    match kind {
        ShapeKind::Rectangle => {
            content.push_str(&format!(
                "{:.2} {:.2} {:.2} {:.2} re\n",
                rect.x, rect.y, rect.w, rect.h
            ));
            content.push_str(if fill.is_some() { "B\n" } else { "S\n" });
        }
        ShapeKind::Ellipse => {
            let cx = rect.x + rect.w * 0.5;
            let cy = rect.y + rect.h * 0.5;
            let rx = rect.w * 0.5;
            let ry = rect.h * 0.5;
            let k = 0.5522847498;
            content.push_str(&format!("{:.2} {:.2} m\n", cx + rx, cy));
            content.push_str(&format!(
                "{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c\n",
                cx + rx,
                cy + k * ry,
                cx + k * rx,
                cy + ry,
                cx,
                cy + ry
            ));
            content.push_str(&format!(
                "{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c\n",
                cx - k * rx,
                cy + ry,
                cx - rx,
                cy + k * ry,
                cx - rx,
                cy
            ));
            content.push_str(&format!(
                "{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c\n",
                cx - rx,
                cy - k * ry,
                cx - k * rx,
                cy - ry,
                cx,
                cy - ry
            ));
            content.push_str(&format!(
                "{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c\n",
                cx + k * rx,
                cy - ry,
                cx + rx,
                cy - k * ry,
                cx + rx,
                cy
            ));
            content.push_str(if fill.is_some() { "B\n" } else { "S\n" });
        }
        ShapeKind::Line | ShapeKind::Arrow => {
            content.push_str(&format!(
                "{:.2} {:.2} m {:.2} {:.2} l S\n",
                rect.x,
                rect.y,
                rect.x + rect.w,
                rect.y + rect.h
            ));
            if kind == ShapeKind::Arrow {
                let (x1, y1) = (rect.x + rect.w, rect.y + rect.h);
                let angle = (rect.h).atan2(rect.w);
                let len = 10.0_f32;
                let a1 = angle + 2.7;
                let a2 = angle - 2.7;
                content.push_str(&format!(
                    "{:.2} {:.2} m {:.2} {:.2} l S\n",
                    x1,
                    y1,
                    x1 + len * a1.cos(),
                    y1 + len * a1.sin()
                ));
                content.push_str(&format!(
                    "{:.2} {:.2} m {:.2} {:.2} l S\n",
                    x1,
                    y1,
                    x1 + len * a2.cos(),
                    y1 + len * a2.sin()
                ));
            }
        }
    }
    content.push_str("Q\n");
}

fn apply_crop_bytes(
    bytes: &[u8],
    format: ImageFormat,
    crop: Option<[f32; 4]>,
) -> Result<Vec<u8>, FlattenError> {
    let Some([x0, y0, x1, y1]) = crop else {
        return Ok(bytes.to_vec());
    };
    let img = image::load_from_memory(bytes).map_err(|e| FlattenError::Msg(e.to_string()))?;
    let (w, h) = (img.width(), img.height());
    let left = ((x0.clamp(0.0, 1.0)) * w as f32).round() as u32;
    let top = ((y0.clamp(0.0, 1.0)) * h as f32).round() as u32;
    let right = ((x1.clamp(0.0, 1.0)) * w as f32).round() as u32;
    let bottom = ((y1.clamp(0.0, 1.0)) * h as f32).round() as u32;
    if right <= left || bottom <= top {
        return Ok(bytes.to_vec());
    }
    let cropped = img.crop_imm(left, top, right - left, bottom - top);
    let mut out = Vec::new();
    match format {
        ImageFormat::Png => {
            cropped
                .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
                .map_err(|e| FlattenError::Msg(e.to_string()))?;
        }
        ImageFormat::Jpeg => {
            cropped
                .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Jpeg)
                .map_err(|e| FlattenError::Msg(e.to_string()))?;
        }
    }
    Ok(out)
}

fn rgba_f(c: RgbaColor) -> (f32, f32, f32, f32) {
    (
        c.r as f32 / 255.0,
        c.g as f32 / 255.0,
        c.b as f32 / 255.0,
        c.a as f32 / 255.0,
    )
}

fn escape_pdf_string_ascii(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        match ch {
            '(' => out.push_str("\\("),
            ')' => out.push_str("\\)"),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if c.is_ascii() => out.push(c),
            _ => out.push('?'),
        }
    }
    out
}

fn embed_image(
    doc: &mut Document,
    bytes: &[u8],
    format: ImageFormat,
) -> Result<ObjectId, FlattenError> {
    match format {
        ImageFormat::Jpeg => {
            let img = image::load_from_memory(bytes)
                .map_err(|e| FlattenError::Msg(format!("image: {e}")))?;
            let (w, h) = (img.width(), img.height());
            let mut dict = Dictionary::new();
            dict.set("Type", Object::Name(b"XObject".to_vec()));
            dict.set("Subtype", Object::Name(b"Image".to_vec()));
            dict.set("Width", Object::Integer(w as i64));
            dict.set("Height", Object::Integer(h as i64));
            dict.set("ColorSpace", Object::Name(b"DeviceRGB".to_vec()));
            dict.set("BitsPerComponent", Object::Integer(8));
            dict.set("Filter", Object::Name(b"DCTDecode".to_vec()));
            Ok(doc.add_object(Object::Stream(Stream::new(dict, bytes.to_vec()))))
        }
        ImageFormat::Png => {
            let img = image::load_from_memory(bytes)
                .map_err(|e| FlattenError::Msg(format!("image: {e}")))?
                .to_rgb8();
            let (w, h) = (img.width(), img.height());
            let mut dict = Dictionary::new();
            dict.set("Type", Object::Name(b"XObject".to_vec()));
            dict.set("Subtype", Object::Name(b"Image".to_vec()));
            dict.set("Width", Object::Integer(w as i64));
            dict.set("Height", Object::Integer(h as i64));
            dict.set("ColorSpace", Object::Name(b"DeviceRGB".to_vec()));
            dict.set("BitsPerComponent", Object::Integer(8));
            let raw = img.into_raw();
            let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
            enc.write_all(&raw)
                .map_err(|e| FlattenError::Msg(e.to_string()))?;
            let compressed = enc
                .finish()
                .map_err(|e| FlattenError::Msg(e.to_string()))?;
            dict.set("Filter", Object::Name(b"FlateDecode".to_vec()));
            Ok(doc.add_object(Object::Stream(Stream::new(dict, compressed))))
        }
    }
}

fn ensure_resources_dict(page: &mut Dictionary) -> Result<&mut Dictionary, FlattenError> {
    if !page.has(b"Resources") {
        page.set("Resources", Object::Dictionary(Dictionary::new()));
    }
    match page
        .get_mut(b"Resources")
        .map_err(|_| FlattenError::Msg("resources".into()))?
    {
        Object::Dictionary(d) => Ok(d),
        Object::Reference(_) => Err(FlattenError::Msg(
            "page Resources is an indirect reference; unsupported in flatten".into(),
        )),
        _ => Err(FlattenError::Msg("invalid Resources".into())),
    }
}

fn ensure_font_and_gs(doc: &mut Document, page_id: ObjectId) -> Result<(), FlattenError> {
    {
        let page = doc.get_object_mut(page_id)?.as_dict_mut()?;
        let resources = ensure_resources_dict(page)?;
        let has_f1 = resources
            .get(b"Font")
            .ok()
            .and_then(|o| o.as_dict().ok())
            .is_some_and(|d| d.has(b"F1"));
        if !has_f1 {
            let mut helv = Dictionary::new();
            helv.set("Type", Object::Name(b"Font".to_vec()));
            helv.set("Subtype", Object::Name(b"Type1".to_vec()));
            helv.set("BaseFont", Object::Name(b"Helvetica".to_vec()));
            let font_id = doc.add_object(Object::Dictionary(helv));
            let page = doc.get_object_mut(page_id)?.as_dict_mut()?;
            let resources = ensure_resources_dict(page)?;
            let mut font_dict = resources
                .get(b"Font")
                .ok()
                .and_then(|o| o.as_dict().ok().cloned())
                .unwrap_or_default();
            font_dict.set("F1", Object::Reference(font_id));
            resources.set("Font", Object::Dictionary(font_dict));
        }
    }
    {
        let page = doc.get_object_mut(page_id)?.as_dict_mut()?;
        let resources = ensure_resources_dict(page)?;
        if resources.get(b"ExtGState").is_err() {
            let mut gs = Dictionary::new();
            gs.set("ca", Object::Real(0.4));
            gs.set("CA", Object::Real(0.4));
            let gs_id = doc.add_object(Object::Dictionary(gs));
            let page = doc.get_object_mut(page_id)?.as_dict_mut()?;
            let resources = ensure_resources_dict(page)?;
            let mut eg = Dictionary::new();
            eg.set("GS0", Object::Reference(gs_id));
            resources.set("ExtGState", Object::Dictionary(eg));
        }
    }
    Ok(())
}

fn append_content_stream(
    doc: &mut Document,
    page_id: ObjectId,
    extra: &[u8],
) -> Result<(), FlattenError> {
    let mut dict = Dictionary::new();
    dict.set("Length", Object::Integer(extra.len() as i64));
    let stream_id = doc.add_object(Object::Stream(Stream::new(dict, extra.to_vec())));

    let page = doc.get_object_mut(page_id)?.as_dict_mut()?;
    match page.get(b"Contents").ok() {
        None => {
            page.set("Contents", Object::Reference(stream_id));
        }
        Some(Object::Reference(r)) => {
            page.set(
                "Contents",
                Object::Array(vec![Object::Reference(*r), Object::Reference(stream_id)]),
            );
        }
        Some(Object::Array(arr)) => {
            let mut new_arr = arr.clone();
            new_arr.push(Object::Reference(stream_id));
            page.set("Contents", Object::Array(new_arr));
        }
        Some(_) => {
            page.set("Contents", Object::Reference(stream_id));
        }
    }
    Ok(())
}
