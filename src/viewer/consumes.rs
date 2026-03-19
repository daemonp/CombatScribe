//! Consumes tab: raid overview, per-player breakdown, encounter matrix, and timeline.

use super::charts::{
    ConsumeChart, build_consume_layout, consume_chart_height, translate_aura_to_consume,
};
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
            ConsumesViewMode::RaidOverview,
            ConsumesViewMode::PlayerBreakdown,
            ConsumesViewMode::EncounterMatrix,
            ConsumesViewMode::Timeline,
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

        // Timeline mode has a different layout (sidebar + chart), not scrollable
        if self.consumes_mode == ConsumesViewMode::Timeline {
            return self.view_consume_timeline(header);
        }

        let content = match self.consumes_mode {
            ConsumesViewMode::RaidOverview => self.view_raid_overview(&consumables),
            ConsumesViewMode::PlayerBreakdown => self.view_player_breakdown(&consumables),
            ConsumesViewMode::EncounterMatrix => self.view_encounter_matrix(&consumables),
            ConsumesViewMode::Timeline => unreachable!(),
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

    // ── Timeline View ────────────────────────────────────────────────────

    /// Consumable timeline waterfall chart with a sidebar category picker.
    ///
    /// Sidebar (fixed 260px) contains: view mode toggle (Bars/Ticks), category
    /// checkboxes with Select All / Clear All.  The chart fills the remaining
    /// width with hybrid bar/tick rendering.
    #[allow(clippy::too_many_lines)] // iced UI layout — sidebar picker + full-width chart + tooltip
    fn view_consume_timeline<'a>(
        &'a self,
        header: Row<'a, ViewerMessage>,
    ) -> Element<'a, ViewerMessage> {
        let td = &self.timeline_data;
        let consume_accent = theme::TIMELINE_CONSUME;

        // ── Sidebar: view mode toggle ───────────────────────────────
        let bars_active = self.consume_view_mode == log_data::ConsumeViewMode::Bars;
        let ticks_active = self.consume_view_mode == log_data::ConsumeViewMode::Ticks;

        let mode_toggle = row![
            button(text("Bars").size(9).color(if bars_active {
                Color::WHITE
            } else {
                theme::TEXT_MUTED
            }))
            .on_press(ViewerMessage::SetConsumeViewMode(
                log_data::ConsumeViewMode::Bars
            ))
            .padding([2, 8])
            .style(move |_theme: &iced::Theme, status| {
                consume_mode_button_style(status, bars_active, consume_accent)
            }),
            button(text("Ticks").size(9).color(if ticks_active {
                Color::WHITE
            } else {
                theme::TEXT_MUTED
            }))
            .on_press(ViewerMessage::SetConsumeViewMode(
                log_data::ConsumeViewMode::Ticks
            ))
            .padding([2, 8])
            .style(move |_theme: &iced::Theme, status| {
                consume_mode_button_style(status, ticks_active, consume_accent)
            }),
        ]
        .spacing(4);

        // ── Sidebar: category checkboxes ────────────────────────────
        let mut category_list = Column::new().spacing(1);
        for &cat in &td.available_consume_categories {
            let is_tracked = self.tracked_consume_categories.contains(&cat);
            let display = crate::consumable_data::category_display_name(cat);
            let cat_color = theme::consumable_category_color(cat);
            let check_label = if is_tracked {
                format!("[x] {display}")
            } else {
                format!("[ ] {display}")
            };
            let text_color = if is_tracked {
                cat_color
            } else {
                theme::TEXT_SECONDARY
            };
            category_list = category_list.push(
                button(text(check_label).size(11).color(text_color))
                    .on_press(ViewerMessage::ToggleConsumeCategory(cat))
                    .padding([3, 6])
                    .width(Fill)
                    .style(move |_theme: &iced::Theme, status| {
                        let bg = match status {
                            button::Status::Hovered => Some(iced::Background::Color(
                                Color::from_rgba8(255, 255, 255, 0.08),
                            )),
                            _ => {
                                if is_tracked {
                                    Some(iced::Background::Color(Color {
                                        a: 0.08,
                                        ..cat_color
                                    }))
                                } else {
                                    None
                                }
                            }
                        };
                        button::Style {
                            background: bg,
                            text_color,
                            border: iced::Border {
                                radius: 2.0.into(),
                                ..Default::default()
                            },
                            shadow: iced::Shadow::default(),
                            snap: true,
                        }
                    }),
            );
        }

        // ── Sidebar: Select All / Clear All buttons ─────────────────
        let all_selected = !td.available_consume_categories.is_empty()
            && td
                .available_consume_categories
                .iter()
                .all(|cat| self.tracked_consume_categories.contains(cat));
        let any_selected = !self.tracked_consume_categories.is_empty();

        let mut action_row = Row::new().spacing(4);
        if !all_selected && !td.available_consume_categories.is_empty() {
            action_row = action_row.push(
                button(text("Select All").size(9).color(theme::TEXT_SECONDARY))
                    .on_press(ViewerMessage::SelectAllConsumes)
                    .padding([2, 6])
                    .style(|_theme: &iced::Theme, status| {
                        let bg = match status {
                            button::Status::Hovered => Some(iced::Background::Color(
                                Color::from_rgba8(255, 255, 255, 0.08),
                            )),
                            _ => None,
                        };
                        button::Style {
                            background: bg,
                            text_color: theme::TEXT_SECONDARY,
                            border: iced::Border {
                                radius: 3.0.into(),
                                ..Default::default()
                            },
                            shadow: iced::Shadow::default(),
                            snap: true,
                        }
                    }),
            );
        }
        if any_selected {
            action_row = action_row.push(
                button(text("Clear All").size(9).color(theme::TEXT_MUTED))
                    .on_press(ViewerMessage::ClearConsumes)
                    .padding([2, 6])
                    .style(|_theme: &iced::Theme, status| {
                        let bg = match status {
                            button::Status::Hovered => Some(iced::Background::Color(
                                Color::from_rgba8(255, 80, 80, 0.15),
                            )),
                            _ => None,
                        };
                        button::Style {
                            background: bg,
                            text_color: theme::TEXT_MUTED,
                            border: iced::Border {
                                radius: 3.0.into(),
                                ..Default::default()
                            },
                            shadow: iced::Shadow::default(),
                            snap: true,
                        }
                    }),
            );
        }

        let sidebar = container(
            column![
                row![
                    text("View Mode").size(10).color(theme::TEXT_MUTED),
                    Space::new().width(Fill),
                    mode_toggle,
                ]
                .align_y(Center),
                rule::horizontal(1),
                text("Categories").size(10).color(theme::TEXT_MUTED),
                scrollable(category_list).height(Fill),
                action_row,
            ]
            .spacing(6)
            .width(Fill)
            .height(Fill),
        )
        .padding(8)
        .width(Length::Fixed(260.0))
        .height(Fill)
        .style(|_theme: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(Color::from_rgba8(20, 22, 28, 0.6))),
            border: iced::Border {
                color: theme::SURFACE_BORDER,
                width: 1.0,
                radius: 4.0.into(),
            },
            ..Default::default()
        });

        // ── Chart area ──────────────────────────────────────────────
        let consume_layout =
            build_consume_layout(td, &self.tracked_consume_categories, self.consume_view_mode);

        let chart_area: Element<ViewerMessage> = if consume_layout.is_empty() {
            container(
                text("Select categories from the sidebar to view consumable usage on the timeline")
                    .size(13)
                    .color(theme::TEXT_MUTED),
            )
            .padding(40)
            .center_x(Fill)
            .center_y(Fill)
            .width(Fill)
            .height(Fill)
            .into()
        } else {
            let chart_height = consume_chart_height(&consume_layout);
            // Allow the chart to take decent vertical space — use a minimum
            // height of 300px or the natural chart height, whichever is larger.
            let canvas_height = chart_height.max(300.0);

            let consume_canvas = canvas::Canvas::new(ConsumeChart {
                data: td,
                layout: consume_layout,
                mode: self.consume_view_mode,
                hover_second: self.consume_hover_second,
                zoom: None, // No zoom in the Consumes tab
            })
            .width(Fill)
            .height(canvas_height);

            // Build tooltip as a fixed-height strip so it doesn't cause layout
            // jumping when content appears/disappears on hover.
            let consume_tooltip = self.build_consume_tooltip(td);

            // Use a stack to overlay the tooltip at the bottom of the chart
            // area so it doesn't push the canvas around.
            let chart_scroll: Element<ViewerMessage> = scrollable(
                container(consume_canvas).width(Fill).padding(iced::Padding {
                    top: 0.0,
                    right: 0.0,
                    bottom: 22.0,
                    left: 0.0,
                }),
            )
            .width(Fill)
            .height(Fill)
            .into();

            // Tooltip pinned to the bottom of the chart area
            let tooltip_layer: Element<ViewerMessage> = container(consume_tooltip)
                .width(Fill)
                .height(Fill)
                .align_y(iced::alignment::Vertical::Bottom)
                .into();

            iced::widget::stack![chart_scroll, tooltip_layer]
                .width(Fill)
                .height(Fill)
                .into()
        };

        // ── Assemble: header + horizontal split (sidebar | chart) ───
        let body = row![sidebar, chart_area]
            .spacing(12)
            .width(Fill)
            .height(Fill);

        container(
            column![header, rule::horizontal(1), body]
                .spacing(8)
                .width(Fill)
                .height(Fill),
        )
        .padding(12)
        .width(Fill)
        .height(Fill)
        .style(panel_style)
        .into()
    }

    /// Build the hover tooltip for the consumable chart.
    ///
    /// Shows both active buff intervals (for bar categories) and nearby
    /// consumable use events (for tick categories / instant-use items).
    fn build_consume_tooltip<'a>(
        &self,
        td: &TimelineData,
    ) -> Element<'a, ViewerMessage> {
        let Some(second) = self.consume_hover_second else {
            return column![].into();
        };

        let mut parts: Vec<String> = Vec::new();

        // Active aura intervals (bar data)
        // Aura intervals use encounter-relative offsets; translate to
        // consume-timeline coordinates before comparing with hover second.
        let segments = &td.consume_aura_offset_segments;
        for (aura_name, &cat) in &td.consume_aura_categories {
            if !self.tracked_consume_categories.contains(&cat) {
                continue;
            }
            if let Some(intervals) = td.aura_intervals.get(aura_name.as_str()) {
                let mut active: Vec<&str> = intervals
                    .iter()
                    .filter(|iv| {
                        let start = translate_aura_to_consume(iv.start, segments);
                        let end = translate_aura_to_consume(iv.end, segments);
                        start <= second && second <= end
                    })
                    .map(|iv| iv.player.as_str())
                    .collect();
                active.sort_unstable();
                active.dedup();
                if !active.is_empty() {
                    parts.push(format!("{aura_name}: {}", active.join(", ")));
                }
            }
        }

        // Nearby consumable use events (tick data).
        // Use a hover window scaled to ~0.5% of the visible duration so ticks
        // are easy to hit regardless of how wide the timeline is.  Clamped to
        // a [5s, 30s] range for usability.
        let hover_radius = (td.consume_duration * 0.005).clamp(5.0, 30.0);
        let mut tick_items: HashMap<String, Vec<&str>> = HashMap::new();
        for mark in &td.consume_marks {
            if !self.tracked_consume_categories.contains(&mark.category) {
                continue;
            }
            // Skip items that have aura intervals (already shown as bars above)
            if td.aura_intervals.contains_key(mark.consumable.as_str())
                && self.consume_view_mode == log_data::ConsumeViewMode::Bars
            {
                continue;
            }
            if (mark.offset - second).abs() <= hover_radius {
                tick_items
                    .entry(mark.consumable.clone())
                    .or_default()
                    .push(&mark.player);
            }
        }
        // Sort and deduplicate tick items for display
        let mut tick_sorted: Vec<(&str, Vec<&str>)> = tick_items
            .iter()
            .map(|(name, players)| {
                let mut p = players.clone();
                p.sort_unstable();
                p.dedup();
                (name.as_str(), p)
            })
            .collect();
        tick_sorted.sort_by_key(|(name, _)| *name);
        for (name, players) in &tick_sorted {
            parts.push(format!("{name}: {}", players.join(", ")));
        }

        if parts.is_empty() {
            return column![].into();
        }

        let consume_accent = theme::TIMELINE_CONSUME;
        container(text(parts.join("  |  ")).size(11).color(Color::WHITE))
            .padding([3, 8])
            .style(move |_theme: &iced::Theme| container::Style {
                background: Some(iced::Background::Color(Color::from_rgba8(30, 30, 40, 0.9))),
                border: iced::Border {
                    color: Color {
                        a: 0.3,
                        ..consume_accent
                    },
                    width: 1.0,
                    radius: 4.0.into(),
                },
                ..Default::default()
            })
            .into()
    }

    // ── Raid Overview View ──────────────────────────────────────────────

    /// Raid-wide per-player expandable list: each player's consumables grouped
    /// by category, sorted by total usage descending. Click a player header to
    /// expand/collapse their consumable list.
    #[allow(clippy::too_many_lines)] // iced UI layout — per-player expandable list with category sections
    fn view_raid_overview<'a>(
        &'a self,
        consumables: &[&log_data::ConsumableUse],
    ) -> Element<'a, ViewerMessage> {
        if consumables.is_empty() {
            return empty_state("No consumable usage recorded");
        }

        // Aggregate: player → category → item → count
        let mut player_data: HashMap<&str, HashMap<ConsumableCategory, HashMap<&str, u64>>> =
            HashMap::new();
        for c in consumables {
            if self.log_data.combatants.contains_key(c.player.as_str()) {
                *player_data
                    .entry(&c.player)
                    .or_default()
                    .entry(c.category)
                    .or_default()
                    .entry(&c.consumable)
                    .or_insert(0) += 1;
            }
        }

        if player_data.is_empty() {
            return empty_state("No consumable usage recorded");
        }

        // Sort players by total consumable count descending
        let mut player_totals: Vec<(&str, u64)> = player_data
            .iter()
            .map(|(&player, cats)| {
                let total: u64 = cats.values().flat_map(HashMap::values).sum();
                (player, total)
            })
            .collect();
        player_totals.sort_by_key(|p| std::cmp::Reverse(p.1));

        let mut content = Column::new().spacing(4);

        for &(player, total) in &player_totals {
            let class = self.log_data.player_class(player);
            let class_color = theme::class_color(class);
            let is_collapsed = self.collapsed_consume_players.contains(player);

            // Player header row: icon + name + total + expand/collapse indicator
            let icon = image(theme::class_icon(class)).width(20).height(20);
            let arrow = if is_collapsed { "\u{25B6}" } else { "\u{25BC}" }; // ▶ or ▼

            let player_header: Element<ViewerMessage> = row![
                icon,
                text(player.to_string()).size(13).color(class_color),
                Space::new().width(Fill),
                text(format!("{total} uses"))
                    .size(12)
                    .color(theme::TEXT_SECONDARY),
                text(arrow).size(10).color(theme::TEXT_MUTED),
            ]
            .spacing(6)
            .align_y(Center)
            .width(Fill)
            .into();

            let header_btn = button(player_header)
                .on_press(ViewerMessage::ToggleConsumePlayer(player.to_string()))
                .padding([6, 8])
                .width(Fill)
                .style(player_header_button_style);

            content = content.push(header_btn);

            // If expanded, show consumables grouped by category
            if !is_collapsed && let Some(cats) = player_data.get(player) {
                let mut sorted_cats: Vec<ConsumableCategory> = cats.keys().copied().collect();
                sorted_cats.sort();

                let mut items_col = Column::new().spacing(1);

                for cat in &sorted_cats {
                    let cat_color = theme::consumable_category_color(*cat);
                    let items = &cats[cat];

                    // Sort items by count descending
                    let mut sorted_items: Vec<(&str, u64)> =
                        items.iter().map(|(&name, &count)| (name, count)).collect();
                    sorted_items.sort_by_key(|i| std::cmp::Reverse(i.1));

                    let cat_total: u64 = sorted_items.iter().map(|(_, c)| *c).sum();

                    // Category sub-header
                    let dot: Element<ViewerMessage> = container("")
                        .width(6)
                        .height(6)
                        .style(move |_theme: &iced::Theme| container::Style {
                            background: Some(iced::Background::Color(cat_color)),
                            border: iced::Border {
                                radius: 3.0.into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        })
                        .into();

                    items_col = items_col.push(
                        row![
                            Space::new().width(28), // indent past icon
                            dot,
                            text(cat.to_string()).size(11).color(cat_color),
                            text(format!("({cat_total})"))
                                .size(10)
                                .color(theme::TEXT_MUTED),
                        ]
                        .spacing(4)
                        .align_y(Center),
                    );

                    // Item rows
                    for (name, count) in &sorted_items {
                        items_col = items_col.push(
                            row![
                                Space::new().width(44), // indent past icon + dot
                                text((*name).to_string())
                                    .size(11)
                                    .color(Color::from_rgb8(200, 205, 210))
                                    .width(Length::FillPortion(4)),
                                text(count.to_string())
                                    .size(11)
                                    .color(theme::TEXT_SECONDARY)
                                    .width(Length::FillPortion(1)),
                            ]
                            .spacing(4),
                        );
                    }
                }

                content = content.push(items_col);
                content = content.push(Space::new().height(4));
            }
        }

        content.width(Fill).into()
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

/// Button style for the Bars/Ticks mode toggle in the consumable timeline sidebar.
fn consume_mode_button_style(
    status: button::Status,
    active: bool,
    accent: Color,
) -> button::Style {
    let bg = if active {
        Some(iced::Background::Color(Color { a: 0.3, ..accent }))
    } else {
        match status {
            button::Status::Hovered => Some(iced::Background::Color(Color::from_rgba8(
                255, 255, 255, 0.06,
            ))),
            _ => None,
        }
    };
    button::Style {
        background: bg,
        text_color: if active {
            Color::WHITE
        } else {
            theme::TEXT_MUTED
        },
        border: iced::Border {
            radius: 3.0.into(),
            color: if active {
                Color { a: 0.4, ..accent }
            } else {
                Color::TRANSPARENT
            },
            width: if active { 1.0 } else { 0.0 },
        },
        shadow: iced::Shadow::default(),
        snap: true,
    }
}

/// Button style for player headers in the raid overview — subtle background
/// with a slightly more visible hover state than transparent buttons.
fn player_header_button_style(_theme: &iced::Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => Color::from_rgba(1.0, 1.0, 1.0, 0.08),
        _ => Color::from_rgba(1.0, 1.0, 1.0, 0.03),
    };
    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: Color::WHITE,
        border: iced::Border {
            color: Color::from_rgba(1.0, 1.0, 1.0, 0.05),
            width: 1.0,
            radius: 4.0.into(),
        },
        shadow: iced::Shadow::default(),
        snap: true,
    }
}
