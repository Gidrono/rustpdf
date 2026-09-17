use doxo_document::{DocumentId, ExtractedPageText, PageId};
use serde::{Deserialize, Serialize};

/// Requests from UI → PdfWorker. All PDF I/O happens on the worker thread.
#[derive(Debug, Clone)]
pub enum WorkerRequest {
    OpenDocument { path: std::path::PathBuf },
    /// Reload from in-memory bytes (after page ops / flatten), keeping path.
    ReloadBytes {
        bytes: Vec<u8>,
        path: Option<std::path::PathBuf>,
        title: String,
    },
    CloseDocument,
    RenderPage {
        document_id: DocumentId,
        page: PageId,
        zoom: f32,
        /// egui `pixels_per_point` (Retina DPR); raster size = zoom × DPR.
        pixels_per_point: f32,
        priority: RenderPriority,
    },
    RenderThumbnail {
        document_id: DocumentId,
        page: PageId,
        max_width: u32,
    },
    ExtractText {
        document_id: DocumentId,
        page: PageId,
    },
    Search {
        document_id: DocumentId,
        query: String,
    },
    /// Flatten scene overlays into PDF and write atomically.
    SaveFlattened {
        document_id: DocumentId,
        path: std::path::PathBuf,
        scene: doxo_document::EditorScene,
        page_heights: Vec<(u32, f32)>,
        /// If true, also reload worker from saved bytes.
        reload: bool,
    },
    ApplyPageOp {
        document_id: DocumentId,
        op: doxo_pdf_lowlevel::PageOp,
        /// Optional path to write atomically after op (save-in-place).
        save_path: Option<std::path::PathBuf>,
    },
    /// Extract pages into a brand-new PDF file (does not modify the open doc).
    ExtractPages {
        document_id: DocumentId,
        indices: Vec<u32>,
        dest: std::path::PathBuf,
    },
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RenderPriority {
    Visible,
    Prefetch,
    Thumbnail,
}

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub page: PageId,
    pub start: usize,
    pub end: usize,
    pub context: String,
}

/// Events from PdfWorker → UI.
#[derive(Debug, Clone)]
pub enum WorkerEvent {
    BackendReady {
        kind: crate::BackendKind,
        detail: String,
    },
    DocumentOpened {
        model: doxo_document::PdfDocumentModel,
        backend: crate::BackendKind,
    },
    DocumentClosed {
        document_id: DocumentId,
    },
    PageReady {
        document_id: DocumentId,
        page: PageId,
        zoom: f32,
        pixels_per_point: f32,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
    ThumbnailReady {
        document_id: DocumentId,
        page: PageId,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
    TextExtracted {
        document_id: DocumentId,
        text: ExtractedPageText,
    },
    SearchResults {
        document_id: DocumentId,
        query: String,
        hits: Vec<SearchHit>,
    },
    SaveComplete {
        path: std::path::PathBuf,
    },
    ExtractComplete {
        path: std::path::PathBuf,
        page_count: u32,
    },
    PageOpComplete {
        model: doxo_document::PdfDocumentModel,
    },
    Error {
        message: String,
    },
    ShutdownComplete,
}
