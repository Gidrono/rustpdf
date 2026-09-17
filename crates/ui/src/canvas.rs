use doxo_document::{PageId, PdfDocumentModel, RectPts};
use doxo_graphics::normalize_shape_rect;
use egui::{Color32, Pos2, Rect, Sense, Ui, Vec2};

use crate::app_state::PageMap;
use crate::editor::{DragMode, EditorState};
use crate::overlay::{paint_overlays, screen_to_pdf};
use crate::tools::EditorTool;

const PAGE_GAP: f32 = 16.0;

#[derive(Default)]
pub struct CanvasView;

pub struct CanvasOutcome {
    pub visible: Vec<PageId>,
    pub near: Vec<PageId>,
    pub need_text: Vec<PageId>,
}

impl CanvasView {
    pub fn show(
        &mut self,
        ui: &mut Ui,
        document: Option<&PdfDocumentModel>,
        zoom: f32,
        scroll_y: &mut f32,
        current_page: &mut PageId,
        textures: &PageMap,
        editor: &mut EditorState,
    ) -> CanvasOutcome {
        let mut outcome = CanvasOutcome {
            visible: Vec::new(),
            near: Vec::new(),
            need_text: Vec::new(),
        };

        let Some(doc) = document else {
            ui.centered_and_justified(|ui| {
                ui.label("Open a PDF (⌘O)");
            });
            return outcome;
        };

        let scale = zoom * (96.0 / 72.0);
        let total_height: f32 = doc
            .pages
            .iter()
            .map(|p| p.height_pts * scale + PAGE_GAP)
            .sum::<f32>()
            + PAGE_GAP;

        let avail = ui.available_size();
        let (response, painter) = ui.allocate_painter(avail, Sense::click_and_drag());

        // Pan with middle mouse or space — otherwise tools own drag
        let pan = ui.input(|i| i.pointer.middle_down() || i.key_down(egui::Key::Space));
        if pan && response.dragged() {
            *scroll_y = (*scroll_y - response.drag_delta().y / zoom.max(0.1)).max(0.0);
        }

        let max_scroll = (total_height - avail.y).max(0.0) / scale;
        *scroll_y = scroll_y.clamp(0.0, max_scroll.max(0.0));

        let origin = response.rect.min;
        let view_top = *scroll_y * scale;
        let view_bottom = view_top + avail.y;

        painter.rect_filled(response.rect, 0.0, Color32::from_rgb(48, 48, 52));

        let mut y = PAGE_GAP;
        let mut first_visible: Option<PageId> = None;
        let mut page_screens: Vec<(PageId, Rect, f32, f32)> = Vec::new();

        for page in &doc.pages {
            let w = page.width_pts * scale;
            let h = page.height_pts * scale;
            let page_top = y;
            let page_bottom = y + h;

            if page_bottom >= view_top && page_top <= view_bottom {
                outcome.visible.push(page.id);
                if first_visible.is_none() {
                    first_visible = Some(page.id);
                }
                if !editor.extracted.contains_key(&page.id.0) {
                    outcome.need_text.push(page.id);
                }

                let x = origin.x + ((avail.x - w) * 0.5).max(8.0);
                let screen_y = origin.y + (page_top - view_top);
                let rect = Rect::from_min_size(Pos2::new(x, screen_y), Vec2::new(w, h));
                page_screens.push((page.id, rect, page.width_pts, page.height_pts));

                painter.rect_filled(
                    rect.translate(Vec2::new(2.0, 3.0)),
                    2.0,
                    Color32::from_black_alpha(60),
                );

                if let Some(tex) = textures.get(&(doc.id, page.id)) {
                    painter.image(
                        tex.texture().id(),
                        rect,
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );
                } else {
                    painter.rect_filled(rect, 0.0, Color32::from_rgb(230, 230, 225));
                    painter.text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        format!("Page {}", page.id.0 + 1),
                        egui::FontId::proportional(14.0),
                        Color32::DARK_GRAY,
                    );
                }
            }
            y += h + PAGE_GAP;
        }

        if let Some(p) = first_visible {
            *current_page = p;
        }

        // Prefetch
        if let (Some(first), Some(last)) = (outcome.visible.first(), outcome.visible.last()) {
            let start = first.0.saturating_sub(2);
            let end = (last.0 + 2).min(doc.page_count.saturating_sub(1));
            for idx in start..=end {
                let pid = PageId(idx);
                if !outcome.visible.contains(&pid) {
                    outcome.near.push(pid);
                }
            }
        }

        // Interact with topmost visible page under pointer
        let pointer = response.interact_pointer_pos();
        let mut active: Option<(PageId, Rect, f32, f32)> = None;
        if let Some(pos) = pointer {
            for (pid, rect, pw, ph) in &page_screens {
                if rect.contains(pos) {
                    active = Some((*pid, *rect, *pw, *ph));
                    break;
                }
            }
        }

        // Paint overlays for visible pages
        for (pid, rect, pw, ph) in &page_screens {
            let text_quads = if let Some((sp, a, b)) = editor.text_sel {
                if sp == *pid {
                    editor
                        .extracted
                        .get(&pid.0)
                        .map(|t| t.range_bounds(a, b))
                        .unwrap_or_default()
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };
            paint_overlays(
                ui,
                *rect,
                *pid,
                *pw,
                *ph,
                &editor.scene,
                editor.selected,
                &editor.draft_points,
                editor.draft_shape,
                &text_quads,
                &editor.snap_guides,
            );
        }

        if !pan {
            handle_tool_input(ui, &response, active, editor);
        }

        outcome
    }
}

fn handle_tool_input(
    ui: &mut Ui,
    response: &egui::Response,
    active: Option<(PageId, Rect, f32, f32)>,
    editor: &mut EditorState,
) {
    let Some((page, page_rect, page_w, page_h)) = active else {
        if response.drag_stopped() {
            match editor.drag {
                DragMode::DrawInk => editor.finish_ink(editor.text_sel.map(|(p, _, _)| p).unwrap_or(PageId(0))),
                DragMode::DrawShape { .. } => {
                    // need page — finish with current_page not available; leave draft
                }
                _ => editor.drag = DragMode::None,
            }
        }
        return;
    };

    let pos = response.interact_pointer_pos();
    let Some(pos) = pos else { return };
    let (pdf_x, pdf_y) = screen_to_pdf(page_rect, page_w, page_h, pos);

    match editor.tool {
        EditorTool::Select => {
            // Triple / double / single click, then drag. Order matters: multi-click first.
            if response.triple_clicked() {
                select_begin_at(editor, page, pdf_x, pdf_y, SelectClickKind::Triple);
            } else if response.double_clicked() {
                select_begin_at(editor, page, pdf_x, pdf_y, SelectClickKind::Double);
            } else if response.clicked() || response.drag_started() {
                select_begin_at(editor, page, pdf_x, pdf_y, SelectClickKind::Single);
            }

            if response.dragged() {
                match editor.drag {
                    DragMode::MoveObject { id, last } => {
                        let raw_dx = pdf_x - last.0;
                        let raw_dy = pdf_y - last.1;
                        if raw_dx.abs() + raw_dy.abs() > 0.1 {
                            if let Some(obj) = editor.scene.find(id) {
                                let moving = obj.bounds();
                                let others: Vec<_> = editor
                                    .scene
                                    .objects_on_page(page)
                                    .filter(|o| o.id() != id)
                                    .map(|o| o.bounds())
                                    .collect();
                                let snap = doxo_graphics::snap_rect_move(
                                    moving, raw_dx, raw_dy, page_w, page_h, &others,
                                );
                                editor.snap_guides =
                                    snap.guides.into_iter().flatten().collect();
                                editor.apply(doxo_document::EditCommand::MoveObject {
                                    id,
                                    dx: snap.dx,
                                    dy: snap.dy,
                                });
                                editor.drag = DragMode::MoveObject {
                                    id,
                                    last: (last.0 + snap.dx, last.1 + snap.dy),
                                };
                            }
                        }
                    }
                    DragMode::ResizeObject { id } => {
                        if let Some(obj) = editor.scene.find(id) {
                            let b = obj.bounds();
                            let new_rect = RectPts {
                                x: b.x,
                                y: pdf_y.min(b.y + b.h - 10.0),
                                w: (pdf_x - b.x).max(10.0),
                                h: (b.y + b.h - pdf_y).max(10.0),
                            };
                            editor.apply(doxo_document::EditCommand::SetObjectRect {
                                id,
                                rect: new_rect,
                            });
                        }
                    }
                    DragMode::SelectText { anchor } => {
                        if let Some(extracted) = editor.extracted.get(&page.0) {
                            if let Some(idx) = extracted.nearest_char_index(pdf_x, pdf_y) {
                                let (a, b) = if idx >= anchor {
                                    (anchor, idx + 1)
                                } else {
                                    (idx, anchor + 1)
                                };
                                editor.text_sel = Some((page, a, b));
                            }
                        }
                    }
                    _ => {}
                }
            }
            if response.drag_stopped() {
                editor.drag = DragMode::None;
                editor.snap_guides.clear();
            }
        }
        EditorTool::Text => {
            if response.clicked() {
                editor.add_textbox_at(page, pdf_x, pdf_y);
            }
        }
        EditorTool::Pencil => {
            if response.drag_started() {
                editor.drag = DragMode::DrawInk;
                editor.draft_points.clear();
                editor.draft_points.push([pdf_x, pdf_y]);
            }
            if response.dragged() && matches!(editor.drag, DragMode::DrawInk) {
                editor.draft_points.push([pdf_x, pdf_y]);
            }
            if response.drag_stopped() && matches!(editor.drag, DragMode::DrawInk) {
                editor.finish_ink(page);
            }
        }
        EditorTool::Highlight => {
            if response.drag_started() {
                if let Some(extracted) = editor.extracted.get(&page.0) {
                    if let Some(idx) = extracted.nearest_char_index(pdf_x, pdf_y) {
                        editor.drag = DragMode::DrawHighlight { start_char: idx };
                        editor.text_sel = Some((page, idx, idx + 1));
                    } else {
                        // Free highlight
                        editor.drag = DragMode::DrawShape {
                            start: (pdf_x, pdf_y),
                        };
                    }
                }
            }
            if response.dragged() {
                match editor.drag {
                    DragMode::DrawHighlight { start_char } => {
                        if let Some(extracted) = editor.extracted.get(&page.0) {
                            if let Some(idx) = extracted.nearest_char_index(pdf_x, pdf_y) {
                                let (a, b) = if idx >= start_char {
                                    (start_char, idx + 1)
                                } else {
                                    (idx, start_char + 1)
                                };
                                editor.text_sel = Some((page, a, b));
                            }
                        }
                    }
                    DragMode::DrawShape { start } => {
                        editor.draft_shape =
                            Some(normalize_shape_rect(start.0, start.1, pdf_x, pdf_y));
                    }
                    _ => {}
                }
            }
            if response.drag_stopped() {
                match editor.drag {
                    DragMode::DrawHighlight { .. } => {
                        editor.highlight_selection();
                        editor.text_sel = None;
                    }
                    DragMode::DrawShape { .. } => {
                        if let Some(rect) = editor.draft_shape.take() {
                            let obj = doxo_document::make_highlight(
                                page,
                                vec![rect],
                                editor.highlight_color,
                                false,
                            );
                            editor.apply(doxo_document::EditCommand::AddObject { object: obj });
                        }
                    }
                    _ => {}
                }
                editor.drag = DragMode::None;
            }
        }
        EditorTool::Eraser => {
            if response.clicked() {
                if let Some(id) = editor.scene.hit_test_top(page, pdf_x, pdf_y) {
                    editor.apply(doxo_document::EditCommand::RemoveObject { id });
                    if editor.selected == Some(id) {
                        editor.selected = None;
                    }
                }
            }
        }
        EditorTool::Rectangle | EditorTool::Ellipse | EditorTool::Line | EditorTool::Arrow => {
            if response.drag_started() {
                editor.drag = DragMode::DrawShape {
                    start: (pdf_x, pdf_y),
                };
            }
            if response.dragged() {
                if let DragMode::DrawShape { start } = editor.drag {
                    editor.draft_shape = Some(normalize_shape_rect(start.0, start.1, pdf_x, pdf_y));
                }
            }
            if response.drag_stopped() {
                editor.finish_shape(page);
            }
        }
        EditorTool::Image => {
            if response.clicked() {
                if let Some(bytes) = doxo_platform_macos::clipboard_image_png() {
                    editor.paste_image(page, bytes);
                } else if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Images", &["png", "jpg", "jpeg"])
                    .pick_file()
                {
                    if let Ok(bytes) = std::fs::read(path) {
                        editor.paste_image(page, bytes);
                    }
                }
                editor.tool = EditorTool::Select;
            }
        }
    }

    let _ = ui;
}

#[derive(Clone, Copy)]
enum SelectClickKind {
    Single,
    Double,
    Triple,
}

fn select_begin_at(
    editor: &mut EditorState,
    page: PageId,
    pdf_x: f32,
    pdf_y: f32,
    kind: SelectClickKind,
) {
    // Overlay objects (excluding highlights) win over PDF text.
    if let Some(id) = editor.scene.hit_test_top_for_select(page, pdf_x, pdf_y) {
        editor.text_sel = None;
        editor.pdf_text_edit_draft = None;
        editor.selected = Some(id);
        match kind {
            SelectClickKind::Double | SelectClickKind::Triple => {
                if matches!(
                    editor.scene.find(id),
                    Some(doxo_document::EditorObject::TextBox { .. })
                        | Some(doxo_document::EditorObject::VisualTextPatch { .. })
                        | Some(doxo_document::EditorObject::NativeTextReplace { .. })
                ) {
                    editor.begin_overlay_text_edit(id);
                    editor.drag = DragMode::None;
                    return;
                }
            }
            SelectClickKind::Single => {}
        }
        if let Some(obj) = editor.scene.find(id) {
            let is_image = matches!(obj, doxo_document::EditorObject::Image { .. });
            let b = obj.bounds();
            let handle = RectPts {
                x: b.x + b.w - 8.0,
                y: b.y,
                w: 12.0,
                h: 12.0,
            };
            if is_image && handle.contains(pdf_x, pdf_y) {
                editor.drag = DragMode::ResizeObject { id };
            } else {
                editor.drag = DragMode::MoveObject {
                    id,
                    last: (pdf_x, pdf_y),
                };
            }
        }
        return;
    }

    // PDF text selection
    if let Some(extracted) = editor.extracted.get(&page.0) {
        if let Some(idx) = extracted.nearest_char_index(pdf_x, pdf_y) {
            editor.selected = None;
            match kind {
                SelectClickKind::Triple => {
                    if let Some((a, b, _)) = extracted.line_span_containing(idx, idx + 1) {
                        editor.text_sel = Some((page, a, b));
                        editor.drag = DragMode::None;
                        editor.begin_pdf_text_edit();
                    } else {
                        editor.text_sel = Some((page, idx, idx + 1));
                        editor.drag = DragMode::SelectText { anchor: idx };
                    }
                }
                SelectClickKind::Double => {
                    if let Some((a, b)) = extracted.word_span_at(idx) {
                        editor.text_sel = Some((page, a, b));
                        editor.drag = DragMode::None;
                        editor.begin_pdf_text_edit();
                    } else {
                        editor.text_sel = Some((page, idx, idx + 1));
                        editor.drag = DragMode::SelectText { anchor: idx };
                    }
                }
                SelectClickKind::Single => {
                    editor.pdf_text_edit_draft = None;
                    editor.text_sel = Some((page, idx, idx + 1));
                    editor.drag = DragMode::SelectText { anchor: idx };
                }
            }
            return;
        }
    }

    // Fallback: select highlight if present, else clear.
    if let Some(id) = editor.scene.hit_test_highlight(page, pdf_x, pdf_y) {
        editor.selected = Some(id);
        editor.text_sel = None;
        editor.pdf_text_edit_draft = None;
        editor.drag = DragMode::MoveObject {
            id,
            last: (pdf_x, pdf_y),
        };
    } else {
        editor.selected = None;
        editor.text_sel = None;
        editor.pdf_text_edit_draft = None;
        editor.drag = DragMode::None;
    }
}
