//! Compiled regex patterns and spell-name constants for combat log parsing.

use std::sync::LazyLock;

use regex::Regex;

// ── Spell Lists ─────────────────────────────────────────────────────────────

pub(super) const DISPEL_SPELLS: &[&str] = &[
    "Dispel Magic",
    "Remove Curse",
    "Cleanse",
    "Purify",
    "Abolish Disease",
    "Abolish Poison",
    "Cure Disease",
    "Cure Poison",
    "Remove Lesser Curse",
    "Purge",
];

pub(super) const RESURRECT_SPELLS: &[&str] = &[
    "Resurrection",
    "Redemption",
    "Ancestral Spirit",
    "Rebirth",
    "Soulstone Resurrection",
    "Revive",
];

pub(super) const INTERRUPT_SPELLS: &[&str] = &[
    "Kick",
    "Pummel",
    "Earth Shock",
    "Counterspell",
    "Shield Bash",
    "Feral Charge",
    "Bash",
    "Spell Lock",
];

// ── Compiled Regexes ────────────────────────────────────────────────────────

pub(super) static RE_CAST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?:CAST:\s*)?([A-Za-z]+)\s+casts\s+([A-Za-z\s]+?)(?:\(\d+\))?(?:\(Rank \d+\))?\s+on\s+([A-Za-z\s]+)",
    )
    .unwrap()
});

/// Extract caster, spell name, and rank from addon `CAST:` lines.
///
/// Matches: `CAST: Druid casts Regrowth(8910)(Rank 4) on Warrior.`
/// Groups: 1=caster, 2=spell, 3=rank number.
pub(super) static RE_CAST_RANK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"CAST:\s*([A-Za-z]+(?:\s[A-Za-z]+)*)\s+(?:casts|begins to cast)\s+([A-Za-z][A-Za-z '\-]+?)(?:\(\d+\))+\(Rank (\d+)\)",
    )
    .unwrap()
});

pub(super) static RE_DMG_SPELL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"([A-Za-z]+(?:\s[A-Za-z]+)*(?:\s*\([^)]+\))?) 's (?:\(pet\) )?([A-Za-z\s']+) (?:hits|crits) ([A-Za-z\s']+) for (\d+)",
    )
    .unwrap()
});

pub(super) static RE_DMG_AUTO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"([A-Za-z]+(?:\s[A-Za-z]+)*) (?:hits|crits) ([A-Za-z\s']+) for (\d+)\.").unwrap()
});

pub(super) static RE_DMG_SUFFER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"([A-Za-z\s']+) suffers (\d+) (?:\w+ )?damage from ([A-Za-z]+(?:\s[A-Za-z]+)*(?:\s*\([^)]+\))?) 's (?:\(pet\) )?([A-Za-z\s']+)",
    )
    .unwrap()
});

pub(super) static RE_HEAL_SPELL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"([A-Za-z]+(?:\s[A-Za-z]+)*(?:\s*\([^)]+\))?) 's ([A-Za-z\s']+) (?:heals|critically heals) ([A-Za-z\s']+(?:\s*\([^)]+\))?) for (\d+)",
    )
    .unwrap()
});

pub(super) static RE_HEAL_GAIN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"([A-Za-z]+(?:\s[A-Za-z]+)*(?:\s*\([^)]+\))?) gains (\d+) health from ([A-Za-z]+(?:\s[A-Za-z]+)*(?:\s*\([^)]+\))?) 's ([A-Za-z\s']+)",
    )
    .unwrap()
});

pub(super) static RE_DODGE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([A-Za-z\s']+) attacks\. ([A-Za-z]+) dodges\.").unwrap());

pub(super) static RE_PARRY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([A-Za-z\s']+) attacks\. ([A-Za-z]+) parries\.").unwrap());

pub(super) static RE_MISS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([A-Za-z\s']+) misses ([A-Za-z]+)\.").unwrap());

pub(super) static RE_BUFF_GAIN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([A-Za-z]+) gains ([A-Za-z\s':]+?) \((\d+)\)\.").unwrap());

pub(super) static RE_BUFF_FADE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([A-Za-z\s':]+?) fades from ([A-Za-z]+)\.").unwrap());

pub(super) static RE_AFFLICTED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"([A-Za-z]+) is afflicted by ([A-Za-z\s':]+?)(?:\s+\((\d+)\))?\.").unwrap()
});

pub(super) static RE_LOOT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"LOOT:.*?&([A-Za-z]+) receives (?:loot|item): \|cff([a-f0-9]{6})\|Hitem:(\d+):[^|]+\|h\[([^\]]+)\]\|h\|rx?(\d+)",
    )
    .unwrap()
});

pub(super) static RE_TRADE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"LOOT_TRADE:.*?&([A-Za-z]+) trades item (.+?) to ([A-Za-z]+)\.").unwrap()
});

pub(super) static RE_PET_OWNER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([A-Za-z]+)\s+\(([A-Za-z]+)\)").unwrap());

pub(super) static RE_ABSORB: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\((\d+) absorbed\)").unwrap());
pub(super) static RE_RESISTED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\((\d+) resisted\)").unwrap());
pub(super) static RE_BLOCKED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\((\d+) blocked\)").unwrap());

/// V1 consumable line: `PlayerName uses ConsumableName.` or `...on Target.`
pub(super) static RE_CONSUMABLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"([A-Za-z]+) uses ([A-Za-z][A-Za-z '\-]+?)(?:\s+on\s+[A-Za-z\s]+)?\.").unwrap()
});
