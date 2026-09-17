//! macOS platform helpers: PDFium paths, clipboard, fonts, recovery dir.

use std::path::PathBuf;

use anyhow::Result;

/// Directories to search for `libpdfium.dylib` at runtime.
pub fn pdfium_search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Ok(custom) = std::env::var("DOXO_PDFIUM_DIR") {
        paths.push(PathBuf::from(custom));
    }
    if let Ok(custom) = std::env::var("PDFIUM_DYNAMIC_LIB_PATH") {
        paths.push(PathBuf::from(custom));
    }

    paths.push(PathBuf::from("vendor/pdfium"));
    paths.push(PathBuf::from("./vendor/pdfium"));

    if let Ok(cwd) = std::env::current_dir() {
        paths.push(cwd.join("vendor/pdfium"));
    }

    paths.push(PathBuf::from("/opt/homebrew/lib"));
    paths.push(PathBuf::from("/usr/local/lib"));

    paths
}

pub fn register_document_types_stub() {}

/// Application support directory for crash journals / prefs.
pub fn app_support_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let dir = PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("doxo");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Read image bytes from the macOS pasteboard via `osascript`/`pngpaste`-free path:
/// uses `pbpaste` is text-only; for images we shell out to Swift/osascript.
/// V1: try reading a temporary paste via `osascript` JPEG, else None.
pub fn clipboard_image_png() -> Option<Vec<u8>> {
    // Prefer reading from NSPasteboard via a tiny osascript that writes a temp file.
    let tmp = std::env::temp_dir().join(format!("doxo-paste-{}.png", std::process::id()));
    let script = format!(
        r#"set theFile to POSIX file "{}"
try
  set pngData to the clipboard as «class PNGf»
  set f to open for access theFile with write permission
  write pngData to f
  close access f
  return "ok"
on error
  try
    close access theFile
  end try
  return "fail"
end try"#,
        tmp.display()
    );
    let status = std::process::Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .ok()?;
    if !status.status.success() {
        return None;
    }
    let bytes = std::fs::read(&tmp).ok()?;
    let _ = std::fs::remove_file(&tmp);
    if bytes.is_empty() {
        None
    } else {
        Some(bytes)
    }
}

pub fn clipboard_text() -> Option<String> {
    let out = std::process::Command::new("pbpaste").output().ok()?;
    if out.status.success() {
        String::from_utf8(out.stdout).ok().filter(|s| !s.is_empty())
    } else {
        None
    }
}

pub fn set_clipboard_text(text: &str) -> Result<()> {
    use std::io::Write;
    let mut child = std::process::Command::new("pbcopy")
        .stdin(std::process::Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes())?;
    }
    let _ = child.wait()?;
    Ok(())
}

/// Common macOS font directories for DOXO_FONT_PATH discovery.
pub fn system_font_candidates() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/System/Library/Fonts/Supplemental/Arial.ttf"),
        PathBuf::from("/System/Library/Fonts/Helvetica.ttc"),
        PathBuf::from("/Library/Fonts/Arial.ttf"),
    ]
}
