//! Full combat log parser — port of `app.js` `parseLog()`.
//!
//! Operates on raw lines from an already-selected session, producing
//! a fully populated [`LogData`] structure.

mod combat;
mod damage;
mod events;
mod healing;
mod helpers;
mod post_process;
mod regex;

use std::collections::{HashMap, HashSet};

use crate::log_data::LogData;
use crate::parser;

use combat::{
    detect_boss_from_combat, has_known_boss, parse_combat_state, parse_metadata, parse_unit_died,
};
use damage::parse_damage_events;
use events::{
    parse_avoidance, parse_buff_events, parse_cast_events, parse_consumable, parse_loot_trade,
    parse_pet_ownership,
};
use healing::parse_healing_events;
use post_process::post_process;
use regex::RE_CAST_RANK;

// ── Spell Rank Extraction ───────────────────────────────────────────────────

/// Extract spell rank from addon `CAST:` lines and store in the rank map.
///
/// Parses lines like `CAST: Druid casts Regrowth(8910)(Rank 4) on Warrior.`
/// and records `spell_ranks["Druid"]["Regrowth"] = 4`.
fn extract_spell_rank(
    trimmed: &str,
    spell_ranks: &mut HashMap<String, HashMap<String, u8>>,
) {
    if let Some(caps) = RE_CAST_RANK.captures(trimmed) {
        let caster = caps.get(1).map_or("", |m| m.as_str());
        let spell = caps.get(2).map_or("", |m| m.as_str().trim());
        let rank: u8 = caps
            .get(3)
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(0);
        if !caster.is_empty() && !spell.is_empty() && rank > 0 {
            spell_ranks
                .entry(caster.to_string())
                .or_default()
                .insert(spell.to_string(), rank);
        }
    }
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Last damage information for death attribution.
#[derive(Debug, Clone)]
pub(super) struct LastDamageInfo {
    source: String,
    spell: String,
    amount: u64,
}

/// Mutable state tracked across the main parse loop.
pub(super) struct ParseState {
    pub(super) in_combat: bool,
    pub(super) combat_start: Option<f64>,
    pub(super) current_boss: Option<String>,
    pub(super) current_boss_killed: bool,
    pub(super) current_zone: Option<String>,
    /// Per-combatant damage deficit for effective healing calculation.
    ///
    /// Every damage event increments the target's deficit; every heal is capped at
    /// `min(heal_amount, deficit)` to produce effective heal vs overheal.
    pub(super) health_deficit: HashMap<String, u64>,
    /// Players who died during the current encounter (unique names).
    pub(super) encounter_deaths: HashSet<String>,
    /// Players who participated in the current encounter (dealt/took damage).
    pub(super) encounter_active: HashSet<String>,
    /// Last damage received by each player (for death attribution).
    ///
    /// Maps target name → last damage info. Cleared on encounter boundaries
    /// to prevent cross-encounter attribution.
    pub(super) last_damage: HashMap<String, LastDamageInfo>,
    /// Last-used spell rank per (caster, spell) from addon `CAST:` lines.
    ///
    /// Used to suffix healing spell names with rank (e.g. "Regrowth (Rank 4)").
    /// Only populated when `CAST:` lines with `(Rank N)` are present (addon format).
    pub(super) spell_ranks: HashMap<String, HashMap<String, u8>>,
}

/// Parse the session lines into a fully populated `LogData`.
///
/// Uses keyword-based dispatch to avoid running all regexes on every line.
/// Most lines are damage/healing events; metadata/loot/buff lines are rare.
#[allow(clippy::too_many_lines)] // keyword-dispatch loop — linear sequence of checks
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
        last_damage: HashMap::new(),
        spell_ranks: HashMap::new(),
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

        // Spell rank extraction from addon CAST: lines (e.g. "Regrowth(8910)(Rank 4)")
        if trimmed.contains("CAST:") && trimmed.contains("Rank ") {
            extract_spell_rank(trimmed, &mut state.spell_ranks);
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
            parse_damage_events(
                trimmed,
                timestamp,
                &mut data,
                &mut state.health_deficit,
                &mut state.last_damage,
            );
        }

        // Healing: "heals" or "health from"
        if trimmed.contains("heals ") || trimmed.contains(" health from ") {
            parse_healing_events(
                trimmed,
                timestamp,
                &mut data,
                &mut state.health_deficit,
                &state.spell_ranks,
            );
        }

        // Boss detection from combat lines (during active combat only)
        // Skip if we already identified a known boss for this combat window
        if state.in_combat && !has_known_boss(state.current_boss.as_deref()) {
            detect_boss_from_combat(trimmed, &data, &mut state);
        }

        // Track active players for per-encounter scoreboard (wipe detection).
        // Check the last entry added — if source or target is a known combatant
        // (player), record them as active in this encounter.
        if state.in_combat
            && let Some(entry) = data.entries.last()
        {
            let src = entry.source();
            if data.all_combatants.contains_key(src) {
                state.encounter_active.insert(src.to_string());
            }
            if let Some(tgt) = entry.target()
                && data.all_combatants.contains_key(tgt)
            {
                state.encounter_active.insert(tgt.to_string());
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

#[cfg(test)]
mod tests {
    use super::*;

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
