//! Session detection logic for `WoW` combat logs.

use std::collections::HashSet;

use crate::raid_data;

use super::boss::{
    get_boss_count, instance_from_boss_kills, is_known_boss, is_raid_zone, normalize_zone_name,
};
use super::extraction::{
    bytecount_char, extract_combatant, extract_combatant_year, extract_unit_died,
    extract_you_player_name, extract_zone,
};
use super::timestamp::parse_timestamp_fast;
use super::Session;

/// Session gap threshold (30 minutes in seconds).
const SESSION_GAP_SECS: f64 = 30.0 * 60.0;

#[derive(Debug, Clone, Copy)]
enum EventType {
    Zone,
    Player,
    /// `COMBATANT_INFO` with full talent info (2 `}` chars) — the "You" player.
    FullPlayer,
    CombatStart,
    CombatEnd,
    BossKill,
    /// A non-boss NPC died that maps to a known instance via `npc_raid()`.
    /// Used for zone disambiguation (e.g. Lower vs Upper Karazhan).
    NpcZone,
}

#[derive(Debug)]
struct ScanEvent {
    timestamp_secs: f64,
    line_index: usize,
    event_type: EventType,
    /// Index into the names vec (for Zone, Player, `FullPlayer`, `BossKill` events).
    name_idx: u32,
}

// ── Session Detection ───────────────────────────────────────────────────────

/// Quick-scan the log to detect sessions without full parsing.
///
/// Avoids regex entirely — uses manual byte-level parsing for maximum speed.
pub fn detect_sessions(lines: &[String]) -> Vec<Session> {
    let (names, events, log_year) = scan_events(lines);

    if events.is_empty() {
        return vec![];
    }

    build_sessions(&names, events, lines.len(), log_year)
}

/// First phase: scan all lines and extract structured events.
///
/// Returns `(names, events, log_year)` where `log_year` is the calendar year
/// extracted from the first `COMBATANT_INFO` date field (e.g. 2026).
#[allow(clippy::cast_possible_truncation)] // name_idx will never exceed u32::MAX for real logs
fn scan_events(lines: &[String]) -> (Vec<String>, Vec<ScanEvent>, Option<i32>) {
    let mut names: Vec<String> = Vec::new();
    let mut events: Vec<ScanEvent> = Vec::with_capacity(lines.len() / 20);
    let mut log_year: Option<i32> = None;

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let bytes = trimmed.as_bytes();
        let Some((ts_secs, _ts_end)) = parse_timestamp_fast(bytes) else {
            continue;
        };

        if trimmed.contains("ZONE_INFO:") {
            if let Some(zone) = extract_zone(trimmed) {
                let canonical = normalize_zone_name(zone);
                let idx = names.len() as u32;
                names.push(canonical);
                events.push(ScanEvent {
                    timestamp_secs: ts_secs,
                    line_index: i,
                    event_type: EventType::Zone,
                    name_idx: idx,
                });
            }
        }

        if trimmed.contains("COMBATANT_INFO:") {
            // Extract calendar year from the first COMBATANT_INFO date field.
            // Format: "COMBATANT_INFO:DD.MM.YY HH:MM:SS&name&..."
            if log_year.is_none() {
                log_year = extract_combatant_year(trimmed);
            }

            if let Some((name, _class)) = extract_combatant(trimmed) {
                let idx = names.len() as u32;
                names.push(name.to_string());
                events.push(ScanEvent {
                    timestamp_secs: ts_secs,
                    line_index: i,
                    event_type: EventType::Player,
                    name_idx: idx,
                });

                // Check if this is a "full" COMBATANT_INFO (2 '}' = has talent data)
                if bytecount_char(bytes, b'}') == 2 {
                    if let Some(amp_name) = extract_you_player_name(trimmed) {
                        let fidx = names.len() as u32;
                        names.push(amp_name.to_string());
                        events.push(ScanEvent {
                            timestamp_secs: ts_secs,
                            line_index: i,
                            event_type: EventType::FullPlayer,
                            name_idx: fidx,
                        });
                    }
                }
            }
        }

        if trimmed.contains("PLAYER_REGEN_DISABLED") {
            events.push(ScanEvent {
                timestamp_secs: ts_secs,
                line_index: i,
                event_type: EventType::CombatStart,
                name_idx: 0,
            });
        }
        if trimmed.contains("PLAYER_REGEN_ENABLED") {
            events.push(ScanEvent {
                timestamp_secs: ts_secs,
                line_index: i,
                event_type: EventType::CombatEnd,
                name_idx: 0,
            });
        }

        if trimmed.contains("UNIT_DIED:") {
            if let Some(dead_unit) = extract_unit_died(trimmed) {
                if is_known_boss(dead_unit) {
                    let idx = names.len() as u32;
                    names.push(dead_unit.to_string());
                    events.push(ScanEvent {
                        timestamp_secs: ts_secs,
                        line_index: i,
                        event_type: EventType::BossKill,
                        name_idx: idx,
                    });
                } else if let Some(raid_zone) = raid_data::npc_raid(dead_unit) {
                    // Non-boss NPC mapped to an instance — record for zone
                    // disambiguation (e.g. Lower vs Upper Karazhan trash).
                    let idx = names.len() as u32;
                    names.push(raid_zone.to_string());
                    events.push(ScanEvent {
                        timestamp_secs: ts_secs,
                        line_index: i,
                        event_type: EventType::NpcZone,
                        name_idx: idx,
                    });
                }
            }
        }
    }

    (names, events, log_year)
}

struct SessionBuilder {
    start_time_secs: f64,
    end_time_secs: f64,
    start_line: usize,
    end_line: usize,
    primary_zone: Option<String>,
    players: HashSet<String>,
    you_players: HashSet<String>,
    combat_count: usize,
    boss_kills: Vec<String>,
    start_year: Option<i32>,
    /// Instance zones seen from non-boss NPC deaths (via `npc_raid()`).
    /// Used for zone disambiguation when boss kills are absent.
    npc_raid_zones: HashSet<String>,
}

/// Second phase: group sorted events into sessions.
///
/// Session boundaries are determined by:
/// 1. **Time gaps** > 30 minutes between events -> new session.
/// 2. **Entering a raid zone** from a non-raid session -> new session starts.
/// 3. **Entering a *different* raid zone** while already in a raid -> new session.
/// 4. **Boss kill from a different instance** than the current session -> new session.
///    This handles back-to-back raids (BWL -> AQ40 -> Onyxia) without zone events.
///
/// Once a session has a raid zone, it is "sticky" — overworld/city zone blips
/// (e.g. `deadwind pass` between Karazhan boss attempts) do NOT split the session.
/// This is critical because zone events from raid members interleave with
/// overworld zones as players zone in/out.
#[allow(clippy::too_many_lines)] // Session grouping logic with NPC zone tracking
#[allow(clippy::cast_possible_truncation)] // name_idx indexing
fn build_sessions(
    names: &[String],
    mut events: Vec<ScanEvent>,
    total_lines: usize,
    log_year: Option<i32>,
) -> Vec<Session> {
    events.sort_unstable_by(|a, b| a.timestamp_secs.total_cmp(&b.timestamp_secs));

    // Phase 1: Group events into logical sessions by timestamp order.
    // Each event gets tagged with a session index.
    let mut session_tags: Vec<usize> = Vec::with_capacity(events.len());
    let mut sessions: Vec<SessionBuilder> = Vec::new();
    let mut current: Option<SessionBuilder> = None;
    let mut current_idx: usize = 0;

    for event in &events {
        let zone_name = match event.event_type {
            EventType::Zone => Some(names[event.name_idx as usize].as_str()),
            _ => None,
        };

        // Check if this boss kill belongs to a different instance than the current session
        let boss_splits = matches!(event.event_type, EventType::BossKill)
            && current.as_ref().is_some_and(|cur| {
                let boss = &names[event.name_idx as usize];
                if let Some(boss_inst) = raid_data::boss_raid(boss) {
                    // If the current session already has boss kills from a specific instance,
                    // and this new boss is from a DIFFERENT instance -> split
                    if let Some(cur_inst) = instance_from_boss_kills(&cur.boss_kills) {
                        return cur_inst != boss_inst;
                    }
                    // If the session has a raid zone set and boss is from a different raid -> split
                    if let Some(pz) = cur.primary_zone.as_ref() {
                        if is_raid_zone(pz) && pz != boss_inst {
                            return true;
                        }
                    }
                }
                false
            });

        let should_start_new = boss_splits
            || current.as_ref().is_none_or(|cur| {
                // Time gap exceeds threshold
                if event.timestamp_secs - cur.end_time_secs > SESSION_GAP_SECS {
                    return true;
                }
                if let Some(zone) = zone_name {
                    if is_raid_zone(zone) {
                        let cur_is_raid =
                            cur.primary_zone.as_ref().is_some_and(|pz| is_raid_zone(pz));
                        if !cur_is_raid {
                            // Entering a raid zone from a non-raid session -> split
                            return true;
                        }
                        // Already in a raid — only split if it's a DIFFERENT raid
                        let same_raid = cur.primary_zone.as_ref().is_some_and(|pz| pz == zone);
                        if !same_raid {
                            return true;
                        }
                    }
                }
                false
            });

        if should_start_new {
            if let Some(session) = current.take() {
                sessions.push(session);
            }
            current_idx = sessions.len();
            current = Some(SessionBuilder {
                start_time_secs: event.timestamp_secs,
                end_time_secs: event.timestamp_secs,
                // Line ranges are computed in Phase 2 from file-order boundaries.
                start_line: 0,
                end_line: 0,
                primary_zone: None,
                players: HashSet::new(),
                you_players: HashSet::new(),
                combat_count: 0,
                boss_kills: Vec::new(),
                start_year: log_year,
                npc_raid_zones: HashSet::new(),
            });
        }

        session_tags.push(current_idx);

        let cur = current
            .as_mut()
            .expect("current always Some after session init");
        cur.end_time_secs = event.timestamp_secs;

        match event.event_type {
            EventType::Zone => {
                let zone = &names[event.name_idx as usize];
                // Raid zones always take priority and are sticky — once set,
                // only a different raid zone can replace them.
                // Non-raid zones only apply if the current zone is also non-raid.
                let cur_is_raid = cur.primary_zone.as_ref().is_some_and(|pz| is_raid_zone(pz));
                if is_raid_zone(zone) || !cur_is_raid {
                    cur.primary_zone = Some(zone.clone());
                }
            }
            EventType::Player => {
                cur.players.insert(names[event.name_idx as usize].clone());
            }
            EventType::FullPlayer => {
                cur.you_players
                    .insert(names[event.name_idx as usize].clone());
            }
            EventType::CombatStart => {
                cur.combat_count += 1;
            }
            EventType::CombatEnd => {}
            EventType::BossKill => {
                cur.boss_kills.push(names[event.name_idx as usize].clone());
            }
            EventType::NpcZone => {
                cur.npc_raid_zones
                    .insert(names[event.name_idx as usize].clone());
            }
        }
    }

    if let Some(session) = current.take() {
        sessions.push(session);
    }

    // Phase 2: Compute non-overlapping line ranges from file-order boundaries.
    // Events were grouped by timestamp, but file positions may differ.  Walk the
    // events in file order and find where the majority session changes to
    // determine clean split points.
    compute_session_line_ranges(&events, &session_tags, &mut sessions, total_lines);

    finalize_sessions(sessions)
}

/// Compute generous line-range hints for sessions from file-ordered events.
///
/// These ranges are scan windows — `extract_session_lines()` uses timestamp
/// filtering within these ranges for precise line selection.  The hints must
/// include all possible lines for the session but may overlap between sessions.
fn compute_session_line_ranges(
    events: &[ScanEvent],
    session_tags: &[usize],
    sessions: &mut [SessionBuilder],
    total_lines: usize,
) {
    if sessions.is_empty() {
        return;
    }

    let num_sessions = sessions.len();

    // For each session, find the min and max line_index from its events.
    let mut min_line = vec![usize::MAX; num_sessions];
    let mut max_line = vec![0_usize; num_sessions];

    for (event, &sid) in events.iter().zip(session_tags) {
        if sid < num_sessions {
            min_line[sid] = min_line[sid].min(event.line_index);
            max_line[sid] = max_line[sid].max(event.line_index);
        }
    }

    // Sort sessions by start timestamp to find neighbors.
    let mut by_time: Vec<usize> = (0..num_sessions)
        .filter(|&i| min_line[i] != usize::MAX)
        .collect();
    by_time.sort_unstable_by(|&a, &b| {
        sessions[a]
            .start_time_secs
            .total_cmp(&sessions[b].start_time_secs)
    });

    for (pos, &sid) in by_time.iter().enumerate() {
        // Start: use the session's earliest event line.
        sessions[sid].start_line = min_line[sid];

        // End: use the next session's earliest event minus 1, or the session's
        // own last event, whichever is larger.  This ensures we capture all
        // combat lines between the session's events while not extending
        // infinitely.  Timestamp filtering handles precision.
        let next_boundary = if pos + 1 < by_time.len() {
            let next_sid = by_time[pos + 1];
            // The next session's min_line is a natural boundary, but our
            // session might have events beyond that (interleaved).
            // Use max of our max_line and next's min_line.
            max_line[sid].max(min_line[next_sid])
        } else {
            total_lines.saturating_sub(1)
        };
        sessions[sid].end_line = next_boundary;
    }
}

/// Convert `SessionBuilder`s into final `Session` structs.
///
/// Uses a two-tier zone resolution strategy:
/// 1. If boss kills are present and all belong to one instance, use that instance name
///    (boss-to-instance mapping is the most reliable signal).
/// 2. Otherwise, use the `ZONE_INFO`-derived zone (already normalized by aliases).
fn finalize_sessions(sessions: Vec<SessionBuilder>) -> Vec<Session> {
    sessions
        .into_iter()
        .filter(|s| s.combat_count > 0)
        .map(|s| {
            let duration = s.end_time_secs - s.start_time_secs;

            // Try to determine instance from boss kills first (most reliable).
            // Fall back to the zone reported by ZONE_INFO.
            let boss_zone = instance_from_boss_kills(&s.boss_kills);
            let zone_info = s.primary_zone.as_deref().unwrap_or("unknown");

            // Boss kills are the most reliable signal for instance identity.
            // Always prefer the boss-derived zone when available — it fixes
            // cases where ZONE_INFO reports a stale or incorrect raid zone
            // (e.g. ZG zone event lingering while actually in AQ40).
            let zone = if let Some(bz) = boss_zone {
                bz
            } else {
                zone_info
            };

            // Karazhan disambiguation: The addon reports both Lower and Upper
            // Karazhan with ambiguous zone names ("Karazhan" or "Tower of Karazhan").
            // When no boss kills confirm the instance, use NPC deaths to disambiguate
            // — the trash mob lists are completely disjoint between Lower and Upper.
            let zone =
                if (zone == "lower karazhan" || zone == "upper karazhan") && boss_zone.is_none() {
                    if s.npc_raid_zones.contains("lower karazhan") {
                        "lower karazhan"
                    } else if s.npc_raid_zones.contains("upper karazhan") {
                        "upper karazhan"
                    } else {
                        // No NPC evidence either way — trust the zone alias default
                        zone
                    }
                } else {
                    zone
                };

            let zone_display = raid_data::format_zone_name(zone);

            let is_dungeon = raid_data::is_dungeon_zone(zone);

            let name = if is_raid_zone(zone) {
                let total_bosses = get_boss_count(zone).unwrap_or(0);
                let kill_count = s.boss_kills.len();
                if total_bosses > 0 && kill_count > 0 {
                    if kill_count >= total_bosses {
                        format!("{zone_display} Full Clear")
                    } else {
                        format!("{zone_display} ({kill_count}/{total_bosses})")
                    }
                } else if s.combat_count > 0 && kill_count == 0 && !is_dungeon {
                    // "wipes" label only for raids — dungeon sessions without
                    // boss kills are normal (trash clearing, partial runs).
                    format!("{zone_display} (wipes)")
                } else {
                    zone_display
                }
            } else {
                zone_display
            };

            let mut you_players: Vec<String> = s.you_players.into_iter().collect();
            you_players.sort();

            Session {
                name,
                start_line: s.start_line,
                end_line: s.end_line,
                start_time: s.start_time_secs,
                end_time: s.end_time_secs,
                combat_count: s.combat_count,
                duration_secs: duration,
                is_raid: is_raid_zone(zone),
                start_year: s.start_year,
                you_players,
            }
        })
        .collect()
}

/// Extract lines for a specific session using timestamp boundaries.
///
/// Uses the session's `start_time`/`end_time` as the timestamp filter, expanded
/// to include all lines between this session and its neighbors (no gaps, no
/// overlaps).  The `all_sessions` slice provides neighbor timestamps.
///
/// Line ranges (`start_line`/`end_line`) provide a rough file-position hint to
/// avoid scanning the entire file.
pub fn extract_session_lines(
    lines: &[String],
    session: &Session,
    all_sessions: &[Session],
) -> Vec<String> {
    let start = session.start_line.min(lines.len());
    let end = (session.end_line + 1).min(lines.len());
    if start >= end {
        return Vec::new();
    }

    // Compute effective timestamp boundaries using midpoints with neighbors.
    // This ensures no gaps or overlaps between adjacent sessions.
    let idx = all_sessions
        .iter()
        .position(|s| std::ptr::eq(s, session))
        .unwrap_or(0);

    let t_start = if idx > 0 {
        let prev_end = all_sessions[idx - 1].end_time;
        // Midpoint between previous session's end and this session's start
        f64::midpoint(prev_end, session.start_time)
    } else {
        // First session: no lower bound
        f64::NEG_INFINITY
    };

    let t_end = if idx + 1 < all_sessions.len() {
        let next_start = all_sessions[idx + 1].start_time;
        // Midpoint between this session's end and next session's start
        f64::midpoint(session.end_time, next_start)
    } else {
        // Last session: no upper bound
        f64::INFINITY
    };

    let mut result = Vec::new();
    for line in &lines[start..end] {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some((ts, _)) = parse_timestamp_fast(trimmed.as_bytes()) {
            if ts >= t_start && ts <= t_end {
                result.push(line.clone());
            }
        } else {
            result.push(line.clone());
        }
    }
    result
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal combat log snippet with two back-to-back raid sessions
    /// (ES then ZG) that share no time gap.  Verify that session detection
    /// splits them and that `extract_session_lines` assigns lines to the
    /// correct session using timestamps (no cross-contamination).
    #[test]
    fn test_session_split_es_then_zg_no_overlap() {
        let lines: Vec<String> = vec![
            // -- Emerald Sanctum session --
            "1/27 20:00:00.000  ZONE_INFO: 1&Emerald Sanctum&0".into(),
            "1/27 20:00:01.000  COMBATANT_INFO: 27.01.26 20:00:01&Tank&WARRIOR&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".into(),
            "1/27 20:01:00.000  PLAYER_REGEN_DISABLED".into(),
            "1/27 20:01:30.000  Tank hits Solnius for 500.".into(),
            "1/27 20:05:00.000  UNIT_DIED:Solnius:0xF130003A6C000001".into(),
            "1/27 20:05:01.000  PLAYER_REGEN_ENABLED".into(),
            // -- Zul'gurub session (different raid zone -> split) --
            "1/27 20:10:00.000  ZONE_INFO: 1&Zul'Gurub&0".into(),
            "1/27 20:10:01.000  COMBATANT_INFO: 27.01.26 20:10:01&Tank&WARRIOR&Human&2&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&nil&0x0000000000000001&nil".into(),
            "1/27 20:11:00.000  PLAYER_REGEN_DISABLED".into(),
            "1/27 20:11:10.000  Tank hits Ohgan for 400.".into(),
            "1/27 20:14:00.000  UNIT_DIED:Ohgan:0xF130003A6C000002".into(),
            "1/27 20:14:01.000  PLAYER_REGEN_ENABLED".into(),
        ];

        let sessions = detect_sessions(&lines);
        assert!(
            sessions.len() >= 2,
            "Should detect at least 2 sessions, got {}",
            sessions.len()
        );

        // Find the ES and ZG sessions by name
        let es = sessions.iter().find(|s| s.name.contains("Emerald Sanctum"));
        let zg = sessions.iter().find(|s| s.name.contains("Zul'gurub"));
        assert!(es.is_some(), "Should have an Emerald Sanctum session");
        assert!(zg.is_some(), "Should have a Zul'gurub session");

        let es = es.unwrap();
        let zg = zg.unwrap();

        // Extract lines for each session
        let es_lines = extract_session_lines(&lines, es, &sessions);
        let zg_lines = extract_session_lines(&lines, zg, &sessions);

        // ES lines should contain Solnius but not Ohgan
        let es_text = es_lines.join("\n");
        assert!(
            es_text.contains("Solnius"),
            "ES session should contain Solnius"
        );
        assert!(
            !es_text.contains("Ohgan"),
            "ES session should NOT contain Ohgan"
        );

        // ZG lines should contain Ohgan but not Solnius
        let zg_text = zg_lines.join("\n");
        assert!(zg_text.contains("Ohgan"), "ZG session should contain Ohgan");
        assert!(
            !zg_text.contains("Solnius"),
            "ZG session should NOT contain Solnius"
        );
    }

    /// Verify that timestamp-based extraction doesn't produce overlapping data
    /// when two sessions are very close together (10-second gap).
    #[test]
    fn test_session_extract_no_cross_contamination_tight_gap() {
        let lines: Vec<String> = vec![
            // Session A: BWL boss
            "1/27 20:00:00.000  ZONE_INFO: 1&Blackwing Lair&0".into(),
            "1/27 20:00:01.000  PLAYER_REGEN_DISABLED".into(),
            "1/27 20:00:10.000  A_combat_line_session_A".into(),
            "1/27 20:03:00.000  UNIT_DIED:Razorgore the Untamed:0xF130003A6C000001".into(),
            "1/27 20:03:01.000  PLAYER_REGEN_ENABLED".into(),
            // Session B: AQ boss (different instance -> boss split)
            // Only 9 seconds after session A ends
            "1/27 20:03:10.000  ZONE_INFO: 1&Ahn'Qiraj Temple&0".into(),
            "1/27 20:03:11.000  PLAYER_REGEN_DISABLED".into(),
            "1/27 20:03:20.000  B_combat_line_session_B".into(),
            "1/27 20:06:00.000  UNIT_DIED:The Prophet Skeram:0xF130003A6C000002".into(),
            "1/27 20:06:01.000  PLAYER_REGEN_ENABLED".into(),
        ];

        let sessions = detect_sessions(&lines);
        assert!(
            sessions.len() >= 2,
            "Should detect at least 2 sessions, got {}",
            sessions.len()
        );

        let bwl = sessions
            .iter()
            .find(|s| s.name.contains("Blackwing Lair"))
            .expect("Should have BWL session");
        let aq = sessions
            .iter()
            .find(|s| s.name.contains("Ahn'qiraj") || s.name.contains("Ahn'Qiraj"))
            .expect("Should have AQ session");

        let bwl_lines = extract_session_lines(&lines, bwl, &sessions);
        let aq_lines = extract_session_lines(&lines, aq, &sessions);

        let bwl_text = bwl_lines.join("\n");
        let aq_text = aq_lines.join("\n");

        assert!(
            bwl_text.contains("A_combat_line_session_A"),
            "BWL should contain its combat line"
        );
        assert!(
            !bwl_text.contains("B_combat_line_session_B"),
            "BWL should NOT contain AQ's combat line"
        );
        assert!(
            aq_text.contains("B_combat_line_session_B"),
            "AQ should contain its combat line"
        );
        assert!(
            !aq_text.contains("A_combat_line_session_A"),
            "AQ should NOT contain BWL's combat line"
        );
    }

    /// Verify that `detect_sessions` produces sessions with valid
    /// `start_time` / `end_time` fields and that line ranges don't panic.
    #[test]
    fn test_session_timestamps_populated() {
        let lines: Vec<String> = vec![
            "1/27 20:00:00.000  ZONE_INFO: 1&Molten Core&0".into(),
            "1/27 20:00:01.000  PLAYER_REGEN_DISABLED".into(),
            "1/27 20:05:00.000  PLAYER_REGEN_ENABLED".into(),
        ];

        let sessions = detect_sessions(&lines);
        assert_eq!(sessions.len(), 1);
        let s = &sessions[0];
        assert!(s.start_time > 0.0, "start_time should be positive");
        assert!(
            s.end_time >= s.start_time,
            "end_time should be >= start_time"
        );
        assert!(s.start_line <= s.end_line, "line range should be valid");

        // Extracting should not panic and should return lines
        let extracted = extract_session_lines(&lines, s, &sessions);
        assert!(!extracted.is_empty(), "Should extract at least some lines");
    }
}
