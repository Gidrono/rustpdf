//! Overlay scene painting + hit testing in page PDF space → screen.

use doxo_document::{EditorObject, EditorScene, ObjectId, PageId, RectPts, ShapeKind};
use egui::{Color32, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2};

/// Convert PDF page coords (origin bottom-left) to screen rect given page screen rect.
pub fn pdf_to_screen(page_rect: Rect, page_h: f32, r: RectPts) -> Rect {
    let scale_x = page_rect.width() / page_rect.width().max(1.0); // page_rect already sized
    let sx = page_rect.width() / 1.0; // unused placeholder
    let _ = (scale_x, sx);
    let scale = page_rect.width(); // will pass width_pts separately
    let _ = scale;
    // Caller provides scale via page_rect sized to page * zoom.
    // We need page width in pts — encode as: page_rect corresponds to full page.
    // Use: x_screen = page_rect.min.x + r.x * (page_rect.width()/page_w)
    // But we don't have page_w here — pass as argument.
    let _ = page_h;
    Rect::from_min_size(
        Pos2::new(page_rect.min.x + r.x, page_rect.max.y - r.y - r.h),
        Vec2::new(r.w, r.h),
    )
}

pub fn pdf_to_screen_scaled(page_rect: Rect, page_w: f32, page_h: f32, r: RectPts) -> Rect {
    let sx = page_rect.width() / page_w.max(1.0);
    let sy = page_rect.height() / page_h.max(1.0);
    Rect::from_min_size(
        Pos2::new(
            page_rect.min.x + r.x * sx,
            page_rect.max.y - (r.y + r.h) * sy,
        ),
        Vec2::new(r.w * sx, r.h * sy),
    )
}

pub fn screen_to_pdf(page_rect: Rect, page_w: f32, page_h: f32, pos: Pos2) -> (f32, f32) {
    let sx = page_rect.width() / page_w.max(1.0);
    let sy = page_rect.height() / page_h.max(1.0);
    let x = (pos.x - page_rect.min.x) / sx;
    let y = (page_rect.max.y - pos.y) / sy;
    (x, y)
}

pub fn paint_overlays(
    ui: &mut Ui,
    page_rect: Rect,
    page: PageId,
    page_w: f32,
    page_h: f32,
    scene: &EditorScene,
    selected: Option<ObjectId>,
    draft_points: &[[f32; 2]],
    draft_shape: Option<RectPts>,
    text_sel_quads: &[RectPts],
    snap_guides: &[doxo_graphics::SnapGuide],
) {
    let painter = ui.painter_at(page_rect);

    for g in snap_guides {
        if g.vertical {
            let x = page_rect.min.x + (g.pos / page_w.max(1.0)) * page_rect.width();
            painter.line_segment(
                [
                    Pos2::new(x, page_rect.min.y),
                    Pos2::new(x, page_rect.max.y),
                ],
                Stroke::new(1.0_f32, Color32::from_rgb(255, 80, 160)),
            );
        } else {
            let y = page_rect.max.y - (g.pos / page_h.max(1.0)) * page_rect.height();
            painter.line_segment(
                [
                    Pos2::new(page_rect.min.x, y),
                    Pos2::new(page_rect.max.x, y),
                ],
                Stroke::new(1.0_f32, Color32::from_rgb(255, 80, 160)),
            );
        }
    }

    for q in text_sel_quads {
        let r = pdf_to_screen_scaled(page_rect, page_w, page_h, *q);
        painter.rect_filled(r, 0.0, Color32::from_rgba_unmultiplied(80, 140, 255, 80));
    }

    for obj in scene.objects_on_page(page) {
        let is_sel = selected == Some(obj.id());
        match obj {
            EditorObject::TextBox {
                rect, text, style, ..
            }
            | EditorObject::VisualTextPatch {
                cover: rect,
                text,
                style,
                ..
            }
            | EditorObject::NativeTextReplace {
                cover: rect,
                replacement: text,
                style,
                ..
            } => {
                if matches!(
                    obj,
                    EditorObject::VisualTextPatch { .. } | EditorObject::NativeTextReplace { .. }
                ) {
                    let r = pdf_to_screen_scaled(page_rect, page_w, page_h, *rect);
                    // Native shows a light dashed hint until flatten rewrites
                    let fill = if matches!(obj, EditorObject::NativeTextReplace { .. }) {
                        Color32::from_rgba_unmultiplied(255, 255, 220, 180)
                    } else {
                        Color32::WHITE
                    };
                    painter.rect_filled(r, 0.0, fill);
                }
                let r = pdf_to_screen_scaled(page_rect, page_w, page_h, *rect);
                let color = Color32::from_rgba_unmultiplied(
                    style.color.r,
                    style.color.g,
                    style.color.b,
                    style.color.a,
                );
                painter.text(
                    Pos2::new(r.min.x, r.min.y),
                    egui::Align2::LEFT_TOP,
                    text,
                    egui::FontId::proportional(style.font_size * (page_rect.height() / page_h)),
                    color,
                );
                if is_sel {
                    painter.rect_stroke(
                        r,
                        0.0,
                        Stroke::new(1.5_f32, Color32::from_rgb(40, 120, 255)),
                        egui::StrokeKind::Outside,
                    );
                }
            }
            EditorObject::Highlight { quads, color, .. } => {
                for q in quads {
                    let r = pdf_to_screen_scaled(page_rect, page_w, page_h, *q);
                    painter.rect_filled(
                        r,
                        0.0,
                        Color32::from_rgba_unmultiplied(color.r, color.g, color.b, color.a),
                    );
                }
            }
            EditorObject::InkStroke {
                points, color, width, ..
            } => {
                if points.len() >= 2 {
                    let stroke = Stroke::new(
                        *width * (page_rect.width() / page_w),
                        Color32::from_rgba_unmultiplied(color.r, color.g, color.b, color.a),
                    );
                    let pts: Vec<Pos2> = points
                        .iter()
                        .map(|p| {
                            pdf_to_screen_scaled(
                                page_rect,
                                page_w,
                                page_h,
                                RectPts {
                                    x: p[0],
                                    y: p[1],
                                    w: 0.0,
                                    h: 0.0,
                                },
                            )
                            .min
                        })
                        .collect();
                    painter.add(Shape::line(pts, stroke));
                }
                if is_sel {
                    let b = obj.bounds();
                    let r = pdf_to_screen_scaled(page_rect, page_w, page_h, b);
                    painter.rect_stroke(
                        r,
                        0.0,
                        Stroke::new(1.0_f32, Color32::from_rgb(40, 120, 255)),
                        egui::StrokeKind::Outside,
                    );
                }
            }
            EditorObject::Shape {
                kind,
                rect,
                stroke,
                fill,
                stroke_width,
                ..
            } => {
                let r = pdf_to_screen_scaled(page_rect, page_w, page_h, *rect);
                let sc = Color32::from_rgba_unmultiplied(stroke.r, stroke.g, stroke.b, stroke.a);
                let sw = *stroke_width * (page_rect.width() / page_w);
                match kind {
                    ShapeKind::Rectangle => {
                        if let Some(f) = fill {
                            painter.rect_filled(
                                r,
                                0.0,
                                Color32::from_rgba_unmultiplied(f.r, f.g, f.b, f.a),
                            );
                        }
                        painter.rect_stroke(r, 0.0, Stroke::new(sw, sc), egui::StrokeKind::Middle);
                    }
                    ShapeKind::Ellipse => {
                        if let Some(f) = fill {
                            painter.circle_filled(
                                r.center(),
                                r.width().min(r.height()) * 0.5,
                                Color32::from_rgba_unmultiplied(f.r, f.g, f.b, f.a),
                            );
                        }
                        painter.circle_stroke(
                            r.center(),
                            r.width().min(r.height()) * 0.5,
                            Stroke::new(sw, sc),
                        );
                    }
                    ShapeKind::Line | ShapeKind::Arrow => {
                        painter.line_segment([r.left_bottom(), r.right_top()], Stroke::new(sw, sc));
                        if *kind == ShapeKind::Arrow {
                            painter.circle_filled(r.right_top(), 3.0, sc);
                        }
                    }
                }
                if is_sel {
                    painter.rect_stroke(
                        r,
                        0.0,
                        Stroke::new(1.5_f32, Color32::from_rgb(40, 120, 255)),
                        egui::StrokeKind::Outside,
                    );
                }
            }
            EditorObject::Image {
                rect,
                opacity,
                rotation_deg,
                crop,
                ..
            } => {
                let r = pdf_to_screen_scaled(page_rect, page_w, page_h, *rect);
                let a = (*opacity * 180.0) as u8;
                painter.rect_filled(r, 2.0, Color32::from_rgba_unmultiplied(200, 200, 210, a));
                let label = format!("image {:.0}°", rotation_deg);
                painter.text(
                    r.center(),
                    egui::Align2::CENTER_CENTER,
                    label,
                    egui::FontId::proportional(12.0),
                    Color32::DARK_GRAY,
                );
                if let Some([x0, y0, x1, y1]) = crop {
                    let cr = Rect::from_min_max(
                        Pos2::new(
                            r.min.x + x0 * r.width(),
                            r.min.y + y0 * r.height(),
                        ),
                        Pos2::new(
                            r.min.x + x1 * r.width(),
                            r.min.y + y1 * r.height(),
                        ),
                    );
                    painter.rect_stroke(
                        cr,
                        0.0,
                        Stroke::new(1.0_f32, Color32::from_rgb(255, 160, 40)),
                        egui::StrokeKind::Middle,
                    );
                }
                if is_sel {
                    painter.rect_stroke(
                        r,
                        0.0,
                        Stroke::new(1.5_f32, Color32::from_rgb(40, 120, 255)),
                        egui::StrokeKind::Outside,
                    );
                    let handle = Rect::from_center_size(r.right_bottom(), Vec2::splat(10.0));
                    painter.rect_filled(handle, 1.0, Color32::from_rgb(40, 120, 255));
                    // Rotate handle (top-center)
                    let rot = Rect::from_center_size(
                        Pos2::new(r.center().x, r.min.y - 14.0),
                        Vec2::splat(10.0),
                    );
                    painter.circle_filled(rot.center(), 5.0, Color32::from_rgb(40, 180, 120));
                }
            }
        }
    }

    if draft_points.len() >= 2 {
        let pts: Vec<Pos2> = draft_points
            .iter()
            .map(|p| {
                pdf_to_screen_scaled(
                    page_rect,
                    page_w,
                    page_h,
                    RectPts {
                        x: p[0],
                        y: p[1],
                        w: 0.0,
                        h: 0.0,
                    },
                )
                .min
            })
            .collect();
        painter.add(Shape::line(
            pts,
            Stroke::new(2.0_f32, Color32::from_rgb(220, 50, 50)),
        ));
    }
    if let Some(r) = draft_shape {
        let sr = pdf_to_screen_scaled(page_rect, page_w, page_h, r);
        painter.rect_stroke(
            sr,
            0.0,
            Stroke::new(1.5_f32, Color32::from_rgb(40, 120, 255)),
            egui::StrokeKind::Middle,
        );
    }

    let _ = Sense::click();
}
