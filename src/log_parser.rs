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
    LazyLock::new(|| Regex::new(r"([A-Za-z]+) gains ([A-Za-z\s']+) \((\d+)\)\.").unwrap());

static RE_BUFF_FADE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([A-Za-z\s']+) fades from ([A-Za-z]+)\.").unwrap());

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
/// This is the Rust port of `app.js` `parseLog()`.
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

        parse_metadata(trimmed, timestamp, &mut data, &mut state);
        parse_combat_state(trimmed, timestamp, &mut data, &mut state);
        parse_unit_died(trimmed, timestamp, &mut data, &mut state);
        parse_cast_events(trimmed, timestamp, &mut data);
        parse_pet_ownership(trimmed, &mut data);

        let absorbed_amount = RE_ABSORB
            .captures(trimmed)
            .and_then(|c| c.get(1)?.as_str().parse::<u64>().ok())
            .unwrap_or(0);
        let is_crit = trimmed.contains(" crits ") || trimmed.contains(" critically ");

        parse_damage_events(trimmed, timestamp, &mut data, is_crit, absorbed_amount);
        parse_healing_events(trimmed, timestamp, &mut data);
        parse_avoidance(trimmed, &mut data);
        parse_buff_events(trimmed, timestamp, &mut data);
        parse_loot_trade(trimmed, timestamp, &mut data);
        parse_consumable(trimmed, timestamp, &mut data);
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
    } else if !data.combatants.contains_key(dead_unit) && dead_unit != "Unknown" {
        if parser::is_known_boss(dead_unit) {
            state.current_boss = Some(dead_unit.to_string());
            state.current_boss_killed = true;
        } else if state
            .current_boss
            .as_ref()
            .is_none_or(|b| dead_unit.len() > b.len())
        {
            state.current_boss = Some(dead_unit.to_string());
        }
    }
}

/// Parse dispels, resurrects, and interrupts from cast events.
fn parse_cast_events(trimmed: &str, timestamp: f64, data: &mut LogData) {
    // Dispels
    for dispel_spell in DISPEL_SPELLS {
        if trimmed.contains(&format!("casts {dispel_spell}")) {
            if let Some(caps) = RE_CAST.captures(trimmed) {
                let caster = caps.get(1).unwrap().as_str().to_string();
                let spell = caps.get(2).unwrap().as_str().trim().to_string();
                let target = caps.get(3).unwrap().as_str().trim().to_string();
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
            return;
        }
    }

    // Resurrects
    for res_spell in RESURRECT_SPELLS {
        if trimmed.contains(&format!("casts {res_spell}"))
            && !trimmed.contains("begins to cast")
            && !trimmed.contains("fails casting")
        {
            if let Some(caps) = RE_CAST.captures(trimmed) {
                let caster = caps.get(1).unwrap().as_str().to_string();
                let spell = caps.get(2).unwrap().as_str().trim().to_string();
                let target = caps.get(3).unwrap().as_str().trim().to_string();
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
            return;
        }
    }

    // Interrupts
    for int_spell in INTERRUPT_SPELLS {
        if trimmed.contains(&format!("casts {int_spell}")) && !trimmed.contains("begins to cast") {
            if let Some(caps) = RE_CAST.captures(trimmed) {
                let caster = caps.get(1).unwrap().as_str().to_string();
                let spell = caps.get(2).unwrap().as_str().trim().to_string();
                let target = caps.get(3).unwrap().as_str().trim().to_string();

                if data.combatants.contains_key(&caster) && !data.combatants.contains_key(&target) {
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
            return;
        }
    }
}

/// Detect pet ownership from `PetName (OwnerName)` patterns.
fn parse_pet_ownership(trimmed: &str, data: &mut LogData) {
    if let Some(caps) = RE_PET_OWNER.captures(trimmed) {
        let pet_name = caps.get(1).unwrap().as_str();
        let owner_name = caps.get(2).unwrap().as_str();
        if data.combatants.contains_key(owner_name) && !data.combatants.contains_key(pet_name) {
            data.pet_owners
                .insert(pet_name.to_string(), owner_name.to_string());
        }
    }
}

/// Parse all 3 damage event formats.
fn parse_damage_events(
    trimmed: &str,
    timestamp: f64,
    data: &mut LogData,
    is_crit: bool,
    absorbed_amount: u64,
) {
    // Format 1: Source 's Spell hits/crits Target for N
    if let Some(caps) = RE_DMG_SPELL.captures(trimmed) {
        let source = caps.get(1).unwrap().as_str().to_string();
        let spell = caps.get(2).unwrap().as_str().trim().to_string();
        let target = caps.get(3).unwrap().as_str().trim().to_string();
        let amount: u64 = caps.get(4).unwrap().as_str().parse().unwrap_or(0);

        let pet_owner = extract_pet_owner(&source, data);
        add_damage(data, &source, &spell, amount, pet_owner.as_deref(), is_crit);
        add_damage_taken(data, &target, amount);
        record_absorb(data, &target, absorbed_amount);

        data.entries.push(LogEntry::Damage {
            timestamp,
            source,
            target,
            spell,
            amount,
            absorbed: absorbed_amount,
            is_crit,
        });
        return;
    }

    // Format 2: Source hits/crits Target for N.
    if let Some(caps) = RE_DMG_AUTO.captures(trimmed) {
        let source = caps.get(1).unwrap().as_str().to_string();
        let target = caps.get(2).unwrap().as_str().trim().to_string();
        let amount: u64 = caps.get(3).unwrap().as_str().parse().unwrap_or(0);

        add_damage(data, &source, "Auto Attack", amount, None, is_crit);
        add_damage_taken(data, &target, amount);
        record_absorb(data, &target, absorbed_amount);

        data.entries.push(LogEntry::Damage {
            timestamp,
            source,
            target,
            spell: "Auto Attack".to_string(),
            amount,
            absorbed: absorbed_amount,
            is_crit,
        });
        return;
    }

    // Format 3: Target suffers N damage from Source 's Spell
    if let Some(caps) = RE_DMG_SUFFER.captures(trimmed) {
        let target = caps.get(1).unwrap().as_str().trim().to_string();
        let amount: u64 = caps.get(2).unwrap().as_str().parse().unwrap_or(0);
        let source = caps.get(3).unwrap().as_str().to_string();
        let spell = caps.get(4).unwrap().as_str().trim().to_string();

        let pet_owner = extract_pet_owner(&source, data);
        add_damage(data, &source, &spell, amount, pet_owner.as_deref(), is_crit);
        add_damage_taken(data, &target, amount);
        record_absorb(data, &target, absorbed_amount);

        data.entries.push(LogEntry::Damage {
            timestamp,
            source,
            target,
            spell,
            amount,
            absorbed: absorbed_amount,
            is_crit,
        });
    }
}

/// Parse both healing event formats.
fn parse_healing_events(trimmed: &str, timestamp: f64, data: &mut LogData) {
    let is_heal_crit = trimmed.contains("critically heals");

    // Format 1: Source 's Spell heals/critically heals Target for N
    if let Some(caps) = RE_HEAL_SPELL.captures(trimmed) {
        let source = caps.get(1).unwrap().as_str().to_string();
        let mut spell = caps.get(2).unwrap().as_str().trim().to_string();
        let target = caps.get(3).unwrap().as_str().trim().to_string();
        let amount: u64 = caps.get(4).unwrap().as_str().parse().unwrap_or(0);

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
        let target = caps.get(1).unwrap().as_str().to_string();
        let amount: u64 = caps.get(2).unwrap().as_str().parse().unwrap_or(0);
        let source = caps.get(3).unwrap().as_str().to_string();
        let spell = caps.get(4).unwrap().as_str().trim().to_string();

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
fn parse_avoidance(trimmed: &str, data: &mut LogData) {
    if let Some(caps) = RE_DODGE.captures(trimmed) {
        let defender = caps.get(2).unwrap().as_str();
        if data.combatants.contains_key(defender) {
            data.avoidance
                .entry(defender.to_string())
                .or_default()
                .dodges += 1;
        }
    }

    if let Some(caps) = RE_PARRY.captures(trimmed) {
        let defender = caps.get(2).unwrap().as_str();
        if data.combatants.contains_key(defender) {
            data.avoidance
                .entry(defender.to_string())
                .or_default()
                .parries += 1;
        }
    }

    if let Some(caps) = RE_MISS.captures(trimmed) {
        let attacker = caps.get(1).unwrap().as_str().trim();
        let defender = caps.get(2).unwrap().as_str();

        if data.combatants.contains_key(defender) && !data.combatants.contains_key(attacker) {
            data.avoidance
                .entry(defender.to_string())
                .or_default()
                .missed_by += 1;
        } else if data.combatants.contains_key(attacker) && !data.combatants.contains_key(defender)
        {
            data.avoidance
                .entry(attacker.to_string())
                .or_default()
                .misses += 1;
        }
    }
}

/// Parse buff gain and fade events.
fn parse_buff_events(trimmed: &str, timestamp: f64, data: &mut LogData) {
    if let Some(caps) = RE_BUFF_GAIN.captures(trimmed) {
        let player = caps.get(1).unwrap().as_str();
        let buff = caps.get(2).unwrap().as_str().to_string();

        if data.combatants.contains_key(player) {
            let buffs = data.buffs.entry(player.to_string()).or_default();
            let stats = buffs.entry(buff).or_default();
            stats.gains += 1;
            if stats.first_gain.is_none() {
                stats.first_gain = Some(timestamp);
            }
        }
    }

    if let Some(caps) = RE_BUFF_FADE.captures(trimmed) {
        let buff = caps.get(1).unwrap().as_str().to_string();
        let player = caps.get(2).unwrap().as_str();

        if data.combatants.contains_key(player) {
            let buffs = data.buffs.entry(player.to_string()).or_default();
            let stats = buffs.entry(buff).or_default();
            stats.fades += 1;
            stats.last_fade = Some(timestamp);
        }
    }
}

/// Parse loot and trade events.
fn parse_loot_trade(trimmed: &str, timestamp: f64, data: &mut LogData) {
    if trimmed.contains("LOOT:") {
        if let Some(caps) = RE_LOOT.captures(trimmed) {
            let player = caps.get(1).unwrap().as_str().to_string();
            let color_code = caps.get(2).unwrap().as_str();
            let item_id: u64 = caps.get(3).unwrap().as_str().parse().unwrap_or(0);
            let item_name = caps.get(4).unwrap().as_str().to_string();
            let quantity: u64 = caps.get(5).unwrap().as_str().parse().unwrap_or(1);

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
            let from_player = caps.get(1).unwrap().as_str().to_string();
            let item_name = caps.get(2).unwrap().as_str().trim().to_string();
            let to_player = caps.get(3).unwrap().as_str().to_string();

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
    if absorbed_amount > 0 && data.combatants.contains_key(target) {
        *data.absorbs.entry(target.to_string()).or_insert(0) += absorbed_amount;
    }
}

/// Extract pet owner from a source string like `PetName (OwnerName)`.
fn extract_pet_owner(source: &str, data: &LogData) -> Option<String> {
    if let Some(caps) = RE_PET_OWNER.captures(source) {
        let _pet_name = caps.get(1).unwrap().as_str();
        let owner_name = caps.get(2).unwrap().as_str();
        if data.combatants.contains_key(owner_name) {
            return Some(owner_name.to_string());
        }
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
        let player = caps.get(1).unwrap().as_str().to_string();
        let consumable = caps.get(2).unwrap().as_str().trim().to_string();

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
    let parts: Vec<&str> = raw.split(':').collect();
    let item_id = parts.first()?.parse::<u32>().ok()?;
    if item_id == 0 {
        return None;
    }
    let enchant_id = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let suffix_id = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
    Some(GearSlot {
        item_id,
        enchant_id,
        suffix_id,
        raw: raw.to_string(),
    })
}
