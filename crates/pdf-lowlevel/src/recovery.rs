use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use doxo_document::{EditCommand, EditorScene, PdfDocumentModel};

#[derive(Debug, Error)]
pub enum RecoveryError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("serde: {0}")]
    Serde(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoverySnapshot {
    pub source_pdf: PathBuf,
    pub model_title: String,
    pub scene: EditorScene,
    pub commands: Vec<EditCommand>,
    pub revision: u64,
    pub saved_at_unix: u64,
}

/// Crash-recovery autosave journal (JSON beside the PDF or in app support dir).
pub struct AutosaveStore {
    path: PathBuf,
}

impl AutosaveStore {
    pub fn for_document(pdf_path: &Path) -> Self {
        let mut p = pdf_path.to_path_buf();
        p.set_extension("doxo-journal.json");
        Self { path: p }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn save(
        &self,
        model: &PdfDocumentModel,
        scene: &EditorScene,
        commands: &[EditCommand],
        revision: u64,
    ) -> Result<(), RecoveryError> {
        let snap = RecoverySnapshot {
            source_pdf: model.path.clone().unwrap_or_default(),
            model_title: model.title.clone(),
            scene: scene.clone(),
            commands: commands.to_vec(),
            revision,
            saved_at_unix: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        };
        let bytes = serde_json::to_vec_pretty(&snap)?;
        crate::atomic::atomic_write(&self.path, &bytes).map_err(|e| {
            RecoveryError::Io(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
        })?;
        Ok(())
    }

    pub fn load(&self) -> Result<Option<RecoverySnapshot>, RecoveryError> {
        if !self.path.exists() {
            return Ok(None);
        }
        let bytes = std::fs::read(&self.path)?;
        let snap: RecoverySnapshot = serde_json::from_slice(&bytes)?;
        Ok(Some(snap))
    }

    pub fn clear(&self) -> Result<(), RecoveryError> {
        if self.path.exists() {
            std::fs::remove_file(&self.path)?;
        }
        Ok(())
    }
}
