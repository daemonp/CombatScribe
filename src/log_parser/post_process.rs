//! Post-processing pass: encounter merging, loot attribution, and combatant filtering.

use std::collections::{HashMap, HashSet};

use crate::log_data::{Encounter, GearSlot, LogData, LogEntry};

use super::combat::KILL_GRACE_SECS;

// ── Post-Processing ─────────────────────────────────────────────────────────

/// Post-process: assign bosses to loot, link trades, number encounter attempts,
/// and build filtered combatant roster.
pub(super) fn post_process(data: &mut LogData) {
    // Extend encounter windows to include combat activity after the logging
    // player's PLAYER_REGEN_ENABLED.  When the logger dies mid-fight, their
    // client drops combat immediately but the rest of the raid keeps fighting.
    // The raw entries still contain all of that activity — we just need to
    // widen the encounter's [start, end] window to include it.
    extend_encounters_to_last_activity(data);

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

/// Maximum time (seconds) after `PLAYER_REGEN_ENABLED` to scan for continued
/// combat activity from other raid members.
///
/// Patchwerk kills typically last ~3-4 minutes.  Long fights with a dead logger
/// could theoretically run 10+ minutes (e.g. a very long wipe on a boss with
/// enrage timer).  Use `KILL_GRACE_SECS` (10 min) so the activity scan has the
/// same ceiling as the retroactive boss-kill window.
const ACTIVITY_SCAN_SECS: f64 = KILL_GRACE_SECS;

/// Minimum gap (seconds) of no combat activity to consider the fight over.
///
/// If no damage/healing event involving an NPC occurs for this long, we treat
/// the encounter as finished even if `ACTIVITY_SCAN_SECS` hasn't elapsed.
/// 30 seconds covers lulls in boss phase transitions while still cutting off
/// after a genuine wipe where everyone is dead and no combat is happening.
const ACTIVITY_IDLE_TIMEOUT: f64 = 30.0;

/// Extend encounter windows to match actual combat activity.
///
/// `PLAYER_REGEN_ENABLED` fires when the *logging player* leaves combat.
/// If the logger dies mid-fight, the encounter's `end` is set to their death
/// time — but the rest of the raid may keep fighting for minutes.  The raw
/// entries still contain all that activity; this function widens each
/// encounter's `[start, end]` window to include it.
///
/// For each encounter we scan `data.entries` forward from `enc.end`, looking
/// for damage or healing events where at least one participant is an NPC
/// (not in `all_combatants`).  We stop at the next encounter's start time,
/// after `ACTIVITY_SCAN_SECS`, or after `ACTIVITY_IDLE_TIMEOUT` of silence.
fn extend_encounters_to_last_activity(data: &mut LogData) {
    if data.encounters.is_empty() {
        return;
    }

    // Pre-compute the ceiling for each encounter: either the next encounter's
    // start time or `enc.end + ACTIVITY_SCAN_SECS`, whichever comes first.
    let ceilings: Vec<f64> = data
        .encounters
        .iter()
        .enumerate()
        .map(|(i, enc)| {
            let scan_limit = enc.end + ACTIVITY_SCAN_SECS;
            if i + 1 < data.encounters.len() {
                scan_limit.min(data.encounters[i + 1].start)
            } else {
                scan_limit
            }
        })
        .collect();

    // Entries are in chronological order.  For each encounter, binary-search
    // to the first entry at or after `enc.end`, then walk forward.
    for (enc_idx, enc) in data.encounters.iter_mut().enumerate() {
        // Kill encounters already have their correct end time:
        //  - Normal kills: PLAYER_REGEN_ENABLED fires after the boss dies.
        //  - Retroactive kills: retroactive_boss_kill() already extended
        //    enc.end to the boss death timestamp.
        // Scanning further would bleed into post-kill trash.
        if enc.is_kill {
            continue;
        }

        let ceiling = ceilings[enc_idx];
        let original_end = enc.end;

        // Binary search: find first entry with timestamp >= original_end
        let start_idx = data
            .entries
            .partition_point(|e| e.timestamp() < original_end);

        let mut last_activity = original_end;

        for entry in &data.entries[start_idx..] {
            let ts = entry.timestamp();
            if ts > ceiling {
                break;
            }
            // Idle timeout: if the gap since last activity is too large,
            // the fight is over.
            if ts - last_activity > ACTIVITY_IDLE_TIMEOUT {
                break;
            }

            // Only count damage/healing events that involve at least one NPC
            // (i.e. at least one participant is NOT a known player).
            let dominated_by_players = match entry {
                LogEntry::Damage { source, target, .. }
                | LogEntry::Healing { source, target, .. } => {
                    data.all_combatants.contains_key(source.as_str())
                        && data.all_combatants.contains_key(target.as_str())
                }
                _ => true, // non-combat events don't extend the window
            };

            if !dominated_by_players {
                last_activity = ts;
            }
        }

        if last_activity > original_end {
            enc.end = last_activity;
            enc.duration = enc.end - enc.start;

            // Recount deaths and active players for the extended window.
            // The original counts only covered [start, original_end]; we need
            // to include events in (original_end, last_activity].
            let mut deaths: HashSet<&str> = HashSet::new();
            let mut active: HashSet<&str> = HashSet::new();

            // Scan the full encounter window [start, end] for death/active counts.
            let full_start_idx = data.entries.partition_point(|e| e.timestamp() < enc.start);
            for entry in &data.entries[full_start_idx..] {
                let ts = entry.timestamp();
                if ts > enc.end {
                    break;
                }
                match entry {
                    LogEntry::Death { player, .. } => {
                        if data.all_combatants.contains_key(player.as_str()) {
                            deaths.insert(player.as_str());
                        }
                    }
                    LogEntry::Damage { source, target, .. }
                    | LogEntry::Healing { source, target, .. } => {
                        if data.all_combatants.contains_key(source.as_str()) {
                            active.insert(source.as_str());
                        }
                        if data.all_combatants.contains_key(target.as_str()) {
                            active.insert(target.as_str());
                        }
                    }
                    _ => {}
                }
            }

            #[allow(clippy::cast_possible_truncation)] // encounter won't have 4B players
            {
                enc.player_deaths = deaths.len() as u32;
                enc.active_players = active.len() as u32;
            }
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

/// Parse `PLAYERS_IN_COMBAT: N/M` into `(in_combat, total)`.
pub(super) fn parse_players_in_combat(trimmed: &str) -> Option<(u32, u32)> {
    let idx = trimmed.find("PLAYERS_IN_COMBAT:")? + "PLAYERS_IN_COMBAT:".len();
    let rest = trimmed[idx..].trim();
    let slash = rest.find('/')?;
    let in_combat = rest[..slash].trim().parse::<u32>().ok()?;
    let total = rest[slash + 1..].trim().parse::<u32>().ok()?;
    Some((in_combat, total))
}

/// Parse a gear slot string like `"16953:2564:0:0"` into a `GearSlot`.
pub(super) fn parse_gear_slot(raw: &str) -> Option<GearSlot> {
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
    use crate::log_parser::parse_log;

    /// When the logging player dies early in a boss fight, PLAYER_REGEN_ENABLED
    /// fires immediately — but the rest of the raid keeps fighting.  The
    /// encounter window must be extended to include that continued activity
    /// so damage/healing stats aren't truncated.
    #[test]
    fn test_encounter_extended_when_logger_dies_early() {
        // Simulate: logger enters combat, boss hits raid, logger dies at T+15s,
        // PLAYER_REGEN_ENABLED at T+16s, but raid fights for 200+ more seconds,
        // boss dies at T+220s.
        let lines: Vec<String> = vec![
            // Combatant info for two players
            "1/27 10:00:00.000  COMBATANT_INFO: 27.01.26 10:00:00&Logger&PRIEST&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            "1/27 10:00:00.000  COMBATANT_INFO: 27.01.26 10:00:00&Tank&WARRIOR&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000002&nil".to_string(),
            // Combat starts
            "1/27 10:00:10.000  PLAYER_REGEN_DISABLED".to_string(),
            // Some initial combat
            "1/27 10:00:12.000  Tank 's Bloodthirst hits Patchwerk for 500.".to_string(),
            "1/27 10:00:15.000  Patchwerk 's Hateful Strike hits Logger for 8000.".to_string(),
            // Logger dies
            "1/27 10:00:25.000  Logger dies.".to_string(),
            "1/27 10:00:25.000  UNIT_DIED:Logger:0x0000000000000001".to_string(),
            // Logger's combat drops
            "1/27 10:00:26.000  PLAYER_REGEN_ENABLED".to_string(),
            // Raid keeps fighting for minutes — NPC damage events continue
            "1/27 10:00:40.000  Tank 's Bloodthirst hits Patchwerk for 520.".to_string(),
            "1/27 10:01:00.000  Patchwerk 's Hateful Strike hits Tank for 7500.".to_string(),
            "1/27 10:01:20.000  Tank 's Bloodthirst hits Patchwerk for 510.".to_string(),
            "1/27 10:01:40.000  Tank hits Patchwerk for 300.".to_string(),
            "1/27 10:02:00.000  Tank 's Bloodthirst hits Patchwerk for 490.".to_string(),
            "1/27 10:02:20.000  Tank hits Patchwerk for 310.".to_string(),
            "1/27 10:02:40.000  Tank 's Bloodthirst hits Patchwerk for 530.".to_string(),
            "1/27 10:03:00.000  Tank hits Patchwerk for 290.".to_string(),
            "1/27 10:03:20.000  Tank 's Bloodthirst crits Patchwerk for 1100.".to_string(),
            // Boss dies after combat dropped for the logger
            "1/27 10:03:30.000  UNIT_DIED:Patchwerk:0xF130003000001234".to_string(),
        ];
        let data = parse_log(&lines);

        // Should have exactly 1 encounter
        assert_eq!(data.encounters.len(), 1, "Should have 1 encounter");
        let enc = &data.encounters[0];
        assert_eq!(enc.name.as_deref(), Some("Patchwerk"));
        assert!(enc.is_boss, "Patchwerk should be marked as boss");
        assert!(enc.is_kill, "Patchwerk should be marked as kill");

        // Duration should cover the full fight (~200s), not just until logger died (~16s)
        assert!(
            enc.duration > 190.0,
            "Duration should be ~200s (full fight), got {:.1}s — encounter was truncated at logger death",
            enc.duration
        );

        // Stats should include Tank's damage after the logger died
        let tank_stats = data.player_stats.get("Tank");
        assert!(
            tank_stats.is_some(),
            "Tank should have stats from the full fight"
        );
        let tank_dmg = tank_stats.unwrap().damage;
        assert!(
            tank_dmg > 3000,
            "Tank damage should include hits after logger died, got {}",
            tank_dmg
        );
    }

    /// When the logging player dies early in a wipe (boss doesn't die),
    /// the encounter window should still extend to cover the rest of the
    /// raid's combat activity.
    #[test]
    fn test_encounter_extended_on_wipe_when_logger_dies_early() {
        let lines: Vec<String> = vec![
            "1/27 10:00:00.000  COMBATANT_INFO: 27.01.26 10:00:00&Logger&PRIEST&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".to_string(),
            "1/27 10:00:00.000  COMBATANT_INFO: 27.01.26 10:00:00&Tank&WARRIOR&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000002&nil".to_string(),
            // Combat starts
            "1/27 10:00:10.000  PLAYER_REGEN_DISABLED".to_string(),
            "1/27 10:00:12.000  Tank hits Patchwerk for 500.".to_string(),
            "1/27 10:00:15.000  Patchwerk 's Hateful Strike hits Logger for 8000.".to_string(),
            // Logger dies at T+15s
            "1/27 10:00:25.000  Logger dies.".to_string(),
            "1/27 10:00:25.000  UNIT_DIED:Logger:0x0000000000000001".to_string(),
            "1/27 10:00:26.000  PLAYER_REGEN_ENABLED".to_string(),
            // Raid keeps fighting but eventually wipes (no UNIT_DIED for boss)
            "1/27 10:00:40.000  Tank hits Patchwerk for 510.".to_string(),
            "1/27 10:01:00.000  Patchwerk 's Hateful Strike hits Tank for 7500.".to_string(),
            "1/27 10:01:20.000  Tank hits Patchwerk for 490.".to_string(),
            "1/27 10:01:40.000  Patchwerk 's Hateful Strike hits Tank for 9000.".to_string(),
            // Tank dies — wipe complete, no more combat activity
            "1/27 10:01:50.000  Tank dies.".to_string(),
            "1/27 10:01:50.000  UNIT_DIED:Tank:0x0000000000000002".to_string(),
        ];
        let data = parse_log(&lines);

        assert_eq!(data.encounters.len(), 1, "Should have 1 encounter");
        let enc = &data.encounters[0];

        // The encounter is a wipe (boss didn't die), but duration should
        // extend to the last combat activity (~100s), not just 16s.
        assert!(
            enc.duration > 80.0,
            "Wipe duration should extend to last combat activity (~100s), got {:.1}s",
            enc.duration
        );

        // Tank's damage should be counted even after logger died
        let tank_stats = data.player_stats.get("Tank");
        assert!(
            tank_stats.is_some(),
            "Tank should have stats from the extended window"
        );
        let tank_dmg = tank_stats.unwrap().damage;
        assert!(
            tank_dmg > 1000,
            "Tank damage should include hits after logger died, got {}",
            tank_dmg
        );
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
}
