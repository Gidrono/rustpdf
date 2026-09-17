use doxo_text_engine::standard_font_families;
use egui::Context;

use crate::app_state::DoxoApp;
use crate::tools::EditorTool;

pub struct Toolbar;

impl Toolbar {
    pub fn show(ctx: &Context, app: &mut DoxoApp) {
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading("doxo");
                ui.separator();
                if ui.button("Open").clicked() {
                    app.trigger_open();
                }
                if ui.button("Save").clicked() {
                    app.save(false);
                }
                if ui.button("Save As").clicked() {
                    app.save(true);
                }
                if ui.button("Export Flat").on_hover_text("Export Flattened Copy").clicked() {
                    app.export_flattened();
                }
                ui.separator();
                for tool in [
                    EditorTool::Select,
                    EditorTool::Text,
                    EditorTool::Pencil,
                    EditorTool::Highlight,
                    EditorTool::Eraser,
                    EditorTool::Rectangle,
                    EditorTool::Ellipse,
                    EditorTool::Line,
                    EditorTool::Arrow,
                    EditorTool::Image,
                ] {
                    let selected = app.editor.tool == tool;
                    if ui.selectable_label(selected, tool.label()).clicked() {
                        app.editor.tool = tool;
                    }
                }
                ui.separator();
                if ui
                    .add_enabled(app.can_undo(), egui::Button::new("Undo"))
                    .clicked()
                {
                    app.undo();
                }
                if ui
                    .add_enabled(app.can_redo(), egui::Button::new("Redo"))
                    .clicked()
                {
                    app.redo();
                }
                ui.separator();
                if ui.button("−").clicked() {
                    app.zoom_out();
                }
                ui.label(format!("{:.0}%", app.zoom() * 100.0));
                if ui.button("+").clicked() {
                    app.zoom_in();
                }
                ui.separator();
                ui.label(app.page_label());
                ui.separator();
                // Text style
                egui::ComboBox::from_id_salt("font")
                    .selected_text(&app.editor.text_style.font_family)
                    .show_ui(ui, |ui| {
                        for f in standard_font_families() {
                            ui.selectable_value(
                                &mut app.editor.text_style.font_family,
                                (*f).to_string(),
                                *f,
                            );
                        }
                    });
                ui.add(
                    egui::DragValue::new(&mut app.editor.text_style.font_size)
                        .range(6.0..=96.0)
                        .prefix("pt "),
                );
                ui.color_edit_button_srgba_unmultiplied(&mut [
                    app.editor.text_style.color.r,
                    app.editor.text_style.color.g,
                    app.editor.text_style.color.b,
                    app.editor.text_style.color.a,
                ]);
            });
            ui.horizontal(|ui| {
                ui.weak(app.status_text());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.weak(app.backend_label());
                });
            });
        });

        // Page ops strip
        egui::TopBottomPanel::top("pageops").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Pages:");
                if ui.button("Delete").clicked() {
                    app.page_delete();
                }
                if ui.button("Rotate 90°").clicked() {
                    app.page_rotate(90);
                }
                if ui.button("Duplicate").clicked() {
                    app.page_duplicate();
                }
                if ui.button("Insert Blank").clicked() {
                    app.page_insert_blank();
                }
                if ui.button("Move ↑").clicked() {
                    app.page_reorder(-1);
                }
                if ui.button("Move ↓").clicked() {
                    app.page_reorder(1);
                }
                if ui.button("Extract…").on_hover_text("Extract selected page(s) to a new PDF").clicked() {
                    app.extract_pages();
                }
                ui.separator();
                if ui.button("Replace Text").on_hover_text("Replace selected PDF text (Level-2 when possible)").clicked() {
                    app.replace_selected_pdf_text();
                }
                if ui.button("Highlight Sel").clicked() {
                    app.editor.highlight_selection();
                }
                ui.separator();
                if ui.button("⟲ 90°").on_hover_text("Rotate selected image").clicked() {
                    app.editor.rotate_selected_image(-90.0);
                }
                if ui.button("⟳ 90°").clicked() {
                    app.editor.rotate_selected_image(90.0);
                }
                if ui
                    .selectable_label(app.editor.crop_mode, "Crop")
                    .on_hover_text("Nudge crop on selected image")
                    .clicked()
                {
                    app.editor.crop_mode = !app.editor.crop_mode;
                }
                if app.editor.crop_mode {
                    use crate::editor::CropEdge;
                    if ui.button("L+").clicked() {
                        app.editor.nudge_selected_crop(CropEdge::Left, 0.05);
                    }
                    if ui.button("R−").clicked() {
                        app.editor.nudge_selected_crop(CropEdge::Right, -0.05);
                    }
                    if ui.button("T+").clicked() {
                        app.editor.nudge_selected_crop(CropEdge::Top, 0.05);
                    }
                    if ui.button("B−").clicked() {
                        app.editor.nudge_selected_crop(CropEdge::Bottom, -0.05);
                    }
                    if ui.button("Reset crop").clicked() {
                        app.editor.reset_selected_crop();
                    }
                }
            });
        });
    }
}
