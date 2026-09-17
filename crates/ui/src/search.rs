use doxo_pdf_engine::SearchHit;
use egui::Ui;

pub struct SearchPanel;

impl SearchPanel {
    pub fn show(
        ui: &mut Ui,
        open: &mut bool,
        query: &mut String,
        hits: &[SearchHit],
        on_search: &mut dyn FnMut(String),
        on_jump: &mut dyn FnMut(SearchHit),
    ) {
        ui.horizontal(|ui| {
            ui.label("Find");
            let response = ui.text_edit_singleline(query);
            if ui.button("Search").clicked()
                || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
            {
                on_search(query.clone());
            }
            if ui.button("Close").clicked() {
                *open = false;
            }
        });
        ui.separator();
        egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
            if hits.is_empty() {
                ui.weak("No results");
            }
            for hit in hits {
                let label = format!("p.{} — {}", hit.page.0 + 1, hit.context.replace('\n', " "));
                if ui.selectable_label(false, label).clicked() {
                    on_jump(hit.clone());
                }
            }
        });
    }
}
