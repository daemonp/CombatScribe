//! Consumes tab: per-player consumable breakdown and encounter matrix.

#[allow(clippy::wildcard_imports)]
// viewer UI — many shared component functions used throughout
use super::components::*;
#[allow(clippy::wildcard_imports)] // viewer UI — many shared types/widgets used throughout
use super::*;
use iced::widget::column;

// ── Row type for the encounter matrix table ─────────────────────────────────

/// Pre-computed row data for the encounter matrix table widget.
#[derive(Clone)]
struct MatrixRow {
    player: String,
    class: String,
    /// Per-category counts, indexed by position in `visible_categories`.
    counts: Vec<u64>,
    total: u64,
}

// ── Consumes Tab ────────────────────────────────────────────────────────────

impl ViewerState {
    pub(super) fn view_consumes_tab(&self) -> Element<'_, ViewerMessage> {
        let modes = vec![
            ConsumesViewMode::PlayerBreakdown,
            ConsumesViewMode::EncounterMatrix,
        ];
        let mode_picker = pick_list(modes, Some(self.consumes_mode), |m| {
            ViewerMessage::SetConsumesMode(m)
        })
        .width(Fill)
        .padding(4);

        let consumables = self.log_data.filtered_consumables(&self.encounter_filter);
        let total_uses: u64 = consumables.len() as u64;
        let total_text = format!("{total_uses} uses");

        let header = row![
            mode_picker,
            text(total_text).size(12).color(theme::TEXT_SECONDARY),
        ]
        .spacing(8)
        .align_y(Center);

        let content = match self.consumes_mode {
            ConsumesViewMode::PlayerBreakdown => self.view_player_breakdown(&consumables),
            ConsumesViewMode::EncounterMatrix => self.view_encounter_matrix(&consumables),
        };

        scrollable(
            container(
                column![header, rule::horizontal(1), content]
                    .spacing(8)
                    .width(Fill),
            )
            .padding(12)
            .width(Fill)
            .style(panel_style),
        )
        .height(Fill)
        .into()
    }

    // ── Player Breakdown View ───────────────────────────────────────────

    fn view_player_breakdown<'a>(
        &'a self,
        consumables: &[&log_data::ConsumableUse],
    ) -> Element<'a, ViewerMessage> {
        let players = aggregate_by_player(
            consumables,
            |c| &c.player,
            &self.log_data.combatants,
            |n| self.log_data.player_class(n),
        );

        let (content, _total_text) = view_event_meters_owned(
            &players,
            theme::BAR_CONSUMABLE,
            "No consumable usage recorded",
            Some(DetailType::Consumables),
            "uses",
        );
        content
    }

    // ── Encounter Matrix View ───────────────────────────────────────────

    #[allow(clippy::too_many_lines)] // iced UI layout — dynamic column matrix table
    fn view_encounter_matrix<'a>(
        &'a self,
        consumables: &[&log_data::ConsumableUse],
    ) -> Element<'a, ViewerMessage> {
        if consumables.is_empty() {
            return empty_state("No consumable usage recorded");
        }

        // Aggregate: player → category → count
        let mut player_cat_counts: HashMap<&str, HashMap<ConsumableCategory, u64>> = HashMap::new();
        for c in consumables {
            if self.log_data.combatants.contains_key(c.player.as_str()) {
                *player_cat_counts
                    .entry(&c.player)
                    .or_default()
                    .entry(c.category)
                    .or_insert(0) += 1;
            }
        }

        if player_cat_counts.is_empty() {
            return empty_state("No consumable usage recorded");
        }

        // Determine which categories have any usage (to hide empty columns)
        let mut category_totals: HashMap<ConsumableCategory, u64> = HashMap::new();
        for cats in player_cat_counts.values() {
            for (&cat, &count) in cats {
                *category_totals.entry(cat).or_insert(0) += count;
            }
        }
        let mut visible_categories: Vec<ConsumableCategory> = category_totals
            .keys()
            .copied()
            .filter(|c| category_totals[c] > 0)
            .collect();
        visible_categories.sort(); // Sort by enum variant order (Flask → ... → Other)

        // Build rows sorted by total descending
        let mut rows: Vec<MatrixRow> = player_cat_counts
            .iter()
            .map(|(&player, cats)| {
                let counts: Vec<u64> = visible_categories
                    .iter()
                    .map(|cat| cats.get(cat).copied().unwrap_or(0))
                    .collect();
                let total: u64 = counts.iter().sum();
                MatrixRow {
                    player: player.to_string(),
                    class: self.log_data.player_class(player).to_string(),
                    counts,
                    total,
                }
            })
            .collect();
        rows.sort_by_key(|r| std::cmp::Reverse(r.total));

        // Build the table manually since the number of category columns varies.
        let mut content = Column::new().spacing(1);

        // Header row
        let mut header_row = Row::new().spacing(2).width(Fill);
        header_row = header_row.push(
            text("Player")
                .size(11)
                .color(theme::TEXT_SECONDARY)
                .width(Length::FillPortion(3)),
        );
        for cat in &visible_categories {
            let cat_color = theme::consumable_category_color(*cat);
            // Use short abbreviation for column headers
            let label = category_short_label(*cat);
            header_row = header_row.push(
                text(label)
                    .size(10)
                    .color(cat_color)
                    .width(Length::FillPortion(1))
                    .align_x(iced::alignment::Horizontal::Center),
            );
        }
        header_row = header_row.push(
            text("Total")
                .size(11)
                .color(theme::TEXT_SECONDARY)
                .width(Length::FillPortion(1))
                .align_x(iced::alignment::Horizontal::Center),
        );
        content = content.push(header_row);
        content = content.push(rule::horizontal(1));

        // Data rows
        for row_data in &rows {
            let class_color = theme::class_color(&row_data.class);
            let player_name = row_data.player.clone();

            let mut data_row = Row::new().spacing(2).width(Fill).align_y(Center);
            // Player cell with class icon
            let icon = image(theme::class_icon(&row_data.class))
                .width(18)
                .height(18);
            let player_cell: Element<ViewerMessage> = row![
                icon,
                text(row_data.player.clone()).size(12).color(class_color),
            ]
            .spacing(4)
            .align_y(Center)
            .width(Length::FillPortion(3))
            .into();
            data_row = data_row.push(player_cell);

            // Category count cells
            for (i, cat) in visible_categories.iter().enumerate() {
                let count = row_data.counts[i];
                let cell_text = if count == 0 {
                    text("\u{2013}") // en-dash for zero
                        .size(11)
                        .color(theme::TEXT_MUTED)
                } else {
                    text(count.to_string())
                        .size(11)
                        .color(theme::consumable_category_color(*cat))
                };
                data_row = data_row.push(
                    cell_text
                        .width(Length::FillPortion(1))
                        .align_x(iced::alignment::Horizontal::Center),
                );
            }

            // Total cell
            data_row = data_row.push(
                text(row_data.total.to_string())
                    .size(12)
                    .color(Color::from_rgb8(220, 225, 230))
                    .width(Length::FillPortion(1))
                    .align_x(iced::alignment::Horizontal::Center),
            );

            // Wrap row in a clickable button to open detail
            let row_element: Element<ViewerMessage> = data_row.into();
            content = content.push(
                button(row_element)
                    .on_press(ViewerMessage::ShowDetail(
                        player_name,
                        DetailType::Consumables,
                    ))
                    .padding([4, 4])
                    .width(Fill)
                    .style(transparent_button_style),
            );
        }

        content.width(Fill).into()
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Short label for a consumable category (used as column header in the matrix).
fn category_short_label(cat: ConsumableCategory) -> &'static str {
    match cat {
        ConsumableCategory::Flask => "Flask",
        ConsumableCategory::Elixir => "Elixir",
        ConsumableCategory::Potion => "Potion",
        ConsumableCategory::Food => "Food",
        ConsumableCategory::WeaponBuff => "Weap.",
        ConsumableCategory::Juju => "Juju",
        ConsumableCategory::BlastedLands => "BL",
        ConsumableCategory::Zanza => "Zanza",
        ConsumableCategory::Scroll => "Scroll",
        ConsumableCategory::Engineering => "Eng.",
        ConsumableCategory::Bandage => "Band.",
        ConsumableCategory::Utility => "Util.",
        ConsumableCategory::Other => "Other",
    }
}
