//! Database-driven consumable item classification.
//!
//! All data is compiled from `data/consumables.toml` at build time by `build.rs`.
//! The generated tables use sorted slices for O(log n) binary-search lookups.

use crate::log_data::ConsumableCategory;

// Include the build.rs-generated static tables.
include!(concat!(env!("OUT_DIR"), "/consumable_data_generated.rs"));

// ── Public Query API ────────────────────────────────────────────────────────

/// Classify a consumable item name into a category.
///
/// Returns `None` for ignored items (toys, conjured food, etc.).
/// Returns `Some(ConsumableCategory::Other)` for unknown-but-valid consumables.
pub(crate) fn classify(name: &str) -> Option<ConsumableCategory> {
    // Check ignored list first (binary search)
    if IGNORED_ITEMS.binary_search(&name).is_ok() {
        return None;
    }
    // Exact match (binary search on sorted item names)
    if let Ok(idx) = ITEM_TO_CATEGORY.binary_search_by_key(&name, |(k, _)| k) {
        return Some(ConsumableCategory::from_index(ITEM_TO_CATEGORY[idx].1));
    }
    // Prefix match (e.g. "Scroll of " catches unlisted scrolls)
    for &(prefix, cat_idx) in CATEGORY_PREFIXES {
        if name.starts_with(prefix) {
            return Some(ConsumableCategory::from_index(cat_idx));
        }
    }
    // Unknown — still a consumable, just uncategorized
    Some(ConsumableCategory::Other)
}

/// Get the display name for a category (e.g. `Flask` → "Flasks").
pub(crate) fn category_display_name(cat: ConsumableCategory) -> &'static str {
    let idx = cat as usize;
    if idx < CATEGORY_DISPLAY_NAMES.len() {
        CATEGORY_DISPLAY_NAMES[idx]
    } else {
        "Other"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_known_items() {
        assert_eq!(
            classify("Flask of Supreme Power"),
            Some(ConsumableCategory::Flask)
        );
        assert_eq!(
            classify("Elixir of the Mongoose"),
            Some(ConsumableCategory::Elixir)
        );
        assert_eq!(
            classify("Major Healing Potion"),
            Some(ConsumableCategory::Potion)
        );
        assert_eq!(classify("Grilled Squid"), Some(ConsumableCategory::Food));
        assert_eq!(
            classify("Elemental Sharpening Stone"),
            Some(ConsumableCategory::WeaponBuff)
        );
        assert_eq!(classify("Juju Power"), Some(ConsumableCategory::Juju));
        assert_eq!(
            classify("Ground Scorpok Assay"),
            Some(ConsumableCategory::BlastedLands)
        );
        assert_eq!(classify("Spirit of Zanza"), Some(ConsumableCategory::Zanza));
        assert_eq!(
            classify("Scroll of Agility IV"),
            Some(ConsumableCategory::Scroll)
        );
        assert_eq!(
            classify("Goblin Sapper Charge"),
            Some(ConsumableCategory::Engineering)
        );
        assert_eq!(
            classify("Heavy Runecloth Bandage"),
            Some(ConsumableCategory::Bandage)
        );
        assert_eq!(classify("Demonic Rune"), Some(ConsumableCategory::Utility));
    }

    #[test]
    fn test_classify_ignored_items() {
        assert_eq!(classify("MOLL-E, Remote Mail Terminal"), None);
        assert_eq!(classify("Goblin Brainwashing Device"), None);
        assert_eq!(classify("Conjured Mana Orange"), None);
    }

    #[test]
    fn test_classify_unknown_items() {
        assert_eq!(
            classify("Some Unknown Item"),
            Some(ConsumableCategory::Other)
        );
    }

    #[test]
    fn test_classify_prefix_match() {
        // An unlisted scroll should match via the "Scroll of " prefix
        assert_eq!(
            classify("Scroll of Stamina III"),
            Some(ConsumableCategory::Scroll)
        );
    }

    #[test]
    fn test_category_display_names() {
        assert_eq!(category_display_name(ConsumableCategory::Flask), "Flasks");
        assert_eq!(
            category_display_name(ConsumableCategory::WeaponBuff),
            "Weapon Buffs"
        );
        assert_eq!(category_display_name(ConsumableCategory::Other), "Other");
    }
}
