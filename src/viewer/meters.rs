#[allow(clippy::wildcard_imports)]
// viewer UI — many shared component functions used throughout
use super::components::*;
#[allow(clippy::wildcard_imports)] // viewer UI — many shared types/widgets used throughout
use super::*;
use iced::widget::column;

// ── Meters Tab ──────────────────────────────────────────────────────────────

impl ViewerState {
    #[allow(clippy::too_many_lines)] // iced UI layout — side-by-side panels with data collection
    pub(super) fn view_meters_tab(&self) -> Element<'_, ViewerMessage> {
        let (stats, duration) = self.log_data.filtered_stats(&self.encounter_filter);

        // Collect owned data for damage panel
        let damage_types_list = vec![
            DamageType::Damage,
            DamageType::DamageWithPets,
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
                    DamageType::DamageWithPets => ps.damage + ps.pet_damage,
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
        let dps = per_second(dmg_total, duration);
        let dmg_total_text = format!(
            "{} ({}/s)",
            theme::format_number(dmg_total),
            theme::format_number_f64(dps)
        );

        // Collect owned data for healing panel
        let mut healing_players: Vec<(String, String, u64)> = stats
            .iter()
            .filter_map(|(name, ps)| {
                if !self.log_data.combatants.contains_key(name) {
                    return None;
                }
                let value = match self.healing_type {
                    HealingType::Effective => ps.effective_healing,
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
        let hps = per_second(heal_total, duration);
        let heal_total_text = format!(
            "{} ({}/s)",
            theme::format_number(heal_total),
            theme::format_number_f64(hps)
        );

        // Build damage panel
        let dmg_type_picker = pick_list(damage_types_list, Some(self.damage_type), |dt| {
            ViewerMessage::SetDamageType(dt)
        })
        .width(Fill)
        .padding(4);

        let dmg_header = row![
            dmg_type_picker,
            text(dmg_total_text).size(12).color(theme::TEXT_SECONDARY)
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
            dmg_col = dmg_col.push(self.meter_bar_row(
                rank + 1,
                name,
                class,
                *value,
                pps,
                percent,
                theme::class_color(class),
                Some((name.clone(), dmg_detail_type)),
            ));
        }

        let damage_panel: Element<ViewerMessage> = container(
            column![dmg_header, horizontal_rule(1), dmg_col]
                .spacing(8)
                .width(Fill),
        )
        .padding(12)
        .width(Length::FillPortion(1))
        .style(panel_style)
        .into();

        // Build healing panel
        let healing_types_list = vec![
            HealingType::Effective,
            HealingType::Raw,
            HealingType::Overhealing,
        ];

        let heal_type_picker = pick_list(healing_types_list, Some(self.healing_type), |ht| {
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
            heal_col = heal_col.push(self.meter_bar_row(
                rank + 1,
                name,
                class,
                *value,
                pps,
                percent,
                theme::class_color(class),
                Some((name.clone(), DetailType::Healing)),
            ));
        }

        let healing_panel: Element<ViewerMessage> = container(
            column![heal_header, horizontal_rule(1), heal_col]
                .spacing(8)
                .width(Fill),
        )
        .padding(12)
        .width(Length::FillPortion(1))
        .style(panel_style)
        .into();

        scrollable(row![damage_panel, healing_panel].spacing(12).width(Fill))
            .height(Fill)
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
        let dispel_types = vec![DispelSubType::Dispels, DispelSubType::Interrupts];
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
            column![header, horizontal_rule(1), meter_col]
                .spacing(8)
                .width(Fill),
        )
        .padding(12)
        .width(Length::FillPortion(1))
        .style(panel_style)
        .into()
    }

    fn view_death_panel(&self) -> Element<'_, ViewerMessage> {
        let death_types = vec![
            DeathSubType::Deaths,
            DeathSubType::Resurrects,
            DeathSubType::Absorbs,
            DeathSubType::Avoidance,
            DeathSubType::Buffs,
            DeathSubType::Consumables,
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
            DeathSubType::Consumables => self.view_consumables_content(),
        };

        let header = row![
            type_picker,
            text(total_text).size(12).color(theme::TEXT_SECONDARY),
        ]
        .spacing(8)
        .align_y(Center);

        container(
            column![header, horizontal_rule(1), content]
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
            None,
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

    fn view_consumables_content(&self) -> (Element<'_, ViewerMessage>, String) {
        let consumables = self.log_data.filtered_consumables(&self.encounter_filter);
        let players = aggregate_by_player(
            &consumables,
            |c| &c.player,
            &self.log_data.combatants,
            |n| self.log_data.player_class(n),
        );
        view_event_meters_owned(
            &players,
            theme::BAR_CONSUMABLE,
            "No consumable usage recorded",
            Some(DetailType::Consumables),
            "uses",
        )
    }
}
