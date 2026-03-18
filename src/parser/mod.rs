//! Session detection and player name extraction for `WoW` combat logs.
//!
//! Scans log lines to identify raid sessions, boss kills, zone changes,
//! and which player `You`/`Your` refers to at each timestamp.
//!
//! All raid/boss/NPC data comes from `raid_data` (compiled from `data/raids.toml`
//! at build time). No hardcoded boss or zone lists in this file.

mod boss;
mod extraction;
mod session;
mod timestamp;

pub use boss::*;
pub use extraction::*;
pub use session::*;
pub(crate) use timestamp::parse_timestamp_fast;

// ── Session Struct ──────────────────────────────────────────────────────────

/// Represents a detected session (segment) in the combat log.
#[derive(Debug, Clone)]
pub struct Session {
    pub name: String,
    pub start_line: usize,
    pub end_line: usize,
    /// Timestamp (seconds since epoch-of-log) of the session's first event.
    pub start_time: f64,
    /// Timestamp (seconds since epoch-of-log) of the session's last event.
    pub end_time: f64,
    pub combat_count: usize,
    pub duration_secs: f64,
    /// Whether this session takes place in a known raid/instance zone.
    pub is_raid: bool,
    /// Calendar year extracted from the `COMBATANT_INFO` date field (e.g. 2026).
    /// `None` if no `COMBATANT_INFO` line was found in the log.
    pub start_year: Option<i32>,
    /// Player names detected from `COMBATANT_INFO` with full talent data (2 `}` chars).
    /// These are the names that "You/Your" will be replaced with.
    pub you_players: Vec<String>,
}

impl std::fmt::Display for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let duration = timestamp::format_duration(self.duration_secs);
        let date = date_display_from_timestamp(self.start_time, self.start_year);
        if self.you_players.is_empty() {
            write!(
                f,
                "{date} - {} - {} encounters, {duration}",
                self.name, self.combat_count
            )
        } else {
            write!(
                f,
                "{date} - {} - {} encounters, {duration} [You: {}]",
                self.name,
                self.combat_count,
                self.you_players.join(", ")
            )
        }
    }
}
