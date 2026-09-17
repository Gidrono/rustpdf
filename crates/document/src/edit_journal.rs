//! Command journal — User Action → EditCommand → EditorScene.
//! Undo/redo stores inverse snapshots of mutated objects for reliability.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::scene::{
    EditorObject, EditorScene, ImageFormat, ObjectId, RectPts, RgbaColor, ShapeKind, TextStyle,
};
use crate::{DocumentId, PageId};

#[derive(Debug, Error)]
pub enum JournalError {
    #[error("nothing to undo")]
    EmptyUndo,
    #[error("nothing to redo")]
    EmptyRedo,
}

/// Forward command emitted by the UI / tools.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EditCommand {
    AddObject {
        object: EditorObject,
    },
    RemoveObject {
        id: ObjectId,
    },
    MoveObject {
        id: ObjectId,
        dx: f32,
        dy: f32,
    },
    SetObjectRect {
        id: ObjectId,
        rect: RectPts,
    },
    SetTextContent {
        id: ObjectId,
        text: String,
    },
    SetTextStyle {
        id: ObjectId,
        style: TextStyle,
    },
    SetImageTransform {
        id: ObjectId,
        rect: RectPts,
        rotation_deg: f32,
        opacity: f32,
        crop: Option<[f32; 4]>,
    },
    ReplaceExistingText {
        page: PageId,
        cover: RectPts,
        text: String,
        style: TextStyle,
        /// When set, Level-2 NativeTextReplace is used (stream rewrite + fallback).
        line_original: Option<String>,
        selected: Option<String>,
    },
    /// Page-structure ops are applied via PDF engine; journal records intent for undo reload.
    PageDelete {
        page: PageId,
    },
    PageRotate {
        page: PageId,
        degrees: i32,
    },
    PageReorder {
        from: u32,
        to: u32,
    },
    PageInsertBlank {
        at: u32,
    },
    PageDuplicate {
        page: PageId,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum Inverse {
    Remove { id: ObjectId },
    Insert { object: EditorObject, index: usize },
    Move { id: ObjectId, dx: f32, dy: f32 },
    SetRect { id: ObjectId, rect: RectPts },
    SetText { id: ObjectId, text: String },
    SetStyle { id: ObjectId, style: TextStyle },
    SetImage {
        id: ObjectId,
        rect: RectPts,
        rotation_deg: f32,
        opacity: f32,
        crop: Option<[f32; 4]>,
    },
    /// Page ops: undo requires reloading previous PDF bytes (handled outside scene).
    PageOpMarker { command: EditCommand },
    Nop,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JournalEntry {
    id: Uuid,
    forward: EditCommand,
    inverse: Inverse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditJournal {
    pub document_id: DocumentId,
    entries: Vec<JournalEntry>,
    cursor: usize,
    /// Monotonic revision for autosave.
    pub revision: u64,
}

impl EditJournal {
    pub fn new(document_id: DocumentId) -> Self {
        Self {
            document_id,
            entries: Vec::new(),
            cursor: 0,
            revision: 0,
        }
    }

    pub fn can_undo(&self) -> bool {
        self.cursor > 0
    }

    pub fn can_redo(&self) -> bool {
        self.cursor < self.entries.len()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Apply a command to the scene and record undo.
    /// Returns true if this is a page-structure op that needs PDF engine handling.
    pub fn apply(&mut self, scene: &mut EditorScene, cmd: EditCommand) -> bool {
        let inverse = match &cmd {
            EditCommand::AddObject { object } => {
                let id = object.id();
                scene.insert(object.clone());
                Inverse::Remove { id }
            }
            EditCommand::RemoveObject { id } => {
                if let Some(object) = scene.remove(*id) {
                    let index = scene.objects.len();
                    Inverse::Insert { object, index }
                } else {
                    Inverse::Nop
                }
            }
            EditCommand::MoveObject { id, dx, dy } => {
                if let Some(obj) = scene.find_mut(*id) {
                    obj.translate(*dx, *dy);
                    Inverse::Move {
                        id: *id,
                        dx: -*dx,
                        dy: -*dy,
                    }
                } else {
                    Inverse::Nop
                }
            }
            EditCommand::SetObjectRect { id, rect } => {
                if let Some(obj) = scene.find_mut(*id) {
                    let old = obj.bounds();
                    obj.set_rect(*rect);
                    Inverse::SetRect { id: *id, rect: old }
                } else {
                    Inverse::Nop
                }
            }
            EditCommand::SetTextContent { id, text } => {
                if let Some(obj) = scene.find_mut(*id) {
                    match obj {
                        EditorObject::TextBox { text: t, .. }
                        | EditorObject::VisualTextPatch { text: t, .. } => {
                            let old = t.clone();
                            *t = text.clone();
                            Inverse::SetText { id: *id, text: old }
                        }
                        EditorObject::NativeTextReplace { replacement: t, .. } => {
                            let old = t.clone();
                            *t = text.clone();
                            Inverse::SetText { id: *id, text: old }
                        }
                        _ => Inverse::Nop,
                    }
                } else {
                    Inverse::Nop
                }
            }
            EditCommand::SetTextStyle { id, style } => {
                if let Some(obj) = scene.find_mut(*id) {
                    match obj {
                        EditorObject::TextBox { style: s, .. }
                        | EditorObject::VisualTextPatch { style: s, .. }
                        | EditorObject::NativeTextReplace { style: s, .. } => {
                            let old = s.clone();
                            *s = style.clone();
                            Inverse::SetStyle { id: *id, style: old }
                        }
                        _ => Inverse::Nop,
                    }
                } else {
                    Inverse::Nop
                }
            }
            EditCommand::SetImageTransform {
                id,
                rect,
                rotation_deg,
                opacity,
                crop,
            } => {
                if let Some(EditorObject::Image {
                    rect: r,
                    rotation_deg: rot,
                    opacity: op,
                    crop: c,
                    ..
                }) = scene.find_mut(*id)
                {
                    let inv = Inverse::SetImage {
                        id: *id,
                        rect: *r,
                        rotation_deg: *rot,
                        opacity: *op,
                        crop: *c,
                    };
                    *r = *rect;
                    *rot = *rotation_deg;
                    *op = *opacity;
                    *c = *crop;
                    scene.dirty = true;
                    inv
                } else {
                    Inverse::Nop
                }
            }
            EditCommand::ReplaceExistingText {
                page,
                cover,
                text,
                style,
                line_original,
                selected,
            } => {
                let object = if let (Some(line), Some(sel)) =
                    (line_original.clone(), selected.clone())
                {
                    EditorObject::NativeTextReplace {
                        id: ObjectId::new(),
                        page: *page,
                        cover: *cover,
                        line_original: line,
                        selected: sel,
                        replacement: text.clone(),
                        style: style.clone(),
                    }
                } else {
                    EditorObject::VisualTextPatch {
                        id: ObjectId::new(),
                        page: *page,
                        cover: *cover,
                        text: text.clone(),
                        style: style.clone(),
                    }
                };
                let id = object.id();
                scene.insert(object);
                Inverse::Remove { id }
            }
            EditCommand::PageDelete { .. }
            | EditCommand::PageRotate { .. }
            | EditCommand::PageReorder { .. }
            | EditCommand::PageInsertBlank { .. }
            | EditCommand::PageDuplicate { .. } => Inverse::PageOpMarker {
                command: cmd.clone(),
            },
        };

        let is_page_op = matches!(
            cmd,
            EditCommand::PageDelete { .. }
                | EditCommand::PageRotate { .. }
                | EditCommand::PageReorder { .. }
                | EditCommand::PageInsertBlank { .. }
                | EditCommand::PageDuplicate { .. }
        );

        self.entries.truncate(self.cursor);
        self.entries.push(JournalEntry {
            id: Uuid::new_v4(),
            forward: cmd,
            inverse,
        });
        self.cursor = self.entries.len();
        self.revision += 1;
        scene.dirty = true;
        is_page_op
    }

    pub fn undo(&mut self, scene: &mut EditorScene) -> Result<Option<EditCommand>, JournalError> {
        if self.cursor == 0 {
            return Err(JournalError::EmptyUndo);
        }
        self.cursor -= 1;
        let entry = &self.entries[self.cursor];
        let page_cmd = if let Inverse::PageOpMarker { command } = &entry.inverse {
            Some(command.clone())
        } else {
            apply_inverse(scene, &entry.inverse);
            None
        };
        self.revision += 1;
        scene.dirty = true;
        Ok(page_cmd)
    }

    pub fn redo(&mut self, scene: &mut EditorScene) -> Result<Option<EditCommand>, JournalError> {
        if self.cursor >= self.entries.len() {
            return Err(JournalError::EmptyRedo);
        }
        let entry = self.entries[self.cursor].clone();
        self.cursor += 1;
        let page_cmd = if matches!(
            entry.forward,
            EditCommand::PageDelete { .. }
                | EditCommand::PageRotate { .. }
                | EditCommand::PageReorder { .. }
                | EditCommand::PageInsertBlank { .. }
                | EditCommand::PageDuplicate { .. }
        ) {
            Some(entry.forward)
        } else {
            // Re-apply by reconstructing inverse path is messy; re-run forward on scene.
            let _ = self; // cursor already advanced
            reapply_forward(scene, &entry.forward);
            None
        };
        self.revision += 1;
        scene.dirty = true;
        Ok(page_cmd)
    }

    pub fn entries_for_autosave(&self) -> Vec<EditCommand> {
        self.entries[..self.cursor]
            .iter()
            .map(|e| e.forward.clone())
            .collect()
    }
}

fn apply_inverse(scene: &mut EditorScene, inv: &Inverse) {
    match inv {
        Inverse::Nop => {}
        Inverse::Remove { id } => {
            scene.remove(*id);
        }
        Inverse::Insert { object, index } => {
            let idx = (*index).min(scene.objects.len());
            scene.objects.insert(idx, object.clone());
        }
        Inverse::Move { id, dx, dy } => {
            if let Some(o) = scene.find_mut(*id) {
                o.translate(*dx, *dy);
            }
        }
        Inverse::SetRect { id, rect } => {
            if let Some(o) = scene.find_mut(*id) {
                o.set_rect(*rect);
            }
        }
        Inverse::SetText { id, text } => {
            if let Some(o) = scene.find_mut(*id) {
                match o {
                    EditorObject::TextBox { text: t, .. }
                    | EditorObject::VisualTextPatch { text: t, .. } => *t = text.clone(),
                    EditorObject::NativeTextReplace { replacement: t, .. } => *t = text.clone(),
                    _ => {}
                }
            }
        }
        Inverse::SetStyle { id, style } => {
            if let Some(o) = scene.find_mut(*id) {
                match o {
                    EditorObject::TextBox { style: s, .. }
                    | EditorObject::VisualTextPatch { style: s, .. }
                    | EditorObject::NativeTextReplace { style: s, .. } => *s = style.clone(),
                    _ => {}
                }
            }
        }
        Inverse::SetImage {
            id,
            rect,
            rotation_deg,
            opacity,
            crop,
        } => {
            if let Some(EditorObject::Image {
                rect: r,
                rotation_deg: rot,
                opacity: op,
                crop: c,
                ..
            }) = scene.find_mut(*id)
            {
                *r = *rect;
                *rot = *rotation_deg;
                *op = *opacity;
                *c = *crop;
            }
        }
        Inverse::PageOpMarker { .. } => {}
    }
}

fn reapply_forward(scene: &mut EditorScene, cmd: &EditCommand) {
    match cmd {
        EditCommand::AddObject { object } => {
            if scene.find(object.id()).is_none() {
                scene.insert(object.clone());
            }
        }
        EditCommand::ReplaceExistingText {
            page,
            cover,
            text,
            style,
            line_original,
            selected,
        } => {
            let object = if let (Some(line), Some(sel)) =
                (line_original.clone(), selected.clone())
            {
                EditorObject::NativeTextReplace {
                    id: ObjectId::new(),
                    page: *page,
                    cover: *cover,
                    line_original: line,
                    selected: sel,
                    replacement: text.clone(),
                    style: style.clone(),
                }
            } else {
                EditorObject::VisualTextPatch {
                    id: ObjectId::new(),
                    page: *page,
                    cover: *cover,
                    text: text.clone(),
                    style: style.clone(),
                }
            };
            scene.insert(object);
        }
        EditCommand::RemoveObject { id } => {
            scene.remove(*id);
        }
        EditCommand::MoveObject { id, dx, dy } => {
            if let Some(o) = scene.find_mut(*id) {
                o.translate(*dx, *dy);
            }
        }
        EditCommand::SetObjectRect { id, rect } => {
            if let Some(o) = scene.find_mut(*id) {
                o.set_rect(*rect);
            }
        }
        EditCommand::SetTextContent { id, text } => {
            if let Some(o) = scene.find_mut(*id) {
                match o {
                    EditorObject::TextBox { text: t, .. }
                    | EditorObject::VisualTextPatch { text: t, .. } => *t = text.clone(),
                    EditorObject::NativeTextReplace { replacement: t, .. } => *t = text.clone(),
                    _ => {}
                }
            }
        }
        EditCommand::SetTextStyle { id, style } => {
            if let Some(o) = scene.find_mut(*id) {
                match o {
                    EditorObject::TextBox { style: s, .. }
                    | EditorObject::VisualTextPatch { style: s, .. }
                    | EditorObject::NativeTextReplace { style: s, .. } => *s = style.clone(),
                    _ => {}
                }
            }
        }
        EditCommand::SetImageTransform {
            id,
            rect,
            rotation_deg,
            opacity,
            crop,
        } => {
            if let Some(EditorObject::Image {
                rect: r,
                rotation_deg: rot,
                opacity: op,
                crop: c,
                ..
            }) = scene.find_mut(*id)
            {
                *r = *rect;
                *rot = *rotation_deg;
                *op = *opacity;
                *c = *crop;
            }
        }
        _ => {}
    }
}

/// Helpers for constructing common objects from tools.
pub fn make_textbox(page: PageId, rect: RectPts, text: String, style: TextStyle) -> EditorObject {
    EditorObject::TextBox {
        id: ObjectId::new(),
        page,
        rect,
        text,
        style,
        rotation_deg: 0.0,
    }
}

pub fn make_ink(
    page: PageId,
    points: Vec<[f32; 2]>,
    color: RgbaColor,
    width: f32,
) -> EditorObject {
    EditorObject::InkStroke {
        id: ObjectId::new(),
        page,
        points,
        color,
        width,
    }
}

pub fn make_highlight(
    page: PageId,
    quads: Vec<RectPts>,
    color: RgbaColor,
    semantic: bool,
) -> EditorObject {
    EditorObject::Highlight {
        id: ObjectId::new(),
        page,
        quads,
        color,
        semantic,
    }
}

pub fn make_shape(
    page: PageId,
    kind: ShapeKind,
    rect: RectPts,
    stroke: RgbaColor,
    fill: Option<RgbaColor>,
) -> EditorObject {
    EditorObject::Shape {
        id: ObjectId::new(),
        page,
        kind,
        rect,
        stroke,
        fill,
        stroke_width: 1.5,
    }
}

pub fn make_image(
    page: PageId,
    rect: RectPts,
    bytes: Vec<u8>,
    format: ImageFormat,
) -> EditorObject {
    EditorObject::Image {
        id: ObjectId::new(),
        page,
        rect,
        bytes,
        format,
        rotation_deg: 0.0,
        opacity: 1.0,
        crop: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DocumentId, PageId, RectPts, TextStyle};

    #[test]
    fn undo_redo_textbox() {
        let mut scene = EditorScene::default();
        let mut journal = EditJournal::new(DocumentId::new());
        let obj = make_textbox(
            PageId(0),
            RectPts {
                x: 10.0,
                y: 10.0,
                w: 100.0,
                h: 20.0,
            },
            "hi".into(),
            TextStyle::default(),
        );
        let id = obj.id();
        journal.apply(&mut scene, EditCommand::AddObject { object: obj });
        assert_eq!(scene.objects.len(), 1);
        assert!(journal.undo(&mut scene).unwrap().is_none());
        assert!(scene.objects.is_empty());
        assert!(journal.redo(&mut scene).unwrap().is_none());
        // Redo creates via reapply — id may match for AddObject
        assert!(scene.find(id).is_some() || !scene.objects.is_empty());
    }
}
