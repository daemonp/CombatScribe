//! Detail overlay rendering: per-player ability breakdowns, damage taken, and opener display.

#[allow(clippy::wildcard_imports)]
// viewer UI — many shared component functions used throughout
use super::components::*;
#[allow(clippy::wildcard_imports)] // viewer UI — many shared types/widgets used throughout
use super::*;
use iced::widget::{column, table};

// ── Row Types for Table Widget ──────────────────────────────────────────────

/// Pre-computed row data for the ability breakdown table.
#[derive(Clone)]
struct AbilityRow {
    spell: String,
    display_total: u64,
    hits: u64,
    crits: u64,
    crit_pct: f64,
    /// Average non-crit hit value (None when no normal hits exist).
    avg_hit: Option<u64>,
    /// Average crit value (None when no crits exist).
    avg_crit: Option<u64>,
    percent: f64,
    /// Overheal percentage — present only for healing breakdowns.
    oh_pct: Option<f64>,
}

/// Pre-computed row data for the damage-taken-per-ability table.
#[derive(Clone)]
struct DamageTakenRow {
    spell: String,
    total: u64,
    hits: u64,
    avg_hit: Option<u64>,
    avg_crit: Option<u64>,
    crits: u64,
    absorbed: u64,
    resisted: u64,
    blocked: u64,
    crushing_hits: u64,
    glancing_hits: u64,
    percent: f64,
}

// ── Detail Overlay ──────────────────────────────────────────────────────────

impl ViewerState {
    pub(super) fn view_detail_overlay<'a>(
        &'a self,
        detail: &'a DetailView,
    ) -> Element<'a, ViewerMessage> {
        let close_btn = button(text("X").size(14))
            .on_press(ViewerMessage::CloseDetail)
            .padding([4, 10]);

        let title = match &detail.detail_type {
            DetailType::Damage => format!("{} - Damage Breakdown", detail.player_name),
            DetailType::DamageTaken => {
                format!("{} - Damage Taken Breakdown", detail.player_name)
            }
            DetailType::Healing => {
                format!("{} - {} Breakdown", detail.player_name, self.healing_type)
            }
            DetailType::Dispels => format!("{} - Dispel Breakdown", detail.player_name),
            DetailType::Interrupts => format!("{} - Interrupt Breakdown", detail.player_name),
            DetailType::Resurrects => format!("{} - Resurrection Breakdown", detail.player_name),
            DetailType::Avoidance => format!("{} - Avoidance Breakdown", detail.player_name),
            DetailType::Buffs => format!("{} - Buff Breakdown", detail.player_name),
            DetailType::Consumables => format!("{} - Consumable Usage", detail.player_name),
            DetailType::Deaths => format!("{} - Death Recap", detail.player_name),
        };

        // Player info bar (spec/race/guild) — shown for all detail types
        let player_info = self.view_player_info_bar(&detail.player_name);

        let header = row![text(title).size(18), Space::new().width(Fill), close_btn,]
            .spacing(8)
            .align_y(Center)
            .width(Fill);

        let content = match &detail.detail_type {
            DetailType::Damage | DetailType::Healing => {
                self.view_ability_breakdown(&detail.player_name, detail.detail_type)
            }
            DetailType::DamageTaken => self.view_damage_taken_breakdown(&detail.player_name),
            DetailType::Dispels => self.view_dispel_detail(&detail.player_name),
            DetailType::Interrupts => self.view_interrupt_detail(&detail.player_name),
            DetailType::Resurrects => self.view_resurrect_detail(&detail.player_name),
            DetailType::Avoidance => self.view_avoidance_detail(&detail.player_name),
            DetailType::Buffs => self.view_buff_detail(&detail.player_name),
            DetailType::Consumables => self.view_consumable_detail(&detail.player_name),
            DetailType::Deaths => self.view_death_detail(&detail.player_name),
        };

        scrollable(
            container(
                column![header, player_info, rule::horizontal(1), content]
                    .spacing(10)
                    .width(Fill),
            )
            .padding(14)
            .width(Fill)
            .style(panel_style),
        )
        .height(Fill)
        .into()
    }

    #[allow(clippy::too_many_lines)] // iced UI layout — ability table with opener sequence
    fn view_ability_breakdown(
        &self,
        player: &str,
        dtype: DetailType,
    ) -> Element<'_, ViewerMessage> {
        let (stats, duration) = self.log_data.filtered_stats(&self.encounter_filter);
        let Some(ps) = stats.get(player) else {
            return text("No data").size(14).into();
        };

        let is_healing_type = dtype == DetailType::Healing;

        let (abilities, total) = match dtype {
            DetailType::Damage => (&ps.abilities, ps.damage),
            DetailType::Healing => {
                let heal_total = match self.healing_type {
                    HealingType::Healing | HealingType::Effective => ps.effective_healing,
                    HealingType::Raw => ps.healing,
                    HealingType::Overhealing => ps.overhealing,
                };
                (&ps.healing_abilities, heal_total)
            }
            _ => return text("Invalid detail type").size(14).into(),
        };

        // Collect into owned data to avoid lifetime issues
        let mut sorted: Vec<(String, AbilityStats)> = abilities
            .iter()
            .map(|(spell, ab)| (spell.clone(), ab.clone()))
            .collect();
        sorted.sort_by_key(|s| Reverse(s.1.total));

        let total_hits: u64 = sorted.iter().map(|(_, a)| a.hits).sum();
        let total_crits: u64 = sorted.iter().map(|(_, a)| a.crits).sum();
        let total_crit_amount: u64 = sorted.iter().map(|(_, a)| a.crit_total).sum();
        let crit_rate = if total_hits > 0 {
            (total_crits as f64 / total_hits as f64) * 100.0
        } else {
            0.0
        };
        let pps = if duration > 0.0 {
            total as f64 / duration
        } else {
            0.0
        };

        // Compute aggregate avg hit / avg crit for summary line
        let normal_hits = total_hits.saturating_sub(total_crits);
        let noncrit_amount = total.saturating_sub(total_crit_amount);
        let avg_hit_str = noncrit_amount
            .checked_div(normal_hits)
            .map_or("-".to_string(), theme::format_number);
        let avg_crit_str = total_crit_amount
            .checked_div(total_crits)
            .map_or("-".to_string(), theme::format_number);

        let heal_value_label = match self.healing_type {
            HealingType::Healing | HealingType::Effective => "Effective",
            HealingType::Raw => "Raw",
            HealingType::Overhealing => "Overheal",
        };

        let summary_str = if dtype == DetailType::Healing {
            let overheal_pct = if ps.healing > 0 {
                #[allow(clippy::cast_precision_loss)] // healing values never approach 2^52
                let pct = ps.overhealing as f64 / ps.healing as f64 * 100.0;
                pct
            } else {
                0.0
            };
            format!(
                "{}: {} | Per Second: {}/s | Duration: {} | Overheal: {:.1}% | Hits: {} | Crits: {} | Crit Rate: {:.1}% | Avg Hit: {} | Avg Crit: {}",
                heal_value_label,
                theme::format_number(total),
                theme::format_number_f64(pps),
                theme::format_duration(duration),
                overheal_pct,
                total_hits,
                total_crits,
                crit_rate,
                avg_hit_str,
                avg_crit_str,
            )
        } else {
            format!(
                "Total: {} | Per Second: {}/s | Duration: {} | Hits: {} | Crits: {} | Crit Rate: {:.1}% | Avg Hit: {} | Avg Crit: {}",
                theme::format_number(total),
                theme::format_number_f64(pps),
                theme::format_duration(duration),
                total_hits,
                total_crits,
                crit_rate,
                avg_hit_str,
                avg_crit_str,
            )
        };
        let summary = text(summary_str).size(12).color(theme::TEXT_SECONDARY);

        // Opener
        let event_type = match dtype {
            DetailType::Damage => PlayerEventType::Damage,
            _ => PlayerEventType::Healing,
        };
        let opener = self
            .log_data
            .opener_sequence(player, event_type, &self.encounter_filter);

        let mut opener_section = Column::new();
        if !opener.is_empty() {
            opener_section =
                opener_section.push(text("Opener (first 10s)").size(13).color(theme::TEXT_MUTED));
            let mut opener_row = Row::new().spacing(4);
            for (i, o) in opener.iter().enumerate() {
                if i > 0 {
                    opener_row = opener_row.push(text("->").size(11).color(theme::TEXT_MUTED));
                }
                let crit_marker = if o.is_crit { "!" } else { "" };
                let delay_str = if o.delay > 0.0 {
                    format!(" +{:.1}s", o.delay)
                } else {
                    String::new()
                };
                opener_row = opener_row.push(
                    container(
                        column![
                            text(format!("{}. {}", i + 1, o.spell)).size(11),
                            text(format!(
                                "{}{}{}",
                                theme::format_number(o.amount),
                                crit_marker,
                                delay_str
                            ))
                            .size(10)
                            .color(theme::TEXT_SECONDARY),
                        ]
                        .spacing(2),
                    )
                    .padding(4)
                    .style(|_theme: &iced::Theme| container::Style {
                        background: Some(iced::Background::Color(Color::from_rgba8(
                            255, 255, 255, 0.05,
                        ))),
                        border: iced::Border {
                            radius: 3.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                );
            }
            opener_section = opener_section.push(opener_row);
        }

        // ── Ability Breakdown Bars ───────────────────────────────────
        //
        // Visual meter bars showing each ability's contribution to total,
        // placed above the detailed stats table.
        let class_str = self.log_data.player_class(player);
        let class_color = theme::class_color(class_str);
        let max_ability = sorted.first().map_or(0, |(_, a)| a.total);
        let bar_scale = adaptive_scale(max_ability, max_ability);

        let mut ability_bars = Column::new().spacing(2);
        #[allow(clippy::cast_precision_loss)] // ability totals never approach 2^52
        for (rank, (spell, ab)) in sorted.iter().enumerate() {
            let display_val = if is_healing_type {
                match self.healing_type {
                    HealingType::Healing | HealingType::Effective => ab.effective,
                    HealingType::Raw => ab.total,
                    HealingType::Overhealing => ab.overheal,
                }
            } else {
                ab.total
            };
            let aps = per_second(display_val, duration);
            let pct = percent_of(display_val, total);
            let bar_pct = scaled_percent(display_val, bar_scale);

            let value_text = format!(
                "{} - {}/s",
                theme::format_number(display_val),
                theme::format_number_f64(aps)
            );
            let pct_text = format!("{pct:.1}%");

            // Inner bar content: rank + spell left, value right
            let inner: Row<'_, ViewerMessage> = row![
                text(format!("{}. {spell}", rank + 1))
                    .size(11)
                    .color(Color::WHITE),
                Space::new().width(Fill),
                text(value_text)
                    .size(11)
                    .color(Color::from_rgb8(220, 225, 230)),
            ]
            .spacing(6)
            .align_y(Center)
            .width(Fill);

            let bar = make_bar(inner, bar_pct, class_color);

            // Percentage label (outside bar, right side)
            let pct_label = text(pct_text)
                .size(11)
                .color(theme::TEXT_MUTED)
                .width(45)
                .align_x(iced::alignment::Horizontal::Right);

            ability_bars =
                ability_bars.push(row![bar, pct_label].spacing(4).align_y(Center).width(Fill));
        }

        let bars_section = column![
            text("Ability Breakdown").size(13).color(theme::TEXT_MUTED),
            ability_bars,
        ]
        .spacing(4)
        .width(Fill);

        // Ability table — healing gets an extra "OH%" column
        let is_healing = dtype == DetailType::Healing;

        // Pre-compute row data for the table widget
        #[allow(clippy::cast_precision_loss)] // ability totals never approach 2^52
        let rows: Vec<AbilityRow> = sorted
            .iter()
            .map(|(spell, ab)| {
                let display_total = if is_healing {
                    match self.healing_type {
                        HealingType::Healing | HealingType::Effective => ab.effective,
                        HealingType::Raw => ab.total,
                        HealingType::Overhealing => ab.overheal,
                    }
                } else {
                    ab.total
                };
                let percent = if total > 0 {
                    display_total as f64 / total as f64 * 100.0
                } else {
                    0.0
                };
                let crit_pct = if ab.hits > 0 {
                    ab.crits as f64 / ab.hits as f64 * 100.0
                } else {
                    0.0
                };
                let normal_hits = ab.hits.saturating_sub(ab.crits);
                let noncrit_total = display_total.saturating_sub(ab.crit_total);
                let avg_hit = noncrit_total.checked_div(normal_hits);
                let avg_crit = ab.crit_total.checked_div(ab.crits);
                let oh_pct = if is_healing && ab.total > 0 {
                    Some(ab.overheal as f64 / ab.total as f64 * 100.0)
                } else {
                    None
                };
                AbilityRow {
                    spell: spell.clone(),
                    display_total,
                    hits: ab.hits,
                    crits: ab.crits,
                    crit_pct,
                    avg_hit,
                    avg_crit,
                    percent,
                    oh_pct,
                }
            })
            .collect();

        let header = |label: &str| text(label.to_string()).size(12);
        let value_label = if is_healing {
            heal_value_label.to_string()
        } else {
            "Total".to_string()
        };

        let fmt_avg =
            |val: Option<u64>| -> String { val.map_or("-".to_string(), theme::format_number) };

        let ability_table: Element<'_, ViewerMessage> = if is_healing {
            let columns = [
                table::column(header("Ability"), |r: AbilityRow| text(r.spell).size(12))
                    .width(Length::FillPortion(3)),
                table::column(header(&value_label), |r: AbilityRow| {
                    text(theme::format_number(r.display_total)).size(12)
                })
                .width(Length::FillPortion(1)),
                table::column(header("OH%"), |r: AbilityRow| {
                    text(format!("{:.1}%", r.oh_pct.unwrap_or(0.0)))
                        .size(12)
                        .color(theme::TEXT_SECONDARY)
                })
                .width(Length::FillPortion(1)),
                table::column(header("Hits"), |r: AbilityRow| {
                    text(r.hits.to_string()).size(12)
                })
                .width(Length::FillPortion(1)),
                table::column(header("Crits"), |r: AbilityRow| {
                    text(r.crits.to_string()).size(12)
                })
                .width(Length::FillPortion(1)),
                table::column(header("Crit%"), |r: AbilityRow| {
                    text(format!("{:.1}%", r.crit_pct)).size(12)
                })
                .width(Length::FillPortion(1)),
                table::column(header("Avg Hit"), move |r: AbilityRow| {
                    text(fmt_avg(r.avg_hit)).size(12)
                })
                .width(Length::FillPortion(1)),
                table::column(header("Avg Crit"), move |r: AbilityRow| {
                    text(fmt_avg(r.avg_crit)).size(12)
                })
                .width(Length::FillPortion(1)),
                table::column(header("%"), |r: AbilityRow| {
                    text(format!("{:.1}%", r.percent)).size(12)
                })
                .width(Length::FillPortion(1)),
            ];
            table::table(columns, rows)
                .padding_x(2)
                .padding_y(2)
                .separator_y(1)
                .into()
        } else {
            let columns = [
                table::column(header("Ability"), |r: AbilityRow| text(r.spell).size(12))
                    .width(Length::FillPortion(3)),
                table::column(header(&value_label), |r: AbilityRow| {
                    text(theme::format_number(r.display_total)).size(12)
                })
                .width(Length::FillPortion(1)),
                table::column(header("Hits"), |r: AbilityRow| {
                    text(r.hits.to_string()).size(12)
                })
                .width(Length::FillPortion(1)),
                table::column(header("Crits"), |r: AbilityRow| {
                    text(r.crits.to_string()).size(12)
                })
                .width(Length::FillPortion(1)),
                table::column(header("Crit%"), |r: AbilityRow| {
                    text(format!("{:.1}%", r.crit_pct)).size(12)
                })
                .width(Length::FillPortion(1)),
                table::column(header("Avg Hit"), move |r: AbilityRow| {
                    text(fmt_avg(r.avg_hit)).size(12)
                })
                .width(Length::FillPortion(1)),
                table::column(header("Avg Crit"), move |r: AbilityRow| {
                    text(fmt_avg(r.avg_crit)).size(12)
                })
                .width(Length::FillPortion(1)),
                table::column(header("%"), |r: AbilityRow| {
                    text(format!("{:.1}%", r.percent)).size(12)
                })
                .width(Length::FillPortion(1)),
            ];
            table::table(columns, rows)
                .padding_x(2)
                .padding_y(2)
                .separator_y(1)
                .into()
        };

        let table_section = column![
            text("Detailed Stats").size(13).color(theme::TEXT_MUTED),
            ability_table,
        ]
        .spacing(4)
        .width(Fill);

        column![
            summary,
            opener_section,
            bars_section,
            rule::horizontal(1),
            table_section,
        ]
        .spacing(10)
        .width(Fill)
        .into()
    }

    // ── Damage Taken Breakdown ──────────────────────────────────────────

    /// Render the damage taken detail view: grouped by source, showing each
    /// ability with mitigation columns (absorbed, resisted, blocked, crush,
    /// glancing).  This lets tanks compare incoming damage and their mitigation
    /// against other tanks on the same encounter.
    #[allow(clippy::too_many_lines)] // iced UI layout — source-grouped ability table with mitigation columns
    fn view_damage_taken_breakdown(&self, player: &str) -> Element<'_, ViewerMessage> {
        let (stats, duration) = self.log_data.filtered_stats(&self.encounter_filter);
        let Some(ps) = stats.get(player) else {
            return text("No data").size(14).into();
        };

        let total = ps.damage_taken;
        if total == 0 {
            return text("No damage taken").size(14).into();
        }

        #[allow(clippy::cast_precision_loss)] // damage values never approach 2^52
        let dtps = if duration > 0.0 {
            total as f64 / duration
        } else {
            0.0
        };

        // Aggregate source-level totals for sorting
        let mut source_totals: Vec<(String, u64)> = ps
            .damage_taken_breakdown
            .iter()
            .map(|(source, abilities)| {
                let src_total: u64 = abilities.values().map(|a| a.total).sum();
                (source.clone(), src_total)
            })
            .collect();
        source_totals.sort_by_key(|s| Reverse(s.1));

        let summary = text(format!(
            "Total Taken: {} | Per Second: {}/s | Duration: {}",
            theme::format_number(total),
            theme::format_number_f64(dtps),
            theme::format_duration(duration),
        ))
        .size(12)
        .color(theme::TEXT_SECONDARY);

        let mut content = Column::new().spacing(12);
        content = content.push(summary);

        let header = |label: &str| text(label.to_string()).size(11);

        for (source_name, source_total) in &source_totals {
            let Some(abilities) = ps.damage_taken_breakdown.get(source_name) else {
                continue;
            };

            // Source header with total and percentage
            #[allow(clippy::cast_precision_loss)] // damage values never approach 2^52
            let src_pct = if total > 0 {
                *source_total as f64 / total as f64 * 100.0
            } else {
                0.0
            };
            let source_header = row![
                text(source_name.clone())
                    .size(14)
                    .color(Color::from_rgb8(200, 160, 100)),
                Space::new().width(Fill),
                text(format!(
                    "{} ({:.1}%)",
                    theme::format_number(*source_total),
                    src_pct
                ))
                .size(12)
                .color(theme::TEXT_SECONDARY),
            ]
            .spacing(8)
            .align_y(Center)
            .width(Fill);

            // Sort abilities within this source by total descending, pre-compute rows
            let mut sorted_abilities: Vec<(&String, &log_data::DamageTakenAbilityStats)> =
                abilities.iter().collect();
            sorted_abilities.sort_by_key(|(_, a)| Reverse(a.total));

            #[allow(clippy::cast_precision_loss)] // damage values never approach 2^52
            let rows: Vec<DamageTakenRow> = sorted_abilities
                .iter()
                .map(|(spell_name, ab)| {
                    let normal_hits = ab.hits.saturating_sub(ab.crits);
                    let noncrit_amount = ab.total.saturating_sub(ab.crit_total);
                    let avg_hit = noncrit_amount.checked_div(normal_hits);
                    let avg_crit = ab.crit_total.checked_div(ab.crits);
                    let pct = if total > 0 {
                        ab.total as f64 / total as f64 * 100.0
                    } else {
                        0.0
                    };
                    DamageTakenRow {
                        spell: (*spell_name).clone(),
                        total: ab.total,
                        hits: ab.hits,
                        avg_hit,
                        avg_crit,
                        crits: ab.crits,
                        absorbed: ab.absorbed,
                        resisted: ab.resisted,
                        blocked: ab.blocked,
                        crushing_hits: ab.crushing_hits,
                        glancing_hits: ab.glancing_hits,
                        percent: pct,
                    }
                })
                .collect();

            let fmt_or_dash = |val: u64| -> String {
                if val > 0 {
                    theme::format_number(val)
                } else {
                    "-".to_string()
                }
            };

            let fmt_avg =
                |val: Option<u64>| -> String { val.map_or("-".to_string(), theme::format_number) };

            let columns = [
                table::column(header("Ability"), |r: DamageTakenRow| {
                    text(r.spell).size(12)
                })
                .width(Length::FillPortion(3)),
                table::column(header("Total"), |r: DamageTakenRow| {
                    text(theme::format_number(r.total)).size(12)
                })
                .width(Length::FillPortion(2)),
                table::column(header("Hits"), |r: DamageTakenRow| {
                    text(r.hits.to_string()).size(12)
                })
                .width(Length::FillPortion(1)),
                table::column(header("Avg Hit"), move |r: DamageTakenRow| {
                    text(fmt_avg(r.avg_hit)).size(12)
                })
                .width(Length::FillPortion(2)),
                table::column(header("Crit"), |r: DamageTakenRow| {
                    text(r.crits.to_string()).size(12)
                })
                .width(Length::FillPortion(1)),
                table::column(header("Avg Crit"), move |r: DamageTakenRow| {
                    text(fmt_avg(r.avg_crit)).size(12)
                })
                .width(Length::FillPortion(2)),
                table::column(header("Absorb"), move |r: DamageTakenRow| {
                    text(fmt_or_dash(r.absorbed))
                        .size(12)
                        .color(theme::TEXT_SECONDARY)
                })
                .width(Length::FillPortion(2)),
                table::column(header("Resist"), move |r: DamageTakenRow| {
                    text(fmt_or_dash(r.resisted))
                        .size(12)
                        .color(theme::TEXT_SECONDARY)
                })
                .width(Length::FillPortion(2)),
                table::column(header("Block"), move |r: DamageTakenRow| {
                    text(fmt_or_dash(r.blocked))
                        .size(12)
                        .color(theme::TEXT_SECONDARY)
                })
                .width(Length::FillPortion(2)),
                table::column(header("Crush"), |r: DamageTakenRow| {
                    let s = if r.crushing_hits > 0 {
                        r.crushing_hits.to_string()
                    } else {
                        "-".to_string()
                    };
                    text(s).size(12).color(Color::from_rgb8(200, 100, 100))
                })
                .width(Length::FillPortion(1)),
                table::column(header("Glance"), |r: DamageTakenRow| {
                    let s = if r.glancing_hits > 0 {
                        r.glancing_hits.to_string()
                    } else {
                        "-".to_string()
                    };
                    text(s).size(12).color(theme::TEXT_SECONDARY)
                })
                .width(Length::FillPortion(1)),
                table::column(header("%"), |r: DamageTakenRow| {
                    text(format!("{:.1}%", r.percent)).size(12)
                })
                .width(Length::FillPortion(1)),
            ];

            let source_table = table::table(columns, rows)
                .padding_x(2)
                .padding_y(2)
                .separator_y(1);

            content = content.push(column![source_header, source_table].spacing(4).width(Fill));
        }

        content.width(Fill).into()
    }

    fn view_dispel_detail(&self, player: &str) -> Element<'_, ViewerMessage> {
        let dispels = self.log_data.filtered_dispels(&self.encounter_filter);
        let events: Vec<(&str, &str)> = dispels
            .iter()
            .filter(|d| d.caster == player)
            .map(|d| (d.spell.as_str(), d.target.as_str()))
            .collect();
        view_spell_target_detail(&events, "Dispels", theme::BAR_DISPEL)
    }

    fn view_interrupt_detail(&self, player: &str) -> Element<'_, ViewerMessage> {
        let interrupts = self.log_data.filtered_interrupts(&self.encounter_filter);
        let events: Vec<(&str, &str)> = interrupts
            .iter()
            .filter(|i| i.caster == player)
            .map(|i| (i.spell.as_str(), i.target.as_str()))
            .collect();
        view_spell_target_detail(&events, "Interrupts", theme::BAR_INTERRUPT)
    }

    fn view_resurrect_detail(&self, player: &str) -> Element<'_, ViewerMessage> {
        let resurrects: Vec<ResurrectEvent> = self
            .log_data
            .filtered_resurrects(&self.encounter_filter)
            .into_iter()
            .filter(|r| r.caster == player)
            .cloned()
            .collect();

        let summary = text(format!("Total Resurrections: {}", resurrects.len()))
            .size(12)
            .color(theme::TEXT_SECONDARY);

        if resurrects.is_empty() {
            return column![summary,].spacing(8).width(Fill).into();
        }

        let header = |label| text(label).size(12);

        let columns = [
            table::column(header("Target"), |r: ResurrectEvent| {
                text(r.target).size(13)
            })
            .width(Length::FillPortion(2)),
            table::column(header("Spell"), |r: ResurrectEvent| text(r.spell).size(13))
                .width(Length::FillPortion(2)),
            table::column(header("Time"), |r: ResurrectEvent| {
                text(format_timestamp(r.timestamp))
                    .size(12)
                    .color(theme::TEXT_SECONDARY)
            })
            .width(Length::FillPortion(1)),
        ];

        let res_table = table::table(columns, resurrects)
            .padding_x(2)
            .padding_y(2)
            .separator_y(1);

        column![summary, res_table,].spacing(8).width(Fill).into()
    }

    fn view_avoidance_detail(&self, player: &str) -> Element<'_, ViewerMessage> {
        let Some(av) = self.log_data.avoidance.get(player) else {
            return text("No avoidance data").size(14).into();
        };

        let summary = text(format!("Total Attacks Avoided: {}", av.total()))
            .size(12)
            .color(theme::TEXT_SECONDARY);

        let avoidance_rows = vec![
            ("Dodges", av.dodges),
            ("Parries", av.parries),
            ("Blocks", av.blocks),
            ("Attacks Missed You", av.missed_by),
            ("Your Attacks Missed", av.misses),
        ];

        let mut avoidance_col = Column::new().spacing(4);
        for (label, count) in avoidance_rows {
            avoidance_col = avoidance_col.push(
                row![
                    text(label).size(13).width(Length::FillPortion(3)),
                    text(count.to_string())
                        .size(13)
                        .width(Length::FillPortion(1)),
                ]
                .spacing(4),
            );
        }

        // Full mitigation section (attacks that dealt 0 damage)
        let full_mit = av.total_full_mitigation();
        let mut content = column![summary, avoidance_col].spacing(8).width(Fill);

        if full_mit > 0 {
            let mit_header = text(format!("Full Mitigation: {full_mit}"))
                .size(12)
                .color(theme::TEXT_SECONDARY);

            let mit_rows = vec![
                ("Full Resists", av.full_resists),
                ("Full Absorbs", av.full_absorbs),
                ("Full Blocks", av.full_blocks),
            ];

            let mut mit_col = Column::new().spacing(4);
            for (label, count) in mit_rows {
                if count > 0 {
                    mit_col = mit_col.push(
                        row![
                            text(label).size(13).width(Length::FillPortion(3)),
                            text(count.to_string())
                                .size(13)
                                .width(Length::FillPortion(1)),
                        ]
                        .spacing(4),
                    );
                }
            }

            content = content
                .push(rule::horizontal(1))
                .push(mit_header)
                .push(mit_col);
        }

        content.into()
    }

    #[allow(clippy::cast_possible_truncation)] // duration seconds safely fit in u64
    fn view_buff_detail(&self, player: &str) -> Element<'_, ViewerMessage> {
        // Compute encounter-filtered uptimes and gains/fades
        let (uptimes, duration) = self.log_data.compute_buff_uptimes(&self.encounter_filter);

        let player_uptimes = uptimes.get(player);
        let empty = HashMap::new();
        let player_data = player_uptimes.unwrap_or(&empty);

        if player_data.is_empty() {
            // Fall back to session-wide buffs if no encounter-filtered data
            let has_buffs = self
                .log_data
                .buffs
                .get(player)
                .is_some_and(|b| !b.is_empty());
            if !has_buffs {
                return text("No buff data").size(14).into();
            }
        }

        // Build rows: (name, gains, fades, fraction, duration_secs)
        let mut sorted: Vec<(String, u64, u64, f64, f64)> = player_data
            .iter()
            .map(|(name, bu)| {
                let dur_secs = bu.fraction * duration;
                (name.clone(), bu.gains, bu.fades, bu.fraction, dur_secs)
            })
            .collect();

        // Sort by uptime descending (primary), then gains descending (secondary)
        sorted.sort_by(|a, b| b.3.total_cmp(&a.3).then_with(|| b.1.cmp(&a.1)));

        let summary = if duration > 0.0 {
            text(format!(
                "Buffs: {} (encounter: {})",
                sorted.len(),
                theme::format_duration(duration),
            ))
            .size(12)
            .color(theme::TEXT_SECONDARY)
        } else {
            text(format!("Buffs: {}", sorted.len()))
                .size(12)
                .color(theme::TEXT_SECONDARY)
        };

        let header = |label| text(label).size(12);

        let columns = [
            table::column(header("Buff"), |row: (String, u64, u64, f64, f64)| {
                text(row.0).size(12)
            })
            .width(Length::FillPortion(3)),
            table::column(header("Gains"), |row: (String, u64, u64, f64, f64)| {
                text(row.1.to_string()).size(12)
            })
            .width(Length::FillPortion(1)),
            table::column(header("Fades"), |row: (String, u64, u64, f64, f64)| {
                text(row.2.to_string()).size(12)
            })
            .width(Length::FillPortion(1)),
            table::column(header("Uptime"), |row: (String, u64, u64, f64, f64)| {
                if row.3 > 0.0 {
                    let mins = (row.4 as u64) / 60;
                    let secs = (row.4 as u64) % 60;
                    text(format!("{:.1}% ({mins}:{secs:02})", row.3 * 100.0)).size(12)
                } else {
                    text("-").size(12).color([0.4, 0.4, 0.4])
                }
            })
            .width(Length::FillPortion(2)),
        ];

        let buff_table = table::table(columns, sorted)
            .padding_x(2)
            .padding_y(2)
            .separator_y(1);

        column![summary, buff_table,].spacing(8).width(Fill).into()
    }

    #[allow(clippy::too_many_lines)] // iced UI layout — grouped consumable detail with category sections
    fn view_consumable_detail(&self, player: &str) -> Element<'_, ViewerMessage> {
        use crate::log_data::ConsumableCategory;

        let consumables = self.log_data.filtered_consumables(&self.encounter_filter);
        let player_cons: Vec<_> = consumables
            .into_iter()
            .filter(|c| c.player == player)
            .collect();

        let summary = text(format!("Total Consumable Uses: {}", player_cons.len()))
            .size(12)
            .color(theme::TEXT_SECONDARY);

        if player_cons.is_empty() {
            return column![summary, empty_state("No consumables used"),]
                .spacing(8)
                .width(Fill)
                .into();
        }

        // Aggregate by category → item name → count
        let mut by_category: HashMap<ConsumableCategory, HashMap<&str, u64>> = HashMap::new();
        for c in &player_cons {
            *by_category
                .entry(c.category)
                .or_default()
                .entry(&c.consumable)
                .or_insert(0) += 1;
        }

        // Sort categories by enum order
        let mut categories: Vec<ConsumableCategory> = by_category.keys().copied().collect();
        categories.sort();

        let mut content = Column::new().spacing(8);
        content = content.push(summary);

        for cat in &categories {
            let cat_color = theme::consumable_category_color(*cat);
            let items = &by_category[cat];

            // Sort items by count descending
            let mut sorted_items: Vec<(String, u64)> = items
                .iter()
                .map(|(name, count)| ((*name).to_string(), *count))
                .collect();
            sorted_items.sort_by_key(|b| Reverse(b.1));

            let cat_total: u64 = sorted_items.iter().map(|(_, c)| *c).sum();

            // Category header with colored dot
            let dot: Element<ViewerMessage> = container("")
                .width(8)
                .height(8)
                .style(move |_theme: &iced::Theme| container::Style {
                    background: Some(iced::Background::Color(cat_color)),
                    border: iced::Border {
                        radius: 4.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .into();

            let cat_header = row![
                dot,
                text(cat.to_string()).size(13).color(cat_color),
                text(format!("({cat_total})"))
                    .size(11)
                    .color(theme::TEXT_MUTED),
            ]
            .spacing(6)
            .align_y(Center);

            // Item rows
            let mut item_col = Column::new().spacing(2);
            for (name, count) in &sorted_items {
                item_col = item_col.push(
                    row![
                        text("  ").size(12), // indent
                        text(name.clone())
                            .size(12)
                            .color(cat_color)
                            .width(Length::FillPortion(3)),
                        text(count.to_string())
                            .size(12)
                            .width(Length::FillPortion(1)),
                    ]
                    .spacing(4),
                );
            }

            content = content.push(rule::horizontal(1));
            content = content.push(cat_header);
            content = content.push(item_col);
        }

        content.width(Fill).into()
    }

    /// Player info bar showing spec, race, guild, and gear count.
    fn view_player_info_bar<'a>(&'a self, player: &'a str) -> Element<'a, ViewerMessage> {
        let Some(combatant) = self.log_data.combatants.get(player) else {
            return row![].into();
        };

        let class_color = theme::class_color(&combatant.class);
        let mut info_parts = Row::new().spacing(12).align_y(Center);

        // Class icon + name
        info_parts = info_parts.push(
            image(theme::class_icon(&combatant.class))
                .width(18)
                .height(18),
        );
        info_parts = info_parts.push(text(player).size(13).color(class_color));

        // Talent spec
        if let Some(ref spec) = combatant.talent_summary {
            info_parts =
                info_parts.push(text(spec).size(12).color(Color::from_rgb8(200, 200, 200)));
        }

        // Race
        if !combatant.race.is_empty() && combatant.race != "nil" {
            info_parts =
                info_parts.push(text(&combatant.race).size(12).color(theme::TEXT_SECONDARY));
        }

        // Guild
        if let Some(ref guild) = combatant.guild {
            info_parts = info_parts.push(
                text(format!("<{guild}>"))
                    .size(12)
                    .color(theme::TEXT_SECONDARY),
            );
        }

        // Gear count
        let filled_slots = combatant.gear.iter().filter(|g| g.is_some()).count();
        if filled_slots > 0 {
            info_parts = info_parts.push(
                text(format!("{filled_slots}/19 gear"))
                    .size(11)
                    .color(theme::TEXT_MUTED),
            );
        }

        info_parts.width(Fill).into()
    }

    // ── Meter Bar Components ────────────────────────────────────────────

    /// Meter bar with custom detail text (for avoidance breakdown).
    #[allow(clippy::too_many_arguments, clippy::unused_self)]
    pub(super) fn meter_bar_row_with_detail_text(
        &self,
        rank: usize,
        name: &str,
        class: &str,
        detail_text: &str,
        percent: f64,
        bar_color: Color,
        on_click: Option<(String, DetailType)>,
    ) -> Element<'_, ViewerMessage> {
        build_meter_row(
            rank,
            name,
            class,
            detail_text,
            "",
            percent,
            bar_color,
            on_click,
        )
    }

    fn view_death_detail(&self, player: &str) -> Element<'_, ViewerMessage> {
        let deaths: Vec<DeathEvent> = self
            .log_data
            .filtered_deaths(&self.encounter_filter)
            .into_iter()
            .filter(|d| d.player == player)
            .cloned()
            .collect();

        if deaths.is_empty() {
            return text("No deaths recorded").size(14).into();
        }

        let summary = text(format!("Total Deaths: {}", deaths.len()))
            .size(12)
            .color(theme::TEXT_SECONDARY);

        let header = |label| text(label).size(12).color(theme::TEXT_MUTED);

        let columns = [
            table::column(header("Time"), |death: DeathEvent| {
                text(format_timestamp(death.timestamp))
                    .size(12)
                    .color(theme::TEXT_SECONDARY)
            })
            .width(Length::FillPortion(2)),
            table::column(header("Killed By"), |death: DeathEvent| {
                text(death.killer.as_deref().unwrap_or("Unknown").to_string())
                    .size(13)
                    .color([0.9, 0.3, 0.3])
            })
            .width(Length::FillPortion(3)),
            table::column(header("Ability"), |death: DeathEvent| {
                text(death.killing_blow.as_deref().unwrap_or("").to_string()).size(13)
            })
            .width(Length::FillPortion(3)),
            table::column(header("Damage"), |death: DeathEvent| {
                text(
                    death
                        .damage_amount
                        .map_or(String::new(), theme::format_number),
                )
                .size(13)
            })
            .width(Length::FillPortion(2)),
        ];

        let death_table = table::table(columns, deaths)
            .padding_x(2)
            .padding_y(3)
            .separator_y(1);

        column![summary, death_table,].spacing(8).width(Fill).into()
    }
}

// ── Shared Helpers ─────────────────────────────────────────────────────────

/// Unified detail view for spell+target event types (dispels, interrupts).
///
/// Shows total count, "By Spell" breakdown, and "Top Targets" (top 5).
pub(super) fn view_spell_target_detail<'a>(
    events: &[(&'a str, &'a str)],
    label: &str,
    accent_color: Color,
) -> Element<'a, ViewerMessage> {
    let mut by_spell: HashMap<&str, u64> = HashMap::new();
    let mut by_target: HashMap<&str, u64> = HashMap::new();
    for (spell, target) in events {
        *by_spell.entry(spell).or_insert(0) += 1;
        *by_target.entry(target).or_insert(0) += 1;
    }

    let summary = text(format!("Total {label}: {}", events.len()))
        .size(12)
        .color(theme::TEXT_SECONDARY);

    let mut spell_col = Column::new().spacing(4);
    spell_col = spell_col.push(text("By Spell").size(13).color(accent_color));
    for (spell, count) in &by_spell {
        spell_col = spell_col.push(
            row![
                text(*spell).size(13).width(Length::FillPortion(3)),
                text(count.to_string())
                    .size(13)
                    .width(Length::FillPortion(1)),
            ]
            .spacing(4),
        );
    }

    let mut sorted_targets: Vec<(&&str, &u64)> = by_target.iter().collect();
    sorted_targets.sort_by(|a, b| b.1.cmp(a.1));

    let mut target_col = Column::new().spacing(4);
    target_col = target_col.push(text("Top Targets").size(13).color(accent_color));
    for (target, count) in sorted_targets.iter().take(5) {
        target_col = target_col.push(
            row![
                text(**target).size(13).width(Length::FillPortion(3)),
                text(count.to_string())
                    .size(13)
                    .width(Length::FillPortion(1)),
            ]
            .spacing(4),
        );
    }

    column![summary, spell_col, target_col,]
        .spacing(10)
        .width(Fill)
        .into()
}
