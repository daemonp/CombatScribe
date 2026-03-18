//! Combat state tracking, boss detection, and `UNIT_DIED` handling.

use crate::log_data::{Combatant, DeathEvent, Encounter, LogData, LogEntry};
use crate::parser;

use super::ParseState;
use super::helpers::{contains_word, is_player_guid, title_case};
use super::post_process::{parse_gear_slot, parse_players_in_combat};

// ── Combat State & Boss Detection ───────────────────────────────────────────

/// Check if an `Option<String>` contains a known boss name.
pub(super) fn has_known_boss(boss: Option<&String>) -> bool {
    boss.is_some_and(|b| parser::is_known_boss(b))
}

/// Parse `COMBATANT_INFO`, `ZONE_INFO`, and `PLAYERS_IN_COMBAT` lines.
pub(super) fn parse_metadata(trimmed: &str, data: &mut LogData, state: &mut ParseState) {
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

    if trimmed.contains("ZONE_INFO:")
        && let Some(zone) = parser::extract_zone(trimmed)
    {
        let canonical = parser::normalize_zone_name(zone);
        data.zone_name.clone_from(&canonical);
        state.current_zone = Some(canonical);
    }

    // PLAYERS_IN_COMBAT: 32/40
    if trimmed.contains("PLAYERS_IN_COMBAT:")
        && let Some(pic) = parse_players_in_combat(trimmed)
    {
        data.raid_size = Some(pic);
    }
}

/// Track combat start/end and create encounter records.
pub(super) fn parse_combat_state(
    trimmed: &str,
    timestamp: f64,
    data: &mut LogData,
    state: &mut ParseState,
) {
    if trimmed.contains("PLAYER_REGEN_DISABLED") {
        state.in_combat = true;
        state.combat_start = Some(timestamp);
        state.encounter_deaths.clear();
        state.encounter_active.clear();
    }

    if trimmed.contains("PLAYER_REGEN_ENABLED") {
        if state.in_combat
            && let Some(start) = state.combat_start
        {
            let duration = timestamp - start;
            if duration > 5.0 {
                let is_boss = has_known_boss(state.current_boss.as_ref());
                #[allow(clippy::cast_possible_truncation)] // encounter won't have 4B players
                let player_deaths = state.encounter_deaths.len() as u32;
                #[allow(clippy::cast_possible_truncation)] // encounter won't have 4B players
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
            // Clear last damage to prevent cross-encounter attribution
            state.last_damage.clear();
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
pub(super) const KILL_GRACE_SECS: f64 = 600.0;

/// Handle `UNIT_DIED` — player deaths and boss/mob detection.
pub(super) fn parse_unit_died(
    trimmed: &str,
    timestamp: f64,
    data: &mut LogData,
    state: &mut ParseState,
) {
    if !trimmed.contains("UNIT_DIED:") {
        return;
    }

    let Some((dead_unit, guid)) = parser::extract_unit_died_with_guid(trimmed) else {
        return;
    };

    if is_player_guid(guid) {
        // Attribute death to last damage received
        let (killer, killing_blow, damage_amount) =
            if let Some(last_dmg) = state.last_damage.get(dead_unit) {
                (
                    Some(last_dmg.source.clone()),
                    Some(last_dmg.spell.clone()),
                    Some(last_dmg.amount),
                )
            } else {
                (None, None, None)
            };

        data.deaths.push(DeathEvent {
            timestamp,
            player: dead_unit.to_string(),
            killer,
            killing_blow,
            damage_amount,
        });
        data.entries.push(LogEntry::Death {
            timestamp,
            player: dead_unit.to_string(),
        });

        // Clear last damage to prevent reuse
        state.last_damage.remove(dead_unit);

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
            // Extend the encounter window to include the boss death.
            // The logging player may have died (dropping combat) well before
            // the boss actually died, so the original `enc.end` can be far
            // too early.  Events between the old end and the boss death are
            // part of the same fight.
            if death_timestamp > enc.end {
                enc.end = death_timestamp;
                enc.duration = enc.end - enc.start;
            }
            return;
        }
    }
}

/// Detect boss names from damage/healing lines during combat.
///
/// On wipe attempts, the boss never dies so `UNIT_DIED` won't fire.
/// Instead, check source and target names in the last-recorded damage entry
/// against the known boss list. A known boss always takes priority.
pub(super) fn detect_boss_from_combat(trimmed: &str, data: &LogData, state: &mut ParseState) {
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

#[cfg(test)]
mod tests {
    use crate::log_parser::parse_log;

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
        // Encounter end should be extended to boss death, not PLAYER_REGEN_ENABLED
        assert!(
            enc.duration > 70.0,
            "Duration should extend to boss death (~77s), got {:.1}s",
            enc.duration
        );
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

    // ── Death Attribution Tests ────────────────────────────────────────

    #[test]
    fn test_death_attribution_basic() {
        let lines = vec![
            "1/27 10:00:00.000  COMBATANT_INFO: 27.01.26 10:00:00&Tank&WARRIOR&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            "1/27 10:00:10.000  PLAYER_REGEN_DISABLED".to_string(),
            "1/27 10:00:15.000  Patchwerk 's Hateful Strike hits Tank for 9500.".to_string(),
            "1/27 10:00:16.000  Tank dies.".to_string(),
            "1/27 10:00:16.000  UNIT_DIED:Tank:0x0000000000000001".to_string(),
            "1/27 10:00:20.000  PLAYER_REGEN_ENABLED".to_string(),
        ];
        let data = parse_log(&lines);

        assert_eq!(data.deaths.len(), 1);
        let death = &data.deaths[0];
        assert_eq!(death.player, "Tank");
        assert_eq!(death.killer, Some("Patchwerk".to_string()));
        assert_eq!(death.killing_blow, Some("Hateful Strike".to_string()));
        assert_eq!(death.damage_amount, Some(9500));
    }

    #[test]
    fn test_death_attribution_no_damage() {
        // Environmental death or disconnect - no prior damage
        let lines = vec![
            "1/27 10:00:00.000  COMBATANT_INFO: 27.01.26 10:00:00&Tank&WARRIOR&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            "1/27 10:00:10.000  PLAYER_REGEN_DISABLED".to_string(),
            "1/27 10:00:15.000  Tank dies.".to_string(),
            "1/27 10:00:15.000  UNIT_DIED:Tank:0x0000000000000001".to_string(),
        ];
        let data = parse_log(&lines);

        assert_eq!(data.deaths.len(), 1);
        let death = &data.deaths[0];
        assert_eq!(death.player, "Tank");
        assert_eq!(death.killer, None);
        assert_eq!(death.killing_blow, None);
        assert_eq!(death.damage_amount, None);
    }

    #[test]
    fn test_death_attribution_cleared_on_encounter_end() {
        // Damage in encounter 1 should not attribute deaths in encounter 2
        let lines = vec![
            "1/27 10:00:00.000  COMBATANT_INFO: 27.01.26 10:00:00&Tank&WARRIOR&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            // Encounter 1
            "1/27 10:00:10.000  PLAYER_REGEN_DISABLED".to_string(),
            "1/27 10:00:15.000  Patchwerk hits Tank for 100.".to_string(),
            "1/27 10:00:20.000  PLAYER_REGEN_ENABLED".to_string(),
            // Encounter 2 - Tank dies with no damage in this encounter
            "1/27 10:05:00.000  PLAYER_REGEN_DISABLED".to_string(),
            "1/27 10:05:10.000  Tank dies.".to_string(),
            "1/27 10:05:10.000  UNIT_DIED:Tank:0x0000000000000001".to_string(),
            "1/27 10:05:15.000  PLAYER_REGEN_ENABLED".to_string(),
        ];
        let data = parse_log(&lines);

        assert_eq!(data.deaths.len(), 1);
        let death = &data.deaths[0];
        assert_eq!(death.player, "Tank");
        // Should NOT be attributed to Patchwerk from encounter 1
        assert_eq!(death.killer, None);
        assert_eq!(death.killing_blow, None);
    }

    #[test]
    fn test_death_attribution_dot() {
        // Death from DoT (periodic damage)
        let lines = vec![
            "1/27 10:00:00.000  COMBATANT_INFO: 27.01.26 10:00:00&Tank&PRIEST&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000002&nil".to_string(),
            "1/27 10:00:10.000  PLAYER_REGEN_DISABLED".to_string(),
            "1/27 10:00:12.000  Tank suffers 1200 Shadow damage from Hakkar 's Corrupted Blood.".to_string(),
            "1/27 10:00:13.000  Tank dies.".to_string(),
            "1/27 10:00:13.000  UNIT_DIED:Tank:0x0000000000000002".to_string(),
            "1/27 10:00:20.000  PLAYER_REGEN_ENABLED".to_string(),
        ];
        let data = parse_log(&lines);

        assert_eq!(data.deaths.len(), 1);
        let death = &data.deaths[0];
        assert_eq!(death.player, "Tank");
        assert_eq!(death.killer, Some("Hakkar".to_string()));
        assert_eq!(death.killing_blow, Some("Corrupted Blood".to_string()));
        assert_eq!(death.damage_amount, Some(1200));
    }

    #[test]
    fn test_death_attribution_auto_attack() {
        // Death from auto attack
        let lines = vec![
            "1/27 10:00:00.000  COMBATANT_INFO: 27.01.26 10:00:00&Tank&WARRIOR&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            "1/27 10:00:10.000  PLAYER_REGEN_DISABLED".to_string(),
            "1/27 10:00:15.000  Patchwerk hits Tank for 3500.".to_string(),
            "1/27 10:00:16.000  Tank dies.".to_string(),
            "1/27 10:00:16.000  UNIT_DIED:Tank:0x0000000000000001".to_string(),
            "1/27 10:00:20.000  PLAYER_REGEN_ENABLED".to_string(),
        ];
        let data = parse_log(&lines);

        assert_eq!(data.deaths.len(), 1);
        let death = &data.deaths[0];
        assert_eq!(death.player, "Tank");
        assert_eq!(death.killer, Some("Patchwerk".to_string()));
        assert_eq!(death.killing_blow, Some("Auto Attack".to_string()));
        assert_eq!(death.damage_amount, Some(3500));
    }
}
