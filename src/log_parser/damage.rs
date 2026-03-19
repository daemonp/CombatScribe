//! Damage event parsing: spell hits, auto-attacks, and DoT/suffer lines.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;

use crate::log_data::{LogData, LogEntry};

use super::helpers::{ensure_stats, record_absorb};
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

impl TrailerData {
    /// Detect full mitigation: 0 damage dealt but mitigation trailer present.
    pub(super) fn full_mitigation_flags(&self, amount: u64) -> (bool, bool, bool) {
        (
            amount == 0 && self.resisted > 0,
            amount == 0 && self.absorbed > 0,
            amount == 0 && self.blocked > 0,
        )
    }
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
pub(super) fn parse_school(trimmed: &str) -> Option<&str> {
    // Look for the pattern: digits followed by a school word followed by "damage"
    // Common schools: Physical, Holy, Fire, Nature, Frost, Shadow, Arcane
    static RE_SCHOOL: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\d+ (Physical|Holy|Fire|Nature|Frost|Shadow|Arcane) damage").unwrap()
    });
    RE_SCHOOL
        .captures(trimmed)
        .and_then(|c| c.get(1).map(|m| m.as_str()))
}

/// Parsed damage event ready to be recorded into stats and entries.
///
/// All string fields borrow from the input line (`trimmed`) to avoid
/// intermediate heap allocations.  Owned `String`s are created only at
/// the point of insertion into `LogEntry` and `HashMap` keys.
#[allow(clippy::struct_excessive_bools)] // Many boolean combat outcomes to track
#[derive(Clone, Copy)]
pub(super) struct DamageEvent<'a> {
    pub(super) timestamp: f64,
    pub(super) source: &'a str,
    pub(super) target: &'a str,
    pub(super) spell: &'a str,
    pub(super) amount: u64,
    pub(super) absorbed: u64,
    pub(super) resisted: u64,
    pub(super) blocked: u64,
    pub(super) is_crit: bool,
    pub(super) is_glancing: bool,
    pub(super) is_crushing: bool,
    pub(super) school: Option<&'a str>,
    /// True when the spell came from a pet (formatter tagged it with `(pet)`).
    /// The `source` is already the owner — the formatter rewrites the source.
    pub(super) is_pet_spell: bool,
    pub(super) is_fully_resisted: bool,
    pub(super) is_fully_absorbed: bool,
    pub(super) is_fully_blocked: bool,
}

/// Record a parsed damage event into stats, entries, and health deficit.
pub(super) fn record_damage(
    data: &mut LogData,
    health_deficit: &mut HashMap<String, u64>,
    last_damage: &mut HashMap<String, super::LastDamageInfo>,
    event: DamageEvent<'_>,
) {
    // Record damage taken + source damage stats (both use &str directly).
    add_damage_taken(data, &event);
    add_damage(
        data,
        event.source,
        event.spell,
        event.amount,
        event.is_pet_spell,
        event.is_crit,
    );
    record_absorb(data, event.target, event.absorbed);

    // Increment target's health deficit for effective healing calculation
    *health_deficit.entry(event.target.to_string()).or_insert(0) += event.amount;

    // Track last damage for death attribution (only for players)
    if data.all_combatants.contains_key(event.target) {
        last_damage.insert(
            event.target.to_string(),
            super::LastDamageInfo {
                source: event.source.to_string(),
                spell: event.spell.to_string(),
                amount: event.amount,
            },
        );

        // Count full mitigation events for avoidance stats
        if event.is_fully_resisted || event.is_fully_absorbed || event.is_fully_blocked {
            let av = data.avoidance.entry(event.target.to_string()).or_default();
            if event.is_fully_resisted {
                av.full_resists += 1;
            }
            if event.is_fully_absorbed {
                av.full_absorbs += 1;
            }
            if event.is_fully_blocked {
                av.full_blocks += 1;
            }
        }
    }

    data.entries.push(LogEntry::Damage {
        timestamp: event.timestamp,
        source: event.source.to_string(),
        target: event.target.to_string(),
        spell: event.spell.to_string(),
        amount: event.amount,
        absorbed: event.absorbed,
        resisted: event.resisted,
        blocked: event.blocked,
        is_crit: event.is_crit,
        is_glancing: event.is_glancing,
        is_crushing: event.is_crushing,
        school: event.school.map(str::to_string),
        is_pet_spell: event.is_pet_spell,
        is_fully_resisted: event.is_fully_resisted,
        is_fully_absorbed: event.is_fully_absorbed,
        is_fully_blocked: event.is_fully_blocked,
    });
}

/// Parse all 3 damage event formats.
#[allow(clippy::too_many_lines)] // Three format variants with trailer/school extraction each
pub(super) fn parse_damage_events(
    trimmed: &str,
    timestamp: f64,
    data: &mut LogData,
    health_deficit: &mut HashMap<String, u64>,
    last_damage: &mut HashMap<String, super::LastDamageInfo>,
) {
    let is_crit = trimmed.contains(" crits ") || trimmed.contains(" critically ");
    let is_pet_spell = trimmed.contains("(pet)");
    let trailer = parse_trailer(trimmed);
    let school = parse_school(trimmed);

    // Format 1: Source 's Spell hits/crits Target for N
    if let Some(caps) = RE_DMG_SPELL.captures(trimmed) {
        let Some(source) = caps.get(1).map(|m| m.as_str()) else {
            return;
        };
        let Some(spell) = caps.get(2).map(|m| m.as_str().trim()) else {
            return;
        };
        let Some(target) = caps.get(3).map(|m| m.as_str().trim()) else {
            return;
        };
        let amount: u64 = caps
            .get(4)
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(0);
        let (is_fully_resisted, is_fully_absorbed, is_fully_blocked) =
            trailer.full_mitigation_flags(amount);
        record_damage(
            data,
            health_deficit,
            last_damage,
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
                is_pet_spell,
                is_fully_resisted,
                is_fully_absorbed,
                is_fully_blocked,
            },
        );
        return;
    }

    // Format 2: Source hits/crits Target for N.
    if let Some(caps) = RE_DMG_AUTO.captures(trimmed) {
        let Some(source) = caps.get(1).map(|m| m.as_str()) else {
            return;
        };
        let Some(target) = caps.get(2).map(|m| m.as_str().trim()) else {
            return;
        };
        let amount: u64 = caps
            .get(3)
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(0);
        let (is_fully_resisted, is_fully_absorbed, is_fully_blocked) =
            trailer.full_mitigation_flags(amount);
        record_damage(
            data,
            health_deficit,
            last_damage,
            DamageEvent {
                timestamp,
                source,
                target,
                spell: "Auto Attack",
                amount,
                absorbed: trailer.absorbed,
                resisted: trailer.resisted,
                blocked: trailer.blocked,
                is_crit,
                is_glancing: trailer.is_glancing,
                is_crushing: trailer.is_crushing,
                school,
                is_pet_spell,
                is_fully_resisted,
                is_fully_absorbed,
                is_fully_blocked,
            },
        );
        return;
    }

    // Format 3: Target suffers N damage from Source 's Spell (periodic/DoT)
    if let Some(caps) = RE_DMG_SUFFER.captures(trimmed) {
        let Some(target) = caps.get(1).map(|m| m.as_str().trim()) else {
            return;
        };
        let amount: u64 = caps
            .get(2)
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(0);
        let Some(source) = caps.get(3).map(|m| m.as_str()) else {
            return;
        };
        let Some(raw_spell) = caps.get(4).map(|m| m.as_str().trim()) else {
            return;
        };
        // "suffers" lines are periodic (DoT) ticks — distinguish from direct hits
        let spell_dot = format!("{raw_spell} (dot)");
        let (is_fully_resisted, is_fully_absorbed, is_fully_blocked) =
            trailer.full_mitigation_flags(amount);
        record_damage(
            data,
            health_deficit,
            last_damage,
            DamageEvent {
                timestamp,
                source,
                target,
                spell: &spell_dot,
                amount,
                absorbed: trailer.absorbed,
                resisted: trailer.resisted,
                blocked: trailer.blocked,
                is_crit,
                is_glancing: trailer.is_glancing,
                is_crushing: trailer.is_crushing,
                school,
                is_pet_spell,
                is_fully_resisted,
                is_fully_absorbed,
                is_fully_blocked,
            },
        );
    }
}

/// Add damage to a source player's stats.
///
/// When `is_pet_spell` is true the formatter has already rewritten the source
/// to the owner — the damage counts toward the owner's total `damage` AND is
/// tracked separately in `pet_damage` so the UI can show a "Personal only"
/// mode that subtracts pet damage.  The ability is recorded with `is_pet: true`
/// so the detail view can visually separate pet abilities.
///
/// For summoned-entity spells like Explosive Trap, the burst hits carry the
/// `(pet)` tag but the periodic ticks do not (the game uses a different format).
/// Once an ability is marked `is_pet` for a given source, subsequent entries
/// of the **same base spell** (ignoring `(dot)` suffix) inherit the flag.
fn add_damage(
    data: &mut LogData,
    source: &str,
    spell: &str,
    amount: u64,
    is_pet_spell: bool,
    is_crit: bool,
) {
    let stats = ensure_stats(data, source);
    stats.accumulate_damage(spell, amount, is_crit);

    // Determine effective pet status: either the formatter tagged it, or a
    // prior entry for the same base spell was already flagged as pet.  This
    // handles summoned-entity DoT ticks (e.g. "Explosive Trap Effect (dot)")
    // whose burst-hit variant was tagged via the formatter.
    let effective_pet = is_pet_spell || {
        let base = spell.strip_suffix(" (dot)").unwrap_or(spell);
        base != spell && stats.abilities.get(base).is_some_and(|a| a.is_pet)
    };

    if effective_pet {
        stats.pet_damage += amount;
        stats.abilities.entry(spell.to_string()).or_default().is_pet = true;
        // When a base spell is first tagged (pet), retroactively mark any
        // existing (dot) variant so earlier ticks aren't orphaned as personal.
        if is_pet_spell {
            let dot_key = format!("{spell} (dot)");
            if let Some(dot_ab) = stats.abilities.get_mut(&dot_key)
                && !dot_ab.is_pet
            {
                dot_ab.is_pet = true;
                // Move the already-accumulated dot damage into pet_damage
                stats.pet_damage += dot_ab.total;
            }
        }
    }
}

/// Add damage taken to a target player's stats, including per-source per-ability
/// breakdown with mitigation details.
fn add_damage_taken(data: &mut LogData, event: &DamageEvent<'_>) {
    ensure_stats(data, event.target).accumulate_damage_taken(
        event.source,
        event.spell,
        event.amount,
        event.absorbed,
        event.resisted,
        event.blocked,
        event.is_crit,
        event.is_crushing,
        event.is_glancing,
    );
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
        assert_eq!(parse_school(line), Some("Fire"));
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
            assert_eq!(parse_school(&line), Some(*school), "Should parse {school}");
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

    // ── Full Mitigation Tests ──────────────────────────────────────────

    #[test]
    fn test_full_resist_detection() {
        let lines = vec![
            "1/27 10:00:00.000  COMBATANT_INFO: 27.01.26 10:00:00&Tank&WARRIOR&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            "1/27 10:00:10.000  Boss 's Fireball hits Tank for 0. (1500 resisted)".to_string(),
        ];
        let data = parse_log(&lines);

        assert_eq!(data.entries.len(), 1);
        if let LogEntry::Damage {
            amount,
            resisted,
            is_fully_resisted,
            is_fully_absorbed,
            is_fully_blocked,
            ..
        } = &data.entries[0]
        {
            assert_eq!(*amount, 0);
            assert_eq!(*resisted, 1500);
            assert!(*is_fully_resisted);
            assert!(!*is_fully_absorbed);
            assert!(!*is_fully_blocked);
        } else {
            panic!("Expected Damage entry");
        }
    }

    #[test]
    fn test_partial_resist_detection() {
        let lines = vec![
            "1/27 10:00:00.000  COMBATANT_INFO: 27.01.26 10:00:00&Tank&WARRIOR&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            "1/27 10:00:10.000  Boss 's Fireball hits Tank for 500. (750 resisted)".to_string(),
        ];
        let data = parse_log(&lines);

        assert_eq!(data.entries.len(), 1);
        if let LogEntry::Damage {
            amount,
            resisted,
            is_fully_resisted,
            ..
        } = &data.entries[0]
        {
            assert_eq!(*amount, 500);
            assert_eq!(*resisted, 750);
            assert!(!*is_fully_resisted); // Partial - damage got through
        } else {
            panic!("Expected Damage entry");
        }
    }

    #[test]
    fn test_full_absorb_detection() {
        let lines = vec![
            "1/27 10:00:00.000  COMBATANT_INFO: 27.01.26 10:00:00&Tank&PRIEST&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000002&nil".to_string(),
            "1/27 10:00:10.000  Patchwerk hits Tank for 0. (5000 absorbed)".to_string(),
        ];
        let data = parse_log(&lines);

        assert_eq!(data.entries.len(), 1);
        if let LogEntry::Damage {
            amount,
            absorbed,
            is_fully_absorbed,
            is_fully_resisted,
            is_fully_blocked,
            ..
        } = &data.entries[0]
        {
            assert_eq!(*amount, 0);
            assert_eq!(*absorbed, 5000);
            assert!(*is_fully_absorbed);
            assert!(!*is_fully_resisted);
            assert!(!*is_fully_blocked);
        } else {
            panic!("Expected Damage entry");
        }
    }

    #[test]
    fn test_partial_absorb_detection() {
        let lines = vec![
            "1/27 10:00:00.000  COMBATANT_INFO: 27.01.26 10:00:00&Tank&PRIEST&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000002&nil".to_string(),
            "1/27 10:00:10.000  Patchwerk hits Tank for 2000. (1500 absorbed)".to_string(),
        ];
        let data = parse_log(&lines);

        assert_eq!(data.entries.len(), 1);
        if let LogEntry::Damage {
            amount,
            absorbed,
            is_fully_absorbed,
            ..
        } = &data.entries[0]
        {
            assert_eq!(*amount, 2000);
            assert_eq!(*absorbed, 1500);
            assert!(!*is_fully_absorbed); // Partial - damage got through
        } else {
            panic!("Expected Damage entry");
        }
    }

    #[test]
    fn test_full_block_detection() {
        let lines = vec![
            "1/27 10:00:00.000  COMBATANT_INFO: 27.01.26 10:00:00&Tank&WARRIOR&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            "1/27 10:00:10.000  Patchwerk hits Tank for 0. (300 blocked)".to_string(),
        ];
        let data = parse_log(&lines);

        assert_eq!(data.entries.len(), 1);
        if let LogEntry::Damage {
            amount,
            blocked,
            is_fully_blocked,
            is_fully_resisted,
            is_fully_absorbed,
            ..
        } = &data.entries[0]
        {
            assert_eq!(*amount, 0);
            assert_eq!(*blocked, 300);
            assert!(*is_fully_blocked);
            assert!(!*is_fully_resisted);
            assert!(!*is_fully_absorbed);
        } else {
            panic!("Expected Damage entry");
        }
    }

    #[test]
    fn test_mixed_mitigation() {
        let lines = vec![
            "1/27 10:00:00.000  COMBATANT_INFO: 27.01.26 10:00:00&Tank&WARRIOR&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            "1/27 10:00:10.000  Boss hits Tank for 1000. (500 resisted) (200 absorbed) (100 blocked)".to_string(),
        ];
        let data = parse_log(&lines);

        assert_eq!(data.entries.len(), 1);
        if let LogEntry::Damage {
            amount,
            resisted,
            absorbed,
            blocked,
            is_fully_resisted,
            is_fully_absorbed,
            is_fully_blocked,
            ..
        } = &data.entries[0]
        {
            assert_eq!(*amount, 1000);
            assert_eq!(*resisted, 500);
            assert_eq!(*absorbed, 200);
            assert_eq!(*blocked, 100);
            // All partial - damage got through
            assert!(!*is_fully_resisted);
            assert!(!*is_fully_absorbed);
            assert!(!*is_fully_blocked);
        } else {
            panic!("Expected Damage entry");
        }
    }

    #[test]
    fn test_avoidance_full_resist_counted() {
        let lines = vec![
            "1/27 10:00:00.000  COMBATANT_INFO: 27.01.26 10:00:00&Tank&WARRIOR&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            "1/27 10:00:10.000  Boss 's Shadow Bolt hits Tank for 0 Shadow damage. (500 resisted)".to_string(),
        ];
        let data = parse_log(&lines);
        let av = data
            .avoidance
            .get("Tank")
            .expect("Tank should have avoidance stats");
        assert_eq!(av.full_resists, 1);
        assert_eq!(av.full_absorbs, 0);
        assert_eq!(av.full_blocks, 0);
    }

    #[test]
    fn test_avoidance_full_absorb_counted() {
        let lines = vec![
            "1/27 10:00:00.000  COMBATANT_INFO: 27.01.26 10:00:00&Tank&WARRIOR&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            "1/27 10:00:10.000  Boss hits Tank for 0. (500 absorbed)".to_string(),
        ];
        let data = parse_log(&lines);
        let av = data
            .avoidance
            .get("Tank")
            .expect("Tank should have avoidance stats");
        assert_eq!(av.full_absorbs, 1);
        assert_eq!(av.full_resists, 0);
        assert_eq!(av.full_blocks, 0);
    }

    #[test]
    fn test_avoidance_full_block_counted() {
        let lines = vec![
            "1/27 10:00:00.000  COMBATANT_INFO: 27.01.26 10:00:00&Tank&WARRIOR&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            "1/27 10:00:10.000  Patchwerk hits Tank for 0. (300 blocked)".to_string(),
        ];
        let data = parse_log(&lines);
        let av = data
            .avoidance
            .get("Tank")
            .expect("Tank should have avoidance stats");
        assert_eq!(av.full_blocks, 1);
        assert_eq!(av.full_resists, 0);
        assert_eq!(av.full_absorbs, 0);
    }

    #[test]
    fn test_avoidance_partial_not_counted() {
        let lines = vec![
            "1/27 10:00:00.000  COMBATANT_INFO: 27.01.26 10:00:00&Tank&WARRIOR&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            "1/27 10:00:10.000  Boss hits Tank for 1000. (500 resisted) (200 absorbed) (100 blocked)".to_string(),
        ];
        let data = parse_log(&lines);
        // Partial mitigation should not create avoidance stats for full mitigation
        let av = data.avoidance.get("Tank");
        let full_mit = av.map_or(0, |a| a.total_full_mitigation());
        assert_eq!(full_mit, 0);
    }

    #[test]
    fn test_avoidance_full_mitigation_multiple() {
        let lines = vec![
            "1/27 10:00:00.000  COMBATANT_INFO: 27.01.26 10:00:00&Tank&WARRIOR&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            "1/27 10:00:10.000  Boss hits Tank for 0. (500 absorbed)".to_string(),
            "1/27 10:00:11.000  Boss hits Tank for 0. (300 blocked)".to_string(),
            "1/27 10:00:12.000  Boss hits Tank for 0. (400 absorbed)".to_string(),
            "1/27 10:00:13.000  Boss hits Tank for 2000.".to_string(),
        ];
        let data = parse_log(&lines);
        let av = data
            .avoidance
            .get("Tank")
            .expect("Tank should have avoidance stats");
        assert_eq!(av.full_absorbs, 2);
        assert_eq!(av.full_blocks, 1);
        assert_eq!(av.full_resists, 0);
        assert_eq!(av.total_full_mitigation(), 3);
    }

    #[test]
    fn test_avoidance_non_player_not_counted() {
        // Damage against non-players should not create full mitigation avoidance stats
        let lines = vec![
            "1/27 10:00:00.000  COMBATANT_INFO: 27.01.26 10:00:00&Tank&WARRIOR&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            "1/27 10:00:10.000  Tank hits Boss for 0. (500 absorbed)".to_string(),
        ];
        let data = parse_log(&lines);
        // Boss is not a combatant, so no avoidance should be recorded for it
        assert!(data.avoidance.get("Boss").is_none());
    }

    // ── DoT Suffix Tests ───────────────────────────────────────────────

    #[test]
    fn test_dot_suffix_on_suffer_line() {
        // "suffers" lines should produce spell names with " (dot)" suffix
        let lines = vec![
            "1/27 10:00:00.000  COMBATANT_INFO: 27.01.26 10:00:00&Druid&DRUID&NightElf&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            "1/27 10:00:10.000  Boss suffers 136 Physical damage from Druid 's Rake.".to_string(),
        ];
        let data = parse_log(&lines);

        let dmg = data
            .entries
            .iter()
            .find(|e| matches!(e, LogEntry::Damage { .. }));
        assert!(dmg.is_some(), "Should have a damage entry");
        if let Some(LogEntry::Damage { spell, amount, .. }) = dmg {
            assert_eq!(
                spell, "Rake (dot)",
                "Suffer line should produce (dot) suffix"
            );
            assert_eq!(*amount, 136);
        }
    }

    #[test]
    fn test_direct_hit_no_dot_suffix() {
        // Direct "hits" lines should NOT have the (dot) suffix
        let lines = vec![
            "1/27 10:00:00.000  COMBATANT_INFO: 27.01.26 10:00:00&Druid&DRUID&NightElf&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            "1/27 10:00:10.000  Druid 's Rake hits Boss for 45.".to_string(),
        ];
        let data = parse_log(&lines);

        let dmg = data
            .entries
            .iter()
            .find(|e| matches!(e, LogEntry::Damage { .. }));
        assert!(dmg.is_some(), "Should have a damage entry");
        if let Some(LogEntry::Damage { spell, .. }) = dmg {
            assert_eq!(spell, "Rake", "Direct hit should NOT have (dot) suffix");
        }
    }

    #[test]
    fn test_dot_and_direct_separate_abilities() {
        // Both direct hit and DoT tick from same spell should produce separate
        // ability entries in player_stats
        let lines = vec![
            "1/27 10:00:00.000  COMBATANT_INFO: 27.01.26 10:00:00&Druid&DRUID&NightElf&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            "1/27 10:00:10.000  PLAYER_REGEN_DISABLED".to_string(),
            "1/27 10:00:11.000  Druid 's Rake hits Boss for 300.".to_string(),
            "1/27 10:00:14.000  Boss suffers 100 Physical damage from Druid 's Rake.".to_string(),
            "1/27 10:00:17.000  Boss suffers 100 Physical damage from Druid 's Rake.".to_string(),
            "1/27 10:00:20.000  Boss suffers 100 Physical damage from Druid 's Rake.".to_string(),
            "1/27 10:00:30.000  PLAYER_REGEN_ENABLED".to_string(),
        ];
        let data = parse_log(&lines);

        let druid = data
            .player_stats
            .get("Druid")
            .expect("Druid should have stats");
        assert_eq!(druid.damage, 600, "Total damage should be 300 + 3*100");

        let rake_direct = druid.abilities.get("Rake");
        assert!(
            rake_direct.is_some(),
            "Should have 'Rake' ability for direct hit"
        );
        assert_eq!(rake_direct.unwrap().total, 300);
        assert_eq!(rake_direct.unwrap().hits, 1);

        let rake_dot = druid.abilities.get("Rake (dot)");
        assert!(
            rake_dot.is_some(),
            "Should have 'Rake (dot)' ability for DoT ticks"
        );
        assert_eq!(rake_dot.unwrap().total, 300);
        assert_eq!(rake_dot.unwrap().hits, 3);
    }
}
