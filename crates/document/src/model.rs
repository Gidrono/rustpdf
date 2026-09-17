use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Stable identity for an open document session (not a PDF object id).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DocumentId(pub Uuid);

impl DocumentId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for DocumentId {
    fn default() -> Self {
        Self::new()
    }
}

/// Zero-based page index with a typed wrapper.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize, Default)]
pub struct PageId(pub u32);

impl PageId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Page geometry in PDF user-space points (1/72").
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PageInfo {
    pub id: PageId,
    pub width_pts: f32,
    pub height_pts: f32,
}

/// Immutable-ish PDF model snapshot used by the viewer.
///
/// The **renderer** consumes this for layout; the **editor model** (journal)
/// mutates a separate layer and eventually invalidates caches.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdfDocumentModel {
    pub id: DocumentId,
    pub path: Option<std::path::PathBuf>,
    pub title: String,
    pub page_count: u32,
    pub pages: Vec<PageInfo>,
    /// True when pages came from a mock / metadata-only backend.
    pub is_placeholder: bool,
}

impl PdfDocumentModel {
    pub fn page(&self, id: PageId) -> Option<&PageInfo> {
        self.pages.get(id.index())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DocumentMeta {
    pub author: Option<String>,
    pub subject: Option<String>,
    pub keywords: Option<String>,
}
