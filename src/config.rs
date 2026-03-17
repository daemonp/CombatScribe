//! Lightweight app config persisted to disk.
//!
//! Stored at `{config_dir}/combatscribe/config.toml`.  All load/save errors
//! are silently swallowed — config is best-effort and must never crash the app.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

// ── Config Path ─────────────────────────────────────────────────────────────

const APP_DIR: &str = "combatscribe";
const CONFIG_FILE: &str = "config.toml";

/// Returns the full path to the config file, e.g.
/// - Linux:   `~/.config/combatscribe/config.toml`
/// - macOS:   `~/Library/Application Support/combatscribe/config.toml`
/// - Windows: `C:\Users\<user>\AppData\Roaming\combatscribe\config.toml`
fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_DIR).join(CONFIG_FILE))
}

// ── AppConfig ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppConfig {
    /// The directory the user last successfully opened a file from.
    pub last_directory: Option<PathBuf>,
}

impl AppConfig {
    /// Load config from disk.  Returns `Default` on any error (missing file,
    /// corrupt TOML, permissions, etc.).
    pub fn load() -> Self {
        let Some(path) = config_path() else {
            return Self::default();
        };
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Persist config to disk.  Silently ignores errors.
    pub fn save(&self) {
        let Some(path) = config_path() else {
            return;
        };
        // Ensure the parent directory exists
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(contents) = toml::to_string_pretty(self) {
            let _ = std::fs::write(&path, contents);
        }
    }

    /// Update `last_directory` from a file path (uses the file's parent dir).
    pub fn set_last_directory_from_file(&mut self, file_path: &std::path::Path) {
        if let Some(parent) = file_path.parent() {
            self.last_directory = Some(parent.to_path_buf());
        }
    }
}
