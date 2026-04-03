//! Log formatting engine — applies all replacement rules to combat log lines.
//!
//! Port of the Python `replace_instances` function from `format_log_for_upload.py`.
//! Handles You/Your → player name conversion, pet attribution, apostrophe
//! normalization, mob name handling, self-damage detection, and loot fixes.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use rayon::prelude::*;
use regex::Regex;

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

/// A compiled replacement rule: either a regex or a literal string match.
enum ReplacementRule {
    /// Regex-based replacement for complex patterns with capture groups.
    Regex {
        regex: Regex,
        replacement: String,
        /// Optional cheap keyword pre-check: skip regex unless line contains this.
        keyword: Option<&'static str>,
    },
    /// Simple literal string replacement (no regex overhead).
    Literal { find: &'static str, replace: String },
}

/// Apply the first matching replacement from a list of rules to a line.
///
/// Returns `Some(modified_line)` if a match was found, `None` otherwise.
/// This avoids allocating a new `String` when no rules match.
#[inline]
fn handle_replacements(line: &str, rules: &[ReplacementRule]) -> Option<String> {
    for rule in rules {
        match rule {
            ReplacementRule::Literal { find, replace } => {
                if line.contains(find) {
                    return Some(line.replace(find, replace));
                }
            }
            ReplacementRule::Regex {
                regex,
                replacement,
                keyword,
            } => {
                if let Some(kw) = keyword
                    && !line.contains(kw)
                {
                    continue;
                }
                if regex.is_match(line) {
                    return Some(regex.replace_all(line, replacement.as_str()).into_owned());
                }
            }
        }
    }
    None
}

/// Build a regex `ReplacementRule`, panicking if the regex is malformed (programming error).
fn rule(pattern: &str, replacement: String) -> ReplacementRule {
    ReplacementRule::Regex {
        regex: Regex::new(pattern).unwrap_or_else(|e| panic!("bad regex {pattern:?}: {e}")),
        replacement,
        keyword: None,
    }
}

/// Case-insensitive variant.
fn rule_ci(pattern: &str, replacement: String) -> ReplacementRule {
    rule(&format!("(?i){pattern}"), replacement)
}

/// Case-sensitive regex rule with a cheap keyword pre-check.
fn rule_kw(pattern: &str, replacement: String, keyword: &'static str) -> ReplacementRule {
    ReplacementRule::Regex {
        regex: Regex::new(pattern).unwrap_or_else(|e| panic!("bad regex {pattern:?}: {e}")),
        replacement,
        keyword: Some(keyword),
    }
}

/// Case-insensitive regex rule with a cheap keyword pre-check.
fn rule_ci_kw(pattern: &str, replacement: String, keyword: &'static str) -> ReplacementRule {
    ReplacementRule::Regex {
        regex: Regex::new(&format!("(?i){pattern}"))
            .unwrap_or_else(|e| panic!("bad regex {pattern:?}: {e}")),
        replacement,
        keyword: Some(keyword),
    }
}

/// Literal string replacement (no regex, fastest path).
fn rule_literal(find: &'static str, replace: String) -> ReplacementRule {
    ReplacementRule::Literal { find, replace }
}

/// Build "You" replacement rules for a given player name.
///
/// Faithfully ported from Python `build_replacement_dicts` — all patterns in order.
fn build_you_replacements(player_name: &str) -> Vec<ReplacementRule> {
    let name = capitalize(player_name.trim());

    vec![
        // Remove failed cast/perform lines
        rule_kw(r".*You fail to cast.*\n", String::new(), "fail to cast"),
        rule_kw(
            r".*You fail to perform.*\n",
            String::new(),
            "fail to perform",
        ),
        // Self-suffer
        rule_kw(
            r" You suffer (.*?) from your",
            format!(" {name} suffers $1 from {name} (self damage) 's"),
            "suffer",
        ),
        // Self-hit
        rule_kw(
            r" Your (.*?) hits you for",
            format!(" {name} (self damage) 's $1 hits {name} for"),
            "hits you for",
        ),
        // Self parry (legacy 'was' instead of 'is')
        rule_kw(
            r" Your (.*?) is parried by",
            format!(" {name} 's $1 was parried by"),
            "parried by",
        ),
        // Your X failed
        rule_kw(
            r" Your (.*?) failed",
            format!(" {name} 's $1 fails"),
            "failed",
        ),
        // Failed. You are immune
        rule_kw(
            r" failed\. You are immune",
            format!(" fails. {name} is immune"),
            "You are immune",
        ),
        // Your -> possessive (very common — matches most lines with "your"/"Your")
        rule_kw(r" [Yy]our ", format!(" {name} 's "), "our "),
        // You gain X from Y's -> gains from other player's spell
        rule_kw(
            r" You gain (.*?) from (.*?)'s",
            format!(" {name} gains $1 from $2 's"),
            "You gain",
        ),
        // You gain X from -> gains from your own spell
        rule_kw(
            r" You gain (.*?) from ",
            format!(" {name} gains $1 from {name} 's "),
            "You gain",
        ),
        // You gain (buff gains) — literal replacements (no regex needed)
        rule_literal(" You gain", format!(" {name} gains")),
        rule_literal(" You hit", format!(" {name} hits")),
        rule_literal(" You crit", format!(" {name} crits")),
        rule_literal(" You are", format!(" {name} is")),
        rule_literal(" You suffer", format!(" {name} suffers")),
        rule_literal(" You lose", format!(" {name} loses")),
        rule_literal(" You die", format!(" {name} dies")),
        rule_literal(" You cast", format!(" {name} casts")),
        rule_literal(" You create", format!(" {name} creates")),
        rule_literal(" You perform", format!(" {name} performs")),
        rule_literal(" You interrupt", format!(" {name} interrupts")),
        rule_literal(" You miss", format!(" {name} misses")),
        rule_literal(" You attack", format!(" {name} attacks")),
        rule_literal(" You block", format!(" {name} blocks")),
        rule_literal(" You parry", format!(" {name} parries")),
        rule_literal(" You dodge", format!(" {name} dodges")),
        rule_literal(" You resist", format!(" {name} resists")),
        rule_literal(" You absorb", format!(" {name} absorbs")),
        rule_literal(" You reflect", format!(" {name} reflects")),
        rule_literal(" You receive", format!(" {name} receives")),
        // &You receive (LOOT etc)
        rule_literal("&You receive", format!("&{name} receives")),
        // &You (any remaining)
        rule_literal("&You", format!("&{name}")),
        rule_literal(" You deflect", format!(" {name} deflects")),
        // Dodged (no 'You' in pattern — SPELLDODGEDOTHERSELF)
        rule_literal("was dodged.", format!("was dodged by {name}.")),
        rule_literal("causes you", format!("causes {name}")),
        rule_literal("heals you", format!("heals {name}")),
        rule_literal("hits you for", format!("hits {name} for")),
        rule_literal("crits you for", format!("crits {name} for")),
        // You have slain — needs regex for capture group
        rule_kw(
            r" You have slain (.*?)!",
            format!(" $1 is slain by {name}."),
            "You have slain",
        ),
        // non-whitespace before you. — needs regex for capture group
        rule_kw(r"(\S)\syou\.", format!("$1 {name}."), "you."),
        // Fall damage
        rule_literal(" You fall and lose", format!(" {name} falls and loses")),
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
///
/// These match the Python `format_log_for_upload.py` output exactly.
/// The `(pet)` tag is only applied to auto-attacks and Arcane Missiles here;
/// other pet spells are tagged by the interstitial `tag_pet_spells()` step.
fn build_pet_replacements() -> Vec<ReplacementRule> {
    let lp = letter_class_plus();
    let np = name_pattern();

    vec![
        // Pet hits/crits/misses -> Auto Attack (pet)
        // Python: r"  \g<2>'s Auto Attack (pet) \g<3>"
        // Uses $2's (no space) — the generic normalization pass adds the space later.
        rule_ci(
            &format!(r"  ({np}) \(({lp})\) (hits|crits|misses)"),
            "  $2's Auto Attack (pet) $3".to_string(),
        ),
        // Pet dismissed — Python has unescaped `.` but real logs always end with `.`
        rule_ci(
            &format!(r"  Your ({np}) \(({lp})\) is dismissed\."),
            "  $2's $1 ($2) is dismissed.".to_string(),
        ),
        // Pet Arcane Missiles (trinket) — (pet) after spell name
        // Python: r"  \g<2> 's Arcane Missiles (pet)"
        rule_ci(
            &format!(r"  ({np}) \(({lp})\)('s| 's) Arcane Missiles"),
            "  $2 's Arcane Missiles (pet)".to_string(),
        ),
        // Generic pet ability — NO (pet) tag, matches Python reference.
        // Pet spells will be tagged by tag_pet_spells() interstitial step.
        // Python: r"  \g<2> 's"
        rule_ci(
            &format!(r"  ({np}) \(({lp})\)('s| 's)"),
            "  $2 's".to_string(),
        ),
        // Pet ability "from" — NO (pet) tag, matches Python reference.
        // Python: r"from \g<2>\g<3>"
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
        rule_ci_kw(r" fades from .*\.", "$0".to_string(), " fades from "),
        rule_ci_kw(r" gains .*\)\.", "$0".to_string(), " gains "),
        rule_ci_kw(
            r" is afflicted by .*\)\.",
            "$0".to_string(),
            " is afflicted by ",
        ),
        // Handle 's at beginning: [double space][name]'s [Capital]
        rule_ci_kw(
            &format!(r"  ([{L}'\- ]*?\S)'s ([A-Z])"),
            "  $1 's $2".to_string(),
            "'s ",
        ),
        // Handle 's after 'from'
        rule_ci_kw(
            &format!(r"from ([{L}'\- ]*?\S)'s ([A-Z])"),
            "from $1 's $2".to_string(),
            "'s ",
        ),
        // Handle 's after 'is immune to'
        rule_ci_kw(
            &format!(r"is immune to ([{L}'\- ]*?\S)'s ([A-Z])"),
            "is immune to $1 's $2".to_string(),
            "is immune to",
        ),
        // Handle 's for pets
        rule_ci_kw(r"\)'s ([A-Z])", ") 's $1".to_string(), ")'s "),
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

/// Fallback pet-only spells for owners whose pets have no CAST lines in the log.
/// Only activated for players who have a pet in `COMBATANT_INFO`.
const KNOWN_PET_SPELLS: &[&str] = &[
    // Hunter pets
    "Bite",
    "Claw",
    "Screech",
    "Savage Rend",
    "Growl",
    "Dash",
    "Dive",
    "Boar Charge",
    "Lightning Breath",
    "Scorpid Poison",
    "Fire Shield",
    "Furious Howl",
    "Gore",
    // Warlock pets
    "Firebolt",
    "Lash of Pain",
    "Torment",
    "Cleave",
    "Soothing Kiss",
    "Blood Pact",
    "Phase Shift",
    // Summoned entities
    "Explosive Trap Effect",
];

/// Pet data collected during `first_pass()` for the interstitial `tag_pet_spells()` step.
#[derive(Debug)]
struct PetInfo {
    /// Map: owner name → set of pet names for that owner (from `COMBATANT_INFO`).
    owner_pets: HashMap<String, HashSet<String>>,
    /// Map: pet name → set of spell names cast by that pet (from `CAST:` lines).
    pet_spells: HashMap<String, HashSet<String>>,
}

/// Regex for extracting caster and spell name from `CAST:` lines.
///
/// Supports multi-word caster names (e.g. "Greater Feral Spirit") and all
/// three verb forms (`casts`, `begins to cast`, `fails casting`).
static RE_CAST_EXTRACT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"CAST:\s*([A-Za-z]+(?:\s[A-Za-z]+)*)\s+(?:casts|begins to cast|fails casting)\s+([A-Za-z][A-Za-z '\-]+?)(?:\(\d+\))+",
    )
    .expect("known-good CAST extraction regex")
});

// ── Public API ──────────────────────────────────────────────────────────────

/// All pre-compiled replacement rule sets used by the second pass.
struct FormatterRules {
    pet: Vec<ReplacementRule>,
    generic: Vec<ReplacementRule>,
    rename: Vec<ReplacementRule>,
    friendly_fire: Vec<ReplacementRule>,
    self_damage: Vec<ReplacementRule>,
}

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

    let rules = FormatterRules {
        pet: build_pet_replacements(),
        generic: build_generic_replacements(),
        rename: build_rename_replacements(),
        friendly_fire: build_friendly_fire_replacements(),
        self_damage: build_self_damage_replacements(),
    };

    let summoned_pet_owner_re = Regex::new(&format!(
        r"({}) \(({})\)",
        name_pattern(),
        letter_class_plus()
    ))
    .expect("known-good summoned pet regex");

    let (pet_rename_rules, owner_names, pet_info) = first_pass(&mut lines, &summoned_pet_owner_re);
    second_pass(
        &mut lines,
        &player_entries,
        &player_you_rules,
        &pet_rename_rules,
        &owner_names,
        &rules,
    );

    // Interstitial step: tag pet spells with (pet) using data from COMBATANT_INFO + CAST lines
    let owner_pet_spells = build_owner_pet_spells(&pet_info);
    if !owner_pet_spells.is_empty() {
        tag_pet_spells(&mut lines, &owner_pet_spells);
    }

    let player_names: Vec<String> = unique_names.into_iter().collect();
    (lines, player_names)
}

// ── Internal passes ─────────────────────────────────────────────────────────

/// First pass: normalize `'s`, collect pet info, handle LOOT and `COMBATANT_INFO` lines.
///
/// Returns `(pet_rename_rules, owner_names, pet_info)`.
fn first_pass(
    lines: &mut [String],
    summoned_pet_owner_re: &Regex,
) -> (Vec<ReplacementRule>, HashSet<String>, PetInfo) {
    let mut pet_rename_rules: Vec<ReplacementRule> = Vec::new();
    let mut owner_names: HashSet<String> = HashSet::new();
    let mut owner_pets: HashMap<String, HashSet<String>> = HashMap::new();
    let mut all_pet_names: HashSet<String> = HashSet::new();
    let mut pet_spells: HashMap<String, HashSet<String>> = HashMap::new();

    for line in lines.iter_mut() {
        // DPSMate logs have " 's" already which breaks parsing, remove the space.
        // In-place removal avoids a new String allocation per matching line.
        while let Some(pos) = line.find(" 's") {
            line.replace_range(pos..=pos, "");
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

                        // Track renamed pet
                        let renamed = format!("{pet_name}Pet");
                        owner_pets
                            .entry(owner_name.clone())
                            .or_default()
                            .insert(renamed.clone());
                        all_pet_names.insert(renamed);
                    } else {
                        // Track normal pet → owner mapping
                        owner_pets
                            .entry(owner_name.clone())
                            .or_default()
                            .insert(pet_name.clone());
                        all_pet_names.insert(pet_name.clone());
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
            if let Some(trimmed) = line.strip_suffix("|h|r.") {
                *line = format!("{trimmed}|h|rx1.");
            }
        } else {
            // Extract pet spell names from CAST lines (e.g. "CAST: Wolf casts Bite(17261)").
            // CAST lines have no possessive `'s` so the normalization above doesn't affect them.
            // We check `all_pet_names` which may still be empty on early lines, but CAST lines
            // from pets always appear after the COMBATANT_INFO that registers the pet.
            if line.contains("CAST:")
                && let Some(caps) = RE_CAST_EXTRACT.captures(line)
            {
                let caster = caps.get(1).map_or("", |m| m.as_str());
                let spell = caps.get(2).map_or("", |m| m.as_str().trim());
                if all_pet_names.contains(caster) && !spell.is_empty() {
                    pet_spells
                        .entry(caster.to_string())
                        .or_default()
                        .insert(spell.to_string());
                }
            }

            for summoned_name in SUMMONED_PET_NAMES {
                if line.contains(summoned_name)
                    && let Some(caps) = summoned_pet_owner_re.captures(line)
                    && let Some(owner) = caps.get(2)
                {
                    owner_names.insert(format!("({})", owner.as_str()));
                }
            }
        }
    }

    let pet_info = PetInfo {
        owner_pets,
        pet_spells,
    };

    (pet_rename_rules, owner_names, pet_info)
}

/// Second pass: apply all replacement rules to every line (parallelized with rayon).
fn second_pass(
    lines: &mut [String],
    player_entries: &[crate::parser::PlayerEntry],
    player_you_rules: &HashMap<String, Vec<ReplacementRule>>,
    pet_rename_rules: &[ReplacementRule],
    owner_names: &HashSet<String>,
    rules: &FormatterRules,
) {
    lines.par_iter_mut().for_each(|line| {
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
        if !pet_rename_rules.is_empty()
            && let Some(replaced) = handle_replacements(line, pet_rename_rules)
        {
            *line = replaced;
        }

        // Pet replacements — skip unless an owner name is present
        // Quick pre-check: all owner names are "(Name)" format, so '(' must be present
        let has_owner = line.contains('(')
            && owner_names
                .iter()
                .any(|owner| line.contains(owner.as_str()));
        if has_owner
            && !line.contains("dies.")
            && !line.contains("is killed by")
            && !IGNORED_PET_NAMES.iter().any(|ign| line.contains(ign))
            && let Some(replaced) = handle_replacements(line, &rules.pet)
        {
            *line = replaced;
        }

        // You/Your replacements — skip if no trigger words
        if (line.contains("you") || line.contains("You") || line.contains("dodged."))
            && let Some(line_ts) = extract_ts(line)
            && let Some(current_player) = get_player_name_for_timestamp(line_ts, player_entries)
            && let Some(rules) = player_you_rules.get(current_player)
        {
            // Apply once
            if let Some(replaced) = handle_replacements(line, rules) {
                *line = replaced;
            }
            // Apply twice for self-casting (matches Python behavior)
            if let Some(replaced) = handle_replacements(line, rules) {
                *line = replaced;
            }
        }

        // Generic replacements — skip if no apostrophe or relevant keywords
        if (line.contains('\'')
            || line.contains(" fades from ")
            || line.contains(" gains ")
            || line.contains(" is afflicted by "))
            && let Some(replaced) = handle_replacements(line, &rules.generic)
        {
            *line = replaced;
        }

        // Renames — skip if no relevant keywords
        if (line.contains("Totem ")
            || line.contains("Lightning Strike")
            || line.contains("Onyxias")
            || line.contains("Sarturas"))
            && let Some(replaced) = handle_replacements(line, &rules.rename)
        {
            *line = replaced;
        }

        // Friendly fire checks — skip if no "Power Overwhelming"
        if line.contains("Power Overwhelming")
            && let Some(replaced) = handle_replacements(line, &rules.friendly_fire)
        {
            *line = replaced;
        }

        // Self damage checks — skip if no " 's " pattern
        if line.contains(" 's ") {
            for rule in &rules.self_damage {
                if let ReplacementRule::Regex {
                    regex, replacement, ..
                } = rule
                    && let Some(caps) = regex.captures(line)
                {
                    // Check group 1 == group 4 (player hitting themselves)
                    if let (Some(g1), Some(g4)) = (caps.get(1), caps.get(4))
                        && g1.as_str().trim() == g4.as_str().trim()
                    {
                        *line = regex.replace_all(line, replacement.as_str()).into_owned();
                        break;
                    }
                }
            }
        }
    });
}

// ── Pet Spell Tagging (Interstitial Step) ───────────────────────────────────

/// Build the owner → pet spell set from collected `PetInfo`.
///
/// Unions all pet spell sets for each owner and adds "Auto Attack" (every pet
/// can melee). Falls back to `KNOWN_PET_SPELLS` for owners whose pets have
/// no observed CAST lines.
fn build_owner_pet_spells(pet_info: &PetInfo) -> HashMap<String, HashSet<String>> {
    let mut result: HashMap<String, HashSet<String>> = HashMap::new();

    for (owner, pet_names) in &pet_info.owner_pets {
        let spells = result.entry(owner.clone()).or_default();

        // Every pet can auto-attack
        spells.insert("Auto Attack".to_string());

        let mut has_cast_data = false;
        for pet_name in pet_names {
            if let Some(pet_spell_set) = pet_info.pet_spells.get(pet_name) {
                has_cast_data = true;
                spells.extend(pet_spell_set.iter().cloned());
            }
        }

        // Fallback: if no CAST lines found for any of this owner's pets,
        // use the known pet-only spells as a safety net.  These are spells
        // exclusive to pet classes — a Hunter can't personally cast Bite,
        // a Warlock can't personally cast Firebolt, etc.
        if !has_cast_data {
            spells.extend(KNOWN_PET_SPELLS.iter().map(|s| (*s).to_string()));
        }
    }

    result
}

/// Interstitial step: tag pet spells with `(pet)` in formatted lines.
///
/// Runs after `second_pass()` completes. For each line containing `" 's "`,
/// checks if the spell name belongs to a known pet owner's pet spell set.
/// If so, inserts `(pet) ` before the spell name.
///
/// Also normalizes `Auto Attack (pet)` (Python format, tag after spell) to
/// `(pet) Auto Attack` (our internal format, tag before spell) so that
/// `RE_DMG_SPELL` matches correctly.
fn tag_pet_spells(lines: &mut [String], owner_pet_spells: &HashMap<String, HashSet<String>>) {
    lines.par_iter_mut().for_each(|line| {
        // Skip lines that can't contain pet damage attribution
        if !line.contains(" 's ") {
            return;
        }
        // Skip lines that already have a (pet) tag in the right position
        if line.contains(" 's (pet) ") {
            return;
        }

        // Step 1: Normalize "Auto Attack (pet)" position.
        // Python format: `Owner 's Auto Attack (pet) hits`
        // Our parser expects: `Owner 's (pet) Auto Attack hits`
        if line.contains("Auto Attack (pet)") {
            *line = line.replace("Auto Attack (pet)", "(pet) Auto Attack");
            return;
        }

        // Step 2: Normalize "Arcane Missiles (pet)" position.
        // Python format: `Owner 's Arcane Missiles (pet)`
        // Our parser expects: `Owner 's (pet) Arcane Missiles`
        if line.contains("Arcane Missiles (pet)") {
            *line = line.replace("Arcane Missiles (pet)", "(pet) Arcane Missiles");
            return;
        }

        // Step 3: Tag pet spells that don't have (pet) yet.
        // Find the owner name by looking for `Owner 's ` pattern.
        // The timestamp's double-space separator anchors the owner name position.
        // DoT lines have `from Owner 's `.
        tag_pet_spell_in_line(line, owner_pet_spells);
    });
}

/// Try to find an owner name and spell name in a line, and insert `(pet)` if
/// the spell belongs to that owner's pet spell set.
///
/// Handles two formats:
/// - `  Owner 's SpellName hits/crits/misses Target for N`
/// - `Target suffers N damage from Owner 's SpellName`
fn tag_pet_spell_in_line(line: &mut String, owner_pet_spells: &HashMap<String, HashSet<String>>) {
    // Try both positions where `Owner 's ` can appear
    for marker in &["  ", "from "] {
        if let Some(tag_pos) = find_pet_tag_position(line, marker, owner_pet_spells) {
            line.insert_str(tag_pos, "(pet) ");
            return;
        }
    }
}

/// Search for `{marker}{OwnerName} 's {SpellName}` in `line`.
///
/// If `SpellName` is in the owner's pet spell set, returns the byte offset
/// where `(pet) ` should be inserted (just before the spell name).
/// Returns `None` if no match or the spell is not a pet spell.
fn find_pet_tag_position(
    line: &str,
    marker: &str,
    owner_pet_spells: &HashMap<String, HashSet<String>>,
) -> Option<usize> {
    // Find the marker in the line
    let marker_pos = line.find(marker)?;
    let after_marker = marker_pos + marker.len();

    // Extract the candidate owner name (single word after marker)
    let rest = &line[after_marker..];
    let owner_end = rest.find(' ')?;
    let owner_name = &rest[..owner_end];

    // Check if this owner has pets
    let pet_spells = owner_pet_spells.get(owner_name)?;

    // Verify the `" 's "` pattern follows the owner name
    let after_owner = &rest[owner_end..];
    if !after_owner.starts_with(" 's ") {
        return None;
    }

    // Extract the spell name: everything after " 's " up to the next verb/punctuation
    let spell_start = owner_end + 4; // " 's " is 4 bytes
    let spell_rest = &rest[spell_start..];

    // The spell name ends at certain verb keywords or punctuation.
    // Multi-word spell names like "Savage Rend" or "Explosive Trap Effect" are common.
    let spell_name = extract_spell_name(spell_rest);

    if !spell_name.is_empty() && pet_spells.contains(spell_name) {
        // Return the absolute byte position where "(pet) " should be inserted
        Some(after_marker + spell_start)
    } else {
        None
    }
}

/// Extract a spell name from the text following `Owner 's `.
///
/// The spell name is a sequence of words (capitalized or not) that ends before
/// a combat verb (`hits`, `crits`, `misses`, `was`, `fails`) or a period/end.
/// For `DoT` lines like `from Owner 's SpellName.`, the spell ends at the period.
fn extract_spell_name(text: &str) -> &str {
    // Combat verbs and copulas that terminate a spell name
    const TERMINATORS: &[&str] = &[
        " hits ", " crits ", " misses ", " was ", " fails ", " missed ", " is ",
    ];

    let mut end = text.len();

    // Check for terminators
    for term in TERMINATORS {
        if let Some(pos) = text.find(term)
            && pos < end
        {
            end = pos;
        }
    }

    // Check for trailing period (DoT "from" lines end with `SpellName.`)
    if let Some(pos) = text.find('.')
        && pos < end
    {
        end = pos;
    }

    text[..end].trim()
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal COMBATANT_INFO line with a pet name.
    /// Fields: date & name & class & race & sex & pet & guild & ...
    /// Needs 6+ `&`-separated fields for `first_pass()` to process the pet.
    fn combatant_info(owner: &str, pet: &str) -> String {
        format!(
            "1/27 12:00:00.000  COMBATANT_INFO: 27.01.26 12:00:00\
             &{owner}&HUNTER&Human&2&{pet}&nil&nil&nil\
             &nil&nil&nil&nil&nil&nil&nil&nil&nil&nil\
             &nil&nil&nil&nil&nil&nil&nil&nil&nil\
             &0x{{00000001}}{{00000001}}&nil"
        )
    }

    #[test]
    fn test_pet_tag_preprocessed_auto_attack() {
        // Python-format auto attack line gets normalized to our internal format
        let lines = vec![
            combatant_info("Hunter", "Wolf"),
            "1/27 12:01:00.000  Hunter 's Auto Attack (pet) hits Boss for 100.".to_string(),
        ];
        let (result, _) = format_log(lines);
        assert!(
            result[1].contains("(pet) Auto Attack"),
            "Expected '(pet) Auto Attack', got: {}",
            result[1]
        );
    }

    #[test]
    fn test_pet_tag_preprocessed_spell() {
        // Pet spell without (pet) tag gets tagged via CAST-line discovery
        let lines = vec![
            combatant_info("Hunter", "Wolf"),
            "1/27 12:00:30.000  CAST: Wolf casts Bite(17261)(Rank 8) on Boss.".to_string(),
            "1/27 12:01:00.000  Hunter 's Bite hits Boss for 50.".to_string(),
        ];
        let (result, _) = format_log(lines);
        assert!(
            result[2].contains("(pet) Bite"),
            "Expected '(pet) Bite', got: {}",
            result[2]
        );
    }

    #[test]
    fn test_pet_tag_raw_log_spell() {
        // Raw-format pet spell: formatter rewrites PetName(Owner) to Owner 's,
        // then interstitial tags with (pet)
        let lines = vec![
            combatant_info("Hunter", "Wolf"),
            "1/27 12:00:30.000  CAST: Wolf casts Bite(17261)(Rank 8) on Boss.".to_string(),
            "1/27 12:01:00.000  Wolf (Hunter)'s Bite hits Boss for 50.".to_string(),
        ];
        let (result, _) = format_log(lines);
        assert!(
            result[2].contains("Hunter 's (pet) Bite"),
            "Expected 'Hunter 's (pet) Bite', got: {}",
            result[2]
        );
    }

    #[test]
    fn test_personal_spell_not_tagged() {
        // Hunter's personal spell should NOT get (pet) tag
        let lines = vec![
            combatant_info("Hunter", "Wolf"),
            "1/27 12:00:30.000  CAST: Wolf casts Bite(17261)(Rank 8) on Boss.".to_string(),
            "1/27 12:01:00.000  Hunter 's Aimed Shot hits Boss for 800.".to_string(),
        ];
        let (result, _) = format_log(lines);
        assert!(
            !result[2].contains("(pet)"),
            "Personal spell should NOT have (pet), got: {}",
            result[2]
        );
    }

    #[test]
    fn test_pet_tag_suffer_line() {
        // DoT tick from pet spell should get tagged
        let lines = vec![
            combatant_info("Hunter", "Wolf"),
            "1/27 12:00:30.000  CAST: Wolf casts Savage Rend(36541)(Rank 6) on Boss.".to_string(),
            "1/27 12:01:00.000  Boss suffers 101 Physical damage from Hunter 's Savage Rend."
                .to_string(),
        ];
        let (result, _) = format_log(lines);
        assert!(
            result[2].contains("(pet) Savage Rend"),
            "Expected '(pet) Savage Rend' in suffer line, got: {}",
            result[2]
        );
    }

    #[test]
    fn test_pet_tag_fallback_known_spells() {
        // Pet with no CAST lines falls back to KNOWN_PET_SPELLS
        let lines = vec![
            combatant_info("Hunter", "Wolf"),
            "1/27 12:01:00.000  Hunter 's Bite hits Boss for 50.".to_string(),
        ];
        let (result, _) = format_log(lines);
        assert!(
            result[1].contains("(pet) Bite"),
            "Fallback KNOWN_PET_SPELLS should tag Bite, got: {}",
            result[1]
        );
    }

    #[test]
    fn test_pet_tag_arcane_missiles_normalized() {
        // Arcane Missiles (pet) from trinket should get normalized
        let lines = vec![
            combatant_info("Hunter", "Wolf"),
            "1/27 12:01:00.000  Hunter 's Arcane Missiles (pet) hits Boss for 200.".to_string(),
        ];
        let (result, _) = format_log(lines);
        assert!(
            result[1].contains("(pet) Arcane Missiles"),
            "Expected '(pet) Arcane Missiles', got: {}",
            result[1]
        );
    }

    #[test]
    fn test_no_pet_owner_no_tagging() {
        // Player without a pet — spells should never get (pet) tagged
        let lines = vec![
            "1/27 12:00:00.000  COMBATANT_INFO: 27.01.26 12:00:00\
             &Mage&MAGE&Human&2&nil&nil&nil&nil\
             &nil&nil&nil&nil&nil&nil&nil&nil&nil&nil\
             &nil&nil&nil&nil&nil&nil&nil&nil&nil\
             &0x{00000002}{00000002}&nil"
                .to_string(),
            "1/27 12:01:00.000  Mage 's Bite hits Boss for 50.".to_string(),
        ];
        let (result, _) = format_log(lines);
        assert!(
            !result[1].contains("(pet)"),
            "Mage with no pet should not get (pet) tag, got: {}",
            result[1]
        );
    }

    #[test]
    fn test_pet_tag_raw_auto_attack() {
        // Raw-format pet auto attack: PetName (OwnerName) hits ...
        // Formatter should convert to Owner 's Auto Attack (pet) then
        // interstitial normalizes to (pet) Auto Attack
        let lines = vec![
            combatant_info("Hunter", "Wolf"),
            "1/27 12:01:00.000  Wolf (Hunter) hits Boss for 100.".to_string(),
        ];
        let (result, _) = format_log(lines);
        assert!(
            result[1].contains("(pet) Auto Attack"),
            "Raw pet auto attack should become '(pet) Auto Attack', got: {}",
            result[1]
        );
    }

    #[test]
    fn test_pet_tag_multiple_pets_per_owner() {
        // Hunter with multiple pets — spells from all pets should be tagged
        let lines = vec![
            combatant_info("Hunter", "Wolf"),
            combatant_info("Hunter", "Cat"),
            "1/27 12:00:30.000  CAST: Wolf casts Bite(17261)(Rank 8) on Boss.".to_string(),
            "1/27 12:00:31.000  CAST: Cat casts Claw(16830)(Rank 9) on Boss.".to_string(),
            "1/27 12:01:00.000  Hunter 's Bite hits Boss for 50.".to_string(),
            "1/27 12:01:01.000  Hunter 's Claw hits Boss for 40.".to_string(),
        ];
        let (result, _) = format_log(lines);
        assert!(
            result[4].contains("(pet) Bite"),
            "Bite from Wolf should be tagged, got: {}",
            result[4]
        );
        assert!(
            result[5].contains("(pet) Claw"),
            "Claw from Cat should be tagged, got: {}",
            result[5]
        );
    }

    #[test]
    fn test_pet_tag_resist_line() {
        // Pet spell resisted — "is resisted" should also get tagged
        let lines = vec![
            combatant_info("Hunter", "Wolf"),
            "1/27 12:00:30.000  CAST: Wolf casts Screech(24580)(Rank 4) on Boss.".to_string(),
            "1/27 12:01:00.000  Hunter 's Screech is resisted by Boss.".to_string(),
        ];
        let (result, _) = format_log(lines);
        assert!(
            result[2].contains("(pet) Screech"),
            "Resisted pet spell should be tagged, got: {}",
            result[2]
        );
    }

    #[test]
    fn test_pet_tag_same_spell_different_owners() {
        // Two hunters with pets that both cast Bite — only the owner whose
        // pet actually casts Bite should get tagged
        let lines = vec![
            combatant_info("Alice", "Fang"),
            combatant_info("Bob", "Rex"),
            "1/27 12:00:30.000  CAST: Fang casts Bite(17261)(Rank 8) on Boss.".to_string(),
            // Rex has no CAST lines — falls back to KNOWN_PET_SPELLS
            "1/27 12:01:00.000  Alice 's Bite hits Boss for 50.".to_string(),
            "1/27 12:01:01.000  Bob 's Bite hits Boss for 40.".to_string(),
        ];
        let (result, _) = format_log(lines);
        assert!(
            result[3].contains("(pet) Bite"),
            "Alice's Bite should be tagged (Fang casts it), got: {}",
            result[3]
        );
        assert!(
            result[4].contains("(pet) Bite"),
            "Bob's Bite should be tagged (fallback KNOWN_PET_SPELLS), got: {}",
            result[4]
        );
    }
}
