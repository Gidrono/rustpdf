//! Editor interaction state (scene, selection, text select, drawing).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use doxo_document::{
    make_highlight, make_image, make_ink, make_shape, make_textbox, EditCommand, EditJournal,
    EditorObject, EditorScene, ExtractedPageText, ImageFormat, ObjectId, PageId, RectPts,
    RgbaColor, TextStyle,
};
use doxo_graphics::simplify_stroke;
use doxo_pdf_lowlevel::AutosaveStore;

use crate::tools::EditorTool;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DragMode {
    None,
    MoveObject { id: ObjectId, last: (f32, f32) },
    ResizeObject { id: ObjectId },
    DrawInk,
    DrawShape { start: (f32, f32) },
    DrawHighlight { start_char: usize },
    SelectText { anchor: usize },
    CreateText,
}

pub struct EditorState {
    pub scene: EditorScene,
    pub journal: Option<EditJournal>,
    pub tool: EditorTool,
    pub selected: Option<ObjectId>,
    pub text_style: TextStyle,
    pub ink_color: RgbaColor,
    pub ink_width: f32,
    pub highlight_color: RgbaColor,
    pub extracted: HashMap<u32, ExtractedPageText>,
    pub text_sel: Option<(PageId, usize, usize)>, // page, start, end
    pub drag: DragMode,
    pub draft_points: Vec<[f32; 2]>,
    pub draft_shape: Option<RectPts>,
    pub editing_text_id: Option<ObjectId>,
    /// Draft while editing an overlay text object (avoids mutating scene until Done).
    pub text_edit_draft: Option<(ObjectId, String, TextStyle)>,
    /// Draft while editing selected existing PDF text (commits via ReplaceExistingText).
    pub pdf_text_edit_draft: Option<String>,
    pub clipboard_objects: Vec<EditorObject>,
    pub last_autosave: Option<Instant>,
    pub autosave_store: Option<AutosaveStore>,
    pub search_open: bool,
    pub search_query: String,
    pub status_note: String,
    pub snap_guides: Vec<doxo_graphics::SnapGuide>,
    pub crop_mode: bool,
}

impl Default for EditorState {
    fn default() -> Self {
        Self {
            scene: EditorScene::default(),
            journal: None,
            tool: EditorTool::Select,
            selected: None,
            text_style: TextStyle::default(),
            ink_color: RgbaColor::RED,
            ink_width: 2.0,
            highlight_color: RgbaColor::YELLOW_HL,
            extracted: HashMap::new(),
            text_sel: None,
            drag: DragMode::None,
            draft_points: Vec::new(),
            draft_shape: None,
            editing_text_id: None,
            text_edit_draft: None,
            pdf_text_edit_draft: None,
            clipboard_objects: Vec::new(),
            last_autosave: None,
            autosave_store: None,
            search_open: false,
            search_query: String::new(),
            status_note: String::new(),
            snap_guides: Vec::new(),
            crop_mode: false,
        }
    }
}

impl EditorState {
    pub fn reset_for_document(&mut self, journal: EditJournal, pdf_path: Option<&std::path::Path>) {
        self.scene.clear();
        self.journal = Some(journal);
        self.selected = None;
        self.extracted.clear();
        self.text_sel = None;
        self.drag = DragMode::None;
        self.editing_text_id = None;
        self.text_edit_draft = None;
        self.pdf_text_edit_draft = None;
        self.autosave_store = pdf_path.map(AutosaveStore::for_document);
        self.last_autosave = None;
        // Offer recovery
        if let Some(store) = &self.autosave_store {
            if let Ok(Some(snap)) = store.load() {
                self.scene = snap.scene;
                self.status_note = format!(
                    "Recovered autosave rev {} — review overlays, then Save",
                    snap.revision
                );
            }
        }
    }

    pub fn apply(&mut self, cmd: EditCommand) -> bool {
        let Some(journal) = self.journal.as_mut() else {
            return false;
        };
        journal.apply(&mut self.scene, cmd)
    }

    pub fn undo(&mut self) -> Option<EditCommand> {
        let journal = self.journal.as_mut()?;
        journal.undo(&mut self.scene).ok().flatten()
    }

    pub fn redo(&mut self) -> Option<EditCommand> {
        let journal = self.journal.as_mut()?;
        journal.redo(&mut self.scene).ok().flatten()
    }

    pub fn maybe_autosave(&mut self, model: &doxo_document::PdfDocumentModel) {
        if !self.scene.dirty {
            return;
        }
        let now = Instant::now();
        if let Some(last) = self.last_autosave {
            if now.duration_since(last) < Duration::from_secs(5) {
                return;
            }
        }
        let Some(store) = &self.autosave_store else {
            return;
        };
        let Some(journal) = &self.journal else {
            return;
        };
        let cmds = journal.entries_for_autosave();
        if store
            .save(model, &self.scene, &cmds, journal.revision)
            .is_ok()
        {
            self.last_autosave = Some(now);
            self.scene.dirty = false; // journal still dirty conceptually; OK for V1
            self.scene.dirty = true; // keep dirty until real save
        }
    }

    pub fn clear_autosave(&self) {
        if let Some(store) = &self.autosave_store {
            let _ = store.clear();
        }
    }

    pub fn delete_selected(&mut self) {
        if let Some(id) = self.selected.take() {
            self.apply(EditCommand::RemoveObject { id });
        }
    }

    pub fn copy_selected(&mut self) {
        if let Some(id) = self.selected {
            if let Some(obj) = self.scene.find(id) {
                self.clipboard_objects = vec![obj.clone()];
            }
        }
    }

    pub fn paste_on_page(&mut self, page: PageId) {
        let objs = self.clipboard_objects.clone();
        for mut obj in objs {
            // New ids + slight offset
            match &mut obj {
                EditorObject::TextBox { id, page: p, rect, .. }
                | EditorObject::VisualTextPatch {
                    id,
                    page: p,
                    cover: rect,
                    ..
                }
                | EditorObject::NativeTextReplace {
                    id,
                    page: p,
                    cover: rect,
                    ..
                }
                | EditorObject::Image { id, page: p, rect, .. }
                | EditorObject::Shape { id, page: p, rect, .. } => {
                    *id = ObjectId::new();
                    *p = page;
                    *rect = rect.translated(12.0, -12.0);
                }
                EditorObject::InkStroke {
                    id,
                    page: p,
                    points,
                    ..
                } => {
                    *id = ObjectId::new();
                    *p = page;
                    for pt in points.iter_mut() {
                        pt[0] += 12.0;
                        pt[1] -= 12.0;
                    }
                }
                EditorObject::Highlight { id, page: p, quads, .. } => {
                    *id = ObjectId::new();
                    *p = page;
                    for q in quads.iter_mut() {
                        *q = q.translated(12.0, -12.0);
                    }
                }
            }
            self.apply(EditCommand::AddObject { object: obj });
        }
    }

    pub fn finish_ink(&mut self, page: PageId) {
        let pts = simplify_stroke(&self.draft_points, 1.5);
        self.draft_points.clear();
        self.drag = DragMode::None;
        if pts.len() < 2 {
            return;
        }
        let obj = make_ink(page, pts, self.ink_color, self.ink_width);
        self.selected = Some(obj.id());
        self.apply(EditCommand::AddObject { object: obj });
    }

    pub fn finish_shape(&mut self, page: PageId) {
        let Some(rect) = self.draft_shape.take() else {
            self.drag = DragMode::None;
            return;
        };
        self.drag = DragMode::None;
        let Some(kind) = self.tool.shape_kind() else {
            return;
        };
        let obj = make_shape(page, kind, rect, self.ink_color, None);
        self.selected = Some(obj.id());
        self.apply(EditCommand::AddObject { object: obj });
    }

    pub fn add_textbox_at(&mut self, page: PageId, x: f32, y: f32) {
        let shaped = doxo_text_engine::shape_text(page, "New text", self.text_style.clone());
        let rect = RectPts {
            x,
            y: y - shaped.height_pts,
            w: shaped.width_pts.max(80.0),
            h: shaped.height_pts,
        };
        let obj = make_textbox(page, rect, "New text".into(), self.text_style.clone());
        let id = obj.id();
        self.apply(EditCommand::AddObject { object: obj });
        self.begin_overlay_text_edit(id);
        self.tool = EditorTool::Select;
    }

    pub fn replace_selected_text(&mut self, replacement: String) {
        let Some((page, start, end)) = self.text_sel else {
            return;
        };
        let Some(extracted) = self.extracted.get(&page.0) else {
            return;
        };
        let selected = extracted.text_range(start, end);
        if selected.is_empty() {
            return;
        }
        let quads = extracted.range_bounds(start, end);
        let Some(cover0) = quads.first().copied() else {
            return;
        };
        let mut cover = cover0;
        for q in &quads[1..] {
            let max_x = (cover.x + cover.w).max(q.x + q.w);
            let max_y = (cover.y + cover.h).max(q.y + q.h);
            cover.x = cover.x.min(q.x);
            cover.y = cover.y.min(q.y);
            cover.w = max_x - cover.x;
            cover.h = max_y - cover.y;
        }
        let mut style = self.text_style.clone();
        if let Some(ch) = extracted.chars.get(start) {
            style.font_size = ch.font_size;
        }

        // Level-2 when selection sits on one line we can reconstruct.
        let (line_original, selected_span) =
            if let Some((_, _, line)) = extracted.line_span_containing(start, end) {
                if line.contains(&selected) {
                    (Some(line), Some(selected.clone()))
                } else {
                    (None, None)
                }
            } else {
                (None, None)
            };

        self.apply(EditCommand::ReplaceExistingText {
            page,
            cover,
            text: replacement,
            style,
            line_original,
            selected: selected_span,
        });
        self.text_sel = None;
    }

    /// Begin in-place edit of an overlay text object via a draft buffer.
    pub fn begin_overlay_text_edit(&mut self, id: ObjectId) {
        let draft = match self.scene.find(id) {
            Some(EditorObject::TextBox { text, style, .. })
            | Some(EditorObject::VisualTextPatch { text, style, .. }) => {
                Some((id, text.clone(), style.clone()))
            }
            Some(EditorObject::NativeTextReplace {
                replacement,
                style,
                ..
            }) => Some((id, replacement.clone(), style.clone())),
            _ => None,
        };
        if let Some(d) = draft {
            self.selected = Some(id);
            self.editing_text_id = Some(id);
            self.text_edit_draft = Some(d);
            self.pdf_text_edit_draft = None;
        }
    }

    /// Begin editing the current PDF text selection (visual/native replace on commit).
    pub fn begin_pdf_text_edit(&mut self) {
        let Some((page, start, end)) = self.text_sel else {
            return;
        };
        let Some(extracted) = self.extracted.get(&page.0) else {
            return;
        };
        let selected = extracted.text_range(start, end);
        if selected.is_empty() {
            return;
        }
        self.editing_text_id = None;
        self.text_edit_draft = None;
        self.pdf_text_edit_draft = Some(selected);
    }

    pub fn commit_overlay_text_edit(&mut self) {
        let Some((id, text, style)) = self.text_edit_draft.take() else {
            self.editing_text_id = None;
            return;
        };
        self.apply(EditCommand::SetTextContent { id, text });
        self.apply(EditCommand::SetTextStyle { id, style });
        self.editing_text_id = None;
    }

    pub fn cancel_text_edit(&mut self) {
        self.editing_text_id = None;
        self.text_edit_draft = None;
        self.pdf_text_edit_draft = None;
    }

    pub fn commit_pdf_text_edit(&mut self) {
        let Some(text) = self.pdf_text_edit_draft.take() else {
            return;
        };
        self.replace_selected_text(text);
    }

    pub fn rotate_selected_image(&mut self, delta_deg: f32) {
        let Some(id) = self.selected else { return };
        if let Some(EditorObject::Image {
            rect,
            rotation_deg,
            opacity,
            crop,
            ..
        }) = self.scene.find(id).cloned()
        {
            self.apply(EditCommand::SetImageTransform {
                id,
                rect,
                rotation_deg: rotation_deg + delta_deg,
                opacity,
                crop,
            });
        }
    }

    pub fn nudge_selected_crop(&mut self, edge: CropEdge, delta: f32) {
        let Some(id) = self.selected else { return };
        if let Some(EditorObject::Image {
            rect,
            rotation_deg,
            opacity,
            crop,
            ..
        }) = self.scene.find(id).cloned()
        {
            let mut c = crop.unwrap_or([0.0, 0.0, 1.0, 1.0]);
            match edge {
                CropEdge::Left => c[0] = (c[0] + delta).clamp(0.0, c[2] - 0.05),
                CropEdge::Top => c[1] = (c[1] + delta).clamp(0.0, c[3] - 0.05),
                CropEdge::Right => c[2] = (c[2] + delta).clamp(c[0] + 0.05, 1.0),
                CropEdge::Bottom => c[3] = (c[3] + delta).clamp(c[1] + 0.05, 1.0),
            }
            self.apply(EditCommand::SetImageTransform {
                id,
                rect,
                rotation_deg,
                opacity,
                crop: Some(c),
            });
        }
    }

    pub fn reset_selected_crop(&mut self) {
        let Some(id) = self.selected else { return };
        if let Some(EditorObject::Image {
            rect,
            rotation_deg,
            opacity,
            ..
        }) = self.scene.find(id).cloned()
        {
            self.apply(EditCommand::SetImageTransform {
                id,
                rect,
                rotation_deg,
                opacity,
                crop: None,
            });
        }
    }

    pub fn highlight_selection(&mut self) {
        let Some((page, start, end)) = self.text_sel else {
            return;
        };
        let Some(extracted) = self.extracted.get(&page.0) else {
            return;
        };
        let quads = extracted.range_bounds(start, end);
        if quads.is_empty() {
            return;
        }
        let obj = make_highlight(page, quads, self.highlight_color, true);
        self.apply(EditCommand::AddObject { object: obj });
    }

    pub fn paste_image(&mut self, page: PageId, bytes: Vec<u8>) {
        let format = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
            ImageFormat::Png
        } else {
            ImageFormat::Jpeg
        };
        let (w, h) = image::load_from_memory(&bytes)
            .ok()
            .map(|i| (i.width() as f32, i.height() as f32))
            .unwrap_or((200.0, 200.0));
        let scale = (200.0 / w.max(1.0)).min(1.0);
        let rect = RectPts {
            x: 72.0,
            y: 400.0,
            w: w * scale,
            h: h * scale,
        };
        let obj = make_image(page, rect, bytes, format);
        self.selected = Some(obj.id());
        self.apply(EditCommand::AddObject { object: obj });
    }
}

#[derive(Debug, Clone, Copy)]
pub enum CropEdge {
    Left,
    Top,
    Right,
    Bottom,
}
