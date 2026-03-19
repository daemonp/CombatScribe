//! Healing event parsing: direct heals, `HoT` gains, and effective heal calculation.

use std::collections::HashMap;

use crate::log_data::{LogData, LogEntry};

use super::helpers::ensure_stats;
use super::regex::{RE_HEAL_GAIN, RE_HEAL_SPELL};

// ── Healing Parsing ─────────────────────────────────────────────────────────

/// Compute effective healing from the target's health deficit.
///
/// Returns `(effective_heal, overheal)`. The effective amount is capped at the
/// target's accumulated damage deficit; the remainder is overheal.
pub(super) fn compute_effective_heal(
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
pub(super) fn parse_healing_events(
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

    // Format 2: Target gains N health from Source 's Spell (periodic/HoT)
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
        let Some(raw_spell) = caps.get(4).map(|m| m.as_str().trim().to_string()) else {
            return;
        };
        // "gains health from" lines are periodic (HoT) ticks — distinguish from direct heals
        let spell = format!("{raw_spell} (hot)");

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
    ensure_stats(data, source).accumulate_healing(spell, amount, effective, overheal, is_crit);
}

#[cfg(test)]
mod tests {
    use super::super::regex::*;
    use super::*;
    use crate::log_parser::parse_log;

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

    // ── HoT Suffix Tests ───────────────────────────────────────────────

    #[test]
    fn test_hot_suffix_on_gain_line() {
        // "gains health from" lines should produce spell names with " (hot)" suffix
        let lines: Vec<String> = vec![
            "1/27 12:23:41.000  COMBATANT_INFO: 27.01.26 12:23:41&Druid&DRUID&NightElf&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            "1/27 12:23:41.000  COMBATANT_INFO: 27.01.26 12:23:41&Warrior&WARRIOR&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000002&nil".to_string(),
            "1/27 12:24:00.000  PLAYER_REGEN_DISABLED".to_string(),
            "1/27 12:24:02.000  Warrior gains 148 health from Druid 's Rejuvenation.".to_string(),
            "1/27 12:35:00.000  PLAYER_REGEN_ENABLED".to_string(),
        ];
        let data = parse_log(&lines);

        let heal = data
            .entries
            .iter()
            .find(|e| matches!(e, LogEntry::Healing { .. }));
        assert!(heal.is_some(), "Should have a healing entry");
        if let Some(LogEntry::Healing { spell, amount, .. }) = heal {
            assert_eq!(
                spell, "Rejuvenation (hot)",
                "Gain line should produce (hot) suffix"
            );
            assert_eq!(*amount, 148);
        }
    }

    #[test]
    fn test_direct_heal_no_hot_suffix() {
        // Direct "heals" lines should NOT have the (hot) suffix
        let lines: Vec<String> = vec![
            "1/27 12:23:41.000  COMBATANT_INFO: 27.01.26 12:23:41&Priest&PRIEST&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            "1/27 12:23:41.000  COMBATANT_INFO: 27.01.26 12:23:41&Warrior&WARRIOR&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000002&nil".to_string(),
            "1/27 12:24:00.000  PLAYER_REGEN_DISABLED".to_string(),
            "1/27 12:24:01.000  Priest 's Flash Heal heals Warrior for 2800.".to_string(),
            "1/27 12:35:00.000  PLAYER_REGEN_ENABLED".to_string(),
        ];
        let data = parse_log(&lines);

        let heal = data
            .entries
            .iter()
            .find(|e| matches!(e, LogEntry::Healing { .. }));
        assert!(heal.is_some(), "Should have a healing entry");
        if let Some(LogEntry::Healing { spell, .. }) = heal {
            assert_eq!(
                spell, "Flash Heal",
                "Direct heal should NOT have (hot) suffix"
            );
        }
    }

    #[test]
    fn test_hot_and_direct_separate_abilities() {
        // Regrowth has both a direct heal and HoT component — they should be
        // separate entries in healing_abilities
        let lines: Vec<String> = vec![
            "1/27 12:23:41.000  COMBATANT_INFO: 27.01.26 12:23:41&Druid&DRUID&NightElf&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            "1/27 12:23:41.000  COMBATANT_INFO: 27.01.26 12:23:41&Warrior&WARRIOR&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000002&nil".to_string(),
            "1/27 12:24:00.000  PLAYER_REGEN_DISABLED".to_string(),
            "1/27 12:24:00.500  Boss hits Warrior for 5000.".to_string(),
            "1/27 12:24:01.000  Druid 's Regrowth heals Warrior for 800.".to_string(),
            "1/27 12:24:03.000  Warrior gains 100 health from Druid 's Regrowth.".to_string(),
            "1/27 12:24:05.000  Warrior gains 100 health from Druid 's Regrowth.".to_string(),
            "1/27 12:24:07.000  Warrior gains 100 health from Druid 's Regrowth.".to_string(),
            "1/27 12:35:00.000  PLAYER_REGEN_ENABLED".to_string(),
        ];
        let data = parse_log(&lines);

        let druid = data
            .player_stats
            .get("Druid")
            .expect("Druid should have stats");
        assert_eq!(druid.healing, 1100, "Total raw healing = 800 + 3*100");

        let direct = druid.healing_abilities.get("Regrowth");
        assert!(direct.is_some(), "Should have 'Regrowth' for direct heal");
        assert_eq!(direct.unwrap().total, 800);
        assert_eq!(direct.unwrap().hits, 1);

        let hot = druid.healing_abilities.get("Regrowth (hot)");
        assert!(hot.is_some(), "Should have 'Regrowth (hot)' for HoT ticks");
        assert_eq!(hot.unwrap().total, 300);
        assert_eq!(hot.unwrap().hits, 3);
    }
}
