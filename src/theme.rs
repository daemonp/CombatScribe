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
pub fn quality_color(quality: crate::log_data::ItemQuality) -> Color {
    use crate::log_data::ItemQuality;
    match quality {
        ItemQuality::Poor => Color::from_rgb8(157, 157, 157), // #9D9D9D
        ItemQuality::Common => Color::from_rgb8(255, 255, 255), // #FFFFFF
        ItemQuality::Uncommon => Color::from_rgb8(30, 255, 0), // #1EFF00
        ItemQuality::Rare => Color::from_rgb8(0, 112, 221),   // #0070DD
        ItemQuality::Epic => Color::from_rgb8(163, 53, 238),  // #A335EE
        ItemQuality::Legendary => Color::from_rgb8(255, 128, 0), // #FF8000
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

// ── Timeline Colors ─────────────────────────────────────────────────────────

/// Raid DPS line/area color (amber-gold).
pub const TIMELINE_DPS: Color = Color {
    r: 0.95,
    g: 0.75,
    b: 0.2,
    a: 1.0,
};

/// Raid DTPS line/area color (red).
pub const TIMELINE_DTPS: Color = Color {
    r: 0.9,
    g: 0.25,
    b: 0.25,
    a: 1.0,
};

/// Raid HPS line/area color (green).
pub const TIMELINE_HPS: Color = Color {
    r: 0.25,
    g: 0.85,
    b: 0.35,
    a: 1.0,
};

/// Boss/enemy heal line/area color (purple/magenta).
pub const TIMELINE_BOSS_HEAL: Color = Color {
    r: 0.75,
    g: 0.3,
    b: 0.85,
    a: 1.0,
};

/// Death marker color (bright red).
pub const TIMELINE_DEATH: Color = Color {
    r: 1.0,
    g: 0.15,
    b: 0.15,
    a: 1.0,
};

/// Big hit marker color (orange-red).
pub const TIMELINE_BIG_HIT: Color = Color {
    r: 1.0,
    g: 0.5,
    b: 0.15,
    a: 1.0,
};

/// Dispel marker color (teal, matching dispel bar).
pub const TIMELINE_DISPEL: Color = Color {
    r: 0.251,
    g: 0.878,
    b: 0.816,
    a: 1.0,
};

/// Resurrect marker color.
pub const TIMELINE_RESURRECT: Color = Color {
    r: 0.267,
    g: 1.0,
    b: 0.267,
    a: 1.0,
};

/// Interrupt marker color (matching interrupt bar).
pub const TIMELINE_INTERRUPT: Color = Color {
    r: 1.0,
    g: 0.6,
    b: 0.2,
    a: 1.0,
};

/// Alive count line color (muted blue).
pub const TIMELINE_ALIVE: Color = Color {
    r: 0.4,
    g: 0.6,
    b: 0.9,
    a: 1.0,
};

/// Color palette for aura overlay bars on the timeline.
/// Cycles through these for different tracked auras.
pub const AURA_COLORS: [Color; 6] = [
    Color {
        r: 0.71,
        g: 0.55,
        b: 1.0,
        a: 1.0,
    }, // lavender
    Color {
        r: 1.0,
        g: 0.55,
        b: 0.55,
        a: 1.0,
    }, // soft red
    Color {
        r: 0.55,
        g: 0.9,
        b: 1.0,
        a: 1.0,
    }, // cyan
    Color {
        r: 1.0,
        g: 0.82,
        b: 0.35,
        a: 1.0,
    }, // gold
    Color {
        r: 0.55,
        g: 1.0,
        b: 0.65,
        a: 1.0,
    }, // mint green
    Color {
        r: 1.0,
        g: 0.55,
        b: 0.82,
        a: 1.0,
    }, // pink
];

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
