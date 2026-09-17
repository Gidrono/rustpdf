use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::thread::{self, JoinHandle};

use crossbeam_channel::{unbounded, Receiver, Sender};
use tracing::{debug, error, info};

use crate::cache::{CachedImage, ThumbnailCache, ThumbnailKey, TileCache, TileKey};
use crate::messages::{RenderPriority, SearchHit, WorkerEvent, WorkerRequest};
use crate::render_backend::{create_backend, OpenedDocument, RenderBackend};

#[derive(Clone)]
pub struct PdfWorkerHandle {
    tx: Sender<WorkerRequest>,
}

impl PdfWorkerHandle {
    pub fn send(&self, req: WorkerRequest) {
        let _ = self.tx.send(req);
    }

    pub fn open(&self, path: PathBuf) {
        self.send(WorkerRequest::OpenDocument { path });
    }

    pub fn close(&self) {
        self.send(WorkerRequest::CloseDocument);
    }

    pub fn request_page(
        &self,
        document_id: doxo_document::DocumentId,
        page: doxo_document::PageId,
        zoom: f32,
        pixels_per_point: f32,
        priority: RenderPriority,
    ) {
        self.send(WorkerRequest::RenderPage {
            document_id,
            page,
            zoom,
            pixels_per_point,
            priority,
        });
    }

    pub fn request_thumbnail(
        &self,
        document_id: doxo_document::DocumentId,
        page: doxo_document::PageId,
        max_width: u32,
    ) {
        self.send(WorkerRequest::RenderThumbnail {
            document_id,
            page,
            max_width,
        });
    }

    pub fn extract_text(
        &self,
        document_id: doxo_document::DocumentId,
        page: doxo_document::PageId,
    ) {
        self.send(WorkerRequest::ExtractText {
            document_id,
            page,
        });
    }

    pub fn search(&self, document_id: doxo_document::DocumentId, query: String) {
        self.send(WorkerRequest::Search {
            document_id,
            query,
        });
    }

    pub fn save_flattened(
        &self,
        document_id: doxo_document::DocumentId,
        path: PathBuf,
        scene: doxo_document::EditorScene,
        page_heights: Vec<(u32, f32)>,
        reload: bool,
    ) {
        self.send(WorkerRequest::SaveFlattened {
            document_id,
            path,
            scene,
            page_heights,
            reload,
        });
    }

    pub fn apply_page_op(
        &self,
        document_id: doxo_document::DocumentId,
        op: doxo_pdf_lowlevel::PageOp,
        save_path: Option<PathBuf>,
    ) {
        self.send(WorkerRequest::ApplyPageOp {
            document_id,
            op,
            save_path,
        });
    }

    pub fn extract_pages(
        &self,
        document_id: doxo_document::DocumentId,
        indices: Vec<u32>,
        dest: PathBuf,
    ) {
        self.send(WorkerRequest::ExtractPages {
            document_id,
            indices,
            dest,
        });
    }

    pub fn shutdown(&self) {
        self.send(WorkerRequest::Shutdown);
    }
}

pub struct PdfWorker {
    handle: PdfWorkerHandle,
    events: Receiver<WorkerEvent>,
    join: Option<JoinHandle<()>>,
}

impl PdfWorker {
    pub fn spawn(pdfium_search_paths: Vec<PathBuf>) -> Self {
        let (req_tx, req_rx) = unbounded::<WorkerRequest>();
        let (evt_tx, evt_rx) = unbounded::<WorkerEvent>();

        let join = thread::Builder::new()
            .name("doxo-pdf-worker".into())
            .spawn(move || {
                worker_loop(req_rx, evt_tx, pdfium_search_paths);
            })
            .expect("spawn pdf worker");

        Self {
            handle: PdfWorkerHandle { tx: req_tx },
            events: evt_rx,
            join: Some(join),
        }
    }

    pub fn handle(&self) -> PdfWorkerHandle {
        self.handle.clone()
    }

    pub fn poll_events(&self) -> Vec<WorkerEvent> {
        let mut out = Vec::new();
        while let Ok(ev) = self.events.try_recv() {
            out.push(ev);
        }
        out
    }

    pub fn shutdown(mut self) {
        self.handle.shutdown();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl Drop for PdfWorker {
    fn drop(&mut self) {
        let _ = self.handle.tx.send(WorkerRequest::Shutdown);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

struct PendingRender {
    document_id: doxo_document::DocumentId,
    page: doxo_document::PageId,
    zoom: f32,
    pixels_per_point: f32,
    #[allow(dead_code)]
    priority: RenderPriority,
}

fn worker_loop(
    req_rx: Receiver<WorkerRequest>,
    evt_tx: Sender<WorkerEvent>,
    pdfium_search_paths: Vec<PathBuf>,
) {
    let mut backend: Box<dyn RenderBackend> = create_backend(&pdfium_search_paths);
    let _ = evt_tx.send(WorkerEvent::BackendReady {
        kind: backend.kind(),
        detail: backend.info().detail.clone(),
    });

    let mut opened: Option<OpenedDocument> = None;
    let mut tiles = TileCache::new(64);
    let mut thumbs = ThumbnailCache::new(256);
    let mut pending: VecDeque<PendingRender> = VecDeque::new();
    let mut pending_thumbs: VecDeque<(doxo_document::DocumentId, doxo_document::PageId, u32)> =
        VecDeque::new();
    let mut text_cache: HashMap<u32, doxo_document::ExtractedPageText> = HashMap::new();

    loop {
        let req = match req_rx.try_recv() {
            Ok(r) => Some(r),
            Err(crossbeam_channel::TryRecvError::Empty) => {
                if pending.is_empty() && pending_thumbs.is_empty() {
                    match req_rx.recv() {
                        Ok(r) => Some(r),
                        Err(_) => break,
                    }
                } else {
                    None
                }
            }
            Err(crossbeam_channel::TryRecvError::Disconnected) => break,
        };

        if let Some(req) = req {
            match req {
                WorkerRequest::Shutdown => {
                    let _ = evt_tx.send(WorkerEvent::ShutdownComplete);
                    break;
                }
                WorkerRequest::CloseDocument => {
                    if let Some(doc) = opened.take() {
                        tiles.clear_document(doc.model.id);
                        thumbs.clear_document(doc.model.id);
                        let _ = evt_tx.send(WorkerEvent::DocumentClosed {
                            document_id: doc.model.id,
                        });
                    }
                    pending.clear();
                    pending_thumbs.clear();
                    text_cache.clear();
                }
                WorkerRequest::OpenDocument { path } => {
                    pending.clear();
                    pending_thumbs.clear();
                    tiles.clear();
                    thumbs.clear();
                    text_cache.clear();
                    match backend.open(&path) {
                        Ok(doc) => {
                            info!(
                                path = %path.display(),
                                pages = doc.model.page_count,
                                backend = ?backend.kind(),
                                "document opened"
                            );
                            let model = doc.model.clone();
                            opened = Some(doc);
                            let _ = evt_tx.send(WorkerEvent::DocumentOpened {
                                model,
                                backend: backend.kind(),
                            });
                        }
                        Err(err) => {
                            error!(error = %err, "open failed");
                            let _ = evt_tx.send(WorkerEvent::Error {
                                message: format!("Failed to open {}: {err}", path.display()),
                            });
                        }
                    }
                }
                WorkerRequest::ReloadBytes {
                    bytes,
                    path,
                    title,
                } => {
                    pending.clear();
                    pending_thumbs.clear();
                    tiles.clear();
                    thumbs.clear();
                    text_cache.clear();
                    match backend.open_bytes(bytes, path, title) {
                        Ok(doc) => {
                            let model = doc.model.clone();
                            opened = Some(doc);
                            let _ = evt_tx.send(WorkerEvent::DocumentOpened {
                                model,
                                backend: backend.kind(),
                            });
                        }
                        Err(err) => {
                            let _ = evt_tx.send(WorkerEvent::Error {
                                message: format!("Reload failed: {err}"),
                            });
                        }
                    }
                }
                WorkerRequest::RenderPage {
                    document_id,
                    page,
                    zoom,
                    pixels_per_point,
                    priority,
                } => {
                    let key = TileKey::new(document_id, page, zoom, pixels_per_point);
                    if let Some(cached) = tiles.get(&key) {
                        let _ = evt_tx.send(WorkerEvent::PageReady {
                            document_id,
                            page,
                            zoom: key.zoom(),
                            pixels_per_point: key.pixels_per_point(),
                            width: cached.width,
                            height: cached.height,
                            rgba: cached.rgba.clone(),
                        });
                    } else {
                        let item = PendingRender {
                            document_id,
                            page,
                            zoom,
                            pixels_per_point,
                            priority,
                        };
                        if priority == RenderPriority::Visible {
                            pending.push_front(item);
                        } else {
                            pending.push_back(item);
                        }
                        while pending.len() > 48 {
                            pending.pop_back();
                        }
                    }
                }
                WorkerRequest::RenderThumbnail {
                    document_id,
                    page,
                    max_width,
                } => {
                    let key = ThumbnailKey {
                        document_id,
                        page,
                    };
                    if let Some(cached) = thumbs.get(&key) {
                        let _ = evt_tx.send(WorkerEvent::ThumbnailReady {
                            document_id,
                            page,
                            width: cached.width,
                            height: cached.height,
                            rgba: cached.rgba.clone(),
                        });
                    } else {
                        pending_thumbs.push_back((document_id, page, max_width));
                        while pending_thumbs.len() > 64 {
                            pending_thumbs.pop_back();
                        }
                    }
                }
                WorkerRequest::ExtractText {
                    document_id,
                    page,
                } => {
                    if let Some(doc) = opened.as_ref() {
                        if doc.model.id != document_id {
                            continue;
                        }
                        if let Some(cached) = text_cache.get(&page.0) {
                            let _ = evt_tx.send(WorkerEvent::TextExtracted {
                                document_id,
                                text: cached.clone(),
                            });
                            continue;
                        }
                        match backend.extract_text(doc, page) {
                            Ok(text) => {
                                text_cache.insert(page.0, text.clone());
                                let _ = evt_tx.send(WorkerEvent::TextExtracted {
                                    document_id,
                                    text,
                                });
                            }
                            Err(err) => {
                                let _ = evt_tx.send(WorkerEvent::Error {
                                    message: format!("text extract: {err}"),
                                });
                            }
                        }
                    }
                }
                WorkerRequest::Search {
                    document_id,
                    query,
                } => {
                    if let Some(doc) = opened.as_ref() {
                        if doc.model.id != document_id {
                            continue;
                        }
                        let q = query.to_lowercase();
                        let mut hits = Vec::new();
                        if q.is_empty() {
                            let _ = evt_tx.send(WorkerEvent::SearchResults {
                                document_id,
                                query,
                                hits,
                            });
                            continue;
                        }
                        for page_info in &doc.model.pages {
                            let text = if let Some(t) = text_cache.get(&page_info.id.0) {
                                t.clone()
                            } else {
                                match backend.extract_text(doc, page_info.id) {
                                    Ok(t) => {
                                        text_cache.insert(page_info.id.0, t.clone());
                                        t
                                    }
                                    Err(_) => continue,
                                }
                            };
                            // Char-index search (not byte offsets) so UI selection matches.
                            let hay: Vec<char> = text
                                .chars
                                .iter()
                                .map(|c| c.ch.to_lowercase().next().unwrap_or(c.ch))
                                .collect();
                            let needle: Vec<char> = q.chars().collect();
                            if !needle.is_empty() {
                                let mut i = 0usize;
                                while i + needle.len() <= hay.len() {
                                    if hay[i..].starts_with(&needle) {
                                        let end = i + needle.len();
                                        let ctx_start = i.saturating_sub(20);
                                        let ctx_end = (end + 20).min(text.chars.len());
                                        let context: String = text.chars[ctx_start..ctx_end]
                                            .iter()
                                            .map(|c| c.ch)
                                            .collect();
                                        hits.push(SearchHit {
                                            page: page_info.id,
                                            start: i,
                                            end,
                                            context,
                                        });
                                        i = end;
                                        if hits.len() > 200 {
                                            break;
                                        }
                                    } else {
                                        i += 1;
                                    }
                                }
                            }
                            if hits.len() > 200 {
                                break;
                            }
                        }
                        let _ = evt_tx.send(WorkerEvent::SearchResults {
                            document_id,
                            query,
                            hits,
                        });
                    }
                }
                WorkerRequest::SaveFlattened {
                    document_id,
                    path,
                    scene,
                    page_heights,
                    reload,
                } => {
                    if let Some(doc) = opened.as_ref() {
                        if doc.model.id != document_id {
                            continue;
                        }
                        let heights: HashMap<u32, f32> = page_heights.into_iter().collect();
                        match doxo_pdf_lowlevel::flatten_scene_into_pdf(&doc.bytes, &scene, &heights)
                        {
                            Ok(bytes) => {
                                match doxo_pdf_lowlevel::atomic_write(&path, &bytes) {
                                    Ok(()) => {
                                        let _ = evt_tx.send(WorkerEvent::SaveComplete {
                                            path: path.clone(),
                                        });
                                        if reload {
                                            match backend.open_bytes(
                                                bytes,
                                                Some(path),
                                                doc.model.title.clone(),
                                            ) {
                                                Ok(new_doc) => {
                                                    tiles.clear();
                                                    thumbs.clear();
                                                    text_cache.clear();
                                                    let model = new_doc.model.clone();
                                                    opened = Some(new_doc);
                                                    let _ = evt_tx.send(
                                                        WorkerEvent::DocumentOpened {
                                                            model,
                                                            backend: backend.kind(),
                                                        },
                                                    );
                                                }
                                                Err(err) => {
                                                    let _ = evt_tx.send(WorkerEvent::Error {
                                                        message: format!(
                                                            "reload after save: {err}"
                                                        ),
                                                    });
                                                }
                                            }
                                        }
                                    }
                                    Err(err) => {
                                        let _ = evt_tx.send(WorkerEvent::Error {
                                            message: format!("atomic save: {err}"),
                                        });
                                    }
                                }
                            }
                            Err(err) => {
                                let _ = evt_tx.send(WorkerEvent::Error {
                                    message: format!("flatten: {err}"),
                                });
                            }
                        }
                    }
                }
                WorkerRequest::ApplyPageOp {
                    document_id,
                    op,
                    save_path,
                } => {
                    if let Some(doc) = opened.as_ref() {
                        if doc.model.id != document_id {
                            continue;
                        }
                        match doxo_pdf_lowlevel::apply_page_op(&doc.bytes, op) {
                            Ok(bytes) => {
                                if let Some(path) = &save_path {
                                    if let Err(err) =
                                        doxo_pdf_lowlevel::atomic_write(path, &bytes)
                                    {
                                        let _ = evt_tx.send(WorkerEvent::Error {
                                            message: format!("save after page op: {err}"),
                                        });
                                        continue;
                                    }
                                }
                                let path = save_path
                                    .or_else(|| doc.model.path.clone())
                                    .unwrap_or_else(|| PathBuf::from("untitled.pdf"));
                                match backend.open_bytes(bytes, Some(path), doc.model.title.clone())
                                {
                                    Ok(new_doc) => {
                                        tiles.clear();
                                        thumbs.clear();
                                        text_cache.clear();
                                        pending.clear();
                                        let model = new_doc.model.clone();
                                        opened = Some(new_doc);
                                        let _ = evt_tx.send(WorkerEvent::PageOpComplete { model });
                                    }
                                    Err(err) => {
                                        let _ = evt_tx.send(WorkerEvent::Error {
                                            message: format!("reload after page op: {err}"),
                                        });
                                    }
                                }
                            }
                            Err(err) => {
                                let _ = evt_tx.send(WorkerEvent::Error {
                                    message: format!("page op: {err}"),
                                });
                            }
                        }
                    }
                }
                WorkerRequest::ExtractPages {
                    document_id,
                    indices,
                    dest,
                } => {
                    if let Some(doc) = opened.as_ref() {
                        if doc.model.id != document_id {
                            continue;
                        }
                        match doxo_pdf_lowlevel::extract_pages(&doc.bytes, &indices) {
                            Ok(bytes) => {
                                match doxo_pdf_lowlevel::atomic_write(&dest, &bytes) {
                                    Ok(()) => {
                                        let _ = evt_tx.send(WorkerEvent::ExtractComplete {
                                            path: dest,
                                            page_count: indices.len() as u32,
                                        });
                                    }
                                    Err(err) => {
                                        let _ = evt_tx.send(WorkerEvent::Error {
                                            message: format!("extract save: {err}"),
                                        });
                                    }
                                }
                            }
                            Err(err) => {
                                let _ = evt_tx.send(WorkerEvent::Error {
                                    message: format!("extract: {err}"),
                                });
                            }
                        }
                    }
                }
            }
        }

        if let Some(item) = pending.pop_front() {
            if let Some(doc) = opened.as_ref() {
                if doc.model.id != item.document_id {
                    continue;
                }
                let key = TileKey::new(
                    item.document_id,
                    item.page,
                    item.zoom,
                    item.pixels_per_point,
                );
                if tiles.get(&key).is_some() {
                    continue;
                }
                match backend.render_page(doc, item.page, item.zoom, item.pixels_per_point) {
                    Ok((width, height, rgba)) => {
                        debug!(
                            page = item.page.0,
                            zoom = item.zoom,
                            ppp = item.pixels_per_point,
                            "page rendered"
                        );
                        tiles.insert(
                            key,
                            CachedImage {
                                width,
                                height,
                                rgba: rgba.clone(),
                            },
                        );
                        let _ = evt_tx.send(WorkerEvent::PageReady {
                            document_id: item.document_id,
                            page: item.page,
                            zoom: key.zoom(),
                            pixels_per_point: key.pixels_per_point(),
                            width,
                            height,
                            rgba,
                        });
                    }
                    Err(err) => {
                        let _ = evt_tx.send(WorkerEvent::Error {
                            message: format!("render page {}: {err}", item.page.0),
                        });
                    }
                }
            }
        } else if let Some((document_id, page, max_width)) = pending_thumbs.pop_front() {
            if let Some(doc) = opened.as_ref() {
                if doc.model.id != document_id {
                    continue;
                }
                let key = ThumbnailKey {
                    document_id,
                    page,
                };
                if thumbs.get(&key).is_some() {
                    continue;
                }
                match backend.render_thumbnail(doc, page, max_width) {
                    Ok((width, height, rgba)) => {
                        thumbs.insert(
                            key,
                            CachedImage {
                                width,
                                height,
                                rgba: rgba.clone(),
                            },
                        );
                        let _ = evt_tx.send(WorkerEvent::ThumbnailReady {
                            document_id,
                            page,
                            width,
                            height,
                            rgba,
                        });
                    }
                    Err(err) => {
                        let _ = evt_tx.send(WorkerEvent::Error {
                            message: format!("thumbnail {}: {err}", page.0),
                        });
                    }
                }
            }
        }
    }
}
