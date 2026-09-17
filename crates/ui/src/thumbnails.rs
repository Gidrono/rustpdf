use doxo_document::{PageId, PdfDocumentModel};
use egui::{Color32, Sense, Ui, Vec2};

use crate::app_state::ThumbMap;

#[derive(Default)]
pub struct ThumbnailSidebar;

impl ThumbnailSidebar {
    pub fn show(
        &mut self,
        ui: &mut Ui,
        document: Option<&PdfDocumentModel>,
        current: PageId,
        textures: &ThumbMap,
    ) -> Option<PageId> {
        let mut clicked = None;

        ui.heading("Pages");
        ui.separator();

        let Some(doc) = document else {
            ui.weak("No document");
            return None;
        };

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for page in &doc.pages {
                    let selected = page.id == current;
                    let thumb_w = ui.available_width().min(140.0);
                    let aspect = page.height_pts / page.width_pts.max(1.0);
                    let thumb_h = thumb_w * aspect;

                    let (rect, response) =
                        ui.allocate_exact_size(Vec2::new(thumb_w, thumb_h + 22.0), Sense::click());

                    let image_rect = egui::Rect::from_min_size(
                        rect.min,
                        Vec2::new(thumb_w, thumb_h),
                    );

                    if selected {
                        ui.painter().rect_stroke(
                            image_rect.expand(3.0),
                            2.0,
                            egui::Stroke::new(2.0_f32, Color32::from_rgb(70, 140, 255)),
                            egui::StrokeKind::Outside,
                        );
                    }

                    if let Some(tex) = textures.get(&(doc.id, page.id)) {
                        ui.painter().image(
                            tex.texture().id(),
                            image_rect,
                            egui::Rect::from_min_max(
                                egui::Pos2::ZERO,
                                egui::Pos2::new(1.0, 1.0),
                            ),
                            Color32::WHITE,
                        );
                    } else {
                        ui.painter()
                            .rect_filled(image_rect, 2.0, Color32::from_rgb(220, 220, 215));
                        ui.painter().text(
                            image_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "…",
                            egui::FontId::proportional(12.0),
                            Color32::GRAY,
                        );
                    }

                    ui.painter().text(
                        egui::Pos2::new(rect.min.x + thumb_w * 0.5, image_rect.max.y + 4.0),
                        egui::Align2::CENTER_TOP,
                        format!("{}", page.id.0 + 1),
                        egui::FontId::proportional(11.0),
                        if selected {
                            Color32::from_rgb(70, 140, 255)
                        } else {
                            Color32::GRAY
                        },
                    );

                    if response.clicked() {
                        clicked = Some(page.id);
                    }

                    ui.add_space(8.0);
                }
            });

        clicked
    }
}
