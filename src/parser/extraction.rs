//! Line extraction and combatant parsing for `WoW` combat log lines.

use super::timestamp::extract_timestamp_str;

// ── Public Types ────────────────────────────────────────────────────────────

/// A player entry detected from `COMBATANT_INFO`.
#[derive(Debug, Clone)]
pub struct PlayerEntry {
    pub timestamp: String,
    pub name: String,
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

// ── Zone Extraction ─────────────────────────────────────────────────────────

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
    if zone.is_empty() { None } else { Some(zone) }
}

// ── Combatant Extraction ────────────────────────────────────────────────────

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

/// Extract the calendar year from a `COMBATANT_INFO` date field.
///
/// The date is at `parts[0]` (before the first `&`), format: `DD.MM.YY HH:MM:SS`.
/// Returns the full 4-digit year (e.g. 2026 from `YY=26`).
pub(super) fn extract_combatant_year(line: &str) -> Option<i32> {
    let idx = line.find("COMBATANT_INFO:")? + "COMBATANT_INFO:".len();
    let rest = &line[idx..];
    let amp = rest.find('&')?;
    let date_field = rest[..amp].trim();
    // Expected: "DD.MM.YY ..." — the year is at bytes 6..8
    if date_field.len() < 8 {
        return None;
    }
    let yy: i32 = date_field[6..8].parse().ok()?;
    Some(if yy >= 90 { 1900 + yy } else { 2000 + yy })
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

// ── Unit Died Extraction ────────────────────────────────────────────────────

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

// ── Player Name Detection ───────────────────────────────────────────────────

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
///
/// Uses binary search (`partition_point`) since entries are sorted by timestamp.
pub fn get_player_name_for_timestamp<'a>(
    timestamp: &str,
    player_entries: &'a [PlayerEntry],
) -> Option<&'a str> {
    if player_entries.is_empty() {
        return None;
    }

    // partition_point returns the first index where the predicate is false,
    // i.e. the first entry with timestamp > our target. We want the one before it.
    let idx = player_entries.partition_point(|e| e.timestamp.as_str() <= timestamp);
    if idx == 0 {
        // All entries are after this timestamp — use the first player
        Some(&player_entries[0].name)
    } else {
        Some(&player_entries[idx - 1].name)
    }
}

/// Extract the timestamp substring from a line (public for formatter use).
pub fn extract_ts(line: &str) -> Option<&str> {
    extract_timestamp_str(line)
}

/// Extract the "You" player name from `COMBATANT_INFO` by splitting on `&`.
///
/// The player name is at index 1 in the `&`-delimited fields.
#[inline]
pub(super) fn extract_you_player_name(line: &str) -> Option<&str> {
    let mut splits = line.splitn(3, '&');
    splits.next()?; // before first &
    let name = splits.next()?;
    if name.is_empty() { None } else { Some(name) }
}

/// Count occurrences of a byte in a byte slice.
#[inline]
#[allow(clippy::naive_bytecount)] // Lines are short; SIMD overhead not worthwhile
pub(super) fn bytecount_char(bytes: &[u8], needle: u8) -> usize {
    bytes.iter().filter(|&&b| b == needle).count()
}
