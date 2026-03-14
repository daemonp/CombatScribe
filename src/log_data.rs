//! Data structures for parsed combat log data.
//!
//! Full port of the `logData` global object from `app.js`, plus
//! encounter filtering helpers.

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
        is_crit: bool,
    },
    Healing {
        timestamp: f64,
        source: String,
        target: String,
        spell: String,
        amount: u64,
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
    pub quality: String,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayerEventType {
    Damage,
    Healing,
}

// ── Encounter Filter ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
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
    /// Returns `(stats_map, total_duration)`.
    pub fn filtered_stats(&self, filter: &EncounterFilter) -> (HashMap<String, PlayerStats>, f64) {
        let duration = self.selected_duration(filter);

        if matches!(filter, EncounterFilter::All) {
            return (self.player_stats.clone(), duration);
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
                    is_crit,
                    ..
                } => {
                    let ps = stats.entry(source.clone()).or_default();
                    ps.healing += amount;
                    let ab = ps.healing_abilities.entry(spell.clone()).or_default();
                    ab.total += amount;
                    ab.hits += 1;
                    if *is_crit {
                        ab.crits += 1;
                    }
                }
                _ => {}
            }
        }

        (stats, duration)
    }

    /// Filter deaths to selection.
    pub fn filtered_deaths(&self, filter: &EncounterFilter) -> Vec<&DeathEvent> {
        self.deaths
            .iter()
            .filter(|d| self.is_in_selection(d.timestamp, filter))
            .collect()
    }

    /// Filter dispels to selection.
    pub fn filtered_dispels(&self, filter: &EncounterFilter) -> Vec<&DispelEvent> {
        self.dispels
            .iter()
            .filter(|d| self.is_in_selection(d.timestamp, filter))
            .collect()
    }

    /// Filter resurrects to selection.
    pub fn filtered_resurrects(&self, filter: &EncounterFilter) -> Vec<&ResurrectEvent> {
        self.resurrects
            .iter()
            .filter(|r| self.is_in_selection(r.timestamp, filter))
            .collect()
    }

    /// Filter interrupts to selection.
    pub fn filtered_interrupts(&self, filter: &EncounterFilter) -> Vec<&InterruptEvent> {
        self.interrupts
            .iter()
            .filter(|i| self.is_in_selection(i.timestamp, filter))
            .collect()
    }

    /// Get the opener sequence for a player (first N abilities in first 10s).
    pub fn opener_sequence(
        &self,
        player: &str,
        event_type: &PlayerEventType,
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
                } if *event_type == PlayerEventType::Damage && source == player => {
                    matching.push((spell, *timestamp, *amount, *is_crit));
                }
                LogEntry::Healing {
                    source,
                    spell,
                    amount,
                    is_crit,
                    timestamp,
                    ..
                } if *event_type == PlayerEventType::Healing && source == player => {
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
        self.consumables
            .iter()
            .filter(|c| self.is_in_selection(c.timestamp, filter))
            .collect()
    }

    /// Get the class of a player, or "UNKNOWN".
    pub fn player_class(&self, name: &str) -> &str {
        self.combatants
            .get(name)
            .map_or("UNKNOWN", |c| c.class.as_str())
    }
}
