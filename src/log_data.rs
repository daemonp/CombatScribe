//! Data structures for parsed combat log data.
//!
//! Full port of the `logData` global object from `app.js`, plus
//! encounter filtering helpers.

use std::borrow::Cow;
use std::collections::HashMap;

// ── Core Data ───────────────────────────────────────────────────────────────

/// All parsed data for a single session.
#[derive(Debug, Clone, Default)]
pub struct LogData {
    pub entries: Vec<LogEntry>,
    /// Filtered combatants — raid participants only (windowed + combat participants).
    pub combatants: HashMap<String, Combatant>,
    /// All `COMBATANT_INFO` entries, including city bystanders.
    pub all_combatants: HashMap<String, Combatant>,
    pub encounters: Vec<Encounter>,
    pub start_time: Option<f64>,
    pub end_time: Option<f64>,
    pub zone_name: String,
    pub player_stats: HashMap<String, PlayerStats>,
    pub deaths: Vec<DeathEvent>,
    pub resurrects: Vec<ResurrectEvent>,
    pub dispels: Vec<DispelEvent>,
    pub interrupts: Vec<InterruptEvent>,
    pub absorbs: HashMap<String, u64>,
    pub pet_owners: HashMap<String, String>,
    pub avoidance: HashMap<String, AvoidanceStats>,
    pub buffs: HashMap<String, HashMap<String, BuffStats>>,
    pub loot: Vec<LootEvent>,
    pub trades: Vec<TradeEvent>,
    pub consumables: Vec<ConsumableUse>,
    /// Last seen `PLAYERS_IN_COMBAT` snapshot: `(in_combat, total)`.
    pub raid_size: Option<(u32, u32)>,
}

// LogData derives Default — all fields are Vec/HashMap/Option/String which default correctly.

// ── Log Entries ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum LogEntry {
    Damage {
        timestamp: f64,
        source: String,
        target: String,
        spell: String,
        amount: u64,
        absorbed: u64,
        resisted: u64,
        blocked: u64,
        is_crit: bool,
        is_glancing: bool,
        is_crushing: bool,
        school: Option<String>,
    },
    Healing {
        timestamp: f64,
        source: String,
        target: String,
        spell: String,
        amount: u64,
        effective_heal: u64,
        overheal: u64,
        is_crit: bool,
    },
    Death {
        timestamp: f64,
        player: String,
    },
    Dispel {
        timestamp: f64,
        caster: String,
        target: String,
        spell: String,
    },
    Resurrect {
        timestamp: f64,
        caster: String,
        target: String,
        #[allow(dead_code)]
        spell: String,
    },
    Interrupt {
        timestamp: f64,
        caster: String,
        target: String,
        spell: String,
    },
}

impl LogEntry {
    /// Return the timestamp of any entry variant.
    pub fn timestamp(&self) -> f64 {
        match self {
            Self::Damage { timestamp, .. }
            | Self::Healing { timestamp, .. }
            | Self::Death { timestamp, .. }
            | Self::Dispel { timestamp, .. }
            | Self::Resurrect { timestamp, .. }
            | Self::Interrupt { timestamp, .. } => *timestamp,
        }
    }

    /// Return the source/actor name.
    pub fn source(&self) -> &str {
        match self {
            Self::Damage { source, .. } | Self::Healing { source, .. } => source,
            Self::Death { player, .. } => player,
            Self::Dispel { caster, .. }
            | Self::Resurrect { caster, .. }
            | Self::Interrupt { caster, .. } => caster,
        }
    }

    /// Return the target name (if applicable).
    pub fn target(&self) -> Option<&str> {
        match self {
            Self::Death { .. } => None,
            Self::Damage { target, .. }
            | Self::Healing { target, .. }
            | Self::Dispel { target, .. }
            | Self::Resurrect { target, .. }
            | Self::Interrupt { target, .. } => Some(target),
        }
    }
}

// ── Supporting Structures ───────────────────────────────────────────────────

/// Equipment slot parsed from `COMBATANT_INFO` gear fields.
///
/// Format in the log: `itemId:enchantId:suffixId:uniqueId`
#[derive(Debug, Clone)]
#[allow(dead_code)] // Data model — fields stored for future gear inspection display
pub struct GearSlot {
    pub item_id: u32,
    pub enchant_id: u32,
    pub suffix_id: i32,
    pub raw: String,
}

/// Consumable/item use event parsed from V1 `"uses"` lines.
#[derive(Debug, Clone)]
pub struct ConsumableUse {
    pub timestamp: f64,
    pub player: String,
    pub consumable: String,
}

#[derive(Debug, Clone, Default)]
pub struct Combatant {
    pub class: String,
    pub race: String,
    pub guild: Option<String>,
    pub gear: Vec<Option<GearSlot>>,
    /// Talent summary like `"31/20/0"`.
    pub talent_summary: Option<String>,
    #[allow(dead_code)]
    pub guid: Option<String>,
    #[allow(dead_code)]
    pub pet_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Encounter {
    pub name: Option<String>,
    pub start: f64,
    pub end: f64,
    pub duration: f64,
    pub is_boss: bool,
    pub is_kill: bool,
    pub zone: Option<String>,
    pub attempt: Option<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct PlayerStats {
    pub damage: u64,
    pub healing: u64,
    pub effective_healing: u64,
    pub overhealing: u64,
    pub damage_taken: u64,
    pub pet_damage: u64,
    pub abilities: HashMap<String, AbilityStats>,
    pub healing_abilities: HashMap<String, AbilityStats>,
}

#[derive(Debug, Clone, Default)]
pub struct AbilityStats {
    pub total: u64,
    pub hits: u64,
    pub crits: u64,
    pub effective: u64,
    pub overheal: u64,
    #[allow(dead_code)]
    pub is_pet: bool,
}

#[derive(Debug, Clone, Default)]
pub struct AvoidanceStats {
    pub dodges: u64,
    pub parries: u64,
    pub blocks: u64,
    pub missed_by: u64,
    pub misses: u64,
}

impl AvoidanceStats {
    /// Total attacks avoided (dodges + parries + blocks + missed by attacker).
    ///
    /// Note: `misses` (player's own missed attacks) is excluded since it
    /// represents offensive misses, not defensive avoidance.
    pub fn total(&self) -> u64 {
        self.dodges + self.parries + self.blocks + self.missed_by
    }
}

#[derive(Debug, Clone, Default)]
pub struct BuffStats {
    pub gains: u64,
    pub fades: u64,
    pub first_gain: Option<f64>,
    pub last_fade: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct LootEvent {
    pub timestamp: f64,
    pub player: String,
    pub item_name: String,
    #[allow(dead_code)]
    pub item_id: u64,
    pub quality: ItemQuality,
    pub quantity: u64,
    pub boss: String,
    pub traded_to: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TradeEvent {
    pub timestamp: f64,
    pub from_player: String,
    pub item_name: String,
    pub to_player: String,
}

#[derive(Debug, Clone)]
pub struct DeathEvent {
    pub timestamp: f64,
    pub player: String,
}

#[derive(Debug, Clone)]
pub struct ResurrectEvent {
    pub timestamp: f64,
    pub caster: String,
    pub target: String,
    pub spell: String,
}

#[derive(Debug, Clone)]
pub struct DispelEvent {
    pub timestamp: f64,
    pub caster: String,
    pub target: String,
    pub spell: String,
}

#[derive(Debug, Clone)]
pub struct InterruptEvent {
    pub timestamp: f64,
    pub caster: String,
    pub target: String,
    pub spell: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerEventType {
    Damage,
    Healing,
}

/// Item quality tier from `WoW` color codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ItemQuality {
    Poor,
    Common,
    Uncommon,
    Rare,
    Epic,
    Legendary,
}

impl ItemQuality {
    /// Parse from the 6-char hex color code in `LOOT:` lines.
    pub fn from_color_code(code: &str) -> Self {
        match code {
            "9d9d9d" => Self::Poor,
            "1eff00" => Self::Uncommon,
            "0070dd" => Self::Rare,
            "a335ee" => Self::Epic,
            "ff8000" => Self::Legendary,
            _ => Self::Common,
        }
    }

    /// Whether this quality is notable (green or above).
    pub fn is_notable(self) -> bool {
        self >= Self::Uncommon
    }
}

// ── Encounter Filter ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncounterFilter {
    All,
    AllKills,
    AllWipes,
    AllTrash,
    Single(usize),
}

// ── Opener ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct OpenerSpell {
    pub spell: String,
    pub delay: f64,
    pub amount: u64,
    pub is_crit: bool,
}

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
    /// Total raid healing done this second.
    pub healing: u64,
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
    /// Total encounter duration in seconds.
    pub duration: f64,
    /// Total raid member count at start.
    pub raid_count: u32,
}

/// Which timeline data series a toggle controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelineSeriesKind {
    Dps,
    Dtps,
    Hps,
    Death,
    BigHit,
    Alive,
}

/// Visibility toggles for each timeline data series.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)] // One bool per toggleable series — clearest representation
pub struct TimelineVisibility {
    pub show_dps: bool,
    pub show_dtps: bool,
    pub show_hps: bool,
    pub show_deaths: bool,
    pub show_big_hits: bool,
    pub show_alive: bool,
}

impl Default for TimelineVisibility {
    fn default() -> Self {
        Self {
            show_dps: true,
            show_dtps: true,
            show_hps: true,
            show_deaths: true,
            show_big_hits: true,
            show_alive: true,
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
            TimelineSeriesKind::Death => self.show_deaths = !self.show_deaths,
            TimelineSeriesKind::BigHit => self.show_big_hits = !self.show_big_hits,
            TimelineSeriesKind::Alive => self.show_alive = !self.show_alive,
        }
    }

    /// Check if a given event kind should be visible.
    #[allow(dead_code)] // Public API — useful for future filtering of chart event markers
    pub fn is_event_visible(&self, kind: TimelineEventKind) -> bool {
        match kind {
            TimelineEventKind::BigHit => self.show_big_hits,
            TimelineEventKind::Dispel | TimelineEventKind::Interrupt => true,
            // Deaths and resurrects are grouped under the same toggle
            TimelineEventKind::Death | TimelineEventKind::Resurrect => self.show_deaths,
        }
    }
}

// ── Event Log Facets ────────────────────────────────────────────────────────

/// Which preset view mode the event log is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EventLogMode {
    /// Show all events matching the type toggles.
    #[default]
    AllEvents,
    /// Show only key events: deaths, big hits, dispels, interrupts, resurrects.
    KeyEvents,
    /// Show events involving each dead player in the seconds before their death.
    DeathLog,
}

impl std::fmt::Display for EventLogMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AllEvents => write!(f, "All Events"),
            Self::KeyEvents => write!(f, "Key Events"),
            Self::DeathLog => write!(f, "Death Log"),
        }
    }
}

/// Which event types are visible in the event log.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)] // One bool per event type — clearest representation
pub struct EventLogTypeFilter {
    pub show_damage: bool,
    pub show_healing: bool,
    pub show_deaths: bool,
    pub show_dispels: bool,
    pub show_interrupts: bool,
}

impl Default for EventLogTypeFilter {
    fn default() -> Self {
        Self {
            show_damage: true,
            show_healing: true,
            show_deaths: true,
            show_dispels: true,
            show_interrupts: true,
        }
    }
}

impl EventLogTypeFilter {
    /// Check if a `LogEntry` passes the type filter.
    pub fn accepts(&self, entry: &LogEntry) -> bool {
        match entry {
            LogEntry::Damage { .. } => self.show_damage,
            LogEntry::Healing { .. } => self.show_healing,
            LogEntry::Death { .. } | LogEntry::Resurrect { .. } => self.show_deaths,
            LogEntry::Dispel { .. } => self.show_dispels,
            LogEntry::Interrupt { .. } => self.show_interrupts,
        }
    }
}

/// Which event type toggle to flip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventLogTypeKind {
    Damage,
    Healing,
    Deaths,
    Dispels,
    Interrupts,
}

// ── Filtering Methods ───────────────────────────────────────────────────────

impl LogData {
    /// Get encounters matching the current filter.
    pub fn selected_encounters(&self, filter: &EncounterFilter) -> Vec<&Encounter> {
        match filter {
            EncounterFilter::All => self.encounters.iter().collect(),
            EncounterFilter::AllKills => self
                .encounters
                .iter()
                .filter(|e| e.is_boss && e.is_kill)
                .collect(),
            EncounterFilter::AllWipes => self
                .encounters
                .iter()
                .filter(|e| e.is_boss && !e.is_kill)
                .collect(),
            EncounterFilter::AllTrash => self.encounters.iter().filter(|e| !e.is_boss).collect(),
            EncounterFilter::Single(idx) => self.encounters.get(*idx).into_iter().collect(),
        }
    }

    /// Check if a timestamp falls within any selected encounter.
    pub fn is_in_selection(&self, timestamp: f64, filter: &EncounterFilter) -> bool {
        if matches!(filter, EncounterFilter::All) {
            return self.start_time.is_some_and(|s| timestamp >= s)
                && self.end_time.is_some_and(|e| timestamp <= e);
        }
        self.selected_encounters(filter)
            .iter()
            .any(|enc| timestamp >= enc.start && timestamp <= enc.end)
    }

    /// Get total combat duration for selected encounters.
    pub fn selected_duration(&self, filter: &EncounterFilter) -> f64 {
        self.selected_encounters(filter)
            .iter()
            .map(|e| e.duration)
            .sum()
    }

    /// Recalculate player stats filtered to selected encounters.
    ///
    /// Returns `(stats_map, total_duration)`. For `EncounterFilter::All`, borrows
    /// the existing stats to avoid a full clone.
    pub fn filtered_stats(
        &self,
        filter: &EncounterFilter,
    ) -> (Cow<'_, HashMap<String, PlayerStats>>, f64) {
        let duration = self.selected_duration(filter);

        if matches!(filter, EncounterFilter::All) {
            return (Cow::Borrowed(&self.player_stats), duration);
        }

        let mut stats: HashMap<String, PlayerStats> = HashMap::new();

        for entry in &self.entries {
            if !self.is_in_selection(entry.timestamp(), filter) {
                continue;
            }

            match entry {
                LogEntry::Damage {
                    source,
                    target,
                    spell,
                    amount,
                    is_crit,
                    ..
                } => {
                    let ps = stats.entry(source.clone()).or_default();
                    ps.damage += amount;
                    let ab = ps.abilities.entry(spell.clone()).or_default();
                    ab.total += amount;
                    ab.hits += 1;
                    if *is_crit {
                        ab.crits += 1;
                    }

                    // Damage taken
                    let ts = stats.entry(target.clone()).or_default();
                    ts.damage_taken += amount;
                }
                LogEntry::Healing {
                    source,
                    spell,
                    amount,
                    effective_heal,
                    overheal,
                    is_crit,
                    ..
                } => {
                    let ps = stats.entry(source.clone()).or_default();
                    ps.healing += amount;
                    ps.effective_healing += effective_heal;
                    ps.overhealing += overheal;
                    let ab = ps.healing_abilities.entry(spell.clone()).or_default();
                    ab.total += amount;
                    ab.effective += effective_heal;
                    ab.overheal += overheal;
                    ab.hits += 1;
                    if *is_crit {
                        ab.crits += 1;
                    }
                }
                _ => {}
            }
        }

        (Cow::Owned(stats), duration)
    }

    /// Filter any timestamped event collection to the current encounter selection.
    fn filtered_by_time<'a, T>(
        &self,
        events: &'a [T],
        get_ts: impl Fn(&T) -> f64,
        filter: &EncounterFilter,
    ) -> Vec<&'a T> {
        events
            .iter()
            .filter(|e| self.is_in_selection(get_ts(e), filter))
            .collect()
    }

    /// Filter deaths to selection.
    pub fn filtered_deaths(&self, filter: &EncounterFilter) -> Vec<&DeathEvent> {
        self.filtered_by_time(&self.deaths, |d| d.timestamp, filter)
    }

    /// Filter dispels to selection.
    pub fn filtered_dispels(&self, filter: &EncounterFilter) -> Vec<&DispelEvent> {
        self.filtered_by_time(&self.dispels, |d| d.timestamp, filter)
    }

    /// Filter resurrects to selection.
    pub fn filtered_resurrects(&self, filter: &EncounterFilter) -> Vec<&ResurrectEvent> {
        self.filtered_by_time(&self.resurrects, |r| r.timestamp, filter)
    }

    /// Filter interrupts to selection.
    pub fn filtered_interrupts(&self, filter: &EncounterFilter) -> Vec<&InterruptEvent> {
        self.filtered_by_time(&self.interrupts, |i| i.timestamp, filter)
    }

    /// Get the opener sequence for a player (first N abilities in first 10s).
    pub fn opener_sequence(
        &self,
        player: &str,
        event_type: PlayerEventType,
        filter: &EncounterFilter,
    ) -> Vec<OpenerSpell> {
        let limit = 8;
        let window_secs = 10.0;

        // Collect matching entries
        let mut matching: Vec<(&str, f64, u64, bool)> = Vec::new();
        for entry in &self.entries {
            if !self.is_in_selection(entry.timestamp(), filter) {
                continue;
            }
            match entry {
                LogEntry::Damage {
                    source,
                    spell,
                    amount,
                    is_crit,
                    timestamp,
                    ..
                } if event_type == PlayerEventType::Damage && source == player => {
                    matching.push((spell, *timestamp, *amount, *is_crit));
                }
                LogEntry::Healing {
                    source,
                    spell,
                    amount,
                    is_crit,
                    timestamp,
                    ..
                } if event_type == PlayerEventType::Healing && source == player => {
                    matching.push((spell, *timestamp, *amount, *is_crit));
                }
                _ => {}
            }
        }

        if matching.is_empty() {
            return Vec::new();
        }

        matching.sort_by(|a, b| a.1.total_cmp(&b.1));
        let first_time = matching[0].1;

        matching
            .iter()
            .filter(|(_, ts, _, _)| *ts <= first_time + window_secs)
            .take(limit)
            .map(|(spell, ts, amount, is_crit)| OpenerSpell {
                spell: spell.to_string(),
                delay: if (*ts - first_time).abs() < f64::EPSILON {
                    0.0
                } else {
                    *ts - first_time
                },
                amount: *amount,
                is_crit: *is_crit,
            })
            .collect()
    }

    /// Filter consumable uses to selection.
    pub fn filtered_consumables(&self, filter: &EncounterFilter) -> Vec<&ConsumableUse> {
        self.filtered_by_time(&self.consumables, |c| c.timestamp, filter)
    }

    /// Get the class of a player, or "UNKNOWN".
    pub fn player_class(&self, name: &str) -> &str {
        self.combatants
            .get(name)
            .map_or("UNKNOWN", |c| c.class.as_str())
    }

    /// Build timeline data for the selected encounter filter.
    ///
    /// Buckets all events into 1-second intervals relative to the encounter start,
    /// and collects discrete events (deaths, big hits, dispels) for overlay markers.
    /// The `big_hit_threshold` marks any single damage-taken event above this value.
    #[allow(clippy::too_many_lines)] // Timeline builder — single cohesive pass over events
    #[allow(clippy::cast_possible_truncation)] // Timestamps/durations never approach usize limits
    #[allow(clippy::cast_sign_loss)] // Duration and offsets are always non-negative
    #[allow(clippy::cast_precision_loss)] // Bucket indices never approach 2^52
    #[allow(clippy::similar_names)] // dps/dtps/hps are standard WoW combat log metrics
    pub fn build_timeline(&self, filter: &EncounterFilter, big_hit_threshold: u64) -> TimelineData {
        let encounters = self.selected_encounters(filter);
        if encounters.is_empty() {
            return TimelineData::default();
        }

        // For Single encounters, use the encounter's own start/end.
        // For multi-encounter filters, concatenate them sequentially.
        let total_duration: f64 = encounters.iter().map(|e| e.duration).sum();
        if total_duration <= 0.0 {
            return TimelineData::default();
        }

        let bucket_count = total_duration.ceil() as usize + 1;
        let mut buckets: Vec<TimelineBucket> = (0..bucket_count)
            .map(|i| TimelineBucket {
                offset: i as f64,
                ..TimelineBucket::default()
            })
            .collect();
        let mut events: Vec<TimelineEvent> = Vec::new();

        // Track alive count: start with all combatants, decrement on death,
        // increment on resurrect.
        let raid_count = self.combatants.len() as u32;
        let mut alive: u32;

        // Offset accumulator for multi-encounter concatenation
        let mut offset_base: f64 = 0.0;

        for enc in &encounters {
            let enc_start = enc.start;
            let enc_duration = enc.duration;

            // Reset alive count for each encounter segment
            alive = raid_count;
            // Anchor the first bucket of this segment to the fresh alive count
            let first_bucket = offset_base.floor() as usize;
            if first_bucket < buckets.len() {
                buckets[first_bucket].alive_count = alive;
            }

            for entry in &self.entries {
                let ts = entry.timestamp();
                if ts < enc_start || ts > enc.end {
                    continue;
                }

                let relative = ts - enc_start + offset_base;
                let bucket_idx = relative.floor() as usize;
                if bucket_idx >= buckets.len() {
                    continue;
                }

                match entry {
                    LogEntry::Damage {
                        target,
                        amount,
                        spell,
                        source,
                        ..
                    } => {
                        // Damage done (from raid members)
                        if self.combatants.contains_key(source.as_str()) {
                            buckets[bucket_idx].damage += amount;
                        }
                        // Damage taken (by raid members)
                        if self.combatants.contains_key(target.as_str()) {
                            buckets[bucket_idx].damage_taken += amount;
                            // Big hit marker
                            if *amount >= big_hit_threshold {
                                events.push(TimelineEvent {
                                    offset: relative,
                                    kind: TimelineEventKind::BigHit,
                                    label: format!(
                                        "{target} takes {amount} from {source}'s {spell}"
                                    ),
                                });
                            }
                        }
                    }
                    LogEntry::Healing {
                        source,
                        effective_heal,
                        ..
                    } => {
                        if self.combatants.contains_key(source.as_str()) {
                            buckets[bucket_idx].healing += effective_heal;
                        }
                    }
                    LogEntry::Death { player, .. } => {
                        if self.combatants.contains_key(player.as_str()) {
                            alive = alive.saturating_sub(1);
                            events.push(TimelineEvent {
                                offset: relative,
                                kind: TimelineEventKind::Death,
                                label: format!("{player} died"),
                            });
                        }
                    }
                    LogEntry::Dispel {
                        caster,
                        target,
                        spell,
                        ..
                    } => {
                        events.push(TimelineEvent {
                            offset: relative,
                            kind: TimelineEventKind::Dispel,
                            label: format!("{caster} dispels {spell} on {target}"),
                        });
                    }
                    LogEntry::Resurrect { caster, target, .. } => {
                        if self.combatants.contains_key(target.as_str()) {
                            alive = alive.saturating_add(1).min(raid_count);
                            events.push(TimelineEvent {
                                offset: relative,
                                kind: TimelineEventKind::Resurrect,
                                label: format!("{caster} resurrects {target}"),
                            });
                        }
                    }
                    LogEntry::Interrupt {
                        caster,
                        target,
                        spell,
                        ..
                    } => {
                        events.push(TimelineEvent {
                            offset: relative,
                            kind: TimelineEventKind::Interrupt,
                            label: format!("{caster} interrupts {target} with {spell}"),
                        });
                    }
                }

                buckets[bucket_idx].alive_count = alive;
            }

            offset_base += enc_duration;
        }

        // Forward-fill alive counts for empty buckets
        let mut last_alive = raid_count;
        for bucket in &mut buckets {
            if bucket.alive_count == 0
                && bucket.damage == 0
                && bucket.healing == 0
                && bucket.damage_taken == 0
            {
                bucket.alive_count = last_alive;
            } else {
                last_alive = bucket.alive_count;
            }
        }

        let max_dps = buckets.iter().map(|b| b.damage).max().unwrap_or(0);
        let max_dtps = buckets.iter().map(|b| b.damage_taken).max().unwrap_or(0);
        let max_hps = buckets.iter().map(|b| b.healing).max().unwrap_or(0);

        TimelineData {
            buckets,
            events,
            max_dps,
            max_dtps,
            max_hps,
            duration: total_duration,
            raid_count,
        }
    }
}
