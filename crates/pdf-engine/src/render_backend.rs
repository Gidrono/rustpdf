use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use doxo_document::{DocumentId, ExtractedChar, ExtractedPageText, PageId, PageInfo, PdfDocumentModel, RectPts};
use doxo_pdf_lowlevel::inspect_pdf;
use image::{Rgba, RgbaImage};
use tracing::{info, warn};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    /// Real PDFium via pdfium-render.
    Pdfium,
    /// Placeholder tiles so the UI shell works without libpdfium.
    Mock,
}

#[derive(Debug, Clone)]
pub struct RenderBackendInfo {
    pub kind: BackendKind,
    pub detail: String,
}

pub struct OpenedDocument {
    pub model: PdfDocumentModel,
    pub path: PathBuf,
    /// Source PDF bytes (for save / page ops / reload).
    pub bytes: Vec<u8>,
}

pub trait RenderBackend: Send {
    fn kind(&self) -> BackendKind;
    fn info(&self) -> RenderBackendInfo;
    fn open(&mut self, path: &Path) -> Result<OpenedDocument>;
    fn open_bytes(
        &mut self,
        bytes: Vec<u8>,
        path: Option<PathBuf>,
        title: String,
    ) -> Result<OpenedDocument>;
    fn render_page(
        &mut self,
        opened: &OpenedDocument,
        page: PageId,
        zoom: f32,
        pixels_per_point: f32,
    ) -> Result<(u32, u32, Vec<u8>)>;
    fn render_thumbnail(
        &mut self,
        opened: &OpenedDocument,
        page: PageId,
        max_width: u32,
    ) -> Result<(u32, u32, Vec<u8>)>;
    fn extract_text(
        &mut self,
        opened: &OpenedDocument,
        page: PageId,
    ) -> Result<ExtractedPageText>;
}

/// Try PDFium first; fall back to mock so the app always runs.
pub fn create_backend(pdfium_search_paths: &[PathBuf]) -> Box<dyn RenderBackend> {
    #[cfg(feature = "pdfium")]
    {
        match PdfiumBackend::try_new(pdfium_search_paths) {
            Ok(backend) => {
                info!(detail = %backend.info().detail, "using PDFium backend");
                return Box::new(backend);
            }
            Err(err) => {
                warn!(error = %err, "PDFium unavailable; falling back to mock renderer");
            }
        }
    }
    #[cfg(not(feature = "pdfium"))]
    {
        let _ = pdfium_search_paths;
        warn!("built without `pdfium` feature; using mock renderer");
    }
    Box::new(MockBackend::new())
}

// --- Mock -------------------------------------------------------------------

pub struct MockBackend;

impl MockBackend {
    pub fn new() -> Self {
        Self
    }
}

impl RenderBackend for MockBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Mock
    }

    fn info(&self) -> RenderBackendInfo {
        RenderBackendInfo {
            kind: BackendKind::Mock,
            detail: "mock tiles (install libpdfium for real rendering)".into(),
        }
    }

    fn open(&mut self, path: &Path) -> Result<OpenedDocument> {
        let bytes = std::fs::read(path).unwrap_or_default();
        let inspection = inspect_pdf(path).unwrap_or_else(|_| {
            doxo_pdf_lowlevel::PdfInspection {
                page_count: 8,
                pages: (0..8)
                    .map(|_| doxo_pdf_lowlevel::PageBox {
                        width_pts: 612.0,
                        height_pts: 792.0,
                    })
                    .collect(),
            }
        });

        let id = DocumentId::new();
        let pages: Vec<PageInfo> = inspection
            .pages
            .iter()
            .enumerate()
            .map(|(i, b)| PageInfo {
                id: PageId(i as u32),
                width_pts: b.width_pts,
                height_pts: b.height_pts,
            })
            .collect();

        let title = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled")
            .to_string();

        Ok(OpenedDocument {
            model: PdfDocumentModel {
                id,
                path: Some(path.to_path_buf()),
                title,
                page_count: pages.len() as u32,
                pages,
                is_placeholder: true,
            },
            path: path.to_path_buf(),
            bytes,
        })
    }

    fn open_bytes(
        &mut self,
        bytes: Vec<u8>,
        path: Option<PathBuf>,
        title: String,
    ) -> Result<OpenedDocument> {
        let tmp = std::env::temp_dir().join(format!("doxo-mock-{}.pdf", uuid_simple()));
        std::fs::write(&tmp, &bytes).ok();
        let inspection = inspect_pdf(&tmp).unwrap_or_else(|_| doxo_pdf_lowlevel::PdfInspection {
            page_count: 1,
            pages: vec![doxo_pdf_lowlevel::PageBox {
                width_pts: 612.0,
                height_pts: 792.0,
            }],
        });
        let _ = std::fs::remove_file(&tmp);
        let pages: Vec<PageInfo> = inspection
            .pages
            .iter()
            .enumerate()
            .map(|(i, b)| PageInfo {
                id: PageId(i as u32),
                width_pts: b.width_pts,
                height_pts: b.height_pts,
            })
            .collect();
        let path = path.unwrap_or_else(|| PathBuf::from("untitled.pdf"));
        Ok(OpenedDocument {
            model: PdfDocumentModel {
                id: DocumentId::new(),
                path: Some(path.clone()),
                title,
                page_count: pages.len() as u32,
                pages,
                is_placeholder: true,
            },
            path,
            bytes,
        })
    }

    fn render_page(
        &mut self,
        opened: &OpenedDocument,
        page: PageId,
        zoom: f32,
        pixels_per_point: f32,
    ) -> Result<(u32, u32, Vec<u8>)> {
        let info = opened
            .model
            .page(page)
            .ok_or_else(|| anyhow!("page out of range"))?;
        let (w, h) = crate::page_size_px(info.width_pts, info.height_pts, zoom, pixels_per_point);
        Ok(render_mock_page(w, h, page.0, zoom, false))
    }

    fn render_thumbnail(
        &mut self,
        opened: &OpenedDocument,
        page: PageId,
        max_width: u32,
    ) -> Result<(u32, u32, Vec<u8>)> {
        let info = opened
            .model
            .page(page)
            .ok_or_else(|| anyhow!("page out of range"))?;
        let aspect = info.height_pts / info.width_pts.max(1.0);
        let w = max_width.max(32);
        let h = ((w as f32) * aspect).round().max(1.0) as u32;
        Ok(render_mock_page(w, h, page.0, 1.0, true))
    }

    fn extract_text(
        &mut self,
        opened: &OpenedDocument,
        page: PageId,
    ) -> Result<ExtractedPageText> {
        let plain = format!("(mock text page {})", page.0 + 1);
        let chars: Vec<ExtractedChar> = plain
            .chars()
            .enumerate()
            .map(|(i, ch)| ExtractedChar {
                ch,
                rect: RectPts {
                    x: 72.0 + i as f32 * 8.0,
                    y: 700.0,
                    w: 8.0,
                    h: 12.0,
                },
                font_size: 12.0,
            })
            .collect();
        Ok(ExtractedPageText {
            page,
            chars,
            plain,
        })
    }
}

fn uuid_simple() -> String {
    format!(
        "{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    )
}

fn render_mock_page(w: u32, h: u32, page_index: u32, zoom: f32, thumb: bool) -> (u32, u32, Vec<u8>) {
    let mut img = RgbaImage::from_pixel(w, h, Rgba([245, 245, 242, 255]));
    // Subtle page tint stripes so scrolling feels real.
    let stripe = if page_index % 2 == 0 { 12u8 } else { 0 };
    for y in 0..h {
        for x in 0..w {
            let edge = x < 2 || y < 2 || x + 2 >= w || y + 2 >= h;
            if edge {
                img.put_pixel(x, y, Rgba([180, 180, 175, 255]));
            } else if (y / 8 + x / 12) % 7 == 0 {
                let c = 220u8.saturating_sub(stripe);
                img.put_pixel(x, y, Rgba([c, c, c.saturating_sub(4), 255]));
            }
        }
    }
    // Corner marker encoding page number (visible without fonts).
    let label_w = (w / 5).max(8).min(64);
    let label_h = (h / 20).max(6).min(24);
    let tone = 40 + ((page_index * 37) % 140) as u8;
    for y in 8..(8 + label_h).min(h) {
        for x in 8..(8 + label_w).min(w) {
            img.put_pixel(x, y, Rgba([tone, tone, 255u8.saturating_sub(tone / 2), 255]));
        }
    }
    if !thumb {
        // Zoom fingerprint bar
        let bar = ((zoom * 40.0) as u32).clamp(4, w.saturating_sub(16));
        for x in 8..(8 + bar) {
            for y in (h.saturating_sub(16))..h.saturating_sub(8) {
                img.put_pixel(x, y, Rgba([60, 120, 200, 255]));
            }
        }
    }
    (w, h, img.into_raw())
}

// --- PDFium -----------------------------------------------------------------

#[cfg(feature = "pdfium")]
pub struct PdfiumBackend {
    pdfium: pdfium_render::prelude::Pdfium,
    detail: String,
    /// Currently open document kept on the worker thread.
    open_bytes: Option<(PathBuf, Vec<u8>)>,
}

#[cfg(feature = "pdfium")]
impl PdfiumBackend {
    pub fn try_new(search_paths: &[PathBuf]) -> Result<Self> {
        use pdfium_render::prelude::*;

        let mut candidates: Vec<PathBuf> = Vec::new();
        for dir in search_paths {
            candidates.push(Pdfium::pdfium_platform_library_name_at_path(dir));
        }
        // CWD / next to binary conveniences
        candidates.push(Pdfium::pdfium_platform_library_name_at_path("./"));
        candidates.push(Pdfium::pdfium_platform_library_name_at_path("./vendor/pdfium/"));
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                candidates.push(Pdfium::pdfium_platform_library_name_at_path(dir));
                candidates.push(dir.join("vendor").join("pdfium").join(
                    Pdfium::pdfium_platform_library_name()
                        .to_string_lossy()
                        .as_ref(),
                ));
            }
        }
        // Common Homebrew locations on Apple Silicon / Intel
        candidates.push(Pdfium::pdfium_platform_library_name_at_path("/opt/homebrew/lib/"));
        candidates.push(Pdfium::pdfium_platform_library_name_at_path("/usr/local/lib/"));

        let mut last_err = None;
        for path in &candidates {
            if !path.exists() {
                continue;
            }
            match Pdfium::bind_to_library(path) {
                Ok(bindings) => {
                    return Ok(Self {
                        pdfium: Pdfium::new(bindings),
                        detail: format!("loaded {}", path.display()),
                        open_bytes: None,
                    });
                }
                Err(e) => last_err = Some(anyhow!("bind {}: {e}", path.display())),
            }
        }

        // System library fallback
        match Pdfium::bind_to_system_library() {
            Ok(bindings) => Ok(Self {
                pdfium: Pdfium::new(bindings),
                detail: "system library".into(),
                open_bytes: None,
            }),
            Err(e) => Err(last_err.unwrap_or_else(|| anyhow!("system pdfium: {e}"))),
        }
    }

    fn with_document<R>(
        &mut self,
        opened: &OpenedDocument,
        f: impl FnOnce(&pdfium_render::prelude::PdfDocument<'_>) -> Result<R>,
    ) -> Result<R> {
        let bytes = if !opened.bytes.is_empty() {
            opened.bytes.clone()
        } else {
            match &self.open_bytes {
                Some((p, b)) if p == &opened.path => b.clone(),
                _ => {
                    let b = std::fs::read(&opened.path)
                        .with_context(|| format!("read {}", opened.path.display()))?;
                    self.open_bytes = Some((opened.path.clone(), b.clone()));
                    b
                }
            }
        };

        let document = self
            .pdfium
            .load_pdf_from_byte_slice(&bytes, None)
            .map_err(|e| anyhow!("pdfium open: {e}"))?;
        f(&document)
    }

    fn open_from_bytes(
        &mut self,
        bytes: Vec<u8>,
        path: Option<PathBuf>,
        title: Option<String>,
    ) -> Result<OpenedDocument> {
        let (page_count, pages) = {
            let document = self
                .pdfium
                .load_pdf_from_byte_slice(&bytes, None)
                .map_err(|e| anyhow!("pdfium open: {e}"))?;

            let page_count = document.pages().len();
            if page_count == 0 {
                return Err(anyhow!("PDF has no pages"));
            }

            let mut pages = Vec::with_capacity(page_count as usize);
            for (i, page) in document.pages().iter().enumerate() {
                let width_pts = page.width().value;
                let height_pts = page.height().value;
                pages.push(PageInfo {
                    id: PageId(i as u32),
                    width_pts,
                    height_pts,
                });
            }
            (page_count, pages)
        };

        let path = path.unwrap_or_else(|| PathBuf::from("untitled.pdf"));
        let title = title.unwrap_or_else(|| {
            path.file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("Untitled")
                .to_string()
        });

        self.open_bytes = Some((path.clone(), bytes.clone()));

        Ok(OpenedDocument {
            model: PdfDocumentModel {
                id: DocumentId::new(),
                path: Some(path.clone()),
                title,
                page_count: u32::from(page_count),
                pages,
                is_placeholder: false,
            },
            path,
            bytes,
        })
    }
}

#[cfg(feature = "pdfium")]
impl RenderBackend for PdfiumBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Pdfium
    }

    fn info(&self) -> RenderBackendInfo {
        RenderBackendInfo {
            kind: BackendKind::Pdfium,
            detail: self.detail.clone(),
        }
    }

    fn open(&mut self, path: &Path) -> Result<OpenedDocument> {
        let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
        self.open_from_bytes(bytes, Some(path.to_path_buf()), None)
    }

    fn open_bytes(
        &mut self,
        bytes: Vec<u8>,
        path: Option<PathBuf>,
        title: String,
    ) -> Result<OpenedDocument> {
        self.open_from_bytes(bytes, path, Some(title))
    }

    fn render_page(
        &mut self,
        opened: &OpenedDocument,
        page: PageId,
        zoom: f32,
        pixels_per_point: f32,
    ) -> Result<(u32, u32, Vec<u8>)> {
        use pdfium_render::prelude::*;

        let info = opened
            .model
            .page(page)
            .ok_or_else(|| anyhow!("page out of range"))?;
        let (target_w, target_h) =
            crate::page_size_px(info.width_pts, info.height_pts, zoom, pixels_per_point);
        let index: u16 = page
            .0
            .try_into()
            .map_err(|_| anyhow!("page index exceeds PDFium u16 limit"))?;

        self.with_document(opened, |document| {
            let pdf_page = document
                .pages()
                .get(index)
                .map_err(|e| anyhow!("page get: {e}"))?;

            let config = PdfRenderConfig::new()
                .set_target_width(target_w as i32)
                .set_maximum_height(target_h as i32);

            let bitmap = pdf_page
                .render_with_config(&config)
                .map_err(|e| anyhow!("render: {e}"))?;

            let image = bitmap.as_image();
            let rgba = image.to_rgba8();
            Ok((rgba.width(), rgba.height(), rgba.into_raw()))
        })
    }

    fn render_thumbnail(
        &mut self,
        opened: &OpenedDocument,
        page: PageId,
        max_width: u32,
    ) -> Result<(u32, u32, Vec<u8>)> {
        use pdfium_render::prelude::*;

        let info = opened
            .model
            .page(page)
            .ok_or_else(|| anyhow!("page out of range"))?;
        let aspect = info.height_pts / info.width_pts.max(1.0);
        let w = max_width.max(32);
        let h = ((w as f32) * aspect).round().max(1.0) as u32;
        let index: u16 = page
            .0
            .try_into()
            .map_err(|_| anyhow!("page index exceeds PDFium u16 limit"))?;

        self.with_document(opened, |document| {
            let pdf_page = document
                .pages()
                .get(index)
                .map_err(|e| anyhow!("page get: {e}"))?;

            let config = PdfRenderConfig::new()
                .set_target_width(w as i32)
                .set_maximum_height(h as i32);

            let bitmap = pdf_page
                .render_with_config(&config)
                .map_err(|e| anyhow!("thumb render: {e}"))?;

            let rgba = bitmap.as_image().to_rgba8();
            Ok((rgba.width(), rgba.height(), rgba.into_raw()))
        })
    }

    fn extract_text(
        &mut self,
        opened: &OpenedDocument,
        page: PageId,
    ) -> Result<ExtractedPageText> {
        let index: u16 = page
            .0
            .try_into()
            .map_err(|_| anyhow!("page index exceeds PDFium u16 limit"))?;
        let page_w = opened
            .model
            .page(page)
            .map(|p| p.width_pts)
            .unwrap_or(612.0);
        let page_h = opened
            .model
            .page(page)
            .map(|p| p.height_pts)
            .unwrap_or(792.0);

        self.with_document(opened, |document| {
            let pdf_page = document
                .pages()
                .get(index)
                .map_err(|e| anyhow!("page get: {e}"))?;
            let text_page = pdf_page
                .text()
                .map_err(|e| anyhow!("text page: {e}"))?;

            let plain = text_page.all();
            let mut chars = Vec::with_capacity(plain.chars().count());

            // Use per-character boxes when available (pdfium-render PdfPageTextChar).
            let char_count = text_page.chars().len();
            if char_count > 0 {
                for i in 0..char_count {
                    if let Ok(ch) = text_page.chars().get(i) {
                        let unicode = ch.unicode_char().unwrap_or('\u{FFFD}');
                        let tight = ch.tight_bounds().ok();
                        let (x, y, w, h) = if let Some(b) = tight {
                            (
                                b.left().value,
                                b.bottom().value,
                                (b.right().value - b.left().value).abs().max(1.0),
                                (b.top().value - b.bottom().value).abs().max(1.0),
                            )
                        } else {
                            (72.0, page_h - 100.0, 8.0, 12.0)
                        };
                        let font_size = ch.unscaled_font_size().value;
                        chars.push(ExtractedChar {
                            ch: unicode,
                            rect: RectPts { x, y, w, h },
                            font_size,
                        });
                    }
                }
            }

            if chars.is_empty() && !plain.is_empty() {
                let n = plain.chars().count().max(1) as f32;
                for (i, ch) in plain.chars().enumerate() {
                    let x = 72.0 + (i as f32 / n) * (page_w - 144.0).max(10.0);
                    chars.push(ExtractedChar {
                        ch,
                        rect: RectPts {
                            x,
                            y: page_h - 100.0,
                            w: ((page_w - 144.0) / n).max(4.0),
                            h: 12.0,
                        },
                        font_size: 12.0,
                    });
                }
            }

            Ok(ExtractedPageText {
                page,
                chars,
                plain,
            })
        })
    }
}
