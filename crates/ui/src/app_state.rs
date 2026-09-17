use std::collections::HashMap;
use std::path::PathBuf;

use doxo_document::{DocumentId, EditCommand, EditJournal, ObjectId, PageId, PdfDocumentModel};
use doxo_pdf_engine::{
    BackendKind, PdfWorker, PdfWorkerHandle, RenderPriority, SearchHit, WorkerEvent,
};
use doxo_pdf_lowlevel::PageOp;
use egui::{ColorImage, TextureHandle, TextureOptions};

use crate::canvas::CanvasView;
use crate::editor::EditorState;
use crate::search::SearchPanel;
use crate::thumbnails::ThumbnailSidebar;
use crate::toolbar::Toolbar;
use crate::tools::EditorTool;

pub(crate) struct PageTexture {
    zoom_centi: u32,
    ppp_centi: u32,
    texture: TextureHandle,
}

pub(crate) struct ThumbTexture {
    texture: TextureHandle,
}

pub struct DoxoApp {
    worker: PdfWorker,
    worker_handle: PdfWorkerHandle,
    document: Option<PdfDocumentModel>,
    backend_kind: Option<BackendKind>,
    backend_detail: String,
    status: String,
    zoom: f32,
    scroll_y: f32,
    current_page: PageId,
    page_textures: HashMap<(DocumentId, PageId), PageTexture>,
    thumb_textures: HashMap<(DocumentId, PageId), ThumbTexture>,
    requested_pages: HashMap<(DocumentId, PageId, u32, u32), ()>,
    requested_thumbs: HashMap<(DocumentId, PageId), ()>,
    pending_open: Option<PathBuf>,
    canvas: CanvasView,
    thumbs: ThumbnailSidebar,
    pub(crate) editor: EditorState,
    search_hits: Vec<SearchHit>,
    replace_buf: String,
    /// Last egui `pixels_per_point` used for tile requests (Retina / display moves).
    last_ppp_centi: u32,
}

impl DoxoApp {
    pub fn new(
        _cc: &eframe::CreationContext<'_>,
        worker: PdfWorker,
        initial_path: Option<PathBuf>,
    ) -> Self {
        let worker_handle = worker.handle();
        let mut app = Self {
            worker,
            worker_handle,
            document: None,
            backend_kind: None,
            backend_detail: String::new(),
            status: "Open a PDF to begin".into(),
            zoom: 1.0,
            scroll_y: 0.0,
            current_page: PageId(0),
            page_textures: HashMap::new(),
            thumb_textures: HashMap::new(),
            requested_pages: HashMap::new(),
            requested_thumbs: HashMap::new(),
            pending_open: initial_path,
            canvas: CanvasView::default(),
            thumbs: ThumbnailSidebar::default(),
            editor: EditorState::default(),
            search_hits: Vec::new(),
            replace_buf: String::new(),
            last_ppp_centi: 100,
        };
        if let Some(path) = app.pending_open.take() {
            app.open_path(path);
        }
        app
    }

    fn open_path(&mut self, path: PathBuf) {
        self.status = format!("Opening {}…", path.display());
        self.page_textures.clear();
        self.thumb_textures.clear();
        self.requested_pages.clear();
        self.requested_thumbs.clear();
        self.scroll_y = 0.0;
        self.current_page = PageId(0);
        self.search_hits.clear();
        self.worker_handle.open(path);
    }

    fn open_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("PDF", &["pdf"])
            .pick_file()
        {
            self.open_path(path);
        }
    }

    pub(crate) fn save(&mut self, save_as: bool) {
        let Some(doc) = &self.document else {
            self.status = "No document open".into();
            return;
        };
        let path = if save_as || doc.path.is_none() {
            rfd::FileDialog::new()
                .add_filter("PDF", &["pdf"])
                .set_file_name(&doc.title)
                .save_file()
        } else {
            doc.path.clone()
        };
        let Some(path) = path else { return };
        let heights: Vec<(u32, f32)> = doc
            .pages
            .iter()
            .map(|p| (p.id.0, p.height_pts))
            .collect();
        self.status = format!("Saving {}…", path.display());
        self.worker_handle.save_flattened(
            doc.id,
            path,
            self.editor.scene.clone(),
            heights,
            true,
        );
    }

    pub(crate) fn export_flattened(&mut self) {
        let Some(doc) = &self.document else { return };
        let Some(path) = rfd::FileDialog::new()
            .add_filter("PDF", &["pdf"])
            .set_file_name(format!(
                "{}-flat.pdf",
                doc.title.trim_end_matches(".pdf")
            ))
            .save_file()
        else {
            return;
        };
        let heights: Vec<(u32, f32)> = doc
            .pages
            .iter()
            .map(|p| (p.id.0, p.height_pts))
            .collect();
        self.worker_handle.save_flattened(
            doc.id,
            path,
            self.editor.scene.clone(),
            heights,
            false,
        );
    }

    fn drain_worker(&mut self, ctx: &egui::Context) {
        for ev in self.worker.poll_events() {
            match ev {
                WorkerEvent::BackendReady { kind, detail } => {
                    self.backend_kind = Some(kind);
                    self.backend_detail = detail;
                }
                WorkerEvent::DocumentOpened { model, backend } => {
                    self.status = format!(
                        "{} — {} pages ({backend:?})",
                        model.title, model.page_count
                    );
                    if model.is_placeholder {
                        self.status.push_str(" [mock]");
                    }
                    if !self.editor.status_note.is_empty() {
                        self.status = self.editor.status_note.clone();
                        self.editor.status_note.clear();
                    }
                    let path = model.path.clone();
                    self.editor
                        .reset_for_document(EditJournal::new(model.id), path.as_deref());
                    // If recovery loaded scene, keep it; reset_for_document may load journal
                    self.document = Some(model);
                    self.backend_kind = Some(backend);
                    self.zoom = 1.0;
                    self.scroll_y = 0.0;
                    self.current_page = PageId(0);
                    self.page_textures.clear();
                    self.thumb_textures.clear();
                    self.requested_pages.clear();
                    self.requested_thumbs.clear();
                    ctx.request_repaint();
                }
                WorkerEvent::PageOpComplete { model } => {
                    self.status = format!("Page op done — {} pages", model.page_count);
                    let id = model.id;
                    self.document = Some(model);
                    if let Some(j) = &mut self.editor.journal {
                        j.document_id = id;
                    }
                    self.page_textures.clear();
                    self.thumb_textures.clear();
                    self.requested_pages.clear();
                    self.requested_thumbs.clear();
                    self.editor.extracted.clear();
                    ctx.request_repaint();
                }
                WorkerEvent::DocumentClosed { .. } => {
                    self.document = None;
                    self.editor = EditorState::default();
                    self.page_textures.clear();
                    self.thumb_textures.clear();
                }
                WorkerEvent::PageReady {
                    document_id,
                    page,
                    zoom,
                    pixels_per_point,
                    width,
                    height,
                    rgba,
                } => {
                    let image = ColorImage::from_rgba_unmultiplied(
                        [width as usize, height as usize],
                        &rgba,
                    );
                    let zoom_centi = (zoom * 100.0).round() as u32;
                    let ppp_centi = (pixels_per_point * 100.0).round() as u32;
                    let texture = ctx.load_texture(
                        format!("page-{}-{}-{}", page.0, zoom_centi, ppp_centi),
                        image,
                        TextureOptions::LINEAR,
                    );
                    self.page_textures.insert(
                        (document_id, page),
                        PageTexture {
                            zoom_centi,
                            ppp_centi,
                            texture,
                        },
                    );
                    ctx.request_repaint();
                }
                WorkerEvent::ThumbnailReady {
                    document_id,
                    page,
                    width,
                    height,
                    rgba,
                } => {
                    let image = ColorImage::from_rgba_unmultiplied(
                        [width as usize, height as usize],
                        &rgba,
                    );
                    let texture = ctx.load_texture(
                        format!("thumb-{}", page.0),
                        image,
                        TextureOptions::LINEAR,
                    );
                    self.thumb_textures
                        .insert((document_id, page), ThumbTexture { texture });
                    ctx.request_repaint();
                }
                WorkerEvent::TextExtracted { text, .. } => {
                    self.editor.extracted.insert(text.page.0, text);
                    ctx.request_repaint();
                }
                WorkerEvent::SearchResults { hits, query, .. } => {
                    self.search_hits = hits;
                    self.status = format!("Search “{query}”: {} hits", self.search_hits.len());
                    ctx.request_repaint();
                }
                WorkerEvent::SaveComplete { path } => {
                    self.status = format!("Saved {}", path.display());
                    self.editor.clear_autosave();
                    self.editor.scene.dirty = false;
                    if let Some(doc) = &mut self.document {
                        doc.path = Some(path.clone());
                        self.editor.autosave_store =
                            Some(doxo_pdf_lowlevel::AutosaveStore::for_document(&path));
                    }
                    ctx.request_repaint();
                }
                WorkerEvent::ExtractComplete { path, page_count } => {
                    self.status = format!("Extracted {page_count} page(s) → {}", path.display());
                    ctx.request_repaint();
                }
                WorkerEvent::Error { message } => {
                    self.status = message;
                    ctx.request_repaint();
                }
                WorkerEvent::ShutdownComplete => {}
            }
        }
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        let mods = ctx.input(|i| i.modifiers);
        let cmd = mods.command || mods.ctrl;

        if cmd && ctx.input(|i| i.key_pressed(egui::Key::O)) {
            self.open_dialog();
        }
        if cmd && ctx.input(|i| i.key_pressed(egui::Key::S)) {
            self.save(mods.shift);
        }
        if cmd && ctx.input(|i| i.key_pressed(egui::Key::F)) {
            self.editor.search_open = true;
        }
        if cmd && ctx.input(|i| i.key_pressed(egui::Key::Z)) {
            if mods.shift {
                self.redo();
            } else {
                self.undo();
            }
        }
        if cmd && ctx.input(|i| i.key_pressed(egui::Key::Y)) {
            self.redo();
        }
        if cmd && ctx.input(|i| i.key_pressed(egui::Key::C)) {
            if let Some((page, a, b)) = self.editor.text_sel {
                if let Some(t) = self.editor.extracted.get(&page.0) {
                    let s = t.text_range(a, b);
                    let _ = doxo_platform_macos::set_clipboard_text(&s);
                    self.status = format!("Copied {} chars", s.len());
                }
            } else {
                self.editor.copy_selected();
            }
        }
        if cmd && ctx.input(|i| i.key_pressed(egui::Key::V)) {
            if let Some(img) = doxo_platform_macos::clipboard_image_png() {
                self.editor.paste_image(self.current_page, img);
            } else if let Some(text) = doxo_platform_macos::clipboard_text() {
                if self.editor.text_sel.is_some() {
                    self.editor.replace_selected_text(text);
                } else {
                    self.editor.paste_on_page(self.current_page);
                    // Also allow paste as new text
                    let _ = text;
                }
            } else {
                self.editor.paste_on_page(self.current_page);
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
            && self.editor.editing_text_id.is_none()
            && self.editor.pdf_text_edit_draft.is_none()
            && self.editor.text_edit_draft.is_none()
        {
            self.editor.delete_selected();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.editor.pdf_text_edit_draft.is_some() || self.editor.text_edit_draft.is_some()
            {
                self.editor.cancel_text_edit();
            } else {
                self.editor.tool = EditorTool::Select;
                self.editor.selected = None;
                self.editor.text_sel = None;
                self.editor.search_open = false;
            }
        }
        // Enter opens edit for PDF text selection or selected overlay text.
        if ctx.input(|i| i.key_pressed(egui::Key::Enter))
            && self.editor.pdf_text_edit_draft.is_none()
            && self.editor.text_edit_draft.is_none()
            && !self.editor.search_open
        {
            if self.editor.text_sel.is_some() {
                self.editor.begin_pdf_text_edit();
            } else if let Some(id) = self.editor.selected {
                if matches!(
                    self.editor.scene.find(id),
                    Some(doxo_document::EditorObject::TextBox { .. })
                        | Some(doxo_document::EditorObject::VisualTextPatch { .. })
                        | Some(doxo_document::EditorObject::NativeTextReplace { .. })
                ) {
                    self.editor.begin_overlay_text_edit(id);
                }
            }
        }

        // Tool shortcuts
        if !cmd {
            if ctx.input(|i| i.key_pressed(egui::Key::V)) {
                self.editor.tool = EditorTool::Select;
            }
            if ctx.input(|i| i.key_pressed(egui::Key::T)) {
                self.editor.tool = EditorTool::Text;
            }
            if ctx.input(|i| i.key_pressed(egui::Key::P)) {
                self.editor.tool = EditorTool::Pencil;
            }
            if ctx.input(|i| i.key_pressed(egui::Key::H)) {
                self.editor.tool = EditorTool::Highlight;
            }
        }

        let page_count = self.document.as_ref().map(|d| d.page_count).unwrap_or(0);
        let current = self.current_page.0;
        let alt = ctx.input(|i| i.modifiers.alt);
        if ctx.input(|i| {
            i.key_pressed(egui::Key::ArrowDown)
                || i.key_pressed(egui::Key::PageDown)
                || i.key_pressed(egui::Key::J)
        }) {
            if alt {
                self.jump_to_page(PageId(
                    current.saturating_add(1).min(page_count.saturating_sub(1)),
                ));
            } else {
                self.scroll_y += 80.0 / self.zoom.max(0.1);
            }
        }
        if ctx.input(|i| {
            i.key_pressed(egui::Key::ArrowUp)
                || i.key_pressed(egui::Key::PageUp)
                || i.key_pressed(egui::Key::K)
        }) {
            if alt {
                self.jump_to_page(PageId(current.saturating_sub(1)));
            } else {
                self.scroll_y = (self.scroll_y - 80.0 / self.zoom.max(0.1)).max(0.0);
            }
        }
        if cmd && ctx.input(|i| i.key_pressed(egui::Key::Equals) || i.key_pressed(egui::Key::Plus))
        {
            self.set_zoom(self.zoom * 1.15);
        }
        if cmd && ctx.input(|i| i.key_pressed(egui::Key::Minus)) {
            self.set_zoom(self.zoom / 1.15);
        }
        if cmd && ctx.input(|i| i.key_pressed(egui::Key::Num0)) {
            self.set_zoom(1.0);
        }
    }

    fn set_zoom(&mut self, zoom: f32) {
        let z = zoom.clamp(0.25, 8.0);
        if (z - self.zoom).abs() > f32::EPSILON {
            self.zoom = z;
            self.requested_pages.clear();
        }
    }

    fn jump_to_page(&mut self, page: PageId) {
        let Some(doc) = &self.document else { return };
        if page.0 >= doc.page_count {
            return;
        }
        self.current_page = page;
        let mut y = 0.0f32;
        let gap = 16.0;
        for p in &doc.pages {
            if p.id == page {
                break;
            }
            y += p.height_pts + gap;
        }
        self.scroll_y = y;
    }

    fn request_visible_work(
        &mut self,
        visible: &[PageId],
        near: &[PageId],
        need_text: &[PageId],
        pixels_per_point: f32,
    ) {
        let Some(doc) = &self.document else { return };
        let doc_id = doc.id;
        let zoom = self.zoom;
        let zoom_centi = (zoom * 100.0).round() as u32;
        let ppp_centi = (pixels_per_point * 100.0).round().max(50.0) as u32;
        if ppp_centi != self.last_ppp_centi {
            self.last_ppp_centi = ppp_centi;
            self.requested_pages.clear();
        }

        for page in visible {
            let key = (doc_id, *page, zoom_centi, ppp_centi);
            if self.requested_pages.contains_key(&key) {
                continue;
            }
            if let Some(tex) = self.page_textures.get(&(doc_id, *page)) {
                if tex.zoom_centi == zoom_centi && tex.ppp_centi == ppp_centi {
                    self.requested_pages.insert(key, ());
                    continue;
                }
            }
            self.requested_pages.insert(key, ());
            self.worker_handle.request_page(
                doc_id,
                *page,
                zoom,
                pixels_per_point,
                RenderPriority::Visible,
            );
        }
        for page in near {
            let key = (doc_id, *page, zoom_centi, ppp_centi);
            if self.requested_pages.contains_key(&key) {
                continue;
            }
            self.requested_pages.insert(key, ());
            self.worker_handle.request_page(
                doc_id,
                *page,
                zoom,
                pixels_per_point,
                RenderPriority::Prefetch,
            );
        }
        for page in need_text {
            self.worker_handle.extract_text(doc_id, *page);
        }

        let thumb_w = ((140.0 * pixels_per_point).round() as u32).max(140);
        let start = self.current_page.0.saturating_sub(8);
        let end = (self.current_page.0 + 24).min(doc.page_count.saturating_sub(1));
        for idx in start..=end {
            let page = PageId(idx);
            let key = (doc_id, page);
            if self.requested_thumbs.contains_key(&key) {
                continue;
            }
            if self.thumb_textures.contains_key(&key) {
                self.requested_thumbs.insert(key, ());
                continue;
            }
            self.requested_thumbs.insert(key, ());
            self.worker_handle.request_thumbnail(doc_id, page, thumb_w);
        }
    }

    pub(crate) fn undo(&mut self) {
        if let Some(page_cmd) = self.editor.undo() {
            self.dispatch_page_op(page_cmd);
        }
    }

    pub(crate) fn redo(&mut self) {
        if let Some(page_cmd) = self.editor.redo() {
            self.dispatch_page_op(page_cmd);
        }
    }

    pub(crate) fn can_undo(&self) -> bool {
        self.editor.journal.as_ref().is_some_and(|j| j.can_undo())
    }

    pub(crate) fn can_redo(&self) -> bool {
        self.editor.journal.as_ref().is_some_and(|j| j.can_redo())
    }

    fn dispatch_page_op(&mut self, cmd: EditCommand) {
        let Some(doc) = &self.document else { return };
        let op = match cmd {
            EditCommand::PageDelete { page } => PageOp::Delete { index: page.0 },
            EditCommand::PageRotate { page, degrees } => PageOp::Rotate {
                index: page.0,
                degrees,
            },
            EditCommand::PageReorder { from, to } => PageOp::Reorder { from, to },
            EditCommand::PageInsertBlank { at } => PageOp::InsertBlank { at },
            EditCommand::PageDuplicate { page } => PageOp::Duplicate { index: page.0 },
            _ => return,
        };
        self.worker_handle
            .apply_page_op(doc.id, op, doc.path.clone());
    }

    pub(crate) fn page_delete(&mut self) {
        let page = self.current_page;
        if self.editor.apply(EditCommand::PageDelete { page }) {
            self.dispatch_page_op(EditCommand::PageDelete { page });
        }
    }

    pub(crate) fn page_rotate(&mut self, degrees: i32) {
        let page = self.current_page;
        let cmd = EditCommand::PageRotate { page, degrees };
        if self.editor.apply(cmd.clone()) {
            self.dispatch_page_op(cmd);
        }
    }

    pub(crate) fn page_duplicate(&mut self) {
        let page = self.current_page;
        let cmd = EditCommand::PageDuplicate { page };
        if self.editor.apply(cmd.clone()) {
            self.dispatch_page_op(cmd);
        }
    }

    pub(crate) fn page_insert_blank(&mut self) {
        let at = self.current_page.0 + 1;
        let cmd = EditCommand::PageInsertBlank { at };
        if self.editor.apply(cmd.clone()) {
            self.dispatch_page_op(cmd);
        }
    }

    pub(crate) fn page_reorder(&mut self, delta: i32) {
        let from = self.current_page.0;
        let to = (from as i32 + delta).max(0) as u32;
        let cmd = EditCommand::PageReorder { from, to };
        if self.editor.apply(cmd.clone()) {
            self.dispatch_page_op(cmd);
        }
    }

    pub(crate) fn extract_pages(&mut self) {
        let Some(doc) = &self.document else { return };
        // V1: extract the current page (extendable to multi-select later).
        let indices = vec![self.current_page.0];
        let default_name = format!(
            "{}-p{}.pdf",
            doc.title.trim_end_matches(".pdf"),
            self.current_page.0 + 1
        );
        let Some(dest) = rfd::FileDialog::new()
            .add_filter("PDF", &["pdf"])
            .set_file_name(&default_name)
            .save_file()
        else {
            return;
        };
        self.status = format!("Extracting page {}…", self.current_page.0 + 1);
        self.worker_handle
            .extract_pages(doc.id, indices, dest);
    }

    pub(crate) fn replace_selected_pdf_text(&mut self) {
        if self.editor.text_sel.is_none() {
            self.status = "Select PDF text first (drag or double-click a word)".into();
            return;
        }
        if !self.replace_buf.is_empty() {
            self.editor.replace_selected_text(self.replace_buf.clone());
        } else {
            self.editor.begin_pdf_text_edit();
        }
    }
}

impl eframe::App for DoxoApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_worker(ctx);
        self.handle_keys(ctx);

        if let Some(doc) = &self.document {
            self.editor.maybe_autosave(doc);
        }

        let (scroll, zooming) = ctx.input(|i| {
            let zooming = i.modifiers.command || i.modifiers.ctrl;
            (i.raw_scroll_delta.y, zooming)
        });
        if scroll != 0.0 {
            if zooming {
                let factor = if scroll > 0.0 { 1.05 } else { 1.0 / 1.05 };
                self.set_zoom(self.zoom * factor);
            } else {
                self.scroll_y = (self.scroll_y - scroll / self.zoom.max(0.1)).max(0.0);
            }
        }

        Toolbar::show(ctx, self);

        if self.editor.search_open {
            egui::TopBottomPanel::bottom("search").show(ctx, |ui| {
                let mut q = self.editor.search_query.clone();
                let mut do_search = None;
                let mut jump = None;
                SearchPanel::show(
                    ui,
                    &mut self.editor.search_open,
                    &mut q,
                    &self.search_hits,
                    &mut |query| {
                        do_search = Some(query);
                    },
                    &mut |hit| {
                        jump = Some(hit);
                    },
                );
                ui.horizontal(|ui| {
                    ui.label("Replace with");
                    ui.text_edit_singleline(&mut self.replace_buf);
                    if ui.button("Replace selection").clicked() {
                        self.editor.replace_selected_text(self.replace_buf.clone());
                    }
                });
                self.editor.search_query = q;
                if let Some(query) = do_search {
                    if let Some(doc) = &self.document {
                        self.worker_handle.search(doc.id, query);
                    }
                }
                if let Some(hit) = jump {
                    self.jump_to_page(hit.page);
                    if let Some(t) = self.editor.extracted.get(&hit.page.0) {
                        let start = hit.start.min(t.chars.len().saturating_sub(1));
                        let end = hit.end.min(t.chars.len()).max(start + 1);
                        self.editor.text_sel = Some((hit.page, start, end));
                    }
                }
            });
        }

        // Inline text editor — overlay draft or existing PDF text draft
        if self.editor.pdf_text_edit_draft.is_some() {
            let mut done = false;
            let mut cancel = false;
            egui::Window::new("Edit PDF text")
                .collapsible(false)
                .resizable(true)
                .show(ctx, |ui| {
                    ui.label("Replace selected PDF text:");
                    if let Some(draft) = self.editor.pdf_text_edit_draft.as_mut() {
                        ui.text_edit_multiline(draft);
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Done").clicked() {
                            done = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                    });
                });
            if done {
                self.editor.commit_pdf_text_edit();
            } else if cancel {
                self.editor.cancel_text_edit();
            }
        } else if self.editor.text_edit_draft.is_some() {
            let mut done = false;
            let mut cancel = false;
            egui::Window::new("Edit text")
                .collapsible(false)
                .resizable(true)
                .show(ctx, |ui| {
                    if let Some((_, text, style)) = self.editor.text_edit_draft.as_mut() {
                        ui.text_edit_multiline(text);
                        ui.horizontal(|ui| {
                            ui.label("Size");
                            ui.add(
                                egui::DragValue::new(&mut style.font_size).range(6.0..=96.0),
                            );
                        });
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Done").clicked() {
                            done = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                    });
                });
            if done {
                self.editor.commit_overlay_text_edit();
            } else if cancel {
                self.editor.cancel_text_edit();
            }
        }

        let mut jump: Option<PageId> = None;
        egui::SidePanel::left("thumbs")
            .resizable(true)
            .default_width(160.0)
            .width_range(120.0..=280.0)
            .show(ctx, |ui| {
                jump = self.thumbs.show(
                    ui,
                    self.document.as_ref(),
                    self.current_page,
                    &self.thumb_textures,
                );
            });
        if let Some(page) = jump {
            self.jump_to_page(page);
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            let ppp = ui.ctx().pixels_per_point();
            let outcome = self.canvas.show(
                ui,
                self.document.as_ref(),
                self.zoom,
                &mut self.scroll_y,
                &mut self.current_page,
                &self.page_textures,
                &mut self.editor,
            );
            self.request_visible_work(
                &outcome.visible,
                &outcome.near,
                &outcome.need_text,
                ppp,
            );
        });

        if self.document.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(33));
        }
    }
}

impl DoxoApp {
    pub(crate) fn status_text(&self) -> &str {
        &self.status
    }
    pub(crate) fn zoom(&self) -> f32 {
        self.zoom
    }
    pub(crate) fn backend_label(&self) -> String {
        match self.backend_kind {
            Some(BackendKind::Pdfium) => format!("PDFium — {}", self.backend_detail),
            Some(BackendKind::Mock) => format!("Mock — {}", self.backend_detail),
            None => "Starting…".into(),
        }
    }
    pub(crate) fn page_label(&self) -> String {
        match &self.document {
            Some(d) => format!("{} / {}", self.current_page.0 + 1, d.page_count),
            None => "—".into(),
        }
    }
    pub(crate) fn trigger_open(&mut self) {
        self.open_dialog();
    }
    pub(crate) fn zoom_in(&mut self) {
        self.set_zoom(self.zoom * 1.15);
    }
    pub(crate) fn zoom_out(&mut self) {
        self.set_zoom(self.zoom / 1.15);
    }
    pub(crate) fn zoom_reset(&mut self) {
        self.set_zoom(1.0);
    }
}

pub(crate) type ThumbMap = HashMap<(DocumentId, PageId), ThumbTexture>;
pub(crate) type PageMap = HashMap<(DocumentId, PageId), PageTexture>;

impl PageTexture {
    pub(crate) fn texture(&self) -> &TextureHandle {
        &self.texture
    }
    pub(crate) fn zoom_centi(&self) -> u32 {
        self.zoom_centi
    }
    pub(crate) fn ppp_centi(&self) -> u32 {
        self.ppp_centi
    }
}

impl ThumbTexture {
    pub(crate) fn texture(&self) -> &TextureHandle {
        &self.texture
    }
}

// silence unused import in some cfgs
#[allow(dead_code)]
fn _obj_id() -> ObjectId {
    ObjectId::new()
}
