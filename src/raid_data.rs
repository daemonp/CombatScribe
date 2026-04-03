//! Database-driven raid/instance lookup tables.
//!
//! All data is compiled from `data/raids.toml` at build time by `build.rs`.
//! The generated tables use sorted slices for O(log n) binary-search lookups.

// Include the build.rs-generated static tables.
include!(concat!(env!("OUT_DIR"), "/raid_data_generated.rs"));

// ── Public Query API ────────────────────────────────────────────────────────

/// Look up the canonical raid name for any NPC (boss or trash).
///
/// Input is matched case-insensitively (lowercased internally).
/// Returns `None` for NPCs not associated with any raid instance.
pub(crate) fn npc_raid(name: &str) -> Option<&'static str> {
    let lower = to_lower_checked(name)?;
    binary_search_map(NPC_TO_RAID, &lower)
}

/// Look up the canonical raid name for a boss NPC.
///
/// Returns `None` if the name is not a known boss.
pub(crate) fn boss_raid(name: &str) -> Option<&'static str> {
    let lower = to_lower_checked(name)?;
    binary_search_map(BOSS_TO_RAID, &lower)
}

/// Look up the encounter group name for a boss in a multi-boss fight.
///
/// Returns `Some("The Bug Family")` for "Lord Kri", `None` for single-boss encounters.
pub(crate) fn boss_encounter_name(name: &str) -> Option<&'static str> {
    let lower = to_lower_checked(name)?;
    binary_search_map(BOSS_TO_ENCOUNTER, &lower)
}

/// Check if a name is a known boss.
pub(crate) fn is_boss(name: &str) -> bool {
    let Some(lower) = to_lower_checked(name) else {
        return false;
    };
    ALL_BOSSES.binary_search(&&*lower).is_ok()
}

/// Check if a name is any known raid NPC (boss or trash).
#[allow(dead_code)] // Available for NPC-based instance detection
pub(crate) fn is_raid_npc(name: &str) -> bool {
    let Some(lower) = to_lower_checked(name) else {
        return false;
    };
    NPC_TO_RAID
        .binary_search_by_key(&&*lower, |(k, _)| k)
        .is_ok()
}

/// Check if a zone name (post-normalization) is a known instance zone
/// (raid or dungeon).
pub(crate) fn is_raid_zone(zone: &str) -> bool {
    let lower = zone.to_lowercase();
    ALL_RAID_ZONES.binary_search(&&*lower).is_ok()
}

/// Check if a zone name is a 5-man dungeon (not a raid).
pub(crate) fn is_dungeon_zone(zone: &str) -> bool {
    let lower = zone.to_lowercase();
    DUNGEON_ZONES.binary_search(&&*lower).is_ok()
}

/// Get the encounter count for a raid zone.
pub(crate) fn encounter_count(zone: &str) -> Option<usize> {
    let lower = zone.to_lowercase();
    RAID_ENCOUNTER_COUNTS
        .binary_search_by_key(&&*lower, |(k, _)| k)
        .ok()
        .map(|i| RAID_ENCOUNTER_COUNTS[i].1)
}

/// Normalize an addon-reported zone name to its canonical form.
///
/// Applies the zone alias table, returning the canonical name if
/// an alias matches, or the lowercased original otherwise.
pub(crate) fn normalize_zone(zone: &str) -> String {
    let lower = zone.to_lowercase();
    binary_search_map(ZONE_ALIASES, &lower).map_or(lower, str::to_string)
}

/// Check if a zone is a known non-raid overworld zone.
#[allow(dead_code)] // Available for session detection refinement
pub(crate) fn is_overworld_zone(zone: &str) -> bool {
    let lower = zone.to_lowercase();
    OVERWORLD_ZONES.binary_search(&&*lower).is_ok()
}

/// Determine the instance name from a set of boss kills.
///
/// If all boss kills map to the same instance, returns that name.
/// If bosses come from different instances, returns `None`.
pub(crate) fn instance_from_bosses(boss_kills: &[String]) -> Option<&'static str> {
    let mut instance: Option<&str> = None;
    for boss in boss_kills {
        if let Some(inst) = boss_raid(boss) {
            match instance {
                None => instance = Some(inst),
                Some(prev) if prev == inst => {}
                Some(_) => return None,
            }
        }
    }
    instance
}

/// Return the list of all known boss names (original casing not available;
/// these are lowercased) for substring scanning in combat lines.
pub(crate) fn all_boss_names() -> &'static [&'static str] {
    ALL_BOSSES
}

/// Return boss names that belong to a specific raid zone.
///
/// If `zone` is `None` or doesn't match any raid, returns `None` so the
/// caller can fall back to the full boss list.
pub(crate) fn bosses_for_zone(zone: Option<&str>) -> Option<Vec<&'static str>> {
    let zone_raw = zone?;
    // Also resolve aliases so "ahn'qiraj temple" matches "temple of ahn'qiraj", etc.
    let canonical = normalize_zone(zone_raw);
    let zone_lower = zone_raw.to_lowercase();
    let bosses: Vec<&'static str> = BOSS_TO_RAID
        .iter()
        .filter(|(_, raid)| raid.to_lowercase() == canonical || raid.to_lowercase() == zone_lower)
        .map(|(boss, _)| *boss)
        .collect();
    if bosses.is_empty() {
        None
    } else {
        Some(bosses)
    }
}

/// Title-case a zone name for display (e.g. `"molten core"` → `"Molten Core"`).
pub fn format_zone_name(zone: &str) -> String {
    zone.split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |c| {
                let upper: String = c.to_uppercase().collect();
                format!("{upper}{}", chars.as_str())
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ── Internal Helpers ────────────────────────────────────────────────────────

/// Binary search a sorted `&[(&str, &str)]` slice by key, returning the value.
fn binary_search_map(table: &'static [(&str, &str)], key: &str) -> Option<&'static str> {
    table
        .binary_search_by_key(&key, |(k, _)| k)
        .ok()
        .map(|i| table[i].1)
}

/// Lowercase a name with a length guard.
///
/// Returns `None` if the name exceeds 128 bytes (not a valid NPC name),
/// avoiding unnecessary work on long garbage strings.
fn to_lower_checked(name: &str) -> Option<String> {
    // NPC names are all < 64 chars; skip anything longer
    if name.len() > 128 {
        return None;
    }
    Some(name.to_lowercase())
}
