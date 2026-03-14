//! Log formatting engine — applies all replacement rules to combat log lines.
//!
//! Port of the Python `replace_instances` function from `format_log_for_upload.py`.
//! Handles You/Your → player name conversion, pet attribution, apostrophe
//! normalization, mob name handling, self-damage detection, and loot fixes.

use regex::Regex;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use crate::parser::{detect_player_names, extract_ts, get_player_name_for_timestamp};

/// Unicode-aware letter pattern for names (matching the Python `L` pattern).
const L: &str = r",a-zA-Z\u{00C0}-\u{017F}";

fn letter_class() -> String {
    format!("[{L}]")
}

fn letter_class_plus() -> String {
    format!("[{L}]+")
}

fn name_pattern() -> String {
    let l = letter_class();
    format!("{l}[{L} ]+{l}")
}

/// A compiled replacement rule: regex pattern → replacement string.
struct ReplacementRule {
    regex: Regex,
    replacement: String,
}

/// Apply the first matching replacement from a list of rules to a line.
///
/// Returns `Some(modified_line)` if a match was found, `None` otherwise.
/// This avoids allocating a new `String` when no rules match.
///
/// Uses `replace_all` to match Python's `re.subn` which replaces ALL
/// occurrences of the winning pattern within the line.
#[inline]
fn handle_replacements(line: &str, rules: &[ReplacementRule]) -> Option<String> {
    for rule in rules {
        let result = rule.regex.replace_all(line, rule.replacement.as_str());
        // Cow::Owned means a replacement happened — O(1) check vs string comparison
        if matches!(result, Cow::Owned(_)) {
            return Some(result.into_owned());
        }
    }
    None
}

/// Build a `ReplacementRule`, panicking if the regex is malformed (programming error).
fn rule(pattern: &str, replacement: String) -> ReplacementRule {
    ReplacementRule {
        regex: Regex::new(pattern).unwrap_or_else(|e| panic!("bad regex {pattern:?}: {e}")),
        replacement,
    }
}

/// Case-insensitive variant.
fn rule_ci(pattern: &str, replacement: String) -> ReplacementRule {
    rule(&format!("(?i){pattern}"), replacement)
}

/// Build "You" replacement rules for a given player name.
///
/// Faithfully ported from Python `build_replacement_dicts` — all patterns in order.
fn build_you_replacements(player_name: &str) -> Vec<ReplacementRule> {
    let name = capitalize(player_name.trim());

    vec![
        // Remove failed cast/perform lines
        rule_ci(r".*You fail to cast.*\n", String::new()),
        rule_ci(r".*You fail to perform.*\n", String::new()),
        // Self-suffer
        rule_ci(
            r" You suffer (.*?) from your",
            format!(" {name} suffers $1 from {name} (self damage) 's"),
        ),
        // Self-hit
        rule_ci(
            r" Your (.*?) hits you for",
            format!(" {name} (self damage) 's $1 hits {name} for"),
        ),
        // Self parry (legacy 'was' instead of 'is')
        rule_ci(
            r" Your (.*?) is parried by",
            format!(" {name} 's $1 was parried by"),
        ),
        // Your X failed
        rule_ci(r" Your (.*?) failed", format!(" {name} 's $1 fails")),
        // Failed. You are immune
        rule_ci(
            r" failed\. You are immune",
            format!(" fails. {name} is immune"),
        ),
        // Your -> possessive
        rule_ci(r" [Yy]our ", format!(" {name} 's ")),
        // You gain X from Y's -> gains from other player's spell
        rule_ci(
            r" You gain (.*?) from (.*?)'s",
            format!(" {name} gains $1 from $2 's"),
        ),
        // You gain X from -> gains from your own spell
        rule_ci(
            r" You gain (.*?) from ",
            format!(" {name} gains $1 from {name} 's "),
        ),
        // You gain (buff gains)
        rule_ci(" You gain", format!(" {name} gains")),
        rule_ci(" You hit", format!(" {name} hits")),
        rule_ci(" You crit", format!(" {name} crits")),
        rule_ci(" You are", format!(" {name} is")),
        rule_ci(" You suffer", format!(" {name} suffers")),
        rule_ci(" You lose", format!(" {name} loses")),
        rule_ci(" You die", format!(" {name} dies")),
        rule_ci(" You cast", format!(" {name} casts")),
        rule_ci(" You create", format!(" {name} creates")),
        rule_ci(" You perform", format!(" {name} performs")),
        rule_ci(" You interrupt", format!(" {name} interrupts")),
        rule_ci(" You miss", format!(" {name} misses")),
        rule_ci(" You attack", format!(" {name} attacks")),
        rule_ci(" You block", format!(" {name} blocks")),
        rule_ci(" You parry", format!(" {name} parries")),
        rule_ci(" You dodge", format!(" {name} dodges")),
        rule_ci(" You resist", format!(" {name} resists")),
        rule_ci(" You absorb", format!(" {name} absorbs")),
        rule_ci(" You reflect", format!(" {name} reflects")),
        rule_ci(" You receive", format!(" {name} receives")),
        // &You receive (LOOT etc)
        rule_ci("&You receive", format!("&{name} receives")),
        // &You (any remaining)
        rule_ci("&You", format!("&{name}")),
        rule_ci(r" You deflect", format!(" {name} deflects")),
        // Dodged (no 'You' in pattern — SPELLDODGEDOTHERSELF)
        rule_ci(r"was dodged\.", format!("was dodged by {name}.")),
        rule_ci("causes you", format!("causes {name}")),
        rule_ci("heals you", format!("heals {name}")),
        rule_ci("hits you for", format!("hits {name} for")),
        rule_ci("crits you for", format!("crits {name} for")),
        // You have slain
        rule_ci(
            r" You have slain (.*?)!",
            format!(" $1 is slain by {name}."),
        ),
        // non-whitespace before you.
        rule_ci(r"(\S)\syou\.", format!("$1 {name}.")),
        // Fall damage
        rule_ci(r" You fall and lose", format!(" {name} falls and loses")),
    ]
}

/// Mob names with apostrophes — literal string replacements (no regex needed).
const MOB_APOSTROPHE_PAIRS: &[(&str, &str)] = &[
    ("Onyxia's Elite Guard", "Onyxias Elite Guard"),
    ("Sartura's Royal Guard", "Sarturas Royal Guard"),
    ("Medivh's Merlot Blue Label", "Medivhs Merlot Blue Label"),
    (
        "Ima'ghaol, Herald of Desolation",
        "Imaghaol, Herald of Desolation",
    ),
];

/// Build pet replacement rules (case-insensitive to match Python).
fn build_pet_replacements() -> Vec<ReplacementRule> {
    let lp = letter_class_plus();
    let np = name_pattern();

    vec![
        // Pet hits/crits/misses -> Auto Attack (pet)
        rule_ci(
            &format!(r"  ({np}) \(({lp})\) (hits|crits|misses)"),
            "  $2's Auto Attack (pet) $3".to_string(),
        ),
        // Pet dismissed — Python has unescaped `.` but real logs always end with `.`
        rule_ci(
            &format!(r"  Your ({np}) \(({lp})\) is dismissed\."),
            "  $2's $1 ($2) is dismissed.".to_string(),
        ),
        // Pet Arcane Missiles (trinket)
        rule_ci(
            &format!(r"  ({np}) \(({lp})\)('s| 's) Arcane Missiles"),
            "  $2 's Arcane Missiles (pet)".to_string(),
        ),
        // Generic pet ability
        rule_ci(
            &format!(r"  ({np}) \(({lp})\)('s| 's)"),
            "  $2 's".to_string(),
        ),
        // Pet ability from
        rule_ci(
            &format!(r"from ({np}) \(({lp})\)('s| 's)"),
            "from $2$3".to_string(),
        ),
    ]
}

/// Build generic apostrophe-normalization rules (case-insensitive to match Python).
fn build_generic_replacements() -> Vec<ReplacementRule> {
    vec![
        // Lines with fades/gains/afflicted — preserve 's in buff names
        rule_ci(r" fades from .*\.", "$0".to_string()),
        rule_ci(r" gains .*\)\.", "$0".to_string()),
        rule_ci(r" is afflicted by .*\)\.", "$0".to_string()),
        // Handle 's at beginning: [double space][name]'s [Capital]
        rule_ci(
            &format!(r"  ([{L}'\- ]*?\S)'s ([A-Z])"),
            "  $1 's $2".to_string(),
        ),
        // Handle 's after 'from'
        rule_ci(
            &format!(r"from ([{L}'\- ]*?\S)'s ([A-Z])"),
            "from $1 's $2".to_string(),
        ),
        // Handle 's after 'is immune to'
        rule_ci(
            &format!(r"is immune to ([{L}'\- ]*?\S)'s ([A-Z])"),
            "is immune to $1 's $2".to_string(),
        ),
        // Handle 's for pets
        rule_ci(r"\)'s ([A-Z])", ") 's $1".to_string()),
    ]
}

/// Build rename replacement rules (case-insensitive to match Python).
fn build_rename_replacements() -> Vec<ReplacementRule> {
    vec![
        // Totem spells -> shaman credit
        rule_ci(
            &format!(r"  [A-Z][{L} ]* Totem [IVX]+ \((.*?)\) 's"),
            "  $1 's".to_string(),
        ),
        rule_ci(
            &format!(r" from [A-Z][{L} ]* Totem [IVX]+ \((.*?)\) 's"),
            " from $1 's".to_string(),
        ),
        // Lightning Strike nature portion
        rule_ci(
            r"Lightning Strike was resisted",
            "Lightning Strike (nature) was resisted".to_string(),
        ),
        rule_ci(
            r"Lightning Strike (.*) Nature damage",
            "Lightning Strike (nature) $1 Nature damage".to_string(),
        ),
        // Re-add apostrophes for mob names
        rule_ci("Onyxias Elite Guard", "Onyxia's Elite Guard".to_string()),
        rule_ci("Sarturas Royal Guard", "Sartura's Royal Guard".to_string()),
    ]
}

/// Build friendly fire rules (case-insensitive to match Python).
///
/// Python uses `[L]*?` (zero-or-more, lazy) for the name capture.
fn build_friendly_fire_replacements() -> Vec<ReplacementRule> {
    vec![rule_ci(
        &format!(r"from ([{L}]*?) 's Power Overwhelming"),
        "from $1 (self damage) 's Power Overwhelming".to_string(),
    )]
}

/// Build self-damage rules (case-insensitive to match Python).
fn build_self_damage_replacements() -> Vec<ReplacementRule> {
    vec![
        rule_ci(
            &format!(r"  ([{L}' ]*?) suffers (.*) (damage) from ([{L}' ]*?) 's"),
            "  $1 suffers $2 damage from $4 (self damage) 's".to_string(),
        ),
        rule_ci(
            &format!(r"  ([{L}' ]*?) 's (.*) (hits|crits) ([{L}' ]*?) for"),
            "  $1 (self damage) 's $2 $3 $4 for".to_string(),
        ),
    ]
}

/// Capitalize a string matching Python's `str.capitalize()`:
/// uppercase first char, lowercase the rest.
fn capitalize(s: &str) -> String {
    let s = s.trim();
    let mut chars = s.chars();
    chars.next().map_or_else(String::new, |c| {
        let upper: String = c.to_uppercase().collect();
        let rest: String = chars.as_str().to_lowercase();
        format!("{upper}{rest}")
    })
}

/// Known summoned pet names to associate with owners.
const SUMMONED_PET_NAMES: &[&str] = &[
    "Greater Feral Spirit",
    "Battle Chicken",
    "Arcanite Dragonling",
    "The Lost",
    "Minor Arcane Elemental",
    "Scytheclaw Pureborn",
    "Explosive Trap I",
    "Explosive Trap II",
    "Explosive Trap III",
    "Sproutling",
    "Spirit Protector",
];

const IGNORED_PET_NAMES: &[&str] = &[
    "Razorgore the Untamed (",
    "Deathknight Understudy (",
    "Naxxramas Worshipper (",
];

// ── Public API ──────────────────────────────────────────────────────────────

/// Process the log lines: apply all formatting/replacement rules.
///
/// Takes ownership of lines to avoid an extra clone. Returns
/// `(processed_lines, player_names_found)`.
pub fn format_log(mut lines: Vec<String>) -> (Vec<String>, Vec<String>) {
    // Detect player entries
    let player_entries = detect_player_names(&lines);

    // Build per-player you-replacement dicts
    let unique_names: HashSet<String> = player_entries.iter().map(|e| e.name.clone()).collect();
    let mut player_you_rules: HashMap<String, Vec<ReplacementRule>> = HashMap::new();
    for name in &unique_names {
        player_you_rules.insert(name.clone(), build_you_replacements(name));
    }

    let pet_rules = build_pet_replacements();
    let generic_rules = build_generic_replacements();
    let rename_rules = build_rename_replacements();
    let friendly_fire_rules = build_friendly_fire_replacements();
    let self_damage_rules = build_self_damage_replacements();

    let summoned_pet_owner_re = Regex::new(&format!(
        r"({}) \(({})\)",
        name_pattern(),
        letter_class_plus()
    ))
    .expect("known-good summoned pet regex");

    let (pet_rename_rules, owner_names) = first_pass(&mut lines, &summoned_pet_owner_re);
    second_pass(
        &mut lines,
        &player_entries,
        &player_you_rules,
        &pet_rename_rules,
        &owner_names,
        &pet_rules,
        &generic_rules,
        &rename_rules,
        &friendly_fire_rules,
        &self_damage_rules,
    );

    let player_names: Vec<String> = unique_names.into_iter().collect();
    (lines, player_names)
}

// ── Internal passes ─────────────────────────────────────────────────────────

/// First pass: normalize `'s`, collect pet info, handle LOOT and `COMBATANT_INFO` lines.
///
/// Returns `(pet_rename_rules, owner_names)`.
fn first_pass(
    lines: &mut [String],
    summoned_pet_owner_re: &Regex,
) -> (Vec<ReplacementRule>, HashSet<String>) {
    let mut pet_rename_rules: Vec<ReplacementRule> = Vec::new();
    let mut owner_names: HashSet<String> = HashSet::new();

    for line in lines.iter_mut() {
        // DPSMate logs have " 's" already which breaks parsing, remove the space
        if line.contains(" 's") {
            *line = line.replace(" 's", "'s");
        }

        if line.contains("COMBATANT_INFO") {
            // Extract fields we need before mutating the line.
            // Using owned Strings so we don't hold borrows across mutation.
            let parts: Vec<String> = line.split('&').map(str::to_string).collect();
            if parts.len() > 5 {
                let pet_name = &parts[5];
                let owner_name = &parts[1];

                if pet_name != "nil"
                    && !IGNORED_PET_NAMES
                        .iter()
                        .any(|ign| pet_name.starts_with(ign))
                {
                    if pet_name == owner_name {
                        pet_rename_rules.push(rule(
                            &format!(
                                r"{} \({}\)",
                                regex::escape(pet_name),
                                regex::escape(owner_name)
                            ),
                            format!("{pet_name}Pet ({owner_name})"),
                        ));

                        // Update the COMBATANT_INFO line
                        let mut new_parts = parts.clone();
                        new_parts[5] = format!("{pet_name}Pet");
                        *line = new_parts.join("&");
                    }

                    owner_names.insert(format!("({owner_name})"));
                } else {
                    // Remove pet name from uploaded combatant info
                    let mut new_parts = parts;
                    new_parts[5] = "nil".to_string();
                    *line = new_parts.join("&");
                }
            }
        } else if line.contains("LOOT:") {
            // Loot fix: add quantity 1 to loot messages without quantity
            if line.ends_with("|h|r.") {
                let trimmed = &line[..line.len() - "|h|r.".len()];
                *line = format!("{trimmed}|h|rx1.");
            }
        } else {
            for summoned_name in SUMMONED_PET_NAMES {
                if line.contains(summoned_name) {
                    if let Some(caps) = summoned_pet_owner_re.captures(line) {
                        if let Some(owner) = caps.get(2) {
                            owner_names.insert(format!("({})", owner.as_str()));
                        }
                    }
                }
            }
        }
    }

    (pet_rename_rules, owner_names)
}

/// Second pass: apply all replacement rules to every line.
#[allow(clippy::too_many_arguments)]
fn second_pass(
    lines: &mut [String],
    player_entries: &[crate::parser::PlayerEntry],
    player_you_rules: &HashMap<String, Vec<ReplacementRule>>,
    pet_rename_rules: &[ReplacementRule],
    owner_names: &HashSet<String>,
    pet_rules: &[ReplacementRule],
    generic_rules: &[ReplacementRule],
    rename_rules: &[ReplacementRule],
    friendly_fire_rules: &[ReplacementRule],
    self_damage_rules: &[ReplacementRule],
) {
    for line in lines.iter_mut() {
        // Mob names with apostrophes — literal string replacements
        if line.contains('\'') {
            for &(from, to) in MOB_APOSTROPHE_PAIRS {
                if line.contains(from) {
                    *line = line.replace(from, to);
                    break; // only first match
                }
            }
        }

        // Pet renames
        if !pet_rename_rules.is_empty() {
            if let Some(replaced) = handle_replacements(line, pet_rename_rules) {
                *line = replaced;
            }
        }

        // Pet replacements — skip unless an owner name is present
        let has_owner = owner_names
            .iter()
            .any(|owner| line.contains(owner.as_str()));
        if has_owner
            && !line.contains("dies.")
            && !line.contains("is killed by")
            && !IGNORED_PET_NAMES.iter().any(|ign| line.contains(ign))
        {
            if let Some(replaced) = handle_replacements(line, pet_rules) {
                *line = replaced;
            }
        }

        // You/Your replacements — skip if no trigger words
        if line.contains("you") || line.contains("You") || line.contains("dodged.") {
            if let Some(line_ts) = extract_ts(line) {
                if let Some(current_player) = get_player_name_for_timestamp(line_ts, player_entries)
                {
                    if let Some(rules) = player_you_rules.get(current_player) {
                        // Apply once
                        if let Some(replaced) = handle_replacements(line, rules) {
                            *line = replaced;
                        }
                        // Apply twice for self-casting (matches Python behavior)
                        if let Some(replaced) = handle_replacements(line, rules) {
                            *line = replaced;
                        }
                    }
                }
            }
        }

        // Generic replacements — skip if no apostrophe or relevant keywords
        if line.contains('\'')
            || line.contains(" fades from ")
            || line.contains(" gains ")
            || line.contains(" is afflicted by ")
        {
            if let Some(replaced) = handle_replacements(line, generic_rules) {
                *line = replaced;
            }
        }

        // Renames — skip if no relevant keywords
        if line.contains("Totem ")
            || line.contains("Lightning Strike")
            || line.contains("Onyxias")
            || line.contains("Sarturas")
        {
            if let Some(replaced) = handle_replacements(line, rename_rules) {
                *line = replaced;
            }
        }

        // Friendly fire checks — skip if no "Power Overwhelming"
        if line.contains("Power Overwhelming") {
            if let Some(replaced) = handle_replacements(line, friendly_fire_rules) {
                *line = replaced;
            }
        }

        // Self damage checks — skip if no " 's " pattern
        if line.contains(" 's ") {
            for rule in self_damage_rules {
                if let Some(caps) = rule.regex.captures(line) {
                    // Check group 1 == group 4 (player hitting themselves)
                    if let (Some(g1), Some(g4)) = (caps.get(1), caps.get(4)) {
                        if g1.as_str().trim() == g4.as_str().trim() {
                            *line = rule
                                .regex
                                .replace_all(line, rule.replacement.as_str())
                                .into_owned();
                            break;
                        }
                    }
                }
            }
        }
    }
}
