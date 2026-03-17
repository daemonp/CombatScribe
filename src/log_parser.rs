//! Full combat log parser — port of `app.js` `parseLog()`.
//!
//! Operates on raw lines from an already-selected session, producing
//! a fully populated [`LogData`] structure.

use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use crate::log_data::{
    AbilityStats, Combatant, ConsumableUse, DeathEvent, DispelEvent, Encounter, GearSlot,
    InterruptEvent, ItemQuality, LogData, LogEntry, LootEvent, PlayerStats, ResurrectEvent,
    TradeEvent,
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
        r"([A-Za-z]+(?:\s[A-Za-z]+)*(?:\s*\([^)]+\))?) 's ([A-Za-z\s']+) (?:hits|crits) ([A-Za-z\s']+) for (\d+)",
    )
    .unwrap()
});

static RE_DMG_AUTO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"([A-Za-z]+(?:\s[A-Za-z]+)*) (?:hits|crits) ([A-Za-z\s']+) for (\d+)\.").unwrap()
});

static RE_DMG_SUFFER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"([A-Za-z\s']+) suffers (\d+) (?:\w+ )?damage from ([A-Za-z]+(?:\s[A-Za-z]+)*(?:\s*\([^)]+\))?) 's ([A-Za-z\s']+)",
    )
    .unwrap()
});

static RE_HEAL_SPELL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"([A-Za-z]+(?:\s[A-Za-z]+)*(?:\s*\([^)]+\))?) 's ([A-Za-z\s']+) (?:heals|critically heals) ([A-Za-z\s']+(?:\s*\([^)]+\))?) for (\d+)",
    )
    .unwrap()
});

static RE_HEAL_GAIN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"([A-Za-z]+(?:\s[A-Za-z]+)*(?:\s*\([^)]+\))?) gains (\d+) health from ([A-Za-z]+(?:\s[A-Za-z]+)*(?:\s*\([^)]+\))?) 's ([A-Za-z\s']+)",
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

static RE_AFFLICTED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"([A-Za-z]+) is afflicted by ([A-Za-z\s':]+?)(?:\s+\((\d+)\))?\.").unwrap()
});

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
static RE_RESISTED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\((\d+) resisted\)").unwrap());
static RE_BLOCKED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\((\d+) blocked\)").unwrap());

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
    /// Per-combatant damage deficit for effective healing calculation.
    ///
    /// Every damage event increments the target's deficit; every heal is capped at
    /// `min(heal_amount, deficit)` to produce effective heal vs overheal.
    health_deficit: HashMap<String, u64>,
    /// Players who died during the current encounter (unique names).
    encounter_deaths: HashSet<String>,
    /// Players who participated in the current encounter (dealt/took damage).
    encounter_active: HashSet<String>,
}

/// Check if an `Option<String>` contains a known boss name.
fn has_known_boss(boss: Option<&String>) -> bool {
    boss.is_some_and(|b| parser::is_known_boss(b))
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
        health_deficit: HashMap::new(),
        encounter_deaths: HashSet::new(),
        encounter_active: HashSet::new(),
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
            parse_metadata(trimmed, &mut data, &mut state);
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
            parse_damage_events(trimmed, timestamp, &mut data, &mut state.health_deficit);
        }

        // Healing: "heals" or "health from"
        if trimmed.contains("heals ") || trimmed.contains(" health from ") {
            parse_healing_events(trimmed, timestamp, &mut data, &mut state.health_deficit);
        }

        // Boss detection from combat lines (during active combat only)
        // Skip if we already identified a known boss for this combat window
        if state.in_combat && !has_known_boss(state.current_boss.as_ref()) {
            detect_boss_from_combat(trimmed, &data, &mut state);
        }

        // Track active players for per-encounter scoreboard (wipe detection).
        // Check the last entry added — if source or target is a known combatant
        // (player), record them as active in this encounter.
        if state.in_combat {
            if let Some(entry) = data.entries.last() {
                let src = entry.source();
                if data.all_combatants.contains_key(src) {
                    state.encounter_active.insert(src.to_string());
                }
                if let Some(tgt) = entry.target() {
                    if data.all_combatants.contains_key(tgt) {
                        state.encounter_active.insert(tgt.to_string());
                    }
                }
            }
        }

        // Avoidance: dodge/parry/miss keywords
        if trimmed.contains(" dodges.")
            || trimmed.contains(" parries.")
            || trimmed.contains(" misses ")
        {
            parse_avoidance(trimmed, &mut data);
        }

        // Buff/debuff events: "gains " / " fades from " / "is afflicted by "
        if trimmed.contains(" gains ")
            || trimmed.contains(" fades from ")
            || trimmed.contains(" is afflicted by ")
        {
            parse_buff_events(trimmed, timestamp, &mut data);
        }

        // Consumable usage: " uses "
        if trimmed.contains(" uses ") {
            parse_consumable(trimmed, timestamp, &mut data);
        }
    }

    post_process(&mut data);
    data
}

// ── Line-Level Parse Helpers ────────────────────────────────────────────────

/// Parse `COMBATANT_INFO`, `ZONE_INFO`, and `PLAYERS_IN_COMBAT` lines.
fn parse_metadata(trimmed: &str, data: &mut LogData, state: &mut ParseState) {
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
            data.all_combatants.insert(name_str, combatant);
        } else if let Some((name, class)) = parser::extract_combatant(trimmed) {
            // Fallback: minimal extraction if full parse fails
            let name_str = name.to_string();
            let combatant = Combatant {
                class: class.to_string(),
                ..Combatant::default()
            };
            data.all_combatants.insert(name_str, combatant);
        }
    }

    if trimmed.contains("ZONE_INFO:") {
        if let Some(zone) = parser::extract_zone(trimmed) {
            let canonical = parser::normalize_zone_name(zone);
            data.zone_name.clone_from(&canonical);
            state.current_zone = Some(canonical);
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
        state.encounter_deaths.clear();
        state.encounter_active.clear();
    }

    if trimmed.contains("PLAYER_REGEN_ENABLED") {
        if state.in_combat {
            if let Some(start) = state.combat_start {
                let duration = timestamp - start;
                if duration > 5.0 {
                    let is_boss = has_known_boss(state.current_boss.as_ref());
                    #[allow(clippy::cast_possible_truncation)] // encounter won't have 4B players
                    let player_deaths = state.encounter_deaths.len() as u32;
                    #[allow(clippy::cast_possible_truncation)]
                    let active_players = state.encounter_active.len() as u32;
                    data.encounters.push(Encounter {
                        name: state.current_boss.clone(),
                        start,
                        end: timestamp,
                        duration,
                        is_boss,
                        is_kill: is_boss && state.current_boss_killed,
                        zone: state.current_zone.clone(),
                        attempt: None,
                        player_deaths,
                        active_players,
                    });
                }
                state.current_boss = None;
                state.current_boss_killed = false;
                state.encounter_deaths.clear();
                state.encounter_active.clear();
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
        // Track per-encounter player deaths for wipe detection
        if state.in_combat {
            state.encounter_deaths.insert(dead_unit.to_string());
        }
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
            let dominated = !has_known_boss(state.current_boss.as_ref())
                && state
                    .current_boss
                    .as_ref()
                    .is_none_or(|b| dead_unit.len() > b.len());
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
    if has_known_boss(state.current_boss.as_ref()) {
        return;
    }

    // Check the most recent entry (just pushed by parse_damage_events / parse_healing_events)
    if let Some(entry) = data.entries.last() {
        let names: &[&str] = match entry {
            LogEntry::Damage { source, target, .. } | LogEntry::Healing { source, target, .. } => {
                &[source.as_str(), target.as_str()]
            }
            _ => &[],
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
    }

    // Also check for boss names mentioned directly in the line (e.g. in resist/immune messages)
    if !has_known_boss(state.current_boss.as_ref()) {
        // Quick scan: only worth checking if line contains common combat verbs
        if trimmed.contains("hits ")
            || trimmed.contains("crits ")
            || trimmed.contains("suffers ")
            || trimmed.contains("resisted")
        {
            // Boss names from raid_data are lowercased; lowercase the line for matching.
            let line_lower = trimmed.to_lowercase();

            // Prefer zone-constrained boss list when we know the current zone.
            // This prevents false positives like "Knight" (Upper Karazhan) matching
            // "Necro Knight" combat lines in Naxxramas.
            let zone_bosses = parser::bosses_for_zone(state.current_zone.as_deref());
            let boss_list: &[&str] = zone_bosses.as_deref().unwrap_or(parser::known_boss_names());

            for boss in boss_list {
                if contains_word(&line_lower, boss) {
                    // Title-case for consistent display with other code paths
                    // that preserve original log casing (e.g. UNIT_DIED).
                    state.current_boss = Some(title_case(boss));
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

/// Extract resisted amount from the `(N resisted)` trailer on damage lines.
fn parse_resisted(trimmed: &str) -> u64 {
    RE_RESISTED
        .captures(trimmed)
        .and_then(|c| c.get(1)?.as_str().parse::<u64>().ok())
        .unwrap_or(0)
}

/// Extract blocked amount from the `(N blocked)` trailer on damage lines.
fn parse_blocked(trimmed: &str) -> u64 {
    RE_BLOCKED
        .captures(trimmed)
        .and_then(|c| c.get(1)?.as_str().parse::<u64>().ok())
        .unwrap_or(0)
}

/// Parsed mitigation trailer data from parenthesized suffixes on damage lines.
///
/// Extracts `(N absorbed)`, `(N resisted)`, `(N blocked)`, `(glancing)`, `(crushing)`
/// from lines like: `Boss hits Tank for 5000. (1500 resisted) (200 blocked) (glancing)`
struct TrailerData {
    absorbed: u64,
    resisted: u64,
    blocked: u64,
    is_glancing: bool,
    is_crushing: bool,
}

/// Parse all mitigation data from the trailer portion of a damage line.
fn parse_trailer(trimmed: &str) -> TrailerData {
    TrailerData {
        absorbed: parse_absorbed(trimmed),
        resisted: parse_resisted(trimmed),
        blocked: parse_blocked(trimmed),
        is_glancing: trimmed.contains("(glancing)"),
        is_crushing: trimmed.contains("(crushing)"),
    }
}

/// Extract the damage school word from a damage line.
///
/// Vanilla `WoW` damage lines include the school as `"for N School damage"`, e.g.:
/// `"Ragnaros hits Tank for 5000 Fire damage."` or `"Tank suffers 300 Nature damage"`.
/// Auto-attacks have no school word (just `"for N."`) and default to Physical.
fn parse_school(trimmed: &str) -> Option<String> {
    // Look for the pattern: digits followed by a school word followed by "damage"
    // Common schools: Physical, Holy, Fire, Nature, Frost, Shadow, Arcane
    static RE_SCHOOL: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\d+ (Physical|Holy|Fire|Nature|Frost|Shadow|Arcane) damage").unwrap()
    });
    RE_SCHOOL
        .captures(trimmed)
        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
}

/// Parsed damage event ready to be recorded into stats and entries.
struct DamageEvent<'a> {
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
    pet_owner: Option<&'a str>,
}

/// Record a parsed damage event into stats, entries, and health deficit.
fn record_damage(
    data: &mut LogData,
    health_deficit: &mut HashMap<String, u64>,
    event: DamageEvent<'_>,
) {
    let DamageEvent {
        timestamp,
        source,
        target,
        spell,
        amount,
        absorbed,
        resisted,
        blocked,
        is_crit,
        is_glancing,
        is_crushing,
        school,
        pet_owner,
    } = event;
    add_damage(data, &source, &spell, amount, pet_owner, is_crit);
    add_damage_taken(data, &target, amount);
    record_absorb(data, &target, absorbed);
    // Increment target's health deficit for effective healing calculation
    *health_deficit.entry(target.clone()).or_insert(0) += amount;
    data.entries.push(LogEntry::Damage {
        timestamp,
        source,
        target,
        spell,
        amount,
        absorbed,
        resisted,
        blocked,
        is_crit,
        is_glancing,
        is_crushing,
        school,
    });
}

/// Parse all 3 damage event formats.
#[allow(clippy::too_many_lines)] // Three format variants with trailer/school extraction each
fn parse_damage_events(
    trimmed: &str,
    timestamp: f64,
    data: &mut LogData,
    health_deficit: &mut HashMap<String, u64>,
) {
    let is_crit = trimmed.contains(" crits ") || trimmed.contains(" critically ");
    let trailer = parse_trailer(trimmed);
    let school = parse_school(trimmed);

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
        let pet_owner = extract_pet_owner(&source, data);
        record_damage(
            data,
            health_deficit,
            DamageEvent {
                timestamp,
                source,
                target,
                spell,
                amount,
                absorbed: trailer.absorbed,
                resisted: trailer.resisted,
                blocked: trailer.blocked,
                is_crit,
                is_glancing: trailer.is_glancing,
                is_crushing: trailer.is_crushing,
                school: school.clone(),
                pet_owner: pet_owner.as_deref(),
            },
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
        record_damage(
            data,
            health_deficit,
            DamageEvent {
                timestamp,
                source,
                target,
                spell: "Auto Attack".to_string(),
                amount,
                absorbed: trailer.absorbed,
                resisted: trailer.resisted,
                blocked: trailer.blocked,
                is_crit,
                is_glancing: trailer.is_glancing,
                is_crushing: trailer.is_crushing,
                school: school.clone(),
                pet_owner: None,
            },
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
        let pet_owner = extract_pet_owner(&source, data);
        record_damage(
            data,
            health_deficit,
            DamageEvent {
                timestamp,
                source,
                target,
                spell,
                amount,
                absorbed: trailer.absorbed,
                resisted: trailer.resisted,
                blocked: trailer.blocked,
                is_crit,
                is_glancing: trailer.is_glancing,
                is_crushing: trailer.is_crushing,
                school,
                pet_owner: pet_owner.as_deref(),
            },
        );
    }
}

/// Compute effective healing from the target's health deficit.
///
/// Returns `(effective_heal, overheal)`. The effective amount is capped at the
/// target's accumulated damage deficit; the remainder is overheal.
fn compute_effective_heal(
    health_deficit: &mut HashMap<String, u64>,
    target: &str,
    amount: u64,
) -> (u64, u64) {
    let deficit = health_deficit.entry(target.to_string()).or_insert(0);
    if amount > *deficit {
        let effective = *deficit;
        *deficit = 0;
        (effective, amount - effective)
    } else {
        *deficit -= amount;
        (amount, 0)
    }
}

/// Parse both healing event formats.
///
/// Only updates `player_stats` (via `add_healing`) when the heal target is a
/// known player (`all_combatants`).  Boss-targeted heals (Shadow of Ebonroc,
/// Blood Siphon, etc.) are still stored as `LogEntry::Healing` for the timeline
/// but excluded from the player's healing totals.
fn parse_healing_events(
    trimmed: &str,
    timestamp: f64,
    data: &mut LogData,
    health_deficit: &mut HashMap<String, u64>,
) {
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

        let (effective_heal, overheal) = compute_effective_heal(health_deficit, &target, amount);
        // Only credit player stats when the target is a known player.
        // Boss/NPC/pet targets are still recorded as LogEntry::Healing for the
        // timeline sparkline but excluded from the healer's stats.
        if data.all_combatants.contains_key(target.as_str()) {
            add_healing(
                data,
                &source,
                &spell,
                amount,
                effective_heal,
                overheal,
                is_heal_crit,
            );
        }
        data.entries.push(LogEntry::Healing {
            timestamp,
            source,
            target,
            spell,
            amount,
            effective_heal,
            overheal,
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

        let (effective_heal, overheal) = compute_effective_heal(health_deficit, &target, amount);
        if data.all_combatants.contains_key(target.as_str()) {
            add_healing(
                data,
                &source,
                &spell,
                amount,
                effective_heal,
                overheal,
                false,
            );
        }
        data.entries.push(LogEntry::Healing {
            timestamp,
            source,
            target,
            spell,
            amount,
            effective_heal,
            overheal,
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
                    let stacks: u32 = caps
                        .get(3)
                        .and_then(|m| m.as_str().parse().ok())
                        .unwrap_or(1);
                    let buffs = data.buffs.entry(player.to_string()).or_default();
                    let stats = buffs.entry(buff.clone()).or_default();
                    stats.gains += 1;
                    if stats.first_gain.is_none() {
                        stats.first_gain = Some(timestamp);
                    }
                    data.entries.push(LogEntry::AuraGain {
                        timestamp,
                        player: player.to_string(),
                        aura: buff,
                        stacks,
                    });
                }
            }
        }
        return;
    }

    if let Some(caps) = RE_BUFF_FADE.captures(trimmed) {
        if let Some(buff) = caps.get(1).map(|m| m.as_str().trim().to_string()) {
            if let Some(player) = caps.get(2).map(|m| m.as_str()) {
                if data.all_combatants.contains_key(player) {
                    let buffs = data.buffs.entry(player.to_string()).or_default();
                    let stats = buffs.entry(buff.clone()).or_default();
                    stats.fades += 1;
                    stats.last_fade = Some(timestamp);
                    data.entries.push(LogEntry::AuraFade {
                        timestamp,
                        player: player.to_string(),
                        aura: buff,
                    });
                }
            }
        }
        return;
    }

    // "is afflicted by" — debuff application with optional stack count
    if let Some(caps) = RE_AFFLICTED.captures(trimmed) {
        if let Some(player) = caps.get(1).map(|m| m.as_str()) {
            if let Some(debuff) = caps.get(2).map(|m| m.as_str().trim().to_string()) {
                if data.all_combatants.contains_key(player) {
                    let stacks: u32 = caps
                        .get(3)
                        .and_then(|m| m.as_str().parse().ok())
                        .unwrap_or(1);
                    let buffs = data.buffs.entry(player.to_string()).or_default();
                    let stats = buffs.entry(debuff.clone()).or_default();
                    stats.gains += 1;
                    if stats.first_gain.is_none() {
                        stats.first_gain = Some(timestamp);
                    }
                    data.entries.push(LogEntry::AuraGain {
                        timestamp,
                        player: player.to_string(),
                        aura: debuff,
                        stacks,
                    });
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

            let quality = ItemQuality::from_color_code(color_code);

            data.loot.push(LootEvent {
                timestamp,
                player,
                item_name,
                item_id,
                quality,
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

/// Check whether `name` appears in `line` as a whole word, not as a substring
/// of a longer word (e.g. "Garr" should match "Garr hits" but not "Garrote").
///
/// Uses ASCII-aware boundary checks — characters adjacent to the match must be
/// non-alphanumeric (or absent) for the match to count. Apostrophes and spaces
/// are valid boundaries, so `"Garr 's Magma Shackles"` still matches.
fn contains_word(line: &str, name: &str) -> bool {
    let bytes = line.as_bytes();
    for (idx, _) in line.match_indices(name) {
        let before_ok = idx == 0 || !bytes[idx - 1].is_ascii_alphanumeric();
        let after_pos = idx + name.len();
        let after_ok = after_pos >= bytes.len() || !bytes[after_pos].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
    }
    false
}

/// Check if a GUID represents a player (starts with `0x0000000000`).
fn is_player_guid(guid: &str) -> bool {
    guid.starts_with("0x0000000000")
}

/// Title-case a name (e.g. `"hakkar"` → `"Hakkar"`, `"high priest thekal"` → `"High Priest Thekal"`).
fn title_case(s: &str) -> String {
    s.split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |c| {
                let upper: String = c.to_uppercase().collect();
                format!("{upper}{}", chars.as_str())
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
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
fn add_healing(
    data: &mut LogData,
    source: &str,
    spell: &str,
    amount: u64,
    effective: u64,
    overheal: u64,
    is_crit: bool,
) {
    let stats = ensure_stats(data, source);
    stats.healing += amount;
    stats.effective_healing += effective;
    stats.overhealing += overheal;
    let ab = stats
        .healing_abilities
        .entry(spell.to_string())
        .or_default();
    ab.total += amount;
    ab.effective += effective;
    ab.overheal += overheal;
    ab.hits += 1;
    if is_crit {
        ab.crits += 1;
    }
}

/// Post-process: assign bosses to loot, link trades, number encounter attempts,
/// and build filtered combatant roster.
fn post_process(data: &mut LogData) {
    // Merge fragmented boss encounters.
    // Boss abilities like Hakkar's Mind Control cause brief combat drops
    // (PLAYER_REGEN_ENABLED → PLAYER_REGEN_DISABLED) mid-fight, creating
    // spurious short encounters.  Merge consecutive boss encounters for the
    // same boss when the gap is < 60 seconds.
    merge_boss_encounters(&mut data.encounters);

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

    // Downgrade combat-drop encounters to non-boss.
    // Boss abilities (e.g. Hakkar's Mind Control) can cause brief combat drops
    // where nobody dies.  These are not real wipe attempts — they're just
    // the boss resetting or a phase transition.  A real wipe requires at
    // least ~50% of active players to die (accounting for DI/soulstone/ankh).
    for enc in &mut data.encounters {
        if enc.is_boss && !enc.is_kill && enc.active_players > 0 {
            #[allow(clippy::cast_precision_loss)] // small player counts
            let death_ratio = f64::from(enc.player_deaths) / f64::from(enc.active_players);
            if death_ratio < WIPE_DEATH_THRESHOLD {
                // Not enough deaths to be a real wipe — demote to trash
                enc.is_boss = false;
            }
        }
    }

    // Number encounter attempts (case-insensitive boss name matching)
    let mut boss_attempts: HashMap<String, usize> = HashMap::new();
    for enc in &mut data.encounters {
        if enc.is_boss {
            if let Some(name) = &enc.name {
                let key = name.to_lowercase();
                let count = boss_attempts.entry(key).or_insert(0);
                *count += 1;
                enc.attempt = Some(*count);
            }
        }
    }

    // Build filtered combatants from encounter participation.
    // COMBATANT_INFO fires for ALL nearby players (cities, open world), not just
    // raid/party members.  A time-window heuristic pulls in too many bystanders
    // (e.g. 276 "players" in a 40-man raid).  Instead, require actual combat
    // participation: the player must appear in player_stats (dealt/took damage
    // or healed during an encounter).
    for (name, combatant) in &data.all_combatants {
        if data.player_stats.contains_key(name) {
            data.combatants.insert(name.clone(), combatant.clone());
        }
    }
}

/// Maximum gap (seconds) between consecutive boss encounters to be merged.
///
/// Boss abilities like Mind Control can cause brief combat drops mid-fight.
/// 60 seconds covers all known cases while avoiding merging genuinely separate
/// attempts (wipe recovery + re-pull typically takes > 60 seconds).
const ENCOUNTER_MERGE_GAP: f64 = 60.0;

/// Minimum death ratio to consider a non-kill boss encounter a real wipe.
///
/// If fewer than this fraction of active players died, the encounter is
/// treated as a combat drop / boss reset rather than a genuine wipe attempt.
/// Set to 0.5 (50%) to account for Divine Intervention, soulstone, and ankh
/// survivors — a real wipe will kill most of the raid even with 1-3 survivors.
const WIPE_DEATH_THRESHOLD: f64 = 0.5;

/// Merge consecutive boss encounters for the same boss that are close together.
///
/// When a boss ability (e.g. Hakkar's Mind Control) causes a brief combat drop,
/// the parse loop creates multiple short encounters for what is actually a single
/// fight.  This function collapses those fragments into one encounter spanning
/// from the first fragment's start to the last fragment's end.
fn merge_boss_encounters(encounters: &mut Vec<Encounter>) {
    if encounters.len() < 2 {
        return;
    }

    let mut merged: Vec<Encounter> = Vec::with_capacity(encounters.len());

    for enc in encounters.drain(..) {
        let should_merge = enc.is_boss
            && enc.name.is_some()
            && merged.last().is_some_and(|prev: &Encounter| {
                prev.is_boss
                    && prev.name.is_some()
                    && same_boss_name(prev.name.as_deref(), enc.name.as_deref())
                    && (enc.start - prev.end) < ENCOUNTER_MERGE_GAP
            });

        if should_merge {
            // Merge into the previous encounter
            let prev = merged
                .last_mut()
                .expect("should_merge guarantees a previous element");
            prev.end = enc.end;
            prev.duration = prev.end - prev.start;
            // Kill trumps wipe: if any fragment is a kill, the merged encounter is a kill
            if enc.is_kill {
                prev.is_kill = true;
            }
            // Accumulate death/active counts across merged segments
            prev.player_deaths += enc.player_deaths;
            prev.active_players = prev.active_players.max(enc.active_players);
            // Prefer the title-cased name (from UNIT_DIED) over lowercase
            if let Some(name) = &enc.name {
                if name.chars().next().is_some_and(char::is_uppercase) {
                    prev.name = Some(name.clone());
                }
            }
        } else {
            merged.push(enc);
        }
    }

    *encounters = merged;
}

/// Case-insensitive boss name comparison.
fn same_boss_name(a: Option<&str>, b: Option<&str>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        _ => false,
    }
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
        // Simulate a wipe: boss is fought, player dies, boss does not die
        let lines: Vec<String> = vec![
            "1/27 12:23:41.000  COMBATANT_INFO: 27.01.26 12:23:41&Acedica&PALADIN&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            "1/27 20:45:19.000  PLAYER_REGEN_DISABLED".to_string(),
            "1/27 20:45:21.000  Acedica 's Holy Shield hits Chromaggus for 125 Holy damage.".to_string(),
            "1/27 20:45:22.000  Chromaggus hits Acedica for 1245.".to_string(),
            "1/27 20:45:23.000  Acedica hits Chromaggus for 113.".to_string(),
            // Player dies — real wipe
            "1/27 20:48:12.000  UNIT_DIED:Acedica:0x0000000000000001".to_string(),
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

    #[test]
    fn test_contains_word_boundaries() {
        // Exact match at various positions
        assert!(contains_word("Garr hits Tank for 500.", "Garr"));
        assert!(contains_word("Tank hits Garr for 500.", "Garr"));
        assert!(contains_word("Garr", "Garr"));

        // Apostrophe is a valid word boundary (possessive form)
        assert!(contains_word(
            "Garr 's Magma Shackles hits Tank for 100.",
            "Garr"
        ));

        // Embedded in a longer word — must NOT match
        assert!(!contains_word(
            "Scamilla 's Garrote hits Ancient Core Hound for 166.",
            "Garr"
        ));
        assert!(!contains_word(
            "Ancient Core Hound suffers 166 Physical damage from Scamilla 's Garrote.",
            "Garr"
        ));

        // Multi-word boss names
        assert!(contains_word(
            "Baron Geddon hits Tank for 3000 Fire damage.",
            "Baron Geddon"
        ));
        assert!(contains_word(
            "Tank suffers 500 Fire damage from Sulfuron Harbinger 's Shadow Word: Pain.",
            "Sulfuron Harbinger"
        ));
    }

    #[test]
    fn test_garrote_does_not_create_garr_encounter() {
        // A rogue using Garrote on trash should NOT create a "Garr" boss encounter.
        // This is a regression test for the substring false-positive bug.
        let lines: Vec<String> = vec![
            "1/27 12:23:41.000  COMBATANT_INFO: 27.01.26 12:23:41&Rogue&ROGUE&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            "1/27 19:33:50.000  PLAYER_REGEN_DISABLED".to_string(),
            "1/27 19:33:51.000  Rogue 's Garrote hits Ancient Core Hound for 166.".to_string(),
            "1/27 19:33:54.000  Ancient Core Hound suffers 166 Physical damage from Rogue 's Garrote.".to_string(),
            "1/27 19:33:57.000  Ancient Core Hound suffers 166 Physical damage from Rogue 's Garrote.".to_string(),
            "1/27 19:34:01.000  UNIT_DIED:Ancient Core Hound:0xF130002F1900AA11".to_string(),
            "1/27 19:34:02.000  PLAYER_REGEN_ENABLED".to_string(),
        ];
        let data = parse_log(&lines);
        assert_eq!(data.encounters.len(), 1, "Should have 1 encounter");
        let enc = &data.encounters[0];
        assert_eq!(
            enc.name.as_deref(),
            Some("Ancient Core Hound"),
            "Encounter should be named after the mob, not Garr"
        );
        assert!(
            !enc.is_boss,
            "Trash mob encounter should not be marked as boss"
        );
        assert!(!enc.is_kill, "Trash mob kill should not be a boss kill");
    }

    #[test]
    fn test_garr_substring_scan_detects_real_garr() {
        // When Garr actually appears in a resist/immune line (substring scan path),
        // it should still be detected as the boss.
        let lines: Vec<String> = vec![
            "1/27 12:23:41.000  COMBATANT_INFO: 27.01.26 12:23:41&Tank&WARRIOR&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            "1/27 19:39:51.000  PLAYER_REGEN_DISABLED".to_string(),
            "1/27 19:39:52.000  Garr 's Immolate was resisted by Tank.".to_string(),
            // Wipe — no UNIT_DIED for boss
            "1/27 19:42:55.000  PLAYER_REGEN_ENABLED".to_string(),
        ];
        let data = parse_log(&lines);
        assert_eq!(data.encounters.len(), 1, "Should have 1 encounter");
        let enc = &data.encounters[0];
        assert_eq!(
            enc.name.as_deref(),
            Some("Garr"),
            "Should detect Garr from resist line (title-cased for consistent display)"
        );
        assert!(enc.is_boss, "Garr should be marked as boss");
        assert!(!enc.is_kill, "Wipe should not be marked as kill");
    }

    // ── Encounter Merging Tests ────────────────────────────────────

    #[test]
    fn test_boss_encounter_merge_mc_combat_drop() {
        // Simulate Hakkar's Mind Control causing a brief combat drop mid-fight.
        // Three combat windows within 60s of each other should merge into one encounter.
        let lines: Vec<String> = vec![
            "1/27 12:23:41.000  COMBATANT_INFO: 27.01.26 12:23:41&Tank&WARRIOR&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            // Segment 1: initial pull
            "1/27 20:00:00.000  PLAYER_REGEN_DISABLED".to_string(),
            "1/27 20:00:05.000  Hakkar 's Blood Bolt hits Tank for 800.".to_string(),
            "1/27 20:00:39.000  PLAYER_REGEN_ENABLED".to_string(),
            // MC causes brief drop — gap < 60s
            // Segment 2: combat resumes after MC
            "1/27 20:00:54.000  PLAYER_REGEN_DISABLED".to_string(),
            "1/27 20:01:10.000  Hakkar 's Blood Bolt hits Tank for 900.".to_string(),
            "1/27 20:01:37.000  PLAYER_REGEN_ENABLED".to_string(),
            // Another MC — gap < 60s
            // Segment 3: final segment with boss kill
            "1/27 20:01:52.000  PLAYER_REGEN_DISABLED".to_string(),
            "1/27 20:02:20.000  Hakkar 's Blood Bolt hits Tank for 700.".to_string(),
            "1/27 20:02:23.000  UNIT_DIED:Hakkar:0xF130003A6C000001".to_string(),
            "1/27 20:02:23.000  PLAYER_REGEN_ENABLED".to_string(),
        ];
        let data = parse_log(&lines);
        assert_eq!(
            data.encounters.len(),
            1,
            "Three short combat windows should merge into 1 encounter"
        );
        let enc = &data.encounters[0];
        assert_eq!(enc.name.as_deref(), Some("Hakkar"));
        assert!(enc.is_boss, "Should be a boss encounter");
        assert!(enc.is_kill, "Should be a kill (boss died in segment 3)");
        // Duration spans from first segment start to last segment end
        assert!(
            enc.duration > 120.0,
            "Merged duration should span all three segments"
        );
    }

    #[test]
    fn test_boss_encounters_not_merged_when_far_apart() {
        // Two boss encounters > 60s apart should NOT merge (separate pulls).
        let lines: Vec<String> = vec![
            "1/27 12:23:41.000  COMBATANT_INFO: 27.01.26 12:23:41&Tank&WARRIOR&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            // Attempt 1 (wipe)
            "1/27 20:00:00.000  PLAYER_REGEN_DISABLED".to_string(),
            "1/27 20:00:05.000  Hakkar 's Blood Bolt hits Tank for 800.".to_string(),
            "1/27 20:02:00.000  PLAYER_REGEN_ENABLED".to_string(),
            // 2 minute gap (> 60s) — separate pull
            // Attempt 2 (kill)
            "1/27 20:04:00.000  PLAYER_REGEN_DISABLED".to_string(),
            "1/27 20:04:05.000  Hakkar 's Blood Bolt hits Tank for 900.".to_string(),
            "1/27 20:05:23.000  UNIT_DIED:Hakkar:0xF130003A6C000001".to_string(),
            "1/27 20:05:23.000  PLAYER_REGEN_ENABLED".to_string(),
        ];
        let data = parse_log(&lines);
        assert_eq!(
            data.encounters.len(),
            2,
            "Encounters > 60s apart should stay separate"
        );
        assert!(!data.encounters[0].is_kill, "First should be a wipe");
        assert!(data.encounters[1].is_kill, "Second should be a kill");
    }

    // ── Effective Healing Tests ─────────────────────────────────────

    #[test]
    fn test_effective_healing_basic() {
        // Warrior takes 3000 damage, then receives 2800 heal (fully effective)
        // followed by another 2800 heal (only 200 effective, 2600 overheal)
        let lines: Vec<String> = vec![
            "1/27 12:23:41.000  COMBATANT_INFO: 27.01.26 12:23:41&Priest&PRIEST&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            "1/27 12:23:41.000  COMBATANT_INFO: 27.01.26 12:23:41&Warrior&WARRIOR&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000002&nil".to_string(),
            "1/27 12:24:00.000  PLAYER_REGEN_DISABLED".to_string(),
            "1/27 12:24:01.000  Boss hits Warrior for 3000.".to_string(),
            "1/27 12:24:02.000  Priest 's Greater Heal heals Warrior for 2800.".to_string(),
            "1/27 12:24:03.000  Priest 's Greater Heal heals Warrior for 2800.".to_string(),
            "1/27 12:35:00.000  PLAYER_REGEN_ENABLED".to_string(),
        ];
        let data = parse_log(&lines);

        let stats = data.player_stats.get("Priest").expect("Priest stats");
        assert_eq!(stats.healing, 5600, "Raw healing should be 5600");
        assert_eq!(
            stats.effective_healing, 3000,
            "Effective healing = damage taken = 3000"
        );
        assert_eq!(stats.overhealing, 2600, "Overheal = 5600 - 3000 = 2600");

        // Check individual entries
        let heals: Vec<_> = data
            .entries
            .iter()
            .filter_map(|e| {
                if let LogEntry::Healing {
                    effective_heal,
                    overheal,
                    ..
                } = e
                {
                    Some((*effective_heal, *overheal))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(heals.len(), 2);
        assert_eq!(heals[0], (2800, 0), "First heal: fully effective");
        assert_eq!(
            heals[1],
            (200, 2600),
            "Second heal: 200 effective, 2600 overheal"
        );
    }

    #[test]
    fn test_effective_healing_no_damage() {
        // Heal with no prior damage = 100% overheal
        let lines: Vec<String> = vec![
            "1/27 12:23:41.000  COMBATANT_INFO: 27.01.26 12:23:41&Priest&PRIEST&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            "1/27 12:23:41.000  COMBATANT_INFO: 27.01.26 12:23:41&Warrior&WARRIOR&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000002&nil".to_string(),
            "1/27 12:24:00.000  PLAYER_REGEN_DISABLED".to_string(),
            "1/27 12:24:01.000  Priest 's Greater Heal heals Warrior for 2800.".to_string(),
            "1/27 12:35:00.000  PLAYER_REGEN_ENABLED".to_string(),
        ];
        let data = parse_log(&lines);

        let stats = data.player_stats.get("Priest").expect("Priest stats");
        assert_eq!(stats.healing, 2800);
        assert_eq!(stats.effective_healing, 0, "No damage taken = 0 effective");
        assert_eq!(stats.overhealing, 2800, "All healing is overheal");
    }

    #[test]
    fn test_effective_healing_hot_ticks() {
        // HoT ticks (gains N health from) should also track effective heal
        let lines: Vec<String> = vec![
            "1/27 12:23:41.000  COMBATANT_INFO: 27.01.26 12:23:41&Druid&DRUID&NightElf&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            "1/27 12:23:41.000  COMBATANT_INFO: 27.01.26 12:23:41&Warrior&WARRIOR&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000002&nil".to_string(),
            "1/27 12:24:00.000  PLAYER_REGEN_DISABLED".to_string(),
            "1/27 12:24:01.000  Boss hits Warrior for 500.".to_string(),
            "1/27 12:24:02.000  Warrior gains 148 health from Druid 's Rejuvenation.".to_string(),
            "1/27 12:24:04.000  Warrior gains 148 health from Druid 's Rejuvenation.".to_string(),
            "1/27 12:24:06.000  Warrior gains 148 health from Druid 's Rejuvenation.".to_string(),
            "1/27 12:24:08.000  Warrior gains 148 health from Druid 's Rejuvenation.".to_string(),
            "1/27 12:35:00.000  PLAYER_REGEN_ENABLED".to_string(),
        ];
        let data = parse_log(&lines);

        let stats = data.player_stats.get("Druid").expect("Druid stats");
        assert_eq!(stats.healing, 148 * 4, "Raw healing = 4 ticks of 148");
        // 500 damage deficit, first 3 ticks: 148 + 148 + 148 = 444 (deficit down to 56)
        // 4th tick: 148 but only 56 deficit remains → 56 effective, 92 overheal
        assert_eq!(
            stats.effective_healing, 500,
            "Effective = total damage taken"
        );
        assert_eq!(
            stats.overhealing,
            148 * 4 - 500,
            "Overheal = raw - effective"
        );
    }

    // ── Damage Mitigation Tests ─────────────────────────────────────

    #[test]
    fn test_parse_trailer_resisted() {
        let line = "Boss 's Shadow Bolt hits Warrior for 2000 Shadow damage. (500 resisted)";
        let trailer = parse_trailer(line);
        assert_eq!(trailer.resisted, 500);
        assert_eq!(trailer.absorbed, 0);
        assert_eq!(trailer.blocked, 0);
        assert!(!trailer.is_glancing);
        assert!(!trailer.is_crushing);
    }

    #[test]
    fn test_parse_trailer_multiple() {
        let line =
            "Boss hits Warrior for 3000. (800 resisted) (200 blocked) (100 absorbed) (crushing)";
        let trailer = parse_trailer(line);
        assert_eq!(trailer.resisted, 800);
        assert_eq!(trailer.blocked, 200);
        assert_eq!(trailer.absorbed, 100);
        assert!(!trailer.is_glancing);
        assert!(trailer.is_crushing);
    }

    #[test]
    fn test_parse_trailer_glancing() {
        let line = "Boss hits Warrior for 1500. (glancing)";
        let trailer = parse_trailer(line);
        assert!(trailer.is_glancing);
        assert!(!trailer.is_crushing);
        assert_eq!(trailer.resisted, 0);
    }

    #[test]
    fn test_damage_mitigation_integration() {
        // Full integration: damage line with resisted + absorbed in a parse_log context
        let lines: Vec<String> = vec![
            "1/27 12:23:41.000  COMBATANT_INFO: 27.01.26 12:23:41&Tank&WARRIOR&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            "1/27 12:24:00.000  PLAYER_REGEN_DISABLED".to_string(),
            "1/27 12:24:01.000  Boss 's Shadow Bolt hits Tank for 2000 Shadow damage. (500 resisted) (100 absorbed)".to_string(),
            "1/27 12:35:00.000  PLAYER_REGEN_ENABLED".to_string(),
        ];
        let data = parse_log(&lines);

        let dmg_entry = data
            .entries
            .iter()
            .find(|e| matches!(e, LogEntry::Damage { .. }));
        assert!(dmg_entry.is_some(), "Should have a damage entry");
        if let Some(LogEntry::Damage {
            resisted,
            absorbed,
            school,
            ..
        }) = dmg_entry
        {
            assert_eq!(*resisted, 500, "Should parse 500 resisted");
            assert_eq!(*absorbed, 100, "Should parse 100 absorbed");
            assert_eq!(
                school.as_deref(),
                Some("Shadow"),
                "Should parse Shadow school"
            );
        }
    }

    // ── Damage School Tests ─────────────────────────────────────────

    #[test]
    fn test_parse_school_fire() {
        let line = "Boss hits Tank for 5000 Fire damage.";
        assert_eq!(parse_school(line).as_deref(), Some("Fire"));
    }

    #[test]
    fn test_parse_school_none() {
        // Auto-attack: no school word
        let line = "Boss hits Tank for 500.";
        assert_eq!(parse_school(line), None);
    }

    #[test]
    fn test_parse_school_all_types() {
        for school in &[
            "Physical", "Holy", "Fire", "Nature", "Frost", "Shadow", "Arcane",
        ] {
            let line = format!("Boss hits Tank for 100 {school} damage.");
            assert_eq!(
                parse_school(&line).as_deref(),
                Some(*school),
                "Should parse {school}"
            );
        }
    }

    #[test]
    fn test_damage_school_suffer_format() {
        // "suffers N School damage from" format
        let lines: Vec<String> = vec![
            "1/27 12:23:41.000  COMBATANT_INFO: 27.01.26 12:23:41&Tank&WARRIOR&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            "1/27 12:24:00.000  PLAYER_REGEN_DISABLED".to_string(),
            "1/27 12:24:01.000  Tank suffers 300 Nature damage from Boss 's Poison Bolt.".to_string(),
            "1/27 12:35:00.000  PLAYER_REGEN_ENABLED".to_string(),
        ];
        let data = parse_log(&lines);

        let dmg_entry = data
            .entries
            .iter()
            .find(|e| matches!(e, LogEntry::Damage { .. }));
        assert!(dmg_entry.is_some());
        if let Some(LogEntry::Damage { school, .. }) = dmg_entry {
            assert_eq!(
                school.as_deref(),
                Some("Nature"),
                "Should parse Nature from suffer line"
            );
        }
    }

    // ── Aura Event Tests ────────────────────────────────────────────

    #[test]
    fn test_regex_afflicted() {
        // Without stack count
        let line = "Acedica is afflicted by War Stomp.";
        let caps = RE_AFFLICTED.captures(line).expect("should match");
        assert_eq!(caps.get(1).unwrap().as_str(), "Acedica");
        assert_eq!(caps.get(2).unwrap().as_str().trim(), "War Stomp");
        assert!(caps.get(3).is_none(), "No stack count");

        // With stack count (the common addon format)
        let line2 = "Tinjarro is afflicted by Arcane Overload (1).";
        let caps2 = RE_AFFLICTED
            .captures(line2)
            .expect("should match with stack count");
        assert_eq!(caps2.get(1).unwrap().as_str(), "Tinjarro");
        assert_eq!(caps2.get(2).unwrap().as_str().trim(), "Arcane Overload");
        assert_eq!(caps2.get(3).unwrap().as_str(), "1");

        // Higher stack count
        let line3 = "Tank is afflicted by Sunder Armor (5).";
        let caps3 = RE_AFFLICTED.captures(line3).expect("should match stacks");
        assert_eq!(caps3.get(2).unwrap().as_str().trim(), "Sunder Armor");
        assert_eq!(caps3.get(3).unwrap().as_str(), "5");

        // Debuff with colon
        let line4 = "Tank is afflicted by Shadow Word: Pain.";
        let caps4 = RE_AFFLICTED
            .captures(line4)
            .expect("should match debuff with colon");
        assert_eq!(caps4.get(1).unwrap().as_str(), "Tank");
        assert_eq!(caps4.get(2).unwrap().as_str().trim(), "Shadow Word: Pain");
    }

    #[test]
    fn test_aura_entries_emitted() {
        // Buff gains, debuff afflictions, and fades should all produce LogEntry events
        let lines: Vec<String> = vec![
            "1/27 12:23:41.000  COMBATANT_INFO: 27.01.26 12:23:41&Acedica&PALADIN&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            "1/27 12:24:00.000  PLAYER_REGEN_DISABLED".to_string(),
            "1/27 12:24:01.000  Acedica gains Power Word: Fortitude (1).".to_string(),
            "1/27 12:24:05.000  Acedica is afflicted by Arcane Overload.".to_string(),
            "1/27 12:24:10.000  Power Word: Fortitude fades from Acedica.".to_string(),
            "1/27 12:24:12.000  Arcane Overload fades from Acedica.".to_string(),
            "1/27 12:35:00.000  PLAYER_REGEN_ENABLED".to_string(),
        ];
        let data = parse_log(&lines);

        let aura_gains: Vec<_> = data
            .entries
            .iter()
            .filter(|e| matches!(e, LogEntry::AuraGain { .. }))
            .collect();
        let aura_fades: Vec<_> = data
            .entries
            .iter()
            .filter(|e| matches!(e, LogEntry::AuraFade { .. }))
            .collect();

        assert_eq!(
            aura_gains.len(),
            2,
            "Should have 2 AuraGain entries (buff + afflicted)"
        );
        assert_eq!(aura_fades.len(), 2, "Should have 2 AuraFade entries");

        // Verify the afflicted entry
        if let LogEntry::AuraGain {
            player,
            aura,
            stacks,
            ..
        } = &aura_gains[1]
        {
            assert_eq!(player, "Acedica");
            assert_eq!(aura, "Arcane Overload");
            assert_eq!(*stacks, 1);
        } else {
            panic!("Expected AuraGain");
        }
    }

    #[test]
    fn test_aura_intervals_built() {
        // Build timeline and verify aura intervals are correctly paired
        let lines: Vec<String> = vec![
            "1/27 12:23:41.000  COMBATANT_INFO: 27.01.26 12:23:41&Tank&WARRIOR&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            "1/27 19:39:51.000  PLAYER_REGEN_DISABLED".to_string(),
            "1/27 19:39:52.000  Tank gains Power Word: Fortitude (1).".to_string(),
            "1/27 19:39:55.000  Tank is afflicted by Arcane Overload.".to_string(),
            "1/27 19:39:58.000  Arcane Overload fades from Tank.".to_string(),
            "1/27 19:40:02.000  Power Word: Fortitude fades from Tank.".to_string(),
            "1/27 19:41:00.000  PLAYER_REGEN_ENABLED".to_string(),
        ];
        let data = parse_log(&lines);

        let filter = crate::log_data::EncounterFilter::All;
        let timeline = data.build_timeline(&filter, 5000);

        // Should have both auras available
        assert!(
            timeline
                .available_auras
                .contains(&"Arcane Overload".to_string()),
            "Arcane Overload should be in available auras"
        );
        assert!(
            timeline
                .available_auras
                .contains(&"Power Word: Fortitude".to_string()),
            "PW:F should be in available auras"
        );

        // Verify Arcane Overload interval: gain at +4s, fade at +7s
        let ao_intervals = timeline
            .aura_intervals
            .get("Arcane Overload")
            .expect("Should have Arcane Overload intervals");
        assert_eq!(
            ao_intervals.len(),
            1,
            "Should have 1 Arcane Overload interval"
        );
        assert_eq!(ao_intervals[0].player, "Tank");
        // Timestamps: enc_start = 19:39:51, gain = 19:39:55 (+4s), fade = 19:39:58 (+7s)
        assert!(
            (ao_intervals[0].start - 4.0).abs() < 0.1,
            "Start should be ~4s"
        );
        assert!((ao_intervals[0].end - 7.0).abs() < 0.1, "End should be ~7s");

        // Verify PW:F interval: gain at +1s, fade at +11s
        let pwf_intervals = timeline
            .aura_intervals
            .get("Power Word: Fortitude")
            .expect("Should have PW:F intervals");
        assert_eq!(pwf_intervals.len(), 1);
        assert!(
            (pwf_intervals[0].start - 1.0).abs() < 0.1,
            "PW:F start should be ~1s"
        );
        assert!(
            (pwf_intervals[0].end - 11.0).abs() < 0.1,
            "PW:F end should be ~11s"
        );
    }

    #[test]
    fn test_aura_interval_unclosed_clamped() {
        // Aura gained but never faded — should be clamped to encounter end
        let lines: Vec<String> = vec![
            "1/27 12:23:41.000  COMBATANT_INFO: 27.01.26 12:23:41&Tank&WARRIOR&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            "1/27 19:39:51.000  PLAYER_REGEN_DISABLED".to_string(),
            "1/27 19:39:55.000  Tank is afflicted by Burning Adrenaline.".to_string(),
            // No fade — boss kill or player death
            "1/27 19:40:21.000  PLAYER_REGEN_ENABLED".to_string(),
        ];
        let data = parse_log(&lines);

        let filter = crate::log_data::EncounterFilter::All;
        let timeline = data.build_timeline(&filter, 5000);

        let ba_intervals = timeline
            .aura_intervals
            .get("Burning Adrenaline")
            .expect("Should have Burning Adrenaline intervals");
        assert_eq!(ba_intervals.len(), 1);
        assert_eq!(ba_intervals[0].player, "Tank");
        // Gained at +4s, never faded — should be clamped to encounter duration (~30s)
        assert!((ba_intervals[0].start - 4.0).abs() < 0.1);
        assert!(
            ba_intervals[0].end > 25.0,
            "Unclosed aura end should be clamped to encounter end"
        );
    }
}
