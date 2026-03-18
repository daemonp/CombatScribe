//! Loot tab rendering: boss-grouped item tables with search and quality filtering.

#[allow(clippy::wildcard_imports)]
// viewer UI — many shared component functions used throughout
use super::components::*;
#[allow(clippy::wildcard_imports)] // viewer UI — many shared types/widgets used throughout
use super::*;
use iced::widget::column;

// ── Loot Tab ────────────────────────────────────────────────────────────────

impl ViewerState {
    #[allow(clippy::too_many_lines)] // iced UI layout — grouped loot with boss sections
    pub(super) fn view_loot_tab(&self) -> Element<'_, ViewerMessage> {
        let search_input = text_input("Search items or players...", &self.loot_search)
            .on_input(ViewerMessage::SetLootSearch)
            .padding(6)
            .width(Length::FillPortion(2));

        let expand_btn = button(text("Expand All").size(12))
            .on_press(ViewerMessage::ExpandAllLoot)
            .padding([4, 12]);
        let collapse_btn = button(text("Collapse All").size(12))
            .on_press(ViewerMessage::CollapseAllLoot)
            .padding([4, 12]);

        let header = row![
            text("Loot").size(16),
            Space::new().width(Fill),
            search_input,
            expand_btn,
            collapse_btn,
        ]
        .spacing(8)
        .align_y(Center)
        .width(Fill);

        // Filter loot
        let loot = self.get_filtered_loot();
        let filtered_loot: Vec<&LootEvent> = if self.loot_search.is_empty() {
            loot.iter()
                .copied()
                .filter(|l| l.quality.is_notable())
                .collect()
        } else {
            let term = self.loot_search.to_lowercase();
            loot.iter()
                .copied()
                .filter(|l| {
                    l.item_name.to_lowercase().contains(&term)
                        || l.player.to_lowercase().contains(&term)
                        || l.boss.to_lowercase().contains(&term)
                })
                .collect()
        };

        let mut content_col = Column::new().spacing(4);

        if filtered_loot.is_empty() {
            let msg = if loot.is_empty() {
                "No loot recorded"
            } else {
                "No notable loot (green+ quality) recorded"
            };
            content_col = content_col.push(empty_state(msg));
        } else {
            // Group by boss
            let mut by_boss: Vec<(String, Vec<&LootEvent>)> = Vec::new();
            let mut boss_order: Vec<String> = Vec::new();
            let mut boss_map: HashMap<String, Vec<&LootEvent>> = HashMap::new();

            for item in &filtered_loot {
                boss_map.entry(item.boss.clone()).or_default().push(item);
                if !boss_order.contains(&item.boss) {
                    boss_order.push(item.boss.clone());
                }
            }

            for boss in boss_order {
                if let Some(items) = boss_map.remove(&boss) {
                    by_boss.push((boss, items));
                }
            }

            for (boss, mut items) in by_boss {
                let is_collapsed = self.collapsed_bosses.contains(&boss);

                // Sort by quality
                items.sort_by(|a, b| {
                    b.quality
                        .cmp(&a.quality)
                        .then_with(|| a.item_name.cmp(&b.item_name))
                });

                let arrow = if is_collapsed { ">" } else { "v" };
                let boss_header = button(
                    row![
                        text(arrow).size(12),
                        text(boss.clone()).size(14),
                        text(format!("({} items)", items.len()))
                            .size(12)
                            .color([0.5, 0.5, 0.5]),
                    ]
                    .spacing(8)
                    .align_y(Center),
                )
                .on_press(ViewerMessage::ToggleBossCollapse(boss.clone()))
                .padding([6, 8])
                .width(Fill);

                content_col = content_col.push(boss_header);

                if !is_collapsed {
                    for item in &items {
                        let item_color = theme::quality_color(item.quality);
                        let player_color =
                            theme::class_color(self.log_data.player_class(&item.player));

                        let item_name = if item.quantity > 1 {
                            format!("{} x{}", item.item_name, item.quantity)
                        } else {
                            item.item_name.clone()
                        };

                        let mut item_row = Row::new().spacing(8).align_y(Center);
                        item_row = item_row.push(text("  ").size(12)); // indent
                        item_row = item_row.push(text(item_name).size(13).color(item_color));
                        item_row = item_row.push(Space::new().width(Fill));
                        item_row = item_row.push(text(&item.player).size(13).color(player_color));

                        if let Some(traded_to) = &item.traded_to {
                            let traded_color =
                                theme::class_color(self.log_data.player_class(traded_to));
                            item_row =
                                item_row.push(text("Traded to").size(11).color(theme::TEXT_MUTED));
                            item_row = item_row.push(text(traded_to).size(13).color(traded_color));
                        }

                        content_col = content_col
                            .push(container(item_row.width(Fill)).padding([2, 8]).width(Fill));
                    }
                }
            }
        }

        scrollable(
            container(
                column![header, rule::horizontal(1), content_col]
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

    fn get_filtered_loot(&self) -> Vec<&LootEvent> {
        match &self.encounter_filter {
            EncounterFilter::All => self.log_data.loot.iter().collect(),
            EncounterFilter::AllKills | EncounterFilter::AllWipes => {
                let selected = self.log_data.selected_encounters(&self.encounter_filter);
                let boss_names: HashSet<&str> =
                    selected.iter().filter_map(|e| e.name.as_deref()).collect();
                self.log_data
                    .loot
                    .iter()
                    .filter(|l| boss_names.contains(l.boss.as_str()))
                    .collect()
            }
            EncounterFilter::AllTrash => self
                .log_data
                .loot
                .iter()
                .filter(|l| l.boss == "Trash/Other")
                .collect(),
            EncounterFilter::Single(idx) => {
                if let Some(enc) = self.log_data.encounters.get(*idx)
                    && let Some(name) = &enc.name
                {
                    return self
                        .log_data
                        .loot
                        .iter()
                        .filter(|l| l.boss == *name)
                        .collect();
                }
                Vec::new()
            }
        }
    }
}
