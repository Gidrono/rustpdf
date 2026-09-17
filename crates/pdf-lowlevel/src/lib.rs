//! Low-level PDF I/O: metadata, page ops, flatten overlays, atomic save.

mod atomic;
mod flatten;
mod metadata;
mod page_ops;
mod recovery;
mod text_rewrite;

pub use atomic::{atomic_write, AtomicSaveError};
pub use flatten::{flatten_scene_into_pdf, FlattenError};
pub use metadata::{inspect_pdf, PageBox, PdfInspectError, PdfInspection};
pub use page_ops::{apply_page_op, extract_pages, PageOp, PageOpError};
pub use recovery::{AutosaveStore, RecoverySnapshot};
pub use text_rewrite::{reconstruct_line, try_rewrite_page_literal, RewriteError};

/// Placeholder kept for API stability.
pub fn incremental_save_stub() {}
