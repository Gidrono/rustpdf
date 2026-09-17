//! Document layer: PDF model + editor model (scene + journal).

mod edit_journal;
mod model;
mod scene;

pub use edit_journal::{
    make_highlight, make_image, make_ink, make_shape, make_textbox, EditCommand, EditJournal,
    JournalError,
};
pub use model::{DocumentId, DocumentMeta, PageId, PageInfo, PdfDocumentModel};
pub use scene::{
    EditorObject, EditorScene, ExtractedChar, ExtractedPageText, ImageFormat, ObjectId, RectPts,
    RgbaColor, ShapeKind, TextStyle,
};

pub mod prelude {
    pub use crate::{
        DocumentId, EditCommand, EditJournal, EditorObject, EditorScene, PageId, PageInfo,
        PdfDocumentModel,
    };
}
