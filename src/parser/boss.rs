//! Boss/zone lookups and date formatting.

use crate::raid_data;

// ── Delegates to raid_data ──────────────────────────────────────────────────
// These thin wrappers keep the call sites concise while all actual data lives
// in the build-time-generated raid_data module.

pub(crate) fn is_known_boss(name: &str) -> bool {
    raid_data::is_boss(name)
}

/// Return the known boss name list (lowercased) for substring scanning.
pub(crate) fn known_boss_names() -> &'static [&'static str] {
    raid_data::all_boss_names()
}

/// Return boss names for a specific raid zone (lowercased).
/// Returns `None` if zone is unknown, so caller falls back to the full list.
pub(crate) fn bosses_for_zone(zone: Option<&str>) -> Option<Vec<&'static str>> {
    raid_data::bosses_for_zone(zone)
}

pub(super) fn is_raid_zone(zone: &str) -> bool {
    raid_data::is_raid_zone(zone)
}

pub(super) fn get_boss_count(zone: &str) -> Option<usize> {
    raid_data::encounter_count(zone)
}

/// Normalize an addon-reported zone name to its canonical form.
pub(crate) fn normalize_zone_name(zone: &str) -> String {
    raid_data::normalize_zone(zone)
}

pub(super) fn instance_from_boss_kills(boss_kills: &[String]) -> Option<&'static str> {
    raid_data::instance_from_bosses(boss_kills)
}

pub(crate) fn format_zone_name(zone: &str) -> String {
    raid_data::format_zone_name(zone)
}

/// Reverse-decode a synthetic session timestamp into `(year, month, day)`.
///
/// The synthetic encoding is `(month*31 + day) * 86400 + time_of_day` from
/// `parse_timestamp_fast`. Falls back to `Local::now()` if year is unknown.
#[allow(clippy::cast_possible_truncation)] // month/day values are small integers
#[allow(clippy::cast_sign_loss)] // month/day are always positive
fn decode_session_date(ts: f64, year: Option<i32>) -> (i32, u32, u32) {
    let total_days = (ts / 86400.0).floor() as u32;
    let month = total_days / 31;
    let day = total_days % 31;

    let y = year.unwrap_or_else(|| {
        chrono::Local::now()
            .format("%Y")
            .to_string()
            .parse()
            .unwrap_or(2026)
    });

    (y, month, day)
}

/// Convert a synthetic session timestamp to a `YYYY-MM-DD` date string.
pub fn date_from_session_timestamp(ts: f64, year: Option<i32>) -> String {
    let (y, month, day) = decode_session_date(ts, year);
    format!("{y:04}-{month:02}-{day:02}")
}

/// Format a session timestamp as `DD/MM/YYYY` for UI display.
pub fn date_display_from_timestamp(ts: f64, year: Option<i32>) -> String {
    let (y, month, day) = decode_session_date(ts, year);
    format!("{day:02}/{month:02}/{y:04}")
}
