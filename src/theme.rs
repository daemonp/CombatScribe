//! Visual theme constants for the combat log viewer.
//!
//! `WoW` class colors, item quality colors, meter bar colors,
//! class abbreviations, and number/duration formatting.

use iced::widget::image;
use iced::Color;

// ── Class Colors ────────────────────────────────────────────────────────────

/// Return the `WoW` class color as an iced `Color`.
pub fn class_color(class: &str) -> Color {
    match class {
        "WARRIOR" => Color::from_rgb8(199, 156, 110), // #C79C6E
        "PALADIN" => Color::from_rgb8(245, 140, 186), // #F58CBA
        "HUNTER" => Color::from_rgb8(171, 212, 115),  // #ABD473
        "ROGUE" => Color::from_rgb8(255, 245, 105),   // #FFF569
        "PRIEST" => Color::from_rgb8(255, 255, 255),  // #FFFFFF
        "SHAMAN" => Color::from_rgb8(0, 112, 222),    // #0070DE
        "MAGE" => Color::from_rgb8(105, 204, 240),    // #69CCF0
        "WARLOCK" => Color::from_rgb8(148, 130, 201), // #9482C9
        "DRUID" => Color::from_rgb8(255, 125, 10),    // #FF7D0A
        _ => Color::from_rgb8(128, 128, 128),         // unknown
    }
}

// ── Class Icons (embedded PNGs) ─────────────────────────────────────────────

/// Return an iced `image::Handle` for the `WoW` class icon (64x64 PNG).
pub fn class_icon(class: &str) -> image::Handle {
    let bytes: &[u8] = match class {
        "PALADIN" => include_bytes!("../assets/class_icons/paladin.png"),
        "HUNTER" => include_bytes!("../assets/class_icons/hunter.png"),
        "ROGUE" => include_bytes!("../assets/class_icons/rogue.png"),
        "PRIEST" => include_bytes!("../assets/class_icons/priest.png"),
        "SHAMAN" => include_bytes!("../assets/class_icons/shaman.png"),
        "MAGE" => include_bytes!("../assets/class_icons/mage.png"),
        "WARLOCK" => include_bytes!("../assets/class_icons/warlock.png"),
        "DRUID" => include_bytes!("../assets/class_icons/druid.png"),
        // WARRIOR + unknown classes
        _ => include_bytes!("../assets/class_icons/warrior.png"),
    };
    image::Handle::from_bytes(bytes)
}

// ── Item Quality Colors ─────────────────────────────────────────────────────

/// Return the item quality color.
pub fn quality_color(quality: &str) -> Color {
    match quality {
        "poor" => Color::from_rgb8(157, 157, 157),    // #9D9D9D
        "uncommon" => Color::from_rgb8(30, 255, 0),   // #1EFF00
        "rare" => Color::from_rgb8(0, 112, 221),      // #0070DD
        "epic" => Color::from_rgb8(163, 53, 238),     // #A335EE
        "legendary" => Color::from_rgb8(255, 128, 0), // #FF8000
        _ => Color::from_rgb8(255, 255, 255),
    }
}

// ── Meter Bar Colors ────────────────────────────────────────────────────────

pub const BAR_DISPEL: Color = Color {
    r: 0.251,
    g: 0.878,
    b: 0.816,
    a: 1.0,
}; // #40E0D0 teal
pub const BAR_INTERRUPT: Color = Color {
    r: 1.0,
    g: 0.6,
    b: 0.2,
    a: 1.0,
}; // #FF9933 orange
pub const BAR_DEATH: Color = Color {
    r: 1.0,
    g: 0.267,
    b: 0.267,
    a: 1.0,
}; // #FF4444 red
pub const BAR_RESURRECT: Color = Color {
    r: 0.267,
    g: 1.0,
    b: 0.267,
    a: 1.0,
}; // #44FF44 green
pub const BAR_ABSORB: Color = Color {
    r: 0.667,
    g: 0.533,
    b: 1.0,
    a: 1.0,
}; // #AA88FF purple
pub const BAR_CONSUMABLE: Color = Color {
    r: 1.0,
    g: 0.84,
    b: 0.0,
    a: 1.0,
}; // #FFD700 gold

// ── Surface & Text Colors ───────────────────────────────────────────────────

/// Slightly elevated card/panel background.
pub const SURFACE: Color = Color {
    r: 0.145,
    g: 0.155,
    b: 0.18,
    a: 1.0,
};

/// Subtle border for card/panel containers.
pub const SURFACE_BORDER: Color = Color {
    r: 0.22,
    g: 0.24,
    b: 0.28,
    a: 1.0,
};

/// Muted text for rank numbers, labels.
pub const TEXT_MUTED: Color = Color {
    r: 0.40,
    g: 0.43,
    b: 0.47,
    a: 1.0,
};

/// Secondary text for per-second values, totals.
pub const TEXT_SECONDARY: Color = Color {
    r: 0.60,
    g: 0.63,
    b: 0.67,
    a: 1.0,
};

// ── Formatting Helpers ──────────────────────────────────────────────────────

/// Format a number with comma separators (e.g. `1,234,567`).
pub fn format_number(num: u64) -> String {
    if num < 1_000 {
        return num.to_string();
    }
    let s = num.to_string();
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            result.push(',');
        }
        result.push(ch);
    }
    result
}

/// Format a floating-point per-second value with commas (rounds to nearest integer).
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn format_number_f64(num: f64) -> String {
    format_number(num.round() as u64)
}

/// Format a duration in seconds as `M:SS` or `Xs`.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn format_duration(seconds: f64) -> String {
    let total = seconds as u64;
    let mins = total / 60;
    let secs = total % 60;
    if mins > 0 {
        format!("{mins}:{secs:02}")
    } else {
        format!("{secs}s")
    }
}
