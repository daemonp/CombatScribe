//! Core data types: `LogData`, `LogEntry`, `Encounter`, `PlayerStats`, and supporting structs.

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

#[allow(clippy::struct_excessive_bools)] // Damage variant has many boolean combat outcomes
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
        /// True when the damage came from the player's pet (formatter tagged it
        /// with `(pet)`).  Used by `filtered_stats()` to attribute `pet_damage`.
        is_pet_spell: bool,
        /// True if the attack was fully resisted (0 damage dealt, resist detected).
        #[allow(dead_code)] // Data model — aggregated into AvoidanceStats during parsing
        is_fully_resisted: bool,
        /// True if the attack was fully absorbed (0 damage dealt, absorb detected).
        #[allow(dead_code)] // Data model — aggregated into AvoidanceStats during parsing
        is_fully_absorbed: bool,
        /// True if the attack was fully blocked (0 damage dealt, block detected).
        #[allow(dead_code)] // Data model — aggregated into AvoidanceStats during parsing
        is_fully_blocked: bool,
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
        #[allow(dead_code)] // Data model — stored for future resurrection detail display
        spell: String,
    },
    Interrupt {
        timestamp: f64,
        caster: String,
        target: String,
        spell: String,
    },
    /// Buff or debuff applied to a player ("gains" / "is afflicted by").
    AuraGain {
        timestamp: f64,
        player: String,
        aura: String,
        stacks: u32,
    },
    /// Buff or debuff removed from a player ("fades from").
    AuraFade {
        timestamp: f64,
        player: String,
        aura: String,
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
            | Self::Interrupt { timestamp, .. }
            | Self::AuraGain { timestamp, .. }
            | Self::AuraFade { timestamp, .. } => *timestamp,
        }
    }

    /// Return the source/actor name.
    pub fn source(&self) -> &str {
        match self {
            Self::Damage { source, .. } | Self::Healing { source, .. } => source,
            Self::Death { player, .. }
            | Self::AuraGain { player, .. }
            | Self::AuraFade { player, .. } => player,
            Self::Dispel { caster, .. }
            | Self::Resurrect { caster, .. }
            | Self::Interrupt { caster, .. } => caster,
        }
    }

    /// Return the target name (if applicable).
    pub fn target(&self) -> Option<&str> {
        match self {
            Self::Death { .. } | Self::AuraGain { .. } | Self::AuraFade { .. } => None,
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

/// Classification of consumable items by type.
///
/// Variant order determines sort order in the UI (Flask first, Other last).
/// Backed by `data/consumables.toml` → `build.rs` → `consumable_data.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ConsumableCategory {
    Flask,
    Elixir,
    Potion,
    Food,
    WeaponBuff,
    Juju,
    BlastedLands,
    Zanza,
    Scroll,
    Engineering,
    Bandage,
    Utility,
    Other,
}

impl ConsumableCategory {
    /// Convert a build-time category index back to an enum variant.
    ///
    /// Indices match the order of `[[category]]` sections in `consumables.toml`
    /// which must match the enum variant order above.
    #[must_use]
    pub fn from_index(idx: u8) -> Self {
        match idx {
            0 => Self::Flask,
            1 => Self::Elixir,
            2 => Self::Potion,
            3 => Self::Food,
            4 => Self::WeaponBuff,
            5 => Self::Juju,
            6 => Self::BlastedLands,
            7 => Self::Zanza,
            8 => Self::Scroll,
            9 => Self::Engineering,
            10 => Self::Bandage,
            11 => Self::Utility,
            _ => Self::Other,
        }
    }

    /// All category variants in display order.
    #[allow(dead_code)] // Public API — available for future iteration needs
    pub const ALL: &'static [Self] = &[
        Self::Flask,
        Self::Elixir,
        Self::Potion,
        Self::Food,
        Self::WeaponBuff,
        Self::Juju,
        Self::BlastedLands,
        Self::Zanza,
        Self::Scroll,
        Self::Engineering,
        Self::Bandage,
        Self::Utility,
        Self::Other,
    ];
}

impl std::fmt::Display for ConsumableCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = crate::consumable_data::category_display_name(*self);
        write!(f, "{name}")
    }
}

/// Consumable/item use event parsed from V1 `"uses"` lines.
#[derive(Debug, Clone)]
pub struct ConsumableUse {
    pub timestamp: f64,
    pub player: String,
    pub consumable: String,
    pub category: ConsumableCategory,
}

#[derive(Debug, Clone, Default)]
pub struct Combatant {
    pub class: String,
    pub race: String,
    pub guild: Option<String>,
    pub gear: Vec<Option<GearSlot>>,
    /// Talent summary like `"31/20/0"`.
    pub talent_summary: Option<String>,
    #[allow(dead_code)] // Data model — stored for future gear inspection display
    pub guid: Option<String>,
    /// Pet name from `COMBATANT_INFO` — used by `is_pet_target()` in timeline
    /// to distinguish pet heals from MC'd-mob heals.
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
    /// Number of unique players who died during this encounter.
    pub player_deaths: u32,
    /// Number of unique players active (dealt/took damage) during this encounter.
    pub active_players: u32,
}

#[derive(Debug, Clone, Default)]
pub struct PlayerStats {
    /// Total damage dealt (personal + pet).  The formatter rewrites pet lines
    /// so the owner is the source; damage from both the player and their pets
    /// flows into this field.
    pub damage: u64,
    pub healing: u64,
    pub effective_healing: u64,
    pub overhealing: u64,
    pub damage_taken: u64,
    /// Subset of `damage` that came from the player's pet.  The UI uses this
    /// to offer a "Personal only" mode: `damage - pet_damage`.
    pub pet_damage: u64,
    /// Per-ability stats.  Pet abilities have `is_pet: true`.
    pub abilities: HashMap<String, AbilityStats>,
    pub healing_abilities: HashMap<String, AbilityStats>,
    /// Damage taken broken down by source -> ability -> stats.
    ///
    /// Outer key is the source name (e.g. "Patchwerk"), inner key is the ability
    /// name (e.g. "Hateful Strike").  This lets tanks compare exactly which
    /// abilities hit them and how much was mitigated.
    pub damage_taken_breakdown: HashMap<String, HashMap<String, DamageTakenAbilityStats>>,
}

#[derive(Debug, Clone, Default)]
pub struct AbilityStats {
    pub total: u64,
    pub hits: u64,
    pub crits: u64,
    /// Sum of damage/healing amounts from critical strikes only.
    /// Enables computing avg crit vs avg normal hit.
    pub crit_total: u64,
    pub effective: u64,
    pub overheal: u64,
    /// True for abilities that came from the player's pet, not the player
    /// directly.  Used by the detail view to visually group pet abilities.
    pub is_pet: bool,
}

// ── Stat Accumulation Helpers ───────────────────────────────────────────────

impl PlayerStats {
    /// Accumulate a damage event into this player's stats.
    pub fn accumulate_damage(&mut self, spell: &str, amount: u64, is_crit: bool) {
        self.damage += amount;
        let ab = self.abilities.entry(spell.to_string()).or_default();
        ab.total += amount;
        ab.hits += 1;
        if is_crit {
            ab.crits += 1;
            ab.crit_total += amount;
        }
    }

    /// Accumulate damage taken into this player's stats, including the
    /// per-source per-ability mitigation breakdown.
    #[allow(clippy::too_many_arguments)] // mirrors the LogEntry::Damage fields
    pub fn accumulate_damage_taken(
        &mut self,
        source: &str,
        spell: &str,
        amount: u64,
        absorbed: u64,
        resisted: u64,
        blocked: u64,
        is_crit: bool,
        is_crushing: bool,
        is_glancing: bool,
    ) {
        self.damage_taken += amount;
        let dt_ab = self
            .damage_taken_breakdown
            .entry(source.to_string())
            .or_default()
            .entry(spell.to_string())
            .or_default();
        dt_ab.total += amount;
        dt_ab.hits += 1;
        dt_ab.absorbed += absorbed;
        dt_ab.resisted += resisted;
        dt_ab.blocked += blocked;
        if is_crit {
            dt_ab.crits += 1;
            dt_ab.crit_total += amount;
        }
        if is_crushing {
            dt_ab.crushing_hits += 1;
        }
        if is_glancing {
            dt_ab.glancing_hits += 1;
        }
    }

    /// Accumulate a healing event into this player's stats.
    pub fn accumulate_healing(
        &mut self,
        spell: &str,
        amount: u64,
        effective: u64,
        overheal: u64,
        is_crit: bool,
    ) {
        self.healing += amount;
        self.effective_healing += effective;
        self.overhealing += overheal;
        let ab = self.healing_abilities.entry(spell.to_string()).or_default();
        ab.total += amount;
        ab.effective += effective;
        ab.overheal += overheal;
        ab.hits += 1;
        if is_crit {
            ab.crits += 1;
            ab.crit_total += amount;
        }
    }
}

/// Per-ability breakdown of damage taken, including mitigation details.
///
/// Keyed by `(source, ability)` pairs in `PlayerStats::damage_taken_breakdown`,
/// this lets tanks compare exactly which abilities hit them and how much was
/// mitigated by absorbs, resists, and blocks.
#[derive(Debug, Clone, Default)]
pub struct DamageTakenAbilityStats {
    pub total: u64,
    pub hits: u64,
    pub crits: u64,
    /// Sum of damage amounts from critical strikes only.
    pub crit_total: u64,
    pub absorbed: u64,
    pub resisted: u64,
    pub blocked: u64,
    pub crushing_hits: u64,
    pub glancing_hits: u64,
}

#[derive(Debug, Clone, Default)]
pub struct AvoidanceStats {
    pub dodges: u64,
    pub parries: u64,
    pub blocks: u64,
    pub missed_by: u64,
    pub misses: u64,
    /// Attacks that dealt 0 damage due to full resist.
    pub full_resists: u64,
    /// Attacks that dealt 0 damage due to full absorb (e.g. Power Word: Shield).
    pub full_absorbs: u64,
    /// Attacks that dealt 0 damage due to full block.
    pub full_blocks: u64,
}

impl AvoidanceStats {
    /// Total attacks avoided (dodges + parries + blocks + missed by attacker).
    ///
    /// Note: `misses` (player's own missed attacks) is excluded since it
    /// represents offensive misses, not defensive avoidance.
    pub fn total(&self) -> u64 {
        self.dodges + self.parries + self.blocks + self.missed_by
    }

    /// Total attacks fully mitigated (full resists + full absorbs + full blocks).
    pub fn total_full_mitigation(&self) -> u64 {
        self.full_resists + self.full_absorbs + self.full_blocks
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
    #[allow(dead_code)] // Data model — stored for future item tooltip/link display
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
    /// The killer's name (if known). None for environmental deaths or unknown.
    pub killer: Option<String>,
    /// The ability that dealt the killing blow (if known).
    pub killing_blow: Option<String>,
    /// Damage amount of the killing blow (for overkill analysis).
    pub damage_amount: Option<u64>,
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
