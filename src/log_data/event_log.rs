use super::types::LogEntry;

// ── Event Log Facets ────────────────────────────────────────────────────────

/// Which preset view mode the event log is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EventLogMode {
    /// Show all events matching the type toggles.
    #[default]
    AllEvents,
    /// Show only key events: deaths, big hits, dispels, interrupts, resurrects.
    KeyEvents,
    /// Show events involving each dead player in the seconds before their death.
    DeathLog,
}

impl std::fmt::Display for EventLogMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AllEvents => write!(f, "All Events"),
            Self::KeyEvents => write!(f, "Key Events"),
            Self::DeathLog => write!(f, "Death Log"),
        }
    }
}

/// Selectable lookback window for Death Log mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeathLogWindow {
    #[default]
    Seconds10,
    Seconds15,
    Seconds20,
    Seconds30,
}

impl DeathLogWindow {
    /// All selectable window sizes (for pick list).
    pub const ALL: &[Self] = &[
        Self::Seconds10,
        Self::Seconds15,
        Self::Seconds20,
        Self::Seconds30,
    ];

    /// Window duration in seconds.
    pub fn as_secs(self) -> f64 {
        match self {
            Self::Seconds10 => 10.0,
            Self::Seconds15 => 15.0,
            Self::Seconds20 => 20.0,
            Self::Seconds30 => 30.0,
        }
    }
}

impl std::fmt::Display for DeathLogWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Seconds10 => write!(f, "10s"),
            Self::Seconds15 => write!(f, "15s"),
            Self::Seconds20 => write!(f, "20s"),
            Self::Seconds30 => write!(f, "30s"),
        }
    }
}

/// Which event types are visible in the event log.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)] // One bool per event type — clearest representation
pub struct EventLogTypeFilter {
    pub show_damage: bool,
    pub show_healing: bool,
    pub show_deaths: bool,
    pub show_dispels: bool,
    pub show_interrupts: bool,
}

impl Default for EventLogTypeFilter {
    fn default() -> Self {
        Self {
            show_damage: true,
            show_healing: true,
            show_deaths: true,
            show_dispels: true,
            show_interrupts: true,
        }
    }
}

impl EventLogTypeFilter {
    /// Check if a `LogEntry` passes the type filter.
    pub fn accepts(&self, entry: &LogEntry) -> bool {
        match entry {
            LogEntry::Damage { .. } => self.show_damage,
            LogEntry::Healing { .. } => self.show_healing,
            LogEntry::Death { .. } | LogEntry::Resurrect { .. } => self.show_deaths,
            LogEntry::Dispel { .. } => self.show_dispels,
            LogEntry::Interrupt { .. } => self.show_interrupts,
            // Aura events are not shown in the event log — they are rendered
            // on the dedicated AuraChart canvas as horizontal bars.
            LogEntry::AuraGain { .. } | LogEntry::AuraFade { .. } => false,
        }
    }
}

/// Which event type toggle to flip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventLogTypeKind {
    Damage,
    Healing,
    Deaths,
    Dispels,
    Interrupts,
}
