//! Meters tab rendering: damage and healing meter bars with the utility sub-panel.

#[allow(clippy::wildcard_imports)]
// viewer UI — many shared component functions used throughout
use super::components::*;
#[allow(clippy::wildcard_imports)] // viewer UI — many shared types/widgets used throughout
use super::*;
use iced::widget::{column, tooltip};

// ── Meters Tab ──────────────────────────────────────────────────────────────

/// Collected per-player healing data for combined view rendering.
struct HealingPlayer {
    name: String,
    class: String,
    effective: u64,
    overheal: u64,
}

impl ViewerState {
    #[allow(clippy::too_many_lines)] // iced UI layout — side-by-side panels with data collection
    pub(super) fn view_meters_tab(&self) -> Element<'_, ViewerMessage> {
        let (stats, duration) = self.log_data.filtered_stats(&self.encounter_filter);

        // ── Damage panel data ───────────────────────────────────────
        let damage_types_list: &[DamageType] = &[
            DamageType::Damage,
            DamageType::DamagePersonal,
            DamageType::DamageTaken,
        ];

        let mut damage_players: Vec<(String, String, u64)> = stats
            .iter()
            .filter_map(|(name, ps)| {
                if !self.log_data.combatants.contains_key(name) {
                    return None;
                }
                let value = match self.damage_type {
                    DamageType::Damage => ps.damage,
                    DamageType::DamagePersonal => ps.damage.saturating_sub(ps.pet_damage),
                    DamageType::DamageTaken => ps.damage_taken,
                };
                if value == 0 {
                    return None;
                }
                Some((
                    name.clone(),
                    self.log_data.player_class(name).to_string(),
                    value,
                ))
            })
            .collect();
        damage_players.sort_by_key(|p| Reverse(p.2));

        let dmg_total: u64 = damage_players.iter().map(|(_, _, v)| *v).sum();

        // Adaptive 75%-rule scaling for damage
        let dmg_max = damage_players.first().map_or(0, |(_, _, v)| *v);
        let dmg_scale = adaptive_scale(dmg_max, 0); // no stacked values for damage

        let dmg_total_ps = per_second(dmg_total, duration);
        let dmg_total_text = format!(
            "{} ({}/s)",
            theme::format_number(dmg_total),
            theme::format_number_f64(dmg_total_ps)
        );

        let dmg_type_picker = pick_list(damage_types_list, Some(self.damage_type), |dt| {
            ViewerMessage::SetDamageType(dt)
        })
        .width(Fill)
        .padding(4);

        let dmg_header = row![
            dmg_type_picker,
            text(dmg_total_text).size(12).color(theme::TEXT_SECONDARY),
        ]
        .spacing(8)
        .align_y(Center);

        let dmg_detail_type = match self.damage_type {
            DamageType::DamageTaken => DetailType::DamageTaken,
            _ => DetailType::Damage,
        };

        let mut dmg_col = Column::new().spacing(3);
        for (rank, (name, class, value)) in damage_players.iter().enumerate() {
            let pps = per_second(*value, duration);
            let percent = percent_of(*value, dmg_total);
            let bar_pct = scaled_percent(*value, dmg_scale);

            let value_text = format!(
                "{} - {}/s",
                theme::format_number(*value),
                theme::format_number_f64(pps)
            );
            let pct_text = format!("{percent:.1}%");

            let meter_row = build_meter_row(
                rank + 1,
                name,
                class,
                &value_text,
                &pct_text,
                bar_pct,
                theme::class_color(class),
                Some((name.clone(), dmg_detail_type)),
            );

            // Wrap in tooltip with player stats summary
            let row_with_tooltip: Element<ViewerMessage> =
                if let Some(ps) = stats.get(name.as_str()) {
                    let tip = if self.damage_type == DamageType::DamageTaken {
                        build_damage_taken_tooltip(name, class, ps, duration)
                    } else {
                        build_damage_tooltip(name, class, ps, *value, duration)
                    };
                    tooltip(meter_row, tip, tooltip::Position::FollowCursor)
                        .gap(4)
                        .snap_within_viewport(true)
                        .into()
                } else {
                    meter_row
                };

            dmg_col = dmg_col.push(row_with_tooltip);
        }

        let damage_panel: Element<ViewerMessage> = container(
            column![dmg_header, rule::horizontal(1), dmg_col]
                .spacing(8)
                .width(Fill),
        )
        .padding(12)
        .width(Length::FillPortion(1))
        .style(panel_style)
        .into();

        // ── Healing panel data ──────────────────────────────────────
        let healing_types_list: &[HealingType] = &[
            HealingType::Healing,
            HealingType::Effective,
            HealingType::Raw,
            HealingType::Overhealing,
        ];

        let healing_panel: Element<ViewerMessage> = if self.healing_type == HealingType::Healing {
            self.build_combined_healing_panel(&stats, duration, healing_types_list)
        } else {
            self.build_simple_healing_panel(&stats, duration, healing_types_list)
        };

        scrollable(row![damage_panel, healing_panel].spacing(12).width(Fill))
            .height(Fill)
            .into()
    }

    /// Build the combined "Healing" panel with stacked effective + overheal bars.
    #[allow(clippy::too_many_lines)] // iced UI layout — combined healing with stacked bars
    fn build_combined_healing_panel(
        &self,
        stats: &HashMap<String, log_data::PlayerStats>,
        duration: f64,
        healing_types_list: &[HealingType],
    ) -> Element<'_, ViewerMessage> {
        // Collect healing data with both effective and overheal values
        let mut players: Vec<HealingPlayer> = stats
            .iter()
            .filter_map(|(name, ps)| {
                if !self.log_data.combatants.contains_key(name) {
                    return None;
                }
                if ps.effective_healing == 0 && ps.overhealing == 0 {
                    return None;
                }
                Some(HealingPlayer {
                    name: name.clone(),
                    class: self.log_data.player_class(name).to_string(),
                    effective: ps.effective_healing,
                    overheal: ps.overhealing,
                })
            })
            .collect();
        // Sort by effective healing (descending)
        players.sort_by_key(|p| Reverse(p.effective));

        let total_effective: u64 = players.iter().map(|p| p.effective).sum();
        let total_overheal: u64 = players.iter().map(|p| p.overheal).sum();

        // Adaptive scale: top effective at 75%, safe_scale prevents overflow
        let max_effective = players.first().map_or(0, |p| p.effective);
        let max_combined = players
            .iter()
            .map(|p| p.effective + p.overheal)
            .max()
            .unwrap_or(0);
        let scale = adaptive_scale(max_effective, max_combined);

        let total_eff_ps = per_second(total_effective, duration);
        let oh_pct = if total_effective + total_overheal > 0 {
            total_overheal as f64 / (total_effective + total_overheal) as f64 * 100.0
        } else {
            0.0
        };

        let header_text = format!(
            "{} eff ({} overheal, {oh_pct:.1}%) - {}/s",
            theme::format_number(total_effective),
            theme::format_number(total_overheal),
            theme::format_number_f64(total_eff_ps),
        );

        let heal_type_picker =
            pick_list(healing_types_list.to_vec(), Some(self.healing_type), |ht| {
                ViewerMessage::SetHealingType(ht)
            })
            .width(Fill)
            .padding(4);

        let header = row![
            heal_type_picker,
            text(header_text).size(12).color(theme::TEXT_SECONDARY),
        ]
        .spacing(8)
        .align_y(Center);

        let mut heal_col = Column::new().spacing(3);
        for (rank, player) in players.iter().enumerate() {
            let main_pct = scaled_percent(player.effective, scale);
            let stacked_pct = scaled_percent(player.overheal, scale);

            let player_oh_pct = if player.effective + player.overheal > 0 {
                player.overheal as f64 / (player.effective + player.overheal) as f64 * 100.0
            } else {
                0.0
            };

            let eff_ps = per_second(player.effective, duration);
            let value_text = format!(
                "{} eff ({player_oh_pct:.1}% oh) - {}/s",
                theme::format_number(player.effective),
                theme::format_number_f64(eff_ps),
            );

            let class_str = player.class.clone();
            let class_color = theme::class_color(&class_str);
            let player_name = player.name.clone();

            // Inner bar content: rank + name left, stats right
            let inner: Row<'_, ViewerMessage> = row![
                text(format!("{}.", rank + 1))
                    .size(12)
                    .color(theme::TEXT_MUTED),
                text(player_name.clone()).size(12).color(class_color),
                Space::new().width(Fill),
                text(value_text)
                    .size(12)
                    .color(Color::from_rgb8(220, 225, 230)),
            ]
            .spacing(6)
            .align_y(Center)
            .width(Fill);

            // Class icon
            let icon = image(theme::class_icon(&class_str)).width(22).height(22);

            // Stacked bar
            let bar = make_stacked_bar(inner, main_pct, stacked_pct, class_color);

            let full_row: Element<ViewerMessage> = row![icon, bar]
                .spacing(6)
                .align_y(Center)
                .width(Fill)
                .into();

            let clickable: Element<ViewerMessage> = button(full_row)
                .on_press(ViewerMessage::ShowDetail(
                    player_name.clone(),
                    DetailType::Healing,
                ))
                .padding(0)
                .width(Fill)
                .style(transparent_button_style)
                .into();

            // Wrap in tooltip with healing stats summary
            let row_with_tooltip: Element<ViewerMessage> =
                if let Some(ps) = stats.get(player_name.as_str()) {
                    let tip = build_healing_tooltip(&player_name, &class_str, ps, duration);
                    tooltip(clickable, tip, tooltip::Position::FollowCursor)
                        .gap(4)
                        .snap_within_viewport(true)
                        .into()
                } else {
                    clickable
                };

            heal_col = heal_col.push(row_with_tooltip);
        }

        if players.is_empty() {
            heal_col = heal_col.push(empty_state("No healing recorded"));
        }

        container(
            column![header, rule::horizontal(1), heal_col]
                .spacing(8)
                .width(Fill),
        )
        .padding(12)
        .width(Length::FillPortion(1))
        .style(panel_style)
        .into()
    }

    /// Build a simple (non-combined) healing panel for Effective/Raw/Overhealing modes.
    fn build_simple_healing_panel(
        &self,
        stats: &HashMap<String, log_data::PlayerStats>,
        duration: f64,
        healing_types_list: &[HealingType],
    ) -> Element<'_, ViewerMessage> {
        let mut healing_players: Vec<(String, String, u64)> = stats
            .iter()
            .filter_map(|(name, ps)| {
                if !self.log_data.combatants.contains_key(name) {
                    return None;
                }
                let value = match self.healing_type {
                    HealingType::Healing | HealingType::Effective => ps.effective_healing,
                    HealingType::Raw => ps.healing,
                    HealingType::Overhealing => ps.overhealing,
                };
                if value == 0 {
                    return None;
                }
                Some((
                    name.clone(),
                    self.log_data.player_class(name).to_string(),
                    value,
                ))
            })
            .collect();
        healing_players.sort_by_key(|p| Reverse(p.2));

        let heal_total: u64 = healing_players.iter().map(|(_, _, v)| *v).sum();
        let heal_max = healing_players.first().map_or(0, |(_, _, v)| *v);
        let heal_scale = adaptive_scale(heal_max, 0);

        let hps_total = per_second(heal_total, duration);
        let heal_total_text = format!(
            "{} ({}/s)",
            theme::format_number(heal_total),
            theme::format_number_f64(hps_total)
        );

        let heal_type_picker =
            pick_list(healing_types_list.to_vec(), Some(self.healing_type), |ht| {
                ViewerMessage::SetHealingType(ht)
            })
            .width(Fill)
            .padding(4);

        let heal_header = row![
            heal_type_picker,
            text(heal_total_text).size(12).color(theme::TEXT_SECONDARY),
        ]
        .spacing(8)
        .align_y(Center);

        let mut heal_col = Column::new().spacing(3);
        for (rank, (name, class, value)) in healing_players.iter().enumerate() {
            let pps = per_second(*value, duration);
            let percent = percent_of(*value, heal_total);
            let bar_pct = scaled_percent(*value, heal_scale);

            let value_text = format!(
                "{} - {}/s",
                theme::format_number(*value),
                theme::format_number_f64(pps)
            );
            let pct_text = format!("{percent:.1}%");

            let meter_row = build_meter_row(
                rank + 1,
                name,
                class,
                &value_text,
                &pct_text,
                bar_pct,
                theme::class_color(class),
                Some((name.clone(), DetailType::Healing)),
            );

            // Wrap in tooltip with healing stats summary
            let row_with_tooltip: Element<ViewerMessage> =
                if let Some(ps) = stats.get(name.as_str()) {
                    let tip = build_healing_tooltip(name, class, ps, duration);
                    tooltip(meter_row, tip, tooltip::Position::FollowCursor)
                        .gap(4)
                        .snap_within_viewport(true)
                        .into()
                } else {
                    meter_row
                };

            heal_col = heal_col.push(row_with_tooltip);
        }

        if healing_players.is_empty() {
            heal_col = heal_col.push(empty_state("No healing recorded"));
        }

        container(
            column![heal_header, rule::horizontal(1), heal_col]
                .spacing(8)
                .width(Fill),
        )
        .padding(12)
        .width(Length::FillPortion(1))
        .style(panel_style)
        .into()
    }

    // ── Utility Tab ─────────────────────────────────────────────────────

    pub(super) fn view_utility_tab(&self) -> Element<'_, ViewerMessage> {
        let left_panel = self.view_dispel_panel();
        let right_panel = self.view_death_panel();

        scrollable(row![left_panel, right_panel].spacing(12).width(Fill))
            .height(Fill)
            .into()
    }

    fn view_dispel_panel(&self) -> Element<'_, ViewerMessage> {
        let dispel_types: &[DispelSubType] = &[DispelSubType::Dispels, DispelSubType::Interrupts];
        let type_picker = pick_list(dispel_types, Some(self.dispel_type), |dt| {
            ViewerMessage::SetDispelType(dt)
        })
        .width(Fill)
        .padding(4);

        let (counts, bar_color, detail_type) = match self.dispel_type {
            DispelSubType::Dispels => {
                let dispels = self.log_data.filtered_dispels(&self.encounter_filter);
                let mut by_caster: HashMap<&str, u64> = HashMap::new();
                for d in &dispels {
                    *by_caster.entry(&d.caster).or_insert(0) += 1;
                }
                (by_caster, theme::BAR_DISPEL, DetailType::Dispels)
            }
            DispelSubType::Interrupts => {
                let interrupts = self.log_data.filtered_interrupts(&self.encounter_filter);
                let mut by_caster: HashMap<&str, u64> = HashMap::new();
                for i in &interrupts {
                    *by_caster.entry(&i.caster).or_insert(0) += 1;
                }
                (by_caster, theme::BAR_INTERRUPT, DetailType::Interrupts)
            }
        };

        let mut players: Vec<(&str, &str, u64)> = counts
            .into_iter()
            .filter(|(name, _)| self.log_data.combatants.contains_key(*name))
            .map(|(name, count)| (name, self.log_data.player_class(name), count))
            .collect();
        players.sort_by_key(|p| Reverse(p.2));
        let total: u64 = players.iter().map(|(_, _, v)| *v).sum();
        let total_text = format!("{total} total");

        let header = row![
            type_picker,
            text(total_text).size(12).color(theme::TEXT_SECONDARY),
        ]
        .spacing(8)
        .align_y(Center);

        let meter_col = build_simple_meters(
            &players,
            bar_color,
            match self.dispel_type {
                DispelSubType::Dispels => "No dispels recorded",
                DispelSubType::Interrupts => "No interrupts recorded",
            },
            Some(detail_type),
        );

        container(
            column![header, rule::horizontal(1), meter_col]
                .spacing(8)
                .width(Fill),
        )
        .padding(12)
        .width(Length::FillPortion(1))
        .style(panel_style)
        .into()
    }

    fn view_death_panel(&self) -> Element<'_, ViewerMessage> {
        let death_types: &[DeathSubType] = &[
            DeathSubType::Deaths,
            DeathSubType::Resurrects,
            DeathSubType::Absorbs,
            DeathSubType::Avoidance,
            DeathSubType::Buffs,
        ];
        let type_picker = pick_list(death_types, Some(self.death_type), |dt| {
            ViewerMessage::SetDeathType(dt)
        })
        .width(Fill)
        .padding(4);

        let (content, total_text) = match self.death_type {
            DeathSubType::Deaths => self.view_deaths_content(),
            DeathSubType::Resurrects => self.view_resurrects_content(),
            DeathSubType::Absorbs => self.view_absorbs_content(),
            DeathSubType::Avoidance => self.view_avoidance_content(),
            DeathSubType::Buffs => self.view_buffs_content(),
        };

        let header = row![
            type_picker,
            text(total_text).size(12).color(theme::TEXT_SECONDARY),
        ]
        .spacing(8)
        .align_y(Center);

        container(
            column![header, rule::horizontal(1), content]
                .spacing(8)
                .width(Fill),
        )
        .padding(12)
        .width(Length::FillPortion(1))
        .style(panel_style)
        .into()
    }

    fn view_deaths_content(&self) -> (Element<'_, ViewerMessage>, String) {
        let deaths = self.log_data.filtered_deaths(&self.encounter_filter);
        let players = aggregate_by_player(
            &deaths,
            |d| &d.player,
            &self.log_data.combatants,
            |n| self.log_data.player_class(n),
        );
        view_event_meters_owned(
            &players,
            theme::BAR_DEATH,
            "No deaths recorded",
            Some(DetailType::Deaths),
            "deaths",
        )
    }

    fn view_resurrects_content(&self) -> (Element<'_, ViewerMessage>, String) {
        let resurrects = self.log_data.filtered_resurrects(&self.encounter_filter);
        let players = aggregate_by_player(
            &resurrects,
            |r| &r.caster,
            &self.log_data.combatants,
            |n| self.log_data.player_class(n),
        );
        view_event_meters_owned(
            &players,
            theme::BAR_RESURRECT,
            "No resurrections recorded",
            Some(DetailType::Resurrects),
            "resurrects",
        )
    }

    fn view_absorbs_content(&self) -> (Element<'_, ViewerMessage>, String) {
        let mut players: Vec<(&str, &str, u64)> = self
            .log_data
            .absorbs
            .iter()
            .filter(|(name, amount)| {
                **amount > 0 && self.log_data.combatants.contains_key(name.as_str())
            })
            .map(|(name, amount)| (name.as_str(), self.log_data.player_class(name), *amount))
            .collect();
        players.sort_by_key(|p| Reverse(p.2));

        let total: u64 = players.iter().map(|(_, _, v)| *v).sum();
        let col = build_simple_meters(&players, theme::BAR_ABSORB, "No absorbs recorded", None);
        (
            col.into(),
            format!("{} absorbed", theme::format_number(total)),
        )
    }

    fn view_avoidance_content(&self) -> (Element<'_, ViewerMessage>, String) {
        let mut players: Vec<(&str, &str, u64, &AvoidanceStats)> = self
            .log_data
            .avoidance
            .iter()
            .filter(|(name, _)| self.log_data.combatants.contains_key(name.as_str()))
            .map(|(name, av)| {
                (
                    name.as_str(),
                    self.log_data.player_class(name),
                    av.total(),
                    av,
                )
            })
            .filter(|(_, _, total, _)| *total > 0)
            .collect();
        players.sort_by_key(|p| Reverse(p.2));

        let total: u64 = players.iter().map(|(_, _, v, _)| *v).sum();
        let max_val = players.first().map_or(1, |(_, _, v, _)| *v);

        let mut col = Column::new().spacing(2);
        for (rank, (name, class, value, av)) in players.iter().enumerate() {
            let percent = percent_of(*value, max_val);

            let mut breakdown = Vec::new();
            if av.dodges > 0 {
                breakdown.push(format!("{} dodge", av.dodges));
            }
            if av.parries > 0 {
                breakdown.push(format!("{} parry", av.parries));
            }
            if av.blocks > 0 {
                breakdown.push(format!("{} block", av.blocks));
            }
            if av.missed_by > 0 {
                breakdown.push(format!("{} miss", av.missed_by));
            }
            if av.full_resists > 0 {
                breakdown.push(format!("{} full resist", av.full_resists));
            }
            if av.full_absorbs > 0 {
                breakdown.push(format!("{} full absorb", av.full_absorbs));
            }
            if av.full_blocks > 0 {
                breakdown.push(format!("{} full block", av.full_blocks));
            }
            let detail_str = format!("{} ({})", value, breakdown.join(", "));

            col = col.push(self.meter_bar_row_with_detail_text(
                rank + 1,
                name,
                class,
                &detail_str,
                percent,
                theme::class_color("WARRIOR"),
                Some((name.to_string(), DetailType::Avoidance)),
            ));
        }

        if players.is_empty() {
            col = col.push(empty_state("No avoidance recorded"));
        }

        (col.into(), format!("{total} avoided"))
    }

    fn view_buffs_content(&self) -> (Element<'_, ViewerMessage>, String) {
        let mut players: Vec<(&str, &str, u64)> = self
            .log_data
            .buffs
            .iter()
            .filter(|(name, _)| self.log_data.combatants.contains_key(name.as_str()))
            .map(|(name, buffs)| {
                (
                    name.as_str(),
                    self.log_data.player_class(name),
                    buffs.len() as u64,
                )
            })
            .filter(|(_, _, count)| *count > 0)
            .collect();
        players.sort_by_key(|p| Reverse(p.2));

        let total_unique: HashSet<&String> = self
            .log_data
            .buffs
            .values()
            .flat_map(HashMap::keys)
            .collect();

        let col = build_simple_meters(
            &players,
            Color::WHITE,
            "No buff data recorded",
            Some(DetailType::Buffs),
        );
        (col.into(), format!("{} unique buffs", total_unique.len()))
    }
}
