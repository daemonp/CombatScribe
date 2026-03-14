//! Full combat log parser — port of `app.js` `parseLog()`.
//!
//! Operates on raw lines from an already-selected session, producing
//! a fully populated [`LogData`] structure.

use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;

use crate::log_data::{
    AbilityStats, Combatant, ConsumableUse, DeathEvent, DispelEvent, Encounter, GearSlot,
    InterruptEvent, LogData, LogEntry, LootEvent, PlayerStats, ResurrectEvent, TradeEvent,
};
use crate::parser;

// ── Spell Lists ─────────────────────────────────────────────────────────────

const DISPEL_SPELLS: &[&str] = &[
    "Dispel Magic",
    "Remove Curse",
    "Cleanse",
    "Purify",
    "Abolish Disease",
    "Abolish Poison",
    "Cure Disease",
    "Cure Poison",
    "Remove Lesser Curse",
    "Purge",
];

const RESURRECT_SPELLS: &[&str] = &[
    "Resurrection",
    "Redemption",
    "Ancestral Spirit",
    "Rebirth",
    "Soulstone Resurrection",
    "Revive",
];

const INTERRUPT_SPELLS: &[&str] = &[
    "Kick",
    "Pummel",
    "Earth Shock",
    "Counterspell",
    "Shield Bash",
    "Feral Charge",
    "Bash",
    "Spell Lock",
];

// ── Compiled Regexes ────────────────────────────────────────────────────────

static RE_CAST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?:CAST:\s*)?([A-Za-z]+)\s+casts\s+([A-Za-z\s]+?)(?:\(\d+\))?(?:\(Rank \d+\))?\s+on\s+([A-Za-z\s]+)",
    )
    .unwrap()
});

static RE_DMG_SPELL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"([A-Za-z]+(?:\s*\([^)]+\))?) 's ([A-Za-z\s']+) (?:hits|crits) ([A-Za-z\s']+) for (\d+)",
    )
    .unwrap()
});

static RE_DMG_AUTO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([A-Za-z]+) (?:hits|crits) ([A-Za-z\s']+) for (\d+)\.").unwrap());

static RE_DMG_SUFFER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"([A-Za-z\s']+) suffers (\d+) (?:\w+ )?damage from ([A-Za-z]+(?:\s*\([^)]+\))?) 's ([A-Za-z\s']+)",
    )
    .unwrap()
});

static RE_HEAL_SPELL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"([A-Za-z]+(?:\s*\([^)]+\))?) 's ([A-Za-z\s']+) (?:heals|critically heals) ([A-Za-z\s']+) for (\d+)",
    )
    .unwrap()
});

static RE_HEAL_GAIN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"([A-Za-z]+) gains (\d+) health from ([A-Za-z]+(?:\s*\([^)]+\))?) 's ([A-Za-z\s']+)",
    )
    .unwrap()
});

static RE_DODGE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([A-Za-z\s']+) attacks\. ([A-Za-z]+) dodges\.").unwrap());

static RE_PARRY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([A-Za-z\s']+) attacks\. ([A-Za-z]+) parries\.").unwrap());

static RE_MISS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([A-Za-z\s']+) misses ([A-Za-z]+)\.").unwrap());

static RE_BUFF_GAIN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([A-Za-z]+) gains ([A-Za-z\s':]+?) \((\d+)\)\.").unwrap());

static RE_BUFF_FADE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([A-Za-z\s':]+?) fades from ([A-Za-z]+)\.").unwrap());

static RE_LOOT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"LOOT:.*?&([A-Za-z]+) receives (?:loot|item): \|cff([a-f0-9]{6})\|Hitem:(\d+):[^|]+\|h\[([^\]]+)\]\|h\|rx?(\d+)",
    )
    .unwrap()
});

static RE_TRADE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"LOOT_TRADE:.*?&([A-Za-z]+) trades item (.+?) to ([A-Za-z]+)\.").unwrap()
});

static RE_PET_OWNER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([A-Za-z]+)\s+\(([A-Za-z]+)\)").unwrap());

static RE_ABSORB: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\((\d+) absorbed\)").unwrap());

/// V1 consumable line: `PlayerName uses ConsumableName.` or `...on Target.`
static RE_CONSUMABLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"([A-Za-z]+) uses ([A-Za-z][A-Za-z '\-]+?)(?:\s+on\s+[A-Za-z\s]+)?\.").unwrap()
});

// ── Public API ──────────────────────────────────────────────────────────────

/// Mutable state tracked across the main parse loop.
struct ParseState {
    in_combat: bool,
    combat_start: Option<f64>,
    current_boss: Option<String>,
    current_boss_killed: bool,
    current_zone: Option<String>,
    /// Timestamp of first `COMBATANT_INFO` sighting per player (for windowed filtering).
    combatant_timestamps: HashMap<String, f64>,
}

/// Parse the session lines into a fully populated `LogData`.
///
/// Uses keyword-based dispatch to avoid running all regexes on every line.
/// Most lines are damage/healing events; metadata/loot/buff lines are rare.
pub fn parse_log(lines: &[String]) -> LogData {
    let mut data = LogData::default();
    let mut state = ParseState {
        in_combat: false,
        combat_start: None,
        current_boss: None,
        current_boss_killed: false,
        current_zone: None,
        combatant_timestamps: HashMap::new(),
    };

    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let bytes = trimmed.as_bytes();
        let Some((timestamp, _)) = parser::parse_timestamp_fast(bytes) else {
            continue;
        };

        // Track time range
        if data.start_time.is_none() {
            data.start_time = Some(timestamp);
        }
        data.end_time = Some(timestamp);

        // ── Keyword-based dispatch ──────────────────────────────────────
        // Metadata lines (rare) — check first since they start with known prefixes
        if trimmed.contains("COMBATANT_INFO:")
            || trimmed.contains("ZONE_INFO:")
            || trimmed.contains("PLAYERS_IN_COMBAT:")
        {
            parse_metadata(trimmed, timestamp, &mut data, &mut state);
            continue;
        }

        // Loot/trade lines (rare)
        if trimmed.contains("LOOT:") || trimmed.contains("LOOT_TRADE:") {
            parse_loot_trade(trimmed, timestamp, &mut data);
            continue;
        }

        // Combat state changes (rare)
        if trimmed.contains("PLAYER_REGEN_DISABLED") || trimmed.contains("PLAYER_REGEN_ENABLED") {
            parse_combat_state(trimmed, timestamp, &mut data, &mut state);
            continue;
        }

        // Unit died (rare)
        if trimmed.contains("UNIT_DIED:") {
            parse_unit_died(trimmed, timestamp, &mut data, &mut state);
            continue;
        }

        // Cast events: dispels, resurrects, interrupts (moderately rare)
        if trimmed.contains("casts ") {
            parse_cast_events(trimmed, timestamp, &mut data);
            // Cast lines can also be damage/healing — don't skip below
        }

        // Pet ownership patterns: `Name (Owner)` (moderately common)
        // Cheap check: look for `(` which all pet patterns require
        if trimmed.contains('(') {
            parse_pet_ownership(trimmed, &mut data);
        }

        // ── High-frequency event dispatch ───────────────────────────────
        // Damage: must contain " for " (hits X for N, crits X for N, suffers N from)
        // or " suffers " (suffer format)
        if trimmed.contains(" for ") || trimmed.contains(" suffers ") {
            parse_damage_events(trimmed, timestamp, &mut data);
        }

        // Healing: "heals" or "health from"
        if trimmed.contains("heals ") || trimmed.contains(" health from ") {
            parse_healing_events(trimmed, timestamp, &mut data);
        }

        // Boss detection from combat lines (during active combat only)
        // Skip if we already identified a known boss for this combat window
        if state.in_combat
            && !state
                .current_boss
                .as_ref()
                .is_some_and(|b| parser::is_known_boss(b))
        {
            detect_boss_from_combat(trimmed, &data, &mut state);
        }

        // Avoidance: dodge/parry/miss keywords
        if trimmed.contains(" dodges.")
            || trimmed.contains(" parries.")
            || trimmed.contains(" misses ")
        {
            parse_avoidance(trimmed, &mut data);
        }

        // Buff events: "gains " or " fades from "
        if trimmed.contains(" gains ") || trimmed.contains(" fades from ") {
            parse_buff_events(trimmed, timestamp, &mut data);
        }

        // Consumable usage: " uses "
        if trimmed.contains(" uses ") {
            parse_consumable(trimmed, timestamp, &mut data);
        }
    }

    post_process(&mut data, &state.combatant_timestamps);
    data
}

// ── Line-Level Parse Helpers ────────────────────────────────────────────────

/// Parse `COMBATANT_INFO`, `ZONE_INFO`, and `PLAYERS_IN_COMBAT` lines.
fn parse_metadata(trimmed: &str, timestamp: f64, data: &mut LogData, state: &mut ParseState) {
    if trimmed.contains("COMBATANT_INFO:") {
        if let Some(info) = parser::extract_combatant_full(trimmed) {
            let name_str = info.name.to_string();
            let combatant = Combatant {
                class: info.class.to_string(),
                race: info.race.to_string(),
                guild: info.guild.map(str::to_string),
                gear: info
                    .gear
                    .iter()
                    .map(|g| g.and_then(parse_gear_slot))
                    .collect(),
                talent_summary: info.talent_summary,
                guid: info.guid.map(str::to_string),
                pet_name: info.pet_name.map(str::to_string),
            };
            // Track ALL combatant sightings for full roster preservation
            data.all_combatants
                .insert(name_str.clone(), combatant.clone());
            // Track first-seen timestamp for windowed filtering
            state
                .combatant_timestamps
                .entry(name_str)
                .or_insert(timestamp);
        } else if let Some((name, class)) = parser::extract_combatant(trimmed) {
            // Fallback: minimal extraction if full parse fails
            let name_str = name.to_string();
            let combatant = Combatant {
                class: class.to_string(),
                ..Combatant::default()
            };
            data.all_combatants
                .insert(name_str.clone(), combatant.clone());
            state
                .combatant_timestamps
                .entry(name_str)
                .or_insert(timestamp);
        }
    }

    if trimmed.contains("ZONE_INFO:") {
        if let Some(zone) = parser::extract_zone(trimmed) {
            data.zone_name = zone.to_string();
            state.current_zone = Some(zone.to_string());
        }
    }

    // PLAYERS_IN_COMBAT: 32/40
    if trimmed.contains("PLAYERS_IN_COMBAT:") {
        if let Some(pic) = parse_players_in_combat(trimmed) {
            data.raid_size = Some(pic);
        }
    }
}

/// Track combat start/end and create encounter records.
fn parse_combat_state(trimmed: &str, timestamp: f64, data: &mut LogData, state: &mut ParseState) {
    if trimmed.contains("PLAYER_REGEN_DISABLED") {
        state.in_combat = true;
        state.combat_start = Some(timestamp);
    }

    if trimmed.contains("PLAYER_REGEN_ENABLED") {
        if state.in_combat {
            if let Some(start) = state.combat_start {
                let duration = timestamp - start;
                if duration > 5.0 {
                    let is_boss = state
                        .current_boss
                        .as_ref()
                        .is_some_and(|b| parser::is_known_boss(b));
                    data.encounters.push(Encounter {
                        name: state.current_boss.clone(),
                        start,
                        end: timestamp,
                        duration,
                        is_boss,
                        is_kill: is_boss && state.current_boss_killed,
                        zone: state.current_zone.clone(),
                        attempt: None,
                    });
                }
                state.current_boss = None;
                state.current_boss_killed = false;
            }
        }
        state.in_combat = false;
    }
}

/// Grace period (seconds) after combat ends to still attribute a boss kill.
///
/// Combat can drop (`PLAYER_REGEN_ENABLED`) before the boss actually dies:
/// - `Garr`: adds explode, combat drops briefly before Garr dies
/// - `Majordomo Executus`: surrenders after adds die, `UNIT_DIED` fires ~8 min later
///   after RP completes
///
/// 10 minutes covers all known cases including Majordomo's long RP sequence.
const KILL_GRACE_SECS: f64 = 600.0;

/// Handle `UNIT_DIED` — player deaths and boss/mob detection.
fn parse_unit_died(trimmed: &str, timestamp: f64, data: &mut LogData, state: &mut ParseState) {
    if !trimmed.contains("UNIT_DIED:") {
        return;
    }

    let Some((dead_unit, guid)) = parser::extract_unit_died_with_guid(trimmed) else {
        return;
    };

    if is_player_guid(guid) {
        data.deaths.push(DeathEvent {
            timestamp,
            player: dead_unit.to_string(),
        });
        data.entries.push(LogEntry::Death {
            timestamp,
            player: dead_unit.to_string(),
        });
    } else if !data.all_combatants.contains_key(dead_unit) && dead_unit != "Unknown" {
        if parser::is_known_boss(dead_unit) {
            if state.in_combat {
                // Normal case: boss dies during combat
                state.current_boss = Some(dead_unit.to_string());
                state.current_boss_killed = true;
            } else {
                // Boss died after combat dropped — retroactively mark the
                // most recent encounter as a kill if it matches this boss
                // and occurred within the grace period.
                retroactive_boss_kill(data, dead_unit, timestamp);
            }
        } else if state.in_combat && !state.current_boss_killed {
            // Non-boss mob died during combat — track as potential encounter name.
            // Never overwrite a known boss, and never overwrite after a kill.
            let dominated = state
                .current_boss
                .as_ref()
                .is_none_or(|b| !parser::is_known_boss(b) && dead_unit.len() > b.len());
            if dominated {
                state.current_boss = Some(dead_unit.to_string());
            }
        }
    }
}

/// Retroactively mark a recent encounter as a kill when the boss dies
/// after combat has already ended (player regen re-enabled).
fn retroactive_boss_kill(data: &mut LogData, boss_name: &str, death_timestamp: f64) {
    // Search backwards for the most recent encounter that:
    //  1. Has a matching boss name (or no name yet)
    //  2. Ended within the grace period before the boss death
    for enc in data.encounters.iter_mut().rev() {
        if death_timestamp - enc.end > KILL_GRACE_SECS {
            break; // Too far in the past
        }
        // Match by name if set, or by "unnamed boss encounter in this time window"
        let name_matches = enc
            .name
            .as_ref()
            .is_some_and(|n| n.eq_ignore_ascii_case(boss_name));
        if name_matches || enc.name.is_none() {
            enc.name = Some(boss_name.to_string());
            enc.is_boss = true;
            enc.is_kill = true;
            return;
        }
    }
}

/// Detect boss names from damage/healing lines during combat.
///
/// On wipe attempts, the boss never dies so `UNIT_DIED` won't fire.
/// Instead, check source and target names in the last-recorded damage entry
/// against the known boss list. A known boss always takes priority.
fn detect_boss_from_combat(trimmed: &str, data: &LogData, state: &mut ParseState) {
    if !state.in_combat {
        return;
    }
    // Already found a known boss for this combat window — nothing to do
    if state
        .current_boss
        .as_ref()
        .is_some_and(|b| parser::is_known_boss(b))
    {
        return;
    }

    // Check the most recent entry (just pushed by parse_damage_events / parse_healing_events)
    let Some(entry) = data.entries.last() else {
        return;
    };

    let names: &[&str] = match entry {
        LogEntry::Damage { source, target, .. } | LogEntry::Healing { source, target, .. } => {
            &[source.as_str(), target.as_str()]
        }
        _ => return,
    };

    for &name in names {
        // Skip players — only interested in NPC/boss names.
        // Also skip names with "(self damage)" suffix — these are reformatted player names.
        if data.all_combatants.contains_key(name)
            || name == "Unknown"
            || name.ends_with("(self damage)")
        {
            continue;
        }
        if parser::is_known_boss(name) {
            state.current_boss = Some(name.to_string());
            return;
        }
        // Fall back to longest non-player name if no boss identified yet
        if state
            .current_boss
            .as_ref()
            .is_none_or(|b| name.len() > b.len())
        {
            state.current_boss = Some(name.to_string());
        }
    }

    // Also check for boss names mentioned directly in the line (e.g. in resist/immune messages)
    if state.current_boss.is_none()
        || !state
            .current_boss
            .as_ref()
            .is_some_and(|b| parser::is_known_boss(b))
    {
        // Quick scan: only worth checking if line contains common combat verbs
        if trimmed.contains("hits ")
            || trimmed.contains("crits ")
            || trimmed.contains("suffers ")
            || trimmed.contains("resisted")
        {
            for boss in parser::known_boss_names() {
                if trimmed.contains(boss) {
                    state.current_boss = Some(boss.to_string());
                    return;
                }
            }
        }
    }
}

/// Categorization of cast events for the parameterized handler.
#[derive(Clone, Copy)]
enum CastEventKind {
    Dispel,
    Resurrect,
    Interrupt,
}

/// Check if a line contains `"casts <spell>"` for any spell in the list.
///
/// Uses a cheap `contains("casts ")` pre-check and then `contains(spell)` to
/// avoid the `format!("casts {spell}")` allocation per spell per line.
fn matches_spell_cast<'a>(trimmed: &str, spells: &[&'a str]) -> Option<&'a str> {
    if !trimmed.contains("casts ") {
        return None;
    }
    spells.iter().copied().find(|spell| trimmed.contains(spell))
}

/// Parse dispels, resurrects, and interrupts from cast events.
fn parse_cast_events(trimmed: &str, timestamp: f64, data: &mut LogData) {
    // Dispels — any "casts <dispel_spell>" line
    if matches_spell_cast(trimmed, DISPEL_SPELLS).is_some() {
        try_record_cast(trimmed, timestamp, data, CastEventKind::Dispel);
        return;
    }

    // Resurrects — exclude "begins to cast" and "fails casting"
    if matches_spell_cast(trimmed, RESURRECT_SPELLS).is_some()
        && !trimmed.contains("begins to cast")
        && !trimmed.contains("fails casting")
    {
        try_record_cast(trimmed, timestamp, data, CastEventKind::Resurrect);
        return;
    }

    // Interrupts — exclude "begins to cast", require caster=player, target=non-player
    if matches_spell_cast(trimmed, INTERRUPT_SPELLS).is_some()
        && !trimmed.contains("begins to cast")
    {
        try_record_cast(trimmed, timestamp, data, CastEventKind::Interrupt);
    }
}

/// Extract caster/spell/target from a cast line and record the appropriate event.
fn try_record_cast(trimmed: &str, timestamp: f64, data: &mut LogData, kind: CastEventKind) {
    let Some(caps) = RE_CAST.captures(trimmed) else {
        return;
    };
    let Some(caster) = caps.get(1).map(|m| m.as_str().to_string()) else {
        return;
    };
    let Some(spell) = caps.get(2).map(|m| m.as_str().trim().to_string()) else {
        return;
    };
    let Some(target) = caps.get(3).map(|m| m.as_str().trim().to_string()) else {
        return;
    };

    match kind {
        CastEventKind::Dispel => {
            data.dispels.push(DispelEvent {
                timestamp,
                caster: caster.clone(),
                target: target.clone(),
                spell: spell.clone(),
            });
            data.entries.push(LogEntry::Dispel {
                timestamp,
                caster,
                target,
                spell,
            });
        }
        CastEventKind::Resurrect => {
            data.resurrects.push(ResurrectEvent {
                timestamp,
                caster: caster.clone(),
                target: target.clone(),
                spell: spell.clone(),
            });
            data.entries.push(LogEntry::Resurrect {
                timestamp,
                caster,
                target,
                spell,
            });
        }
        CastEventKind::Interrupt => {
            if data.all_combatants.contains_key(&caster)
                && !data.all_combatants.contains_key(&target)
            {
                data.interrupts.push(InterruptEvent {
                    timestamp,
                    caster: caster.clone(),
                    target: target.clone(),
                    spell: spell.clone(),
                });
                data.entries.push(LogEntry::Interrupt {
                    timestamp,
                    caster,
                    target,
                    spell,
                });
            }
        }
    }
}

/// Detect pet ownership from `PetName (OwnerName)` patterns.
fn parse_pet_ownership(trimmed: &str, data: &mut LogData) {
    if let Some(caps) = RE_PET_OWNER.captures(trimmed) {
        let Some(pet_name) = caps.get(1).map(|m| m.as_str()) else {
            return;
        };
        let Some(owner_name) = caps.get(2).map(|m| m.as_str()) else {
            return;
        };
        if data.all_combatants.contains_key(owner_name)
            && !data.all_combatants.contains_key(pet_name)
        {
            data.pet_owners
                .insert(pet_name.to_string(), owner_name.to_string());
        }
    }
}

/// Extract absorbed amount from a line (only relevant for damage lines).
fn parse_absorbed(trimmed: &str) -> u64 {
    RE_ABSORB
        .captures(trimmed)
        .and_then(|c| c.get(1)?.as_str().parse::<u64>().ok())
        .unwrap_or(0)
}

/// Record a parsed damage event into stats and entries.
#[allow(clippy::too_many_arguments)]
fn record_damage(
    data: &mut LogData,
    timestamp: f64,
    source: String,
    target: String,
    spell: String,
    amount: u64,
    absorbed: u64,
    is_crit: bool,
    pet_owner: Option<&str>,
) {
    add_damage(data, &source, &spell, amount, pet_owner, is_crit);
    add_damage_taken(data, &target, amount);
    record_absorb(data, &target, absorbed);
    data.entries.push(LogEntry::Damage {
        timestamp,
        source,
        target,
        spell,
        amount,
        absorbed,
        is_crit,
    });
}

/// Parse all 3 damage event formats.
fn parse_damage_events(trimmed: &str, timestamp: f64, data: &mut LogData) {
    let is_crit = trimmed.contains(" crits ") || trimmed.contains(" critically ");

    // Format 1: Source 's Spell hits/crits Target for N
    if let Some(caps) = RE_DMG_SPELL.captures(trimmed) {
        let Some(source) = caps.get(1).map(|m| m.as_str().to_string()) else {
            return;
        };
        let Some(spell) = caps.get(2).map(|m| m.as_str().trim().to_string()) else {
            return;
        };
        let Some(target) = caps.get(3).map(|m| m.as_str().trim().to_string()) else {
            return;
        };
        let amount: u64 = caps
            .get(4)
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(0);
        let absorbed = parse_absorbed(trimmed);
        let pet_owner = extract_pet_owner(&source, data);
        record_damage(
            data,
            timestamp,
            source,
            target,
            spell,
            amount,
            absorbed,
            is_crit,
            pet_owner.as_deref(),
        );
        return;
    }

    // Format 2: Source hits/crits Target for N.
    if let Some(caps) = RE_DMG_AUTO.captures(trimmed) {
        let Some(source) = caps.get(1).map(|m| m.as_str().to_string()) else {
            return;
        };
        let Some(target) = caps.get(2).map(|m| m.as_str().trim().to_string()) else {
            return;
        };
        let amount: u64 = caps
            .get(3)
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(0);
        let absorbed = parse_absorbed(trimmed);
        record_damage(
            data,
            timestamp,
            source,
            target,
            "Auto Attack".to_string(),
            amount,
            absorbed,
            is_crit,
            None,
        );
        return;
    }

    // Format 3: Target suffers N damage from Source 's Spell
    if let Some(caps) = RE_DMG_SUFFER.captures(trimmed) {
        let Some(target) = caps.get(1).map(|m| m.as_str().trim().to_string()) else {
            return;
        };
        let amount: u64 = caps
            .get(2)
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(0);
        let Some(source) = caps.get(3).map(|m| m.as_str().to_string()) else {
            return;
        };
        let Some(spell) = caps.get(4).map(|m| m.as_str().trim().to_string()) else {
            return;
        };
        let absorbed = parse_absorbed(trimmed);
        let pet_owner = extract_pet_owner(&source, data);
        record_damage(
            data,
            timestamp,
            source,
            target,
            spell,
            amount,
            absorbed,
            is_crit,
            pet_owner.as_deref(),
        );
    }
}

/// Parse both healing event formats.
fn parse_healing_events(trimmed: &str, timestamp: f64, data: &mut LogData) {
    let is_heal_crit = trimmed.contains("critically heals");

    // Format 1: Source 's Spell heals/critically heals Target for N
    if let Some(caps) = RE_HEAL_SPELL.captures(trimmed) {
        let Some(source) = caps.get(1).map(|m| m.as_str().to_string()) else {
            return;
        };
        let Some(mut spell) = caps.get(2).map(|m| m.as_str().trim().to_string()) else {
            return;
        };
        let Some(target) = caps.get(3).map(|m| m.as_str().trim().to_string()) else {
            return;
        };
        let amount: u64 = caps
            .get(4)
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(0);

        if let Some(stripped) = spell.strip_suffix(" critically") {
            spell = stripped.trim().to_string();
        }

        add_healing(data, &source, &spell, amount, is_heal_crit);
        data.entries.push(LogEntry::Healing {
            timestamp,
            source,
            target,
            spell,
            amount,
            is_crit: is_heal_crit,
        });
        return;
    }

    // Format 2: Target gains N health from Source 's Spell
    if let Some(caps) = RE_HEAL_GAIN.captures(trimmed) {
        let Some(target) = caps.get(1).map(|m| m.as_str().to_string()) else {
            return;
        };
        let amount: u64 = caps
            .get(2)
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(0);
        let Some(source) = caps.get(3).map(|m| m.as_str().to_string()) else {
            return;
        };
        let Some(spell) = caps.get(4).map(|m| m.as_str().trim().to_string()) else {
            return;
        };

        add_healing(data, &source, &spell, amount, false);
        data.entries.push(LogEntry::Healing {
            timestamp,
            source,
            target,
            spell,
            amount,
            is_crit: false,
        });
    }
}

/// Parse dodge, parry, and miss events.
///
/// Uses `all_combatants` (built from `COMBATANT_INFO` during parsing) rather
/// than `combatants` (which is only populated in post-processing).
fn parse_avoidance(trimmed: &str, data: &mut LogData) {
    if let Some(caps) = RE_DODGE.captures(trimmed) {
        if let Some(defender) = caps.get(2).map(|m| m.as_str()) {
            if data.all_combatants.contains_key(defender) {
                data.avoidance
                    .entry(defender.to_string())
                    .or_default()
                    .dodges += 1;
            }
        }
    }

    if let Some(caps) = RE_PARRY.captures(trimmed) {
        if let Some(defender) = caps.get(2).map(|m| m.as_str()) {
            if data.all_combatants.contains_key(defender) {
                data.avoidance
                    .entry(defender.to_string())
                    .or_default()
                    .parries += 1;
            }
        }
    }

    if let Some(caps) = RE_MISS.captures(trimmed) {
        let Some(attacker) = caps.get(1).map(|m| m.as_str().trim()) else {
            return;
        };
        let Some(defender) = caps.get(2).map(|m| m.as_str()) else {
            return;
        };

        if data.all_combatants.contains_key(defender) && !data.all_combatants.contains_key(attacker)
        {
            data.avoidance
                .entry(defender.to_string())
                .or_default()
                .missed_by += 1;
        } else if data.all_combatants.contains_key(attacker)
            && !data.all_combatants.contains_key(defender)
        {
            data.avoidance
                .entry(attacker.to_string())
                .or_default()
                .misses += 1;
        }
    }
}

/// Parse buff gain and fade events.
///
/// Uses `all_combatants` (built from `COMBATANT_INFO` during parsing) rather
/// than `combatants` (which is only populated in post-processing).
fn parse_buff_events(trimmed: &str, timestamp: f64, data: &mut LogData) {
    if let Some(caps) = RE_BUFF_GAIN.captures(trimmed) {
        if let Some(player) = caps.get(1).map(|m| m.as_str()) {
            if let Some(buff) = caps.get(2).map(|m| m.as_str().trim().to_string()) {
                if data.all_combatants.contains_key(player) {
                    let buffs = data.buffs.entry(player.to_string()).or_default();
                    let stats = buffs.entry(buff).or_default();
                    stats.gains += 1;
                    if stats.first_gain.is_none() {
                        stats.first_gain = Some(timestamp);
                    }
                }
            }
        }
    }

    if let Some(caps) = RE_BUFF_FADE.captures(trimmed) {
        if let Some(buff) = caps.get(1).map(|m| m.as_str().trim().to_string()) {
            if let Some(player) = caps.get(2).map(|m| m.as_str()) {
                if data.all_combatants.contains_key(player) {
                    let buffs = data.buffs.entry(player.to_string()).or_default();
                    let stats = buffs.entry(buff).or_default();
                    stats.fades += 1;
                    stats.last_fade = Some(timestamp);
                }
            }
        }
    }
}

/// Parse loot and trade events.
fn parse_loot_trade(trimmed: &str, timestamp: f64, data: &mut LogData) {
    if trimmed.contains("LOOT:") {
        if let Some(caps) = RE_LOOT.captures(trimmed) {
            let Some(player) = caps.get(1).map(|m| m.as_str().to_string()) else {
                return;
            };
            let Some(color_code) = caps.get(2).map(|m| m.as_str()) else {
                return;
            };
            let item_id: u64 = caps
                .get(3)
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(0);
            let Some(item_name) = caps.get(4).map(|m| m.as_str().to_string()) else {
                return;
            };
            let quantity: u64 = caps
                .get(5)
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(1);

            let quality = match color_code {
                "9d9d9d" => "poor",
                "1eff00" => "uncommon",
                "0070dd" => "rare",
                "a335ee" => "epic",
                "ff8000" => "legendary",
                _ => "common",
            };

            data.loot.push(LootEvent {
                timestamp,
                player,
                item_name,
                item_id,
                quality: quality.to_string(),
                quantity,
                boss: String::new(),
                traded_to: None,
            });
        }
    }

    if trimmed.contains("LOOT_TRADE:") {
        if let Some(caps) = RE_TRADE.captures(trimmed) {
            let Some(from_player) = caps.get(1).map(|m| m.as_str().to_string()) else {
                return;
            };
            let Some(item_name) = caps.get(2).map(|m| m.as_str().trim().to_string()) else {
                return;
            };
            let Some(to_player) = caps.get(3).map(|m| m.as_str().to_string()) else {
                return;
            };

            data.trades.push(TradeEvent {
                timestamp,
                from_player,
                item_name,
                to_player,
            });
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Check if a GUID represents a player (starts with `0x0000000000`).
fn is_player_guid(guid: &str) -> bool {
    guid.starts_with("0x0000000000")
}

/// Record absorbed damage on a player target.
fn record_absorb(data: &mut LogData, target: &str, absorbed_amount: u64) {
    if absorbed_amount > 0 && data.all_combatants.contains_key(target) {
        *data.absorbs.entry(target.to_string()).or_insert(0) += absorbed_amount;
    }
}

/// Extract pet owner from a source string like `PetName (OwnerName)`.
fn extract_pet_owner(source: &str, data: &LogData) -> Option<String> {
    let caps = RE_PET_OWNER.captures(source)?;
    let owner_name = caps.get(2)?.as_str();
    if data.all_combatants.contains_key(owner_name) {
        return Some(owner_name.to_string());
    }
    None
}

/// Ensure a `PlayerStats` entry exists for a name.
fn ensure_stats<'a>(data: &'a mut LogData, name: &str) -> &'a mut PlayerStats {
    data.player_stats.entry(name.to_string()).or_default()
}

/// Add damage to a source player's stats.
fn add_damage(
    data: &mut LogData,
    source: &str,
    spell: &str,
    amount: u64,
    pet_owner: Option<&str>,
    is_crit: bool,
) {
    // Stats for the direct source
    let stats = ensure_stats(data, source);
    stats.damage += amount;
    let ab = stats.abilities.entry(spell.to_string()).or_default();
    ab.total += amount;
    ab.hits += 1;
    if is_crit {
        ab.crits += 1;
    }

    // Attribute to pet owner
    if let Some(owner) = pet_owner {
        if owner != source {
            let owner_stats = ensure_stats(data, owner);
            owner_stats.pet_damage += amount;
            let pet_spell = format!("Pet: {spell}");
            let ab = owner_stats
                .abilities
                .entry(pet_spell)
                .or_insert_with(|| AbilityStats {
                    is_pet: true,
                    ..AbilityStats::default()
                });
            ab.total += amount;
            ab.hits += 1;
            if is_crit {
                ab.crits += 1;
            }
        }
    }
}

/// Add damage taken to a target player's stats.
fn add_damage_taken(data: &mut LogData, target: &str, amount: u64) {
    ensure_stats(data, target).damage_taken += amount;
}

/// Add healing to a source player's stats.
fn add_healing(data: &mut LogData, source: &str, spell: &str, amount: u64, is_crit: bool) {
    let stats = ensure_stats(data, source);
    stats.healing += amount;
    let ab = stats
        .healing_abilities
        .entry(spell.to_string())
        .or_default();
    ab.total += amount;
    ab.hits += 1;
    if is_crit {
        ab.crits += 1;
    }
}

/// Post-process: assign bosses to loot, link trades, number encounter attempts,
/// and build windowed combatant roster.
fn post_process(data: &mut LogData, combatant_timestamps: &HashMap<String, f64>) {
    // Assign bosses to loot (within 2 minutes after encounter end)
    for loot in &mut data.loot {
        let mut boss = "Trash/Other".to_string();
        for enc in &data.encounters {
            if loot.timestamp >= enc.start && loot.timestamp <= enc.end + 120.0 {
                if let Some(name) = &enc.name {
                    boss.clone_from(name);
                }
            }
        }
        loot.boss = boss;
    }

    // Link trades to loot items
    for trade in &data.trades {
        for loot in &mut data.loot {
            if loot.item_name == trade.item_name
                && loot.player == trade.from_player
                && trade.timestamp >= loot.timestamp
            {
                loot.traded_to = Some(trade.to_player.clone());
                break;
            }
        }
    }

    // Number encounter attempts
    let mut boss_attempts: HashMap<String, usize> = HashMap::new();
    for enc in &mut data.encounters {
        if enc.is_boss {
            if let Some(name) = &enc.name {
                let count = boss_attempts.entry(name.clone()).or_insert(0);
                *count += 1;
                enc.attempt = Some(*count);
            }
        }
    }

    // Build filtered combatants using windowed heuristic.
    // COMBATANT_INFO fires for ALL nearby players (cities, open world), not just
    // raid/party members.  Use a "raid window" to filter:
    //   - 5 minutes before first encounter start → last encounter end
    //   - Plus any player who appears in player_stats (dealt/took damage, healed)
    let raid_window = build_raid_window(&data.encounters);
    for (name, combatant) in &data.all_combatants {
        let in_window = combatant_timestamps
            .get(name)
            .is_some_and(|&ts| raid_window.as_ref().is_some_and(|w| ts >= w.0 && ts <= w.1));
        let in_combat = data.player_stats.contains_key(name);
        if in_window || in_combat {
            data.combatants.insert(name.clone(), combatant.clone());
        }
    }
}

/// Compute the raid window: 5 min before first encounter start → last encounter end.
/// Returns `None` if there are no encounters.
fn build_raid_window(encounters: &[Encounter]) -> Option<(f64, f64)> {
    let first_start = encounters.iter().map(|e| e.start).reduce(f64::min)?;
    let last_end = encounters.iter().map(|e| e.end).reduce(f64::max)?;
    // 5 minutes = 300 seconds before first encounter
    Some((first_start - 300.0, last_end))
}

/// Parse consumable usage from V1 `"uses"` lines.
fn parse_consumable(trimmed: &str, timestamp: f64, data: &mut LogData) {
    if !trimmed.contains(" uses ") {
        return;
    }
    // Skip CAST: lines — those are V2 format handled separately
    if trimmed.contains("CAST:") {
        return;
    }
    if let Some(caps) = RE_CONSUMABLE.captures(trimmed) {
        let Some(player) = caps.get(1).map(|m| m.as_str().to_string()) else {
            return;
        };
        let Some(consumable) = caps.get(2).map(|m| m.as_str().trim().to_string()) else {
            return;
        };

        data.consumables.push(ConsumableUse {
            timestamp,
            player,
            consumable,
        });
    }
}

/// Parse `PLAYERS_IN_COMBAT: N/M` into `(in_combat, total)`.
fn parse_players_in_combat(trimmed: &str) -> Option<(u32, u32)> {
    let idx = trimmed.find("PLAYERS_IN_COMBAT:")? + "PLAYERS_IN_COMBAT:".len();
    let rest = trimmed[idx..].trim();
    let slash = rest.find('/')?;
    let in_combat = rest[..slash].trim().parse::<u32>().ok()?;
    let total = rest[slash + 1..].trim().parse::<u32>().ok()?;
    Some((in_combat, total))
}

/// Parse a gear slot string like `"16953:2564:0:0"` into a `GearSlot`.
fn parse_gear_slot(raw: &str) -> Option<GearSlot> {
    if raw.is_empty() || raw == "nil" {
        return None;
    }
    let mut parts = raw.split(':');
    let item_id = parts.next()?.parse::<u32>().ok()?;
    if item_id == 0 {
        return None;
    }
    let enchant_id = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let suffix_id = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    Some(GearSlot {
        item_id,
        enchant_id,
        suffix_id,
        raw: raw.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_regex_heal_spell() {
        let line = "Acedica 's Mending Light heals Acedica for 51.";
        let caps = RE_HEAL_SPELL.captures(line).expect("should match");
        assert_eq!(caps.get(1).unwrap().as_str(), "Acedica");
        assert_eq!(caps.get(2).unwrap().as_str().trim(), "Mending Light");
        assert_eq!(caps.get(3).unwrap().as_str().trim(), "Acedica");
        assert_eq!(caps.get(4).unwrap().as_str(), "51");

        // Crit heal
        let crit = "Acedica 's Mending Light critically heals Acedica for 77.";
        assert!(RE_HEAL_SPELL.captures(crit).is_some());

        // With timestamp prefix (what the parser actually sees)
        let ts = "1/24 14:55:35.116  Acedica 's Mending Light heals Acedica for 51.";
        assert!(RE_HEAL_SPELL.captures(ts).is_some());
    }

    #[test]
    fn test_regex_heal_gain() {
        let line = "Acedica gains 148 health from Carnonos 's Rejuvenation.";
        let caps = RE_HEAL_GAIN.captures(line).expect("should match");
        assert_eq!(caps.get(1).unwrap().as_str(), "Acedica");
        assert_eq!(caps.get(2).unwrap().as_str(), "148");
        assert_eq!(caps.get(3).unwrap().as_str(), "Carnonos");
    }

    #[test]
    fn test_regex_dmg_spell() {
        let line = "Acedica 's Holy Shield hits Anvilrage Guardsman for 108 Holy damage.";
        let caps = RE_DMG_SPELL.captures(line).expect("should match");
        assert_eq!(caps.get(1).unwrap().as_str(), "Acedica");
        assert_eq!(caps.get(4).unwrap().as_str(), "108");
    }

    #[test]
    fn test_regex_dmg_auto() {
        let line = "Ashbash hits Plagued Ghoul for 515.";
        let caps = RE_DMG_AUTO.captures(line).expect("should match");
        assert_eq!(caps.get(1).unwrap().as_str(), "Ashbash");
        assert_eq!(caps.get(3).unwrap().as_str(), "515");
    }

    #[test]
    fn test_parse_log_integration() {
        let lines: Vec<String> = vec![
            "1/27 12:23:41.440  COMBATANT_INFO: 27.01.26 12:23:41&Carnonos&DRUID&NightElf&2&nil&nil&nil&nil&47354:1508:0:0&18404:928:0:0&21665:3017:0:0&69107:0:0:0&21680:1891:0:0&47359:92:0:0&23071:1506:0:0&47388:1068:0:0&47341:1887:0:0&21672:2564:0:0&19384:928:0:0&21408:928:0:0&13965:0:0:0&11815:0:0:0&21409:849:0:0&23039:2646:0:0&nil&22397:0:0:0&nil&nil&0x00000000004A2125&nil".to_string(),
            "1/27 12:24:00.000  PLAYER_REGEN_DISABLED".to_string(),
            "1/27 12:24:01.000  Carnonos 's Rejuvenation heals Carnonos for 400.".to_string(),
            "1/27 12:24:02.000  Carnonos 's Regrowth critically heals Carnonos for 800.".to_string(),
            "1/27 12:24:03.000  Carnonos gains 148 health from Carnonos 's Rejuvenation.".to_string(),
            "1/27 12:24:04.000  Carnonos hits Razorgore the Untamed for 500.".to_string(),
            "1/27 12:35:00.000  PLAYER_REGEN_ENABLED".to_string(),
        ];
        let data = parse_log(&lines);

        let stats = data.player_stats.get("Carnonos").expect("Carnonos stats");
        assert_eq!(stats.damage, 500);
        assert_eq!(stats.healing, 400 + 800 + 148);
        assert_eq!(data.encounters.len(), 1);
    }

    #[test]
    fn test_regex_buff_gain() {
        // Simple buff
        let line = "Acedica gains Demon Armor (1).";
        let caps = RE_BUFF_GAIN
            .captures(line)
            .expect("should match simple buff");
        assert_eq!(caps.get(1).unwrap().as_str(), "Acedica");
        assert_eq!(caps.get(2).unwrap().as_str().trim(), "Demon Armor");
        assert_eq!(caps.get(3).unwrap().as_str(), "1");

        // Buff with colon
        let line2 = "Acedica gains Power Word: Fortitude (1).";
        let caps2 = RE_BUFF_GAIN
            .captures(line2)
            .expect("should match buff with colon");
        assert_eq!(
            caps2.get(2).unwrap().as_str().trim(),
            "Power Word: Fortitude"
        );
    }

    #[test]
    fn test_regex_buff_fade() {
        let line = "Power Word: Fortitude fades from Acedica.";
        let caps = RE_BUFF_FADE
            .captures(line)
            .expect("should match fade with colon");
        assert_eq!(
            caps.get(1).unwrap().as_str().trim(),
            "Power Word: Fortitude"
        );
        assert_eq!(caps.get(2).unwrap().as_str(), "Acedica");
    }

    #[test]
    fn test_regex_avoidance() {
        let dodge = "1/24 14:55:36.750  Anvilrage Guardsman attacks. Acedica dodges.";
        let caps = RE_DODGE.captures(dodge).expect("should match dodge");
        assert_eq!(caps.get(2).unwrap().as_str(), "Acedica");

        let parry = "1/24 14:55:35.833  Anvilrage Footman attacks. Acedica parries.";
        let caps2 = RE_PARRY.captures(parry).expect("should match parry");
        assert_eq!(caps2.get(2).unwrap().as_str(), "Acedica");
    }

    #[test]
    fn test_parse_buffs_and_avoidance() {
        let lines: Vec<String> = vec![
            "1/27 12:23:41.000  COMBATANT_INFO: 27.01.26 12:23:41&Acedica&PALADIN&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            "1/27 12:24:00.000  PLAYER_REGEN_DISABLED".to_string(),
            "1/27 12:24:01.000  Acedica gains Power Word: Fortitude (1).".to_string(),
            "1/27 12:24:02.000  Acedica gains Arcane Brilliance (1).".to_string(),
            "1/27 12:24:03.000  Power Word: Fortitude fades from Acedica.".to_string(),
            "1/27 12:24:04.000  Anvilrage Footman attacks. Acedica dodges.".to_string(),
            "1/27 12:24:05.000  Anvilrage Footman attacks. Acedica parries.".to_string(),
            "1/27 12:35:00.000  PLAYER_REGEN_ENABLED".to_string(),
        ];
        let data = parse_log(&lines);

        // Check buffs
        let buffs = data.buffs.get("Acedica");
        assert!(buffs.is_some(), "Acedica should have buffs");
        let buffs = buffs.unwrap();
        assert!(
            buffs.contains_key("Power Word: Fortitude"),
            "Should track PW:F"
        );
        assert!(buffs.contains_key("Arcane Brilliance"), "Should track AB");
        assert_eq!(buffs["Power Word: Fortitude"].gains, 1);
        assert_eq!(buffs["Power Word: Fortitude"].fades, 1);

        // Check avoidance
        let avoid = data.avoidance.get("Acedica");
        assert!(avoid.is_some(), "Acedica should have avoidance");
        let avoid = avoid.unwrap();
        assert_eq!(avoid.dodges, 1);
        assert_eq!(avoid.parries, 1);
    }

    #[test]
    fn test_boss_detection_from_combat_lines() {
        // Simulate a wipe: boss is fought but never dies
        let lines: Vec<String> = vec![
            "1/27 12:23:41.000  COMBATANT_INFO: 27.01.26 12:23:41&Acedica&PALADIN&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            "1/27 20:45:19.000  PLAYER_REGEN_DISABLED".to_string(),
            "1/27 20:45:21.000  Acedica 's Holy Shield hits Chromaggus for 125 Holy damage.".to_string(),
            "1/27 20:45:22.000  Chromaggus hits Acedica for 1245.".to_string(),
            "1/27 20:45:23.000  Acedica hits Chromaggus for 113.".to_string(),
            // Wipe — no UNIT_DIED for boss
            "1/27 20:48:13.000  PLAYER_REGEN_ENABLED".to_string(),
            // Second attempt — kill
            "1/27 20:54:56.000  PLAYER_REGEN_DISABLED".to_string(),
            "1/27 20:54:57.000  Acedica hits Chromaggus for 200.".to_string(),
            "1/27 20:59:26.000  UNIT_DIED:Chromaggus:0xF1300036C4014F18".to_string(),
            "1/27 20:59:26.000  Chromaggus dies.".to_string(),
            "1/27 20:59:27.000  PLAYER_REGEN_ENABLED".to_string(),
        ];
        let data = parse_log(&lines);

        assert_eq!(
            data.encounters.len(),
            2,
            "Should have 2 encounters (wipe + kill)"
        );

        let wipe = &data.encounters[0];
        assert_eq!(
            wipe.name.as_deref(),
            Some("Chromaggus"),
            "Wipe should identify Chromaggus"
        );
        assert!(wipe.is_boss, "Wipe encounter should be marked as boss");
        assert!(!wipe.is_kill, "Wipe should not be marked as kill");

        let kill = &data.encounters[1];
        assert_eq!(
            kill.name.as_deref(),
            Some("Chromaggus"),
            "Kill should identify Chromaggus"
        );
        assert!(kill.is_boss, "Kill encounter should be marked as boss");
        assert!(kill.is_kill, "Kill should be marked as kill");
    }

    #[test]
    fn test_boss_kill_during_combat() {
        // Boss dies during combat — should be marked as Kill
        let lines: Vec<String> = vec![
            "1/27 12:23:41.000  COMBATANT_INFO: 27.01.26 12:23:41&Tank&WARRIOR&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            "1/27 19:39:51.000  PLAYER_REGEN_DISABLED".to_string(),
            "1/27 19:39:52.000  Tank 's Bloodthirst hits Garr for 197.".to_string(),
            "1/27 19:40:05.000  Tank hits Garr for 150.".to_string(),
            "1/27 19:41:06.000  UNIT_DIED:Garr:0xF130002F1900DD21".to_string(),
            "1/27 19:42:30.000  PLAYER_REGEN_ENABLED".to_string(),
        ];
        let data = parse_log(&lines);
        assert_eq!(data.encounters.len(), 1, "Should have 1 encounter");
        let enc = &data.encounters[0];
        assert_eq!(enc.name.as_deref(), Some("Garr"));
        assert!(enc.is_boss, "Garr should be marked as boss");
        assert!(enc.is_kill, "Garr should be marked as kill");
    }

    #[test]
    fn test_boss_kill_after_combat_drops() {
        // Boss dies AFTER combat drops (PLAYER_REGEN_ENABLED fires before UNIT_DIED)
        // This happens with Garr (adds explode, combat drops) and Majordomo
        let lines: Vec<String> = vec![
            "1/27 12:23:41.000  COMBATANT_INFO: 27.01.26 12:23:41&Tank&WARRIOR&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            "1/27 19:39:51.000  PLAYER_REGEN_DISABLED".to_string(),
            "1/27 19:39:52.000  Tank 's Bloodthirst hits Garr for 197.".to_string(),
            "1/27 19:40:05.000  Tank hits Garr for 150.".to_string(),
            // Combat drops before boss dies
            "1/27 19:40:58.000  PLAYER_REGEN_ENABLED".to_string(),
            // Boss dies 10s after combat ended
            "1/27 19:41:08.000  UNIT_DIED:Garr:0xF130002F1900DD21".to_string(),
        ];
        let data = parse_log(&lines);
        assert_eq!(data.encounters.len(), 1, "Should have 1 encounter");
        let enc = &data.encounters[0];
        assert_eq!(enc.name.as_deref(), Some("Garr"));
        assert!(enc.is_boss, "Garr should be marked as boss");
        assert!(enc.is_kill, "Garr should be retroactively marked as kill");
    }

    #[test]
    fn test_dmg_regexes_dont_match_heal_lines() {
        let heal1 = "1/24 14:55:35.116  Acedica 's Mending Light heals Acedica for 51.";
        let heal2 = "1/24 14:55:48.277  Acedica 's Mending Light critically heals Acedica for 77.";
        let heal3 = "1/24 14:56:15.365  Acedica gains 148 health from Carnonos 's Rejuvenation.";

        assert!(
            RE_DMG_SPELL.captures(heal1).is_none(),
            "DMG_SPELL must not match heal line"
        );
        assert!(
            RE_DMG_SPELL.captures(heal2).is_none(),
            "DMG_SPELL must not match crit heal line"
        );
        assert!(
            RE_DMG_AUTO.captures(heal1).is_none(),
            "DMG_AUTO must not match heal spell line"
        );
        assert!(
            RE_DMG_AUTO.captures(heal3).is_none(),
            "DMG_AUTO must not match heal gain line"
        );
        assert!(
            RE_DMG_SUFFER.captures(heal1).is_none(),
            "DMG_SUFFER must not match heal line"
        );
    }
}
