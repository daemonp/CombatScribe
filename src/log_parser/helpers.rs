use crate::log_data::{LogData, PlayerStats};

use super::regex::RE_PET_OWNER;

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Check whether `name` appears in `line` as a whole word, not as a substring
/// of a longer word (e.g. "Garr" should match "Garr hits" but not "Garrote").
///
/// Uses ASCII-aware boundary checks — characters adjacent to the match must be
/// non-alphanumeric (or absent) for the match to count. Apostrophes and spaces
/// are valid boundaries, so `"Garr 's Magma Shackles"` still matches.
pub(super) fn contains_word(line: &str, name: &str) -> bool {
    let bytes = line.as_bytes();
    for (idx, _) in line.match_indices(name) {
        let before_ok = idx == 0 || !bytes[idx - 1].is_ascii_alphanumeric();
        let after_pos = idx + name.len();
        let after_ok = after_pos >= bytes.len() || !bytes[after_pos].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
    }
    false
}

/// Check if a GUID represents a player (starts with `0x0000000000`).
pub(super) fn is_player_guid(guid: &str) -> bool {
    guid.starts_with("0x0000000000")
}

/// Title-case a name (e.g. `"hakkar"` → `"Hakkar"`, `"high priest thekal"` → `"High Priest Thekal"`).
pub(super) fn title_case(s: &str) -> String {
    s.split_whitespace()
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

/// Record absorbed damage on a player target.
pub(super) fn record_absorb(data: &mut LogData, target: &str, absorbed_amount: u64) {
    if absorbed_amount > 0 && data.all_combatants.contains_key(target) {
        *data.absorbs.entry(target.to_string()).or_insert(0) += absorbed_amount;
    }
}

/// Extract pet owner from a source string like `PetName (OwnerName)`.
pub(super) fn extract_pet_owner(source: &str, data: &LogData) -> Option<String> {
    let caps = RE_PET_OWNER.captures(source)?;
    let owner_name = caps.get(2)?.as_str();
    if data.all_combatants.contains_key(owner_name) {
        return Some(owner_name.to_string());
    }
    None
}

/// Ensure a `PlayerStats` entry exists for a name.
pub(super) fn ensure_stats<'a>(data: &'a mut LogData, name: &str) -> &'a mut PlayerStats {
    data.player_stats.entry(name.to_string()).or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contains_word_boundaries() {
        // Exact match at various positions
        assert!(contains_word("Garr hits Tank for 500.", "Garr"));
        assert!(contains_word("Tank hits Garr for 500.", "Garr"));
        assert!(contains_word("Garr", "Garr"));

        // Apostrophe is a valid word boundary (possessive form)
        assert!(contains_word(
            "Garr 's Magma Shackles hits Tank for 100.",
            "Garr"
        ));

        // Embedded in a longer word — must NOT match
        assert!(!contains_word(
            "Scamilla 's Garrote hits Ancient Core Hound for 166.",
            "Garr"
        ));
        assert!(!contains_word(
            "Ancient Core Hound suffers 166 Physical damage from Scamilla 's Garrote.",
            "Garr"
        ));

        // Multi-word boss names
        assert!(contains_word(
            "Baron Geddon hits Tank for 3000 Fire damage.",
            "Baron Geddon"
        ));
        assert!(contains_word(
            "Tank suffers 500 Fire damage from Sulfuron Harbinger 's Shadow Word: Pain.",
            "Sulfuron Harbinger"
        ));
    }
}
