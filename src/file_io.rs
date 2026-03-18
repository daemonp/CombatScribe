// ── Async File Loading & Zip Handling ────────────────────────────────────────
//
// Provides file-picker dialog, async file reading, and zip extraction
// used by the iced application and CLI diagnostic modes.

use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;

pub async fn pick_file(start_dir: Option<PathBuf>) -> Option<PathBuf> {
    let mut dialog = rfd::AsyncFileDialog::new()
        .add_filter("Combat Log", &["txt", "zip"])
        .add_filter("All Files", &["*"])
        .set_title("Select WoW Combat Log");

    if let Some(dir) = start_dir {
        dialog = dialog.set_directory(dir);
    }

    dialog.pick_file().await.map(|f| f.path().to_path_buf())
}

pub async fn load_file(path: PathBuf) -> Result<Arc<Vec<String>>, String> {
    let content = if is_zip_file(&path) {
        // Read the raw bytes and extract the first .txt from the zip
        let bytes = tokio::fs::read(&path)
            .await
            .map_err(|e| format!("Failed to read zip file: {e}"))?;
        read_text_from_zip_bytes(&bytes)?
    } else {
        tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| format!("Failed to read file: {e}"))?
    };

    let lines: Vec<String> = content.lines().map(str::to_string).collect();
    Ok(Arc::new(lines))
}

/// Check if a path looks like a zip file (by extension).
pub fn is_zip_file(path: &std::path::Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
}

/// Extract the first `.txt` file from a zip archive's raw bytes.
pub fn read_text_from_zip_bytes(bytes: &[u8]) -> Result<String, String> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| format!("Failed to open zip archive: {e}"))?;

    // Find the first .txt entry
    let txt_index = (0..archive.len())
        .find(|&i| {
            archive
                .by_index(i)
                .is_ok_and(|f| f.name().to_lowercase().ends_with(".txt"))
        })
        .ok_or_else(|| "No .txt file found inside zip archive".to_string())?;

    let mut file = archive
        .by_index(txt_index)
        .map_err(|e| format!("Failed to read file from zip: {e}"))?;

    let mut content = String::new();
    file.read_to_string(&mut content)
        .map_err(|e| format!("Failed to read text from zip entry '{}': {e}", file.name()))?;

    Ok(content)
}
