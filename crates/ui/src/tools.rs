use doxo_document::ShapeKind;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum EditorTool {
    #[default]
    Select,
    Text,
    Pencil,
    Highlight,
    Eraser,
    Rectangle,
    Ellipse,
    Line,
    Arrow,
    Image,
}

impl EditorTool {
    pub fn label(self) -> &'static str {
        match self {
            Self::Select => "Select",
            Self::Text => "Text",
            Self::Pencil => "Pencil",
            Self::Highlight => "Highlight",
            Self::Eraser => "Eraser",
            Self::Rectangle => "Rect",
            Self::Ellipse => "Ellipse",
            Self::Line => "Line",
            Self::Arrow => "Arrow",
            Self::Image => "Image",
        }
    }

    pub fn shape_kind(self) -> Option<ShapeKind> {
        match self {
            Self::Rectangle => Some(ShapeKind::Rectangle),
            Self::Ellipse => Some(ShapeKind::Ellipse),
            Self::Line => Some(ShapeKind::Line),
            Self::Arrow => Some(ShapeKind::Arrow),
            _ => None,
        }
    }
}
