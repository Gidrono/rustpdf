//! PDF render engine: dedicated worker thread + tile/thumbnail caches.
//!
//! Architecture (Phase 1):
//! - UI sends [`WorkerRequest`]s over a crossbeam channel (never calls PDFium on UI thread)
//! - Worker replies with [`WorkerEvent`]s
//! - Visible pages / zoom levels are rendered lazily into a basic tile cache
//!
//! TODO(Phase 2): invalidate tiles when EditJournal mutates page content.
//! TODO(Phase 2): progressive / tiled sub-rect rendering for very large zooms.
//! TODO(Phase 4): annotation / form overlays as separate render layers.

mod cache;
mod messages;
mod render_backend;
mod worker;

pub use cache::{CacheStats, TileCache, TileKey, ThumbnailCache};
pub use messages::{RenderPriority, SearchHit, WorkerEvent, WorkerRequest};
pub use render_backend::{BackendKind, RenderBackendInfo};
pub use worker::{PdfWorker, PdfWorkerHandle};

/// Points → device pixels: egui 96 DPI baseline × zoom × `pixels_per_point` (Retina DPR).
///
/// Layout stays in egui points (`zoom * 96/72`); bitmaps must include DPR so texels
/// match physical framebuffer pixels (~1:1 on Retina at 100% zoom).
pub fn page_size_px(
    width_pts: f32,
    height_pts: f32,
    zoom: f32,
    pixels_per_point: f32,
) -> (u32, u32) {
    let ppp = pixels_per_point.max(0.5);
    let scale = zoom * (96.0 / 72.0) * ppp;
    let w = (width_pts * scale).round().max(1.0) as u32;
    let h = (height_pts * scale).round().max(1.0) as u32;
    (w, h)
}

#[cfg(test)]
mod tests {
    use super::page_size_px;

    #[test]
    fn page_size_includes_device_pixel_ratio() {
        let (w1, h1) = page_size_px(612.0, 792.0, 1.0, 1.0);
        let (w2, h2) = page_size_px(612.0, 792.0, 1.0, 2.0);
        assert_eq!(w1, 816);
        assert_eq!(h1, 1056);
        assert_eq!(w2, 1632);
        assert_eq!(h2, 2112);
    }
}
