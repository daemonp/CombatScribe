//! Death Log tab: chronological table of all deaths with encounter association.

#[allow(clippy::wildcard_imports)]
// viewer UI — many shared component functions used throughout
use super::components::*;
#[allow(clippy::wildcard_imports)] // viewer UI — many shared types/widgets used throughout
use super::*;
use iced::widget::column;

// ── Death Log Mode ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeathLogMode {
    PlayerDeaths,
    AllDeaths,
}

impl std::fmt::Display for DeathLogMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PlayerDeaths => write!(f, "Player Deaths"),
            Self::AllDeaths => write!(f, "All Deaths"),
        }
    }
}

// ── Death Log Tab ───────────────────────────────────────────────────────────

impl ViewerState {
    #[allow(clippy::too_many_lines)] // iced UI layout — death log table with encounter lookup
    pub(super) fn view_death_log_tab(&self) -> Element<'_, ViewerMessage> {
        let mode_list: &[DeathLogMode] = &[DeathLogMode::PlayerDeaths, DeathLogMode::AllDeaths];
        let mode_picker = pick_list(mode_list, Some(self.death_log_mode), |m| {
            ViewerMessage::SetDeathTabFilter(m)
        })
        .width(Length::Fixed(180.0))
        .padding(4);

        // Collect deaths matching filter
        let deaths = self.log_data.filtered_deaths(&self.encounter_filter);
        let encounters = self.log_data.selected_encounters(&self.encounter_filter);

        // Filter based on mode
        let filtered_deaths: Vec<&DeathEvent> = if self.death_log_mode == DeathLogMode::AllDeaths {
            deaths
        } else {
            deaths
                .into_iter()
                .filter(|d| self.log_data.combatants.contains_key(d.player.as_str()))
                .collect()
        };

        let death_count = filtered_deaths.len();
        let enc_count = encounters.len();
        let summary = format!("{death_count} deaths across {enc_count} encounters");

        let header = row![
            mode_picker,
            text(summary).size(12).color(theme::TEXT_SECONDARY),
        ]
        .spacing(8)
        .align_y(Center);

        // Column headers
        let col_headers = row![
            text("Time")
                .size(11)
                .color(theme::TEXT_MUTED)
                .width(Length::FillPortion(2)),
            text("Encounter")
                .size(11)
                .color(theme::TEXT_MUTED)
                .width(Length::FillPortion(3)),
            text("Player")
                .size(11)
                .color(theme::TEXT_MUTED)
                .width(Length::FillPortion(2)),
            text("Killed By")
                .size(11)
                .color(theme::TEXT_MUTED)
                .width(Length::FillPortion(2)),
            text("Ability")
                .size(11)
                .color(theme::TEXT_MUTED)
                .width(Length::FillPortion(2)),
            text("Damage")
                .size(11)
                .color(theme::TEXT_MUTED)
                .width(Length::FillPortion(1)),
        ]
        .spacing(4)
        .width(Fill);

        // Build rows
        let mut rows_col = Column::new().spacing(1);

        let killer_color = Color::from_rgb(0.9, 0.3, 0.3);

        for death in &filtered_deaths {
            let time_str = format_timestamp(death.timestamp);

            // Find encounter for this death
            let enc_name = encounter_for_death(death, &encounters)
                .map_or_else(|| "Unknown".to_string(), format_encounter_label);

            let player_class = self.log_data.player_class(&death.player);
            let player_color = theme::class_color(player_class);

            let killer_text = death.killer.as_deref().unwrap_or("Unknown").to_string();
            let ability_text = death.killing_blow.as_deref().unwrap_or("").to_string();
            let damage_text = death
                .damage_amount
                .map_or(String::new(), theme::format_number);

            let player_name = death.player.clone();
            let death_row: Element<ViewerMessage> = button(
                row![
                    text(time_str)
                        .size(12)
                        .color(theme::TEXT_MUTED)
                        .width(Length::FillPortion(2)),
                    text(enc_name)
                        .size(12)
                        .color(theme::TEXT_SECONDARY)
                        .width(Length::FillPortion(3)),
                    text(death.player.as_str())
                        .size(12)
                        .color(player_color)
                        .width(Length::FillPortion(2)),
                    text(killer_text)
                        .size(12)
                        .color(killer_color)
                        .width(Length::FillPortion(2)),
                    text(ability_text)
                        .size(12)
                        .color(Color::from_rgb8(220, 225, 230))
                        .width(Length::FillPortion(2)),
                    text(damage_text)
                        .size(12)
                        .color(Color::from_rgb8(200, 200, 200))
                        .width(Length::FillPortion(1)),
                ]
                .spacing(4)
                .align_y(Center)
                .width(Fill),
            )
            .on_press(ViewerMessage::ShowDetail(player_name, DetailType::Deaths))
            .padding([3, 6])
            .width(Fill)
            .style(transparent_button_style)
            .into();

            rows_col = rows_col.push(death_row);
        }

        if filtered_deaths.is_empty() {
            rows_col = rows_col.push(empty_state("No deaths recorded"));
        }

        let content = container(
            column![
                header,
                rule::horizontal(1),
                col_headers,
                rule::horizontal(1),
                rows_col
            ]
            .spacing(6)
            .width(Fill),
        )
        .padding(12)
        .width(Fill)
        .style(panel_style);

        scrollable(content).height(Fill).into()
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Find which encounter a death belongs to by timestamp range.
fn encounter_for_death<'a>(
    death: &DeathEvent,
    encounters: &[&'a log_data::Encounter],
) -> Option<&'a log_data::Encounter> {
    encounters
        .iter()
        .find(|e| death.timestamp >= e.start && death.timestamp <= e.end)
        .copied()
}

/// Format an encounter as a short label for the death log table.
fn format_encounter_label(enc: &log_data::Encounter) -> String {
    let name = enc.name.as_deref().unwrap_or("Trash");
    let result = if enc.is_kill { "Kill" } else { "Wipe" };
    let attempt = enc.attempt.map_or(String::new(), |a| format!(" {a}"));
    format!("{name} - {result}{attempt}")
}
