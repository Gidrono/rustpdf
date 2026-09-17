use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AtomicSaveError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("validation failed: empty output")]
    Empty,
}

/// Write bytes atomically: `.tmp` → fsync → rename over destination.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), AtomicSaveError> {
    if bytes.is_empty() {
        return Err(AtomicSaveError::Empty);
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let mut tmp = PathBuf::from(path);
    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("document.pdf");
    tmp.set_file_name(format!(".{file_name}.doxo.tmp"));

    {
        let mut f = File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }

    // Basic PDF header validation
    if !bytes.starts_with(b"%PDF") {
        let _ = fs::remove_file(&tmp);
        return Err(AtomicSaveError::Empty);
    }

    fs::rename(&tmp, path)?;

    // Best-effort directory fsync on Unix
    #[cfg(unix)]
    {
        if let Ok(dir) = File::open(parent) {
            let _ = dir.sync_all();
        }
    }

    Ok(())
}
