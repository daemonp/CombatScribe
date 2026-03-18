//! Background version check against GitHub Releases.
//!
//! On app startup, fires a single HTTP request to the GitHub API to check for
//! a newer release.  If a newer version exists (and the user hasn't dismissed
//! that specific version), the app shows a non-intrusive banner.
//!
//! All errors are silently swallowed — network failures, DNS issues, rate
//! limits, or malformed responses simply result in no banner.

use semver::Version;

// ── Constants ───────────────────────────────────────────────────────────────

const GITHUB_API_URL: &str = "https://api.github.com/repos/daemonp/CombatScribe/releases/latest";

const RELEASE_PAGE_URL: &str = "https://github.com/daemonp/CombatScribe/releases/latest";

/// Current version from Cargo.toml, injected at compile time.
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

// ── Types ───────────────────────────────────────────────────────────────────

/// Information about a newer release, if one exists.
#[derive(Debug, Clone)]
pub struct NewRelease {
    /// The version string (e.g. "0.9.0").
    pub version: String,
    /// URL to the specific release page on GitHub.
    #[allow(dead_code)] // Stored for potential future use (e.g. tooltip, detail view)
    pub url: String,
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Check GitHub for a newer release.
///
/// This is a blocking function intended to be called inside `Task::perform()`.
/// Returns `Some(NewRelease)` if a newer version is available, `None` otherwise.
/// Never panics — all errors are silently absorbed.
pub fn check_for_update() -> Option<NewRelease> {
    check_for_update_inner().ok().flatten()
}

/// Open the release page in the user's default browser.
/// Silently ignores errors.
pub fn open_release_page() {
    let _ = open::that(RELEASE_PAGE_URL);
}

// ── Implementation ──────────────────────────────────────────────────────────

fn check_for_update_inner() -> Result<Option<NewRelease>, Box<dyn std::error::Error>> {
    let body: String = ureq::get(GITHUB_API_URL)
        .header("Accept", "application/vnd.github.v3+json")
        .header(
            "User-Agent",
            concat!("CombatScribe/", env!("CARGO_PKG_VERSION")),
        )
        .call()?
        .body_mut()
        .read_to_string()?;

    let json: serde_json::Value = serde_json::from_str(&body)?;

    let tag = json
        .get("tag_name")
        .and_then(serde_json::Value::as_str)
        .ok_or("missing tag_name")?;

    // Strip leading 'v' if present (e.g. "v0.9.0" → "0.9.0")
    let version_str = tag.strip_prefix('v').unwrap_or(tag);
    let remote = Version::parse(version_str)?;
    let current = Version::parse(CURRENT_VERSION)?;

    if remote > current {
        let url = json
            .get("html_url")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(RELEASE_PAGE_URL)
            .to_string();

        Ok(Some(NewRelease {
            version: version_str.to_string(),
            url,
        }))
    } else {
        Ok(None)
    }
}
