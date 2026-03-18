//! Events tab rendering: raw combat log entry display with filtering controls.

use std::fmt::Write;

use iced::widget::column;

#[allow(clippy::wildcard_imports)]
// viewer UI — many shared component functions used throughout
use super::components::*;
#[allow(clippy::wildcard_imports)] // viewer UI — many shared types/widgets used throughout
use super::*;

// ── Events Tab ──────────────────────────────────────────────────────────────

impl ViewerState {
    pub(super) fn view_events_tab(&self) -> Element<'_, ViewerMessage> {
        let player_picker = pick_list(
            self.event_player_names.clone(),
            Some(self.event_player_filter.clone()),
            ViewerMessage::SetEventPlayerFilter,
        )
        .width(Length::FillPortion(1))
        .padding(4);

        let header = row![text("Events").size(14), horizontal_space(), player_picker,]
            .spacing(8)
            .align_y(Center)
            .width(Fill);

        // Filter entries
        let filtered: Vec<&LogEntry> = self
            .log_data
            .entries
            .iter()
            .filter(|e| {
                self.log_data
                    .is_in_selection(e.timestamp(), &self.encounter_filter)
            })
            .filter(|e| {
                if self.event_player_filter == "All Players" {
                    true
                } else {
                    e.source() == self.event_player_filter
                        || e.target().is_some_and(|t| t == self.event_player_filter)
                }
            })
            .collect();

        // Show last 500
        let display: Vec<&&LogEntry> = filtered.iter().rev().take(500).collect::<Vec<_>>();

        let mut entries_col = Column::new().spacing(1);
        for entry in display.iter().rev() {
            entries_col = entries_col.push(self.view_log_entry(entry));
        }

        scrollable(
            container(
                column![header, horizontal_rule(1), entries_col]
                    .spacing(4)
                    .width(Fill),
            )
            .padding(12)
            .width(Fill)
            .style(panel_style),
        )
        .height(Fill)
        .into()
    }

    #[allow(clippy::unused_self)] // iced view method pattern — called via self for consistency
    fn view_log_entry<'a>(&self, entry: &'a LogEntry) -> Element<'a, ViewerMessage> {
        let ts = format_timestamp(entry.timestamp());
        let (text_str, color) = match entry {
            LogEntry::Damage {
                source,
                target,
                spell,
                amount,
                absorbed,
                resisted,
                blocked,
                is_glancing,
                is_crushing,
                school,
                ..
            } => {
                let school_str = school.as_deref().unwrap_or("");
                let mut s = if school_str.is_empty() {
                    format!("{source}'s {spell} hits {target} for {amount}")
                } else {
                    format!("{source}'s {spell} hits {target} for {amount} {school_str}")
                };
                if *resisted > 0 {
                    let _ = write!(s, " ({resisted} resisted)");
                }
                if *blocked > 0 {
                    let _ = write!(s, " ({blocked} blocked)");
                }
                if *absorbed > 0 {
                    let _ = write!(s, " ({absorbed} absorbed)");
                }
                if *is_glancing {
                    s.push_str(" (glancing)");
                }
                if *is_crushing {
                    s.push_str(" (crushing)");
                }
                (s, Color::from_rgb8(200, 200, 200))
            }
            LogEntry::Healing {
                source,
                target,
                spell,
                amount,
                effective_heal,
                overheal,
                ..
            } => {
                let mut s = format!("{source}'s {spell} heals {target} for {effective_heal}");
                if *overheal > 0 {
                    let _ = write!(s, " ({overheal} overheal)");
                }
                if *effective_heal != *amount {
                    // Only show raw total if different from effective (i.e. there was overheal)
                }
                (s, Color::from_rgb8(100, 255, 100))
            }
            LogEntry::Death { player, .. } => {
                (format!("{player} has died"), Color::from_rgb8(255, 68, 68))
            }
            LogEntry::Dispel {
                caster,
                target,
                spell,
                ..
            } => (
                format!("{caster} casts {spell} on {target}"),
                Color::from_rgb8(64, 224, 208),
            ),
            LogEntry::Resurrect { caster, target, .. } => (
                format!("{caster} resurrects {target}"),
                Color::from_rgb8(68, 255, 68),
            ),
            LogEntry::Interrupt {
                caster,
                target,
                spell,
                ..
            } => (
                format!("{caster} interrupts {target} with {spell}"),
                Color::from_rgb8(255, 153, 51),
            ),
            LogEntry::AuraGain {
                player,
                aura,
                stacks,
                ..
            } => (
                format!("{player} gains {aura} ({stacks})"),
                Color::from_rgb8(180, 140, 255),
            ),
            LogEntry::AuraFade { player, aura, .. } => (
                format!("{aura} fades from {player}"),
                Color::from_rgb8(140, 100, 200),
            ),
        };

        row![
            text(ts).size(11).color([0.4, 0.4, 0.4]),
            text(text_str).size(12).color(color),
        ]
        .spacing(8)
        .into()
    }
}
