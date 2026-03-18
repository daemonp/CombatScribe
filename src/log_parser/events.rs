//! Discrete event parsing: casts, buffs, dispels, loot, trades, and avoidance.

use crate::log_data::{
    ConsumableUse, DispelEvent, InterruptEvent, ItemQuality, LogData, LogEntry, LootEvent,
    ResurrectEvent, TradeEvent,
};

use super::regex::{
    DISPEL_SPELLS, INTERRUPT_SPELLS, RESURRECT_SPELLS, RE_AFFLICTED, RE_BUFF_FADE, RE_BUFF_GAIN,
    RE_CAST, RE_CONSUMABLE, RE_DODGE, RE_LOOT, RE_MISS, RE_PARRY, RE_PET_OWNER, RE_TRADE,
};

// ── Cast, Buff, Loot & Avoidance Parsing ────────────────────────────────────

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
pub(super) fn parse_cast_events(trimmed: &str, timestamp: f64, data: &mut LogData) {
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
pub(super) fn parse_pet_ownership(trimmed: &str, data: &mut LogData) {
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

/// Parse dodge, parry, and miss events.
///
/// Uses `all_combatants` (built from `COMBATANT_INFO` during parsing) rather
/// than `combatants` (which is only populated in post-processing).
pub(super) fn parse_avoidance(trimmed: &str, data: &mut LogData) {
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
pub(super) fn parse_buff_events(trimmed: &str, timestamp: f64, data: &mut LogData) {
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
pub(super) fn parse_loot_trade(trimmed: &str, timestamp: f64, data: &mut LogData) {
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

/// Parse consumable usage from V1 `"uses"` lines.
pub(super) fn parse_consumable(trimmed: &str, timestamp: f64, data: &mut LogData) {
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

#[cfg(test)]
mod tests {
    use super::super::regex::*;
    use super::*;
    use crate::log_parser::parse_log;

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
}
