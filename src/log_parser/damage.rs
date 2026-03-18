use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;

use crate::log_data::{AbilityStats, LogData, LogEntry};

use super::helpers::{ensure_stats, extract_pet_owner, record_absorb};
use super::regex::{RE_ABSORB, RE_BLOCKED, RE_DMG_AUTO, RE_DMG_SPELL, RE_DMG_SUFFER, RE_RESISTED};

// ── Damage Parsing ──────────────────────────────────────────────────────────

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
pub(super) struct TrailerData {
    pub(super) absorbed: u64,
    pub(super) resisted: u64,
    pub(super) blocked: u64,
    pub(super) is_glancing: bool,
    pub(super) is_crushing: bool,
}

/// Parse all mitigation data from the trailer portion of a damage line.
pub(super) fn parse_trailer(trimmed: &str) -> TrailerData {
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
pub(super) fn parse_school(trimmed: &str) -> Option<String> {
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
pub(super) struct DamageEvent<'a> {
    pub(super) timestamp: f64,
    pub(super) source: String,
    pub(super) target: String,
    pub(super) spell: String,
    pub(super) amount: u64,
    pub(super) absorbed: u64,
    pub(super) resisted: u64,
    pub(super) blocked: u64,
    pub(super) is_crit: bool,
    pub(super) is_glancing: bool,
    pub(super) is_crushing: bool,
    pub(super) school: Option<String>,
    pub(super) pet_owner: Option<&'a str>,
}

/// Record a parsed damage event into stats, entries, and health deficit.
pub(super) fn record_damage(
    data: &mut LogData,
    health_deficit: &mut HashMap<String, u64>,
    event: DamageEvent<'_>,
) {
    // Record damage taken breakdown before destructuring the event.
    add_damage_taken(data, &event);

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
pub(super) fn parse_damage_events(
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

/// Add damage taken to a target player's stats, including per-source per-ability
/// breakdown with mitigation details.
fn add_damage_taken(data: &mut LogData, event: &DamageEvent<'_>) {
    let stats = ensure_stats(data, &event.target);
    stats.damage_taken += event.amount;
    let dt_ab = stats
        .damage_taken_breakdown
        .entry(event.source.clone())
        .or_default()
        .entry(event.spell.clone())
        .or_default();
    dt_ab.total += event.amount;
    dt_ab.hits += 1;
    dt_ab.absorbed += event.absorbed;
    dt_ab.resisted += event.resisted;
    dt_ab.blocked += event.blocked;
    if event.is_crit {
        dt_ab.crits += 1;
    }
    if event.is_crushing {
        dt_ab.crushing_hits += 1;
    }
    if event.is_glancing {
        dt_ab.glancing_hits += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::super::regex::*;
    use super::*;
    use crate::log_parser::parse_log;

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

    #[test]
    fn test_damage_taken_breakdown_populated() {
        let lines: Vec<String> = vec![
            "1/27 12:23:41.000  COMBATANT_INFO: 27.01.26 12:23:41&Tank&WARRIOR&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            "1/27 12:24:00.000  PLAYER_REGEN_DISABLED".to_string(),
            "1/27 12:24:01.000  Patchwerk 's Hateful Strike hits Tank for 6000. (200 blocked)".to_string(),
            "1/27 12:24:02.000  Patchwerk 's Hateful Strike crits Tank for 9000.".to_string(),
            "1/27 12:24:03.000  Patchwerk hits Tank for 3000. (500 absorbed)".to_string(),
            "1/27 12:24:04.000  Boss 's Shadow Bolt hits Tank for 2000 Shadow damage. (300 resisted) (100 absorbed)".to_string(),
            "1/27 12:35:00.000  PLAYER_REGEN_ENABLED".to_string(),
        ];
        let data = parse_log(&lines);

        let tank = data
            .player_stats
            .get("Tank")
            .expect("Tank should have stats");
        assert_eq!(
            tank.damage_taken,
            6000 + 9000 + 3000 + 2000,
            "Total damage taken should be sum of all hits"
        );

        // Check Patchwerk's abilities
        let pw = tank
            .damage_taken_breakdown
            .get("Patchwerk")
            .expect("Should have Patchwerk as source");

        let hs = pw
            .get("Hateful Strike")
            .expect("Should have Hateful Strike");
        assert_eq!(hs.total, 15000, "Hateful Strike total: 6000+9000");
        assert_eq!(hs.hits, 2, "Two Hateful Strike hits");
        assert_eq!(hs.crits, 1, "One Hateful Strike crit");
        assert_eq!(hs.blocked, 200, "200 blocked on first hit");

        let melee = pw.get("Auto Attack").expect("Should have Auto Attack");
        assert_eq!(melee.total, 3000);
        assert_eq!(melee.absorbed, 500);

        // Check Boss's Shadow Bolt
        let boss = tank
            .damage_taken_breakdown
            .get("Boss")
            .expect("Should have Boss as source");
        let sb = boss.get("Shadow Bolt").expect("Should have Shadow Bolt");
        assert_eq!(sb.total, 2000);
        assert_eq!(sb.resisted, 300);
        assert_eq!(sb.absorbed, 100);

        // Verify filtered_stats also populates the breakdown
        let filter = crate::log_data::EncounterFilter::Single(0);
        let (filtered, _) = data.filtered_stats(&filter);
        let ft = filtered.get("Tank").expect("Filtered Tank stats");
        assert!(
            !ft.damage_taken_breakdown.is_empty(),
            "Filtered stats should populate damage_taken_breakdown"
        );
        let fpw = ft
            .damage_taken_breakdown
            .get("Patchwerk")
            .expect("Filtered should have Patchwerk");
        let fhs = fpw
            .get("Hateful Strike")
            .expect("Filtered should have Hateful Strike");
        assert_eq!(fhs.total, 15000);
    }
}
