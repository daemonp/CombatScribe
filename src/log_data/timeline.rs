//! Timeline data model: buckets, events, aura intervals, and visibility toggles.

use std::collections::HashMap;

use super::types::ConsumableCategory;

// ── Timeline Data ───────────────────────────────────────────────────────────

/// One second of aggregated raid activity for the timeline chart.
#[derive(Debug, Clone, Default)]
pub struct TimelineBucket {
    /// Offset in seconds from encounter start.
    pub offset: f64,
    /// Total raid damage done this second.
    pub damage: u64,
    /// Total raid damage taken this second.
    pub damage_taken: u64,
    /// Total raid healing done this second (player-to-player only).
    pub healing: u64,
    /// Total healing done to bosses/enemies this second (e.g. Shadow of Ebonroc, Blood Siphon).
    pub boss_healing: u64,
    /// Number of raid members alive at end of this second.
    pub alive_count: u32,
}

/// A discrete event placed on the timeline (death, dispel, big hit, etc.).
#[derive(Debug, Clone)]
pub struct TimelineEvent {
    /// Offset in seconds from encounter start.
    pub offset: f64,
    pub kind: TimelineEventKind,
    #[allow(dead_code)] // Stored for tooltip display in future hover-over-marker feature
    pub label: String,
}

/// Kind of discrete timeline event, used for color-coding and icon selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelineEventKind {
    Death,
    BigHit,
    Dispel,
    Resurrect,
    Interrupt,
}

/// A single aura interval (gain->fade) for rendering on the `AuraChart`.
///
/// Times are encounter-relative offsets (seconds from encounter start).
#[derive(Debug, Clone)]
pub struct AuraInterval {
    /// Player who received the aura.
    pub player: String,
    /// Start offset (seconds into the encounter).
    pub start: f64,
    /// End offset (seconds). If the aura never faded during the encounter,
    /// this is clamped to the encounter duration.
    pub end: f64,
}

/// A single dispel event positioned on the encounter timeline.
///
/// Used by `DispelChart` to render per-caster waterfall lanes with tick marks.
#[derive(Debug, Clone)]
pub struct DispelMark {
    /// Player who cast the dispel.
    pub caster: String,
    /// Player who was dispelled.
    #[allow(dead_code)] // Stored for future hover tooltip display
    pub target: String,
    /// Dispel spell used (e.g. "Remove Curse", "Cleanse").
    #[allow(dead_code)] // Stored for future hover tooltip display
    pub spell: String,
    /// Encounter-relative offset in seconds.
    pub offset: f64,
}

/// A single consumable use event positioned on the encounter timeline.
///
/// Used by `ConsumeChart` to render per-player, per-category tick marks or
/// to cross-reference with aura intervals for hybrid bar/tick rendering.
#[derive(Debug, Clone)]
pub struct ConsumeMark {
    /// Player who used the consumable.
    pub player: String,
    /// Consumable item name (e.g. "Elixir of the Mongoose").
    pub consumable: String,
    /// Category from `consumables.toml`.
    pub category: ConsumableCategory,
    /// Encounter-relative offset in seconds.
    pub offset: f64,
}

/// Display mode for the consumable timeline waterfall chart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConsumeViewMode {
    /// Show buff uptime bars (intervals) where available; categories with only
    /// instant-use items auto-render as ticks.
    #[default]
    Bars,
    /// Show point-in-time diamond markers for every consumable use event.
    Ticks,
}

/// Precomputed timeline data for the currently selected encounter(s).
#[derive(Debug, Clone, Default)]
pub struct TimelineData {
    pub buckets: Vec<TimelineBucket>,
    pub events: Vec<TimelineEvent>,
    /// Peak DPS across all buckets (for Y-axis scaling).
    pub max_dps: u64,
    /// Peak DTPS across all buckets.
    pub max_dtps: u64,
    /// Peak HPS across all buckets.
    pub max_hps: u64,
    /// Peak boss/enemy HPS across all buckets.
    pub max_boss_hps: u64,
    /// Total encounter duration in seconds.
    pub duration: f64,
    /// Total raid member count at start.
    pub raid_count: u32,
    /// Aura intervals for tracked auras, keyed by aura name.
    ///
    /// Built lazily when the user selects auras to display.
    pub aura_intervals: HashMap<String, Vec<AuraInterval>>,
    /// Unique aura names seen in this encounter, sorted alphabetically.
    pub available_auras: Vec<String>,
    /// Dispel marks for the waterfall chart, ordered by offset.
    pub dispel_marks: Vec<DispelMark>,
    /// Unique dispel casters sorted by count descending (most active first).
    pub dispel_casters: Vec<String>,
    /// Consumable use events positioned on the session timeline.
    ///
    /// Unlike other timeline data (bucketed DPS/HPS, dispel marks), consume
    /// marks span the **full session** time range — not just encounter windows.
    /// This ensures pre-pull potions, food buffs, and between-fight consumables
    /// are captured.
    pub consume_marks: Vec<ConsumeMark>,
    /// Total time span for consumable marks (session end − session start).
    /// Used as the X-axis duration for the consumable chart.
    pub consume_duration: f64,
    /// Mapping from aura name → consumable category for auras that correspond
    /// to a known consumable item (exact name match or `buff_name` override from
    /// `consumables.toml`).
    pub consume_aura_categories: HashMap<String, ConsumableCategory>,
    /// Categories that have at least one consumable use in this session.
    pub available_consume_categories: Vec<ConsumableCategory>,
    /// Encounter boundaries for the consumable chart, expressed as offsets
    /// relative to the consume timeline's start (session start or pre-pull
    /// window start). Each entry is `(start_offset, end_offset, name, is_kill)`.
    pub consume_encounter_bounds: Vec<(f64, f64, String, bool)>,
    /// Translation segments for mapping encounter-relative aura interval offsets
    /// to consume-timeline-relative offsets.
    ///
    /// Each entry is `(aura_offset_start, aura_offset_end, consume_offset_start)`.
    /// For a single encounter, this is `[(0.0, duration, enc.start - t_start)]`.
    /// For multi-encounter concatenation, one segment per encounter.
    ///
    /// To translate: `consume_x = aura_x - seg.0 + seg.2`
    pub consume_aura_offset_segments: Vec<(f64, f64, f64)>,
}

/// Which timeline data series a toggle controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelineSeriesKind {
    Dps,
    Dtps,
    Hps,
    BossHeal,
    Death,
    BigHit,
    Alive,
    Dispel,
}

/// Visibility toggles for each timeline data series.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)] // One bool per toggleable series — clearest representation
pub struct TimelineVisibility {
    pub show_dps: bool,
    pub show_dtps: bool,
    pub show_hps: bool,
    pub show_boss_heals: bool,
    pub show_deaths: bool,
    pub show_big_hits: bool,
    pub show_alive: bool,
    pub show_dispels: bool,
}

impl Default for TimelineVisibility {
    fn default() -> Self {
        Self {
            show_dps: true,
            show_dtps: true,
            show_hps: true,
            show_boss_heals: true,
            show_deaths: true,
            show_big_hits: true,
            show_alive: true,
            show_dispels: false,
        }
    }
}

impl TimelineVisibility {
    /// Toggle the given series on or off.
    pub fn toggle(&mut self, kind: TimelineSeriesKind) {
        match kind {
            TimelineSeriesKind::Dps => self.show_dps = !self.show_dps,
            TimelineSeriesKind::Dtps => self.show_dtps = !self.show_dtps,
            TimelineSeriesKind::Hps => self.show_hps = !self.show_hps,
            TimelineSeriesKind::BossHeal => self.show_boss_heals = !self.show_boss_heals,
            TimelineSeriesKind::Death => self.show_deaths = !self.show_deaths,
            TimelineSeriesKind::BigHit => self.show_big_hits = !self.show_big_hits,
            TimelineSeriesKind::Alive => self.show_alive = !self.show_alive,
            TimelineSeriesKind::Dispel => self.show_dispels = !self.show_dispels,
        }
    }

    /// Check if a given event kind should be visible.
    #[allow(dead_code)] // Public API — useful for future filtering of chart event markers
    pub fn is_event_visible(&self, kind: TimelineEventKind) -> bool {
        match kind {
            TimelineEventKind::BigHit => self.show_big_hits,
            TimelineEventKind::Dispel => self.show_dispels,
            TimelineEventKind::Interrupt => true,
            // Deaths and resurrects are grouped under the same toggle
            TimelineEventKind::Death | TimelineEventKind::Resurrect => self.show_deaths,
        }
    }
}

// ── Aura Presets ────────────────────────────────────────────────────────────

/// A named preset of aura/buff names for quick selection.
pub struct AuraPreset {
    /// Display name shown in the UI.
    pub label: &'static str,
    /// Buff names as they appear in the combat log.
    pub auras: &'static [&'static str],
}

/// All available aura presets.
///
/// Buff names match the exact strings emitted by the vanilla 1.12 combat log
/// (and the Turtle addon format).  Verified against real log data.
///
/// Consumable-specific presets (Tank/Melee/Caster/Healer and Protection Potions)
/// have been moved to the dedicated Consumes tab timeline view.
pub const AURA_PRESETS: &[AuraPreset] = &[AuraPreset {
    label: "World Buffs",
    auras: &[
        "Rallying Cry of the Dragonslayer",
        "Spirit of Zandalar",
        "Songflower Serenade",
        "Warchief's Blessing",
        "Mol'dar's Moxie",
        "Fengus' Ferocity",
        "Slip'kik's Savvy",
    ],
}];
