//! Lightweight app config persisted to disk.
//!
//! Stored at `{config_dir}/combatscribe/config.toml`.  All load/save errors
//! are silently swallowed — config is best-effort and must never crash the app.

use std::collections::HashSet;
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
    /// Aura names the user has selected for display on the timeline waterfall.
    #[serde(default)]
    pub tracked_auras: HashSet<String>,
    /// Version string the user chose to dismiss (e.g. "0.9.0").
    /// The update banner won't show for this version, but will reappear
    /// when an even newer release is published.
    #[serde(default)]
    pub dismissed_version: Option<String>,
    /// Saved view preferences (None = use built-in defaults).
    ///
    /// Role-specific settings (damage/healing type, per-second toggles,
    /// default tab) that the user explicitly saves via the eye icon.
    #[serde(default)]
    pub view: Option<ViewPrefs>,
}

/// Persistent view preferences for role-specific UI settings.
///
/// Fields are stored as strings for forward-compatibility: adding new enum
/// variants won't break existing config files (unrecognized strings fall back
/// to defaults).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ViewPrefs {
    /// Damage panel type: Damage, `DamageWithPets`, or `DamageTaken`.
    pub damage_type: String,
    /// Healing panel type: Healing, Effective, Raw, or Overhealing.
    pub healing_type: String,
    /// Show per-second rates on the damage panel.
    pub damage_per_second: bool,
    /// Show per-second rates on the healing panel.
    pub healing_per_second: bool,
    /// Default tab: Meters, Utility, `DeathLog`, Timeline, Loot, or Events.
    pub default_tab: String,
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

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_view_prefs_round_trip() {
        let prefs = ViewPrefs {
            damage_type: "DamageWithPets".to_string(),
            healing_type: "Healing".to_string(),
            damage_per_second: true,
            healing_per_second: false,
            default_tab: "DeathLog".to_string(),
        };

        let config = AppConfig {
            last_directory: None,
            tracked_auras: HashSet::new(),
            dismissed_version: None,
            view: Some(prefs.clone()),
        };

        let serialized = toml::to_string_pretty(&config).expect("serialize config");
        let deserialized: AppConfig = toml::from_str(&serialized).expect("deserialize config");

        assert_eq!(deserialized.view, Some(prefs));
    }

    #[test]
    fn test_config_missing_view_prefs_defaults_to_none() {
        let toml_str = r#"tracked_auras = []
"#;
        let config: AppConfig = toml::from_str(toml_str).expect("parse minimal config");
        assert!(config.view.is_none());
    }
}
