//! Session detection and player name extraction for `WoW` combat logs.
//!
//! Scans log lines to identify raid sessions, boss kills, zone changes,
//! and which player `You`/`Your` refers to at each timestamp.
//!
//! All raid/boss/NPC data comes from `raid_data` (compiled from `data/raids.toml`
//! at build time). No hardcoded boss or zone lists in this file.

use std::collections::HashSet;

use crate::raid_data;

/// Represents a detected session (segment) in the combat log.
#[derive(Debug, Clone)]
pub struct Session {
    pub name: String,
    pub start_line: usize,
    pub end_line: usize,
    pub combat_count: usize,
    pub duration_secs: f64,
    /// Player names detected from `COMBATANT_INFO` with full talent data (2 `}` chars).
    /// These are the names that "You/Your" will be replaced with.
    pub you_players: Vec<String>,
}

impl std::fmt::Display for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let duration = format_duration(self.duration_secs);
        if self.you_players.is_empty() {
            write!(
                f,
                "{} - {} encounters, {duration}",
                self.name, self.combat_count
            )
        } else {
            write!(
                f,
                "{} - {} encounters, {duration} [You: {}]",
                self.name,
                self.combat_count,
                self.you_players.join(", ")
            )
        }
    }
}

/// A player entry detected from `COMBATANT_INFO`.
#[derive(Debug, Clone)]
pub struct PlayerEntry {
    pub timestamp: String,
    pub name: String,
}

/// Session gap threshold (30 minutes in seconds).
const SESSION_GAP_SECS: f64 = 30.0 * 60.0;

// ── Delegates to raid_data ──────────────────────────────────────────────────
// These thin wrappers keep the call sites in this file concise while all
// actual data lives in the build-time-generated raid_data module.

pub(crate) fn is_known_boss(name: &str) -> bool {
    raid_data::is_boss(name)
}

/// Return the known boss name list (lowercased) for substring scanning.
pub(crate) fn known_boss_names() -> &'static [&'static str] {
    raid_data::all_boss_names()
}

fn is_raid_zone(zone: &str) -> bool {
    raid_data::is_raid_zone(zone)
}

fn get_boss_count(zone: &str) -> Option<usize> {
    raid_data::encounter_count(zone)
}

/// Normalize an addon-reported zone name to its canonical form.
pub(crate) fn normalize_zone_name(zone: &str) -> String {
    raid_data::normalize_zone(zone)
}

fn is_overworld_zone(zone: &str) -> bool {
    raid_data::is_overworld_zone(zone)
}

fn instance_from_boss_kills(boss_kills: &[String]) -> Option<&'static str> {
    raid_data::instance_from_bosses(boss_kills)
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn format_duration(seconds: f64) -> String {
    if seconds < 60.0 {
        format!("{}s", seconds as u64)
    } else if seconds < 3600.0 {
        format!("{}m {}s", (seconds / 60.0) as u64, (seconds % 60.0) as u64)
    } else {
        let hours = (seconds / 3600.0) as u64;
        let mins = ((seconds % 3600.0) / 60.0) as u64;
        format!("{hours}h {mins}m")
    }
}

/// Fast manual timestamp parsing — no regex, no allocation.
///
/// Format: `MM/DD HH:MM:SS.mmm  ...`
///
/// Returns `(seconds_value, end_of_timestamp_index)` or `None`.
#[inline]
pub(crate) fn parse_timestamp_fast(line: &[u8]) -> Option<(f64, usize)> {
    // minimum: "1/1 0:0:0.0"
    if line.len() < 11 || !line[0].is_ascii_digit() {
        return None;
    }

    let slash = memchr(b'/', line, 0, 5)?;
    let month = f64::from(parse_int_fast(line, 0, slash)?);

    let space = memchr(b' ', line, slash + 1, slash + 4)?;
    let day = f64::from(parse_int_fast(line, slash + 1, space)?);

    let c1 = memchr(b':', line, space + 1, space + 4)?;
    let hour = f64::from(parse_int_fast(line, space + 1, c1)?);

    let c2 = memchr(b':', line, c1 + 1, c1 + 4)?;
    let min = f64::from(parse_int_fast(line, c1 + 1, c2)?);

    let dot = memchr(b'.', line, c2 + 1, c2 + 4)?;
    let sec = f64::from(parse_int_fast(line, c2 + 1, dot)?);

    // Parse ms digits (variable length, typically 3)
    let mut ms_end = dot + 1;
    while ms_end < line.len() && line[ms_end].is_ascii_digit() {
        ms_end += 1;
    }
    if ms_end == dot + 1 {
        return None;
    }
    let ms = f64::from(parse_int_fast(line, dot + 1, ms_end)?);

    let secs =
        (month * 31.0 + day).mul_add(86400.0, hour * 3600.0) + min * 60.0 + sec + ms / 1000.0;
    Some((secs, ms_end))
}

/// Extract the timestamp substring from a line (for string comparison in formatter).
#[inline]
fn extract_timestamp_str(line: &str) -> Option<&str> {
    let bytes = line.as_bytes();
    parse_timestamp_fast(bytes).map(|(_, end)| &line[..end])
}

#[inline]
fn memchr(needle: u8, haystack: &[u8], start: usize, max_end: usize) -> Option<usize> {
    let end = max_end.min(haystack.len());
    (start..end).find(|&i| haystack[i] == needle)
}

#[inline]
fn parse_int_fast(bytes: &[u8], start: usize, end: usize) -> Option<u32> {
    if start >= end {
        return None;
    }
    let mut val: u32 = 0;
    for &b in &bytes[start..end] {
        if !b.is_ascii_digit() {
            return None;
        }
        val = val * 10 + u32::from(b - b'0');
    }
    Some(val)
}

/// Extract zone name from a `ZONE_INFO` line without regex.
///
/// Format: `...ZONE_INFO: ...&ZoneName&...`
#[inline]
pub(crate) fn extract_zone(line: &str) -> Option<&str> {
    let idx = line.find("ZONE_INFO:")? + "ZONE_INFO:".len();
    let rest = &line[idx..];
    let amp1 = rest.find('&')?;
    let after = &rest[amp1 + 1..];
    let amp2 = after.find('&')?;
    let zone = &after[..amp2];
    if zone.is_empty() {
        None
    } else {
        Some(zone)
    }
}

/// Extract player name and class from `COMBATANT_INFO` without regex.
///
/// Format: `...COMBATANT_INFO: ...&Name&Class&...`
#[inline]
pub(crate) fn extract_combatant(line: &str) -> Option<(&str, &str)> {
    let idx = line.find("COMBATANT_INFO:")? + "COMBATANT_INFO:".len();
    let rest = &line[idx..];
    let amp1 = rest.find('&')?;
    let after1 = &rest[amp1 + 1..];
    let amp2 = after1.find('&')?;
    let name = &after1[..amp2];
    if name.is_empty() || name == "Unknown" {
        return None;
    }
    let after2 = &after1[amp2 + 1..];
    let amp3 = after2.find('&').unwrap_or(after2.len());
    let class = &after2[..amp3];
    Some((name, class))
}

/// Rich combatant info extracted from a `COMBATANT_INFO` line.
///
/// All fields after `&`-split:
/// `date&name&class&race&sex&pet&guild&guild_rank_name&guild_rank_index&gear1..gear19&talents&guid&pet_guid`
pub(crate) struct CombatantInfo<'a> {
    pub name: &'a str,
    pub class: &'a str,
    pub race: &'a str,
    pub pet_name: Option<&'a str>,
    pub guild: Option<&'a str>,
    pub gear: Vec<Option<&'a str>>,
    pub talent_summary: Option<String>,
    pub guid: Option<&'a str>,
}

/// Extract all fields from a `COMBATANT_INFO` line.
///
/// `COMBATANT_INFO` format (31 `&`-delimited fields after the marker):
/// `date & name & class & race & sex & pet & guild & guild_rank_name & guild_rank_index & gear1..gear19 & talents & guid & pet_guid`
///   [0]   [1]    [2]     [3]   [4]  [5]   [6]     [7]              [8]               [9..27]          [28]       [29]  [30]
#[allow(clippy::similar_names)] // guild/guid are domain names from WoW
pub(crate) fn extract_combatant_full(line: &str) -> Option<CombatantInfo<'_>> {
    let idx = line.find("COMBATANT_INFO:")? + "COMBATANT_INFO:".len();
    let rest = &line[idx..];
    let parts: Vec<&str> = rest.split('&').collect();

    // Need at least 29 fields: date(0) + name(1) + class(2) + race(3) + sex(4) + pet(5) +
    // guild(6) + guild_rank_name(7) + guild_rank_index(8) + 19 gear(9-27) + talents(28)
    if parts.len() < 29 {
        return None;
    }

    let name = parts[1].trim();
    if name.is_empty() || name == "Unknown" || name == "nil" {
        return None;
    }

    let class = parts[2].trim();
    let race = parts[3].trim();
    let pet_name = not_nil(parts[5].trim());
    let guild = not_nil(parts[6].trim());

    // Gear slots 1-19 are at indices 9-27
    let mut gear = Vec::with_capacity(19);
    for i in 9..28 {
        if i < parts.len() {
            gear.push(not_nil(parts[i].trim()));
        } else {
            gear.push(None);
        }
    }

    // Talents at index 28
    let talent_summary = if parts.len() > 28 {
        parse_talent_summary(parts[28].trim())
    } else {
        None
    };

    // GUID at index 29
    let guid = if parts.len() > 29 {
        let g = parts[29].trim();
        if g == "nil" || g == "0x0" || g.is_empty() {
            None
        } else {
            Some(g)
        }
    } else {
        None
    };

    Some(CombatantInfo {
        name,
        class,
        race,
        pet_name,
        guild,
        gear,
        talent_summary,
        guid,
    })
}

/// Return `None` for `"nil"` or empty strings, `Some(s)` otherwise.
fn not_nil(s: &str) -> Option<&str> {
    if s.is_empty() || s == "nil" {
        None
    } else {
        Some(s)
    }
}

/// Parse a talent string like `"}05300501001}20501}00530"` into a summary like `"31/20/0"`.
fn parse_talent_summary(raw: &str) -> Option<String> {
    if raw == "nil" || raw.is_empty() || !raw.contains('}') {
        return None;
    }

    let trees: Vec<u32> = raw
        .split('}')
        .filter(|s| !s.is_empty())
        .map(|tree| tree.chars().filter_map(|c| c.to_digit(10)).sum())
        .collect();

    if trees.is_empty() {
        return None;
    }

    let summary = trees
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("/");
    Some(summary)
}

/// Extract dead unit name from `UNIT_DIED` without regex.
///
/// Format: `...UNIT_DIED:UnitName:GUID...`
#[inline]
pub(crate) fn extract_unit_died(line: &str) -> Option<&str> {
    extract_unit_died_with_guid(line).map(|(name, _)| name)
}

/// Extract dead unit name and GUID from `UNIT_DIED` without regex.
///
/// Format: `...UNIT_DIED:UnitName:GUID`
///
/// Returns `(name, guid)`.
#[inline]
pub(crate) fn extract_unit_died_with_guid(line: &str) -> Option<(&str, &str)> {
    let idx = line.find("UNIT_DIED:")? + "UNIT_DIED:".len();
    let rest = &line[idx..];
    let colon = rest.find(':')?;
    let name = &rest[..colon];
    if name.is_empty() {
        return None;
    }
    let after = &rest[colon + 1..];
    // GUID goes until whitespace or end of string
    let guid_end = after
        .find(|c: char| c.is_whitespace())
        .unwrap_or(after.len());
    let guid = &after[..guid_end];
    Some((name, guid))
}

#[derive(Debug, Clone, Copy)]
enum EventType {
    Zone,
    Player,
    /// `COMBATANT_INFO` with full talent info (2 `}` chars) — the "You" player.
    FullPlayer,
    CombatStart,
    CombatEnd,
    BossKill,
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
    let (names, events) = scan_events(lines);

    if events.is_empty() {
        return vec![];
    }

    build_sessions(&names, events)
}

/// First phase: scan all lines and extract structured events.
#[allow(clippy::cast_possible_truncation)] // name_idx will never exceed u32::MAX for real logs
fn scan_events(lines: &[String]) -> (Vec<String>, Vec<ScanEvent>) {
    let mut names: Vec<String> = Vec::new();
    let mut events: Vec<ScanEvent> = Vec::with_capacity(lines.len() / 20);

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
                }
            }
        }
    }

    (names, events)
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
}

/// Determine the established raid instance for a session from its boss kills.
///
/// Returns `Some(instance_name)` if all boss kills so far belong to one instance.
fn session_instance(boss_kills: &[String]) -> Option<&'static str> {
    raid_data::instance_from_bosses(boss_kills)
}

/// Second phase: group sorted events into sessions.
///
/// Session boundaries are determined by:
/// 1. **Time gaps** > 30 minutes between events → new session.
/// 2. **Entering a raid zone** from a non-raid session → new session starts.
/// 3. **Entering a *different* raid zone** while already in a raid → new session.
/// 4. **Boss kill from a different instance** than the current session → new session.
///    This handles back-to-back raids (BWL → AQ40 → Onyxia) without zone events.
///
/// Once a session has a raid zone, it is "sticky" — overworld/city zone blips
/// (e.g. `deadwind pass` between Karazhan boss attempts) do NOT split the session.
/// This is critical because zone events from raid members interleave with
/// overworld zones as players zone in/out.
#[allow(clippy::cast_possible_truncation)] // name_idx indexing
fn build_sessions(names: &[String], mut events: Vec<ScanEvent>) -> Vec<Session> {
    events.sort_unstable_by(|a, b| a.timestamp_secs.total_cmp(&b.timestamp_secs));

    let mut sessions: Vec<SessionBuilder> = Vec::new();
    let mut current: Option<SessionBuilder> = None;

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
                    // and this new boss is from a DIFFERENT instance → split
                    if let Some(cur_inst) = session_instance(&cur.boss_kills) {
                        return cur_inst != boss_inst;
                    }
                    // If the session has a raid zone set and boss is from a different raid → split
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
                            // Entering a raid zone from a non-raid session → split
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
            current = Some(SessionBuilder {
                start_time_secs: event.timestamp_secs,
                end_time_secs: event.timestamp_secs,
                start_line: event.line_index,
                end_line: event.line_index,
                primary_zone: None,
                players: HashSet::new(),
                you_players: HashSet::new(),
                combat_count: 0,
                boss_kills: Vec::new(),
            });
        }

        let cur = current
            .as_mut()
            .expect("current always Some after session init");
        cur.end_time_secs = event.timestamp_secs;
        cur.end_line = event.line_index;

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
                // COMBATANT_INFO fires for ALL nearby players (cities, world),
                // not just raid/party members, so this count is inherently noisy.
                // We still collect the set for potential future use but don't
                // display it as a reliable "player count".
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
        }
    }

    if let Some(session) = current.take() {
        sessions.push(session);
    }

    finalize_sessions(sessions)
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

            // Use boss-derived zone if:
            //  - Boss kills unambiguously identify an instance, AND
            //  - The ZONE_INFO zone is not a raid OR is an overworld zone
            //    (i.e., boss mapping overrides non-raid/overworld zones but
            //     doesn't override a correct raid zone)
            let zone = if let Some(bz) = boss_zone {
                if !is_raid_zone(zone_info) || is_overworld_zone(zone_info) {
                    bz
                } else {
                    zone_info
                }
            } else {
                zone_info
            };

            let zone_display = raid_data::format_zone_name(zone);

            let name = if is_raid_zone(zone) {
                let total_bosses = get_boss_count(zone).unwrap_or(0);
                let kill_count = s.boss_kills.len();
                if total_bosses > 0 && kill_count > 0 {
                    if kill_count >= total_bosses {
                        format!("{zone_display} Full Clear")
                    } else {
                        format!("{zone_display} ({kill_count}/{total_bosses})")
                    }
                } else if s.combat_count > 0 && kill_count == 0 {
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
                combat_count: s.combat_count,
                duration_secs: duration,
                you_players,
            }
        })
        .collect()
}

pub fn format_zone_name(zone: &str) -> String {
    raid_data::format_zone_name(zone)
}

/// Extract the "You" player name from `COMBATANT_INFO` by splitting on `&`.
///
/// The player name is at index 1 in the `&`-delimited fields.
#[inline]
fn extract_you_player_name(line: &str) -> Option<&str> {
    let mut splits = line.splitn(3, '&');
    splits.next()?; // before first &
    let name = splits.next()?;
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// Count occurrences of a byte in a byte slice.
#[inline]
#[allow(clippy::naive_bytecount)] // Lines are short; SIMD overhead not worthwhile
fn bytecount_char(bytes: &[u8], needle: u8) -> usize {
    bytes.iter().filter(|&&b| b == needle).count()
}

/// Detect player names from `COMBATANT_INFO` lines with full talent info (2 `}` chars).
///
/// Used by the formatter to know which player "You" refers to at each timestamp.
pub fn detect_player_names(lines: &[String]) -> Vec<PlayerEntry> {
    let mut entries = Vec::new();

    for line in lines {
        if !line.contains("COMBATANT_INFO") {
            continue;
        }
        if bytecount_char(line.as_bytes(), b'}') != 2 {
            continue;
        }

        if let Some(ts) = extract_timestamp_str(line) {
            let parts: Vec<&str> = line.splitn(3, '&').collect();
            if parts.len() >= 2 && !parts[1].is_empty() {
                entries.push(PlayerEntry {
                    timestamp: ts.to_string(),
                    name: parts[1].to_string(),
                });
            }
        }
    }

    entries
}

/// Get the player name that applies for a given timestamp.
pub fn get_player_name_for_timestamp<'a>(
    timestamp: &str,
    player_entries: &'a [PlayerEntry],
) -> Option<&'a str> {
    if player_entries.is_empty() {
        return None;
    }

    let mut current_player = &player_entries[0].name;
    for entry in player_entries {
        if entry.timestamp.as_str() <= timestamp {
            current_player = &entry.name;
        } else {
            break;
        }
    }
    Some(current_player)
}

/// Extract the timestamp substring from a line (public for formatter use).
pub fn extract_ts(line: &str) -> Option<&str> {
    extract_timestamp_str(line)
}

/// Extract lines for a specific session.
pub fn extract_session_lines(lines: &[String], session: &Session) -> Vec<String> {
    let start = session.start_line.min(lines.len());
    let end = (session.end_line + 1).min(lines.len());
    lines[start..end].to_vec()
}
