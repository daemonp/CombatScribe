//! Timeline tab rendering: DPS/DTPS/HPS charts, aura lanes, and event log filters.

use super::charts::{
    aura_chart_height, build_aura_layout, AliveChart, AuraChart, DispelChart, TimelineChart,
    DISPEL_LANE_HEIGHT,
};
#[allow(clippy::wildcard_imports)]
// viewer UI — many shared component functions used throughout
use super::components::*;
#[allow(clippy::wildcard_imports)] // viewer UI — many shared types/widgets used throughout
use super::*;
use iced::widget::column;

// ── Timeline Tab ────────────────────────────────────────────────────────────

impl ViewerState {
    #[allow(clippy::too_many_lines)] // iced UI layout — timeline chart with multiple data lanes
    pub(super) fn view_timeline_tab(&self) -> Element<'_, ViewerMessage> {
        let td = &self.timeline_data;
        let vis = &self.timeline_visibility;

        if td.buckets.is_empty() {
            return container(
                text("Select a boss encounter to view its timeline")
                    .size(14)
                    .color(theme::TEXT_MUTED),
            )
            .padding(40)
            .center_x(Fill)
            .into();
        }

        let header = row![
            text("Encounter Timeline").size(16).color(Color::WHITE),
            horizontal_space(),
            text(format!(
                "Duration: {} | Raid: {} players",
                theme::format_duration(td.duration),
                td.raid_count,
            ))
            .size(12)
            .color(theme::TEXT_SECONDARY),
        ]
        .spacing(8)
        .align_y(Center)
        .width(Fill);

        // ── Legend toggles ──────────────────────────────────────────────
        let y_axis_label = if self.timeline_shared_y {
            "Y: Shared"
        } else {
            "Y: Independent"
        };

        let legend = row![
            legend_toggle(
                theme::TIMELINE_DPS,
                "Raid DPS",
                vis.show_dps,
                TimelineSeriesKind::Dps,
            ),
            tooltip(
                legend_toggle(
                    theme::TIMELINE_DTPS,
                    "Raid DTPS",
                    vis.show_dtps,
                    TimelineSeriesKind::Dtps,
                ),
                container(
                    text("Damage Taken Per Second — total raid incoming damage each second")
                        .size(11),
                )
                .padding([4, 8])
                .style(|_theme: &iced::Theme| container::Style {
                    background: Some(iced::Background::Color(
                        Color::from_rgba8(30, 30, 40, 0.95,)
                    )),
                    border: iced::Border {
                        color: Color::from_rgba8(100, 100, 120, 0.5),
                        width: 1.0,
                        radius: 4.0.into(),
                    },
                    ..Default::default()
                }),
                tooltip::Position::Bottom,
            ),
            legend_toggle(
                theme::TIMELINE_HPS,
                "Raid HPS",
                vis.show_hps,
                TimelineSeriesKind::Hps,
            ),
            legend_toggle(
                theme::TIMELINE_BOSS_HEAL,
                "Boss Heals",
                vis.show_boss_heals,
                TimelineSeriesKind::BossHeal,
            ),
            legend_toggle(
                theme::TIMELINE_DEATH,
                "Death",
                vis.show_deaths,
                TimelineSeriesKind::Death,
            ),
            legend_toggle(
                theme::TIMELINE_BIG_HIT,
                "Big Hit",
                vis.show_big_hits,
                TimelineSeriesKind::BigHit,
            ),
            legend_toggle(
                theme::TIMELINE_ALIVE,
                "Alive",
                vis.show_alive,
                TimelineSeriesKind::Alive,
            ),
            legend_toggle(
                theme::TIMELINE_DISPEL,
                "Dispels",
                vis.show_dispels,
                TimelineSeriesKind::Dispel,
            ),
            {
                let aura_count = self.tracked_auras.len();
                let aura_label = if aura_count > 0 {
                    format!("Auras ({aura_count})")
                } else {
                    "Auras".to_string()
                };
                let label_color = if self.aura_picker_open || aura_count > 0 {
                    Color::from_rgb8(180, 140, 255)
                } else {
                    theme::TEXT_MUTED
                };
                button(text(aura_label).size(10).color(label_color))
                    .on_press(ViewerMessage::ToggleAuraPicker)
                    .padding([3, 8])
                    .style(move |_theme: &iced::Theme, status| {
                        let bg = if aura_count > 0 {
                            Some(iced::Background::Color(Color::from_rgba8(
                                180, 140, 255, 0.1,
                            )))
                        } else {
                            match status {
                                button::Status::Hovered => Some(iced::Background::Color(
                                    Color::from_rgba8(255, 255, 255, 0.06),
                                )),
                                _ => None,
                            }
                        };
                        button::Style {
                            background: bg,
                            text_color: label_color,
                            border: iced::Border {
                                radius: 4.0.into(),
                                color: if aura_count > 0 {
                                    Color::from_rgba8(180, 140, 255, 0.4)
                                } else {
                                    Color::TRANSPARENT
                                },
                                width: if aura_count > 0 { 1.0 } else { 0.0 },
                            },
                            shadow: iced::Shadow::default(),
                        }
                    })
            },
            horizontal_space(),
            button(text(y_axis_label).size(10).color(theme::TEXT_SECONDARY))
                .on_press(ViewerMessage::ToggleTimelineYAxis)
                .padding([3, 8])
                .style(transparent_button_style),
        ]
        .spacing(8)
        .align_y(Center);

        // ── Zoom info bar (shown when zoomed in) ────────────────────────
        let zoom_bar: Element<ViewerMessage> = if let Some((lo, hi)) = self.zoom_range {
            let range_text = format!(
                "Zoomed: {} – {} ({:.0}s)",
                format_encounter_time(lo),
                format_encounter_time(hi),
                hi - lo,
            );
            row![
                text(range_text)
                    .size(11)
                    .color(Color::from_rgb8(100, 180, 255)),
                horizontal_space(),
                button(
                    text("Reset Zoom")
                        .size(10)
                        .color(Color::from_rgb8(100, 180, 255))
                )
                .on_press(ViewerMessage::ZoomReset)
                .padding([3, 8])
                .style(|_theme: &iced::Theme, status| {
                    let bg = match status {
                        button::Status::Hovered => Some(iced::Background::Color(
                            Color::from_rgba8(100, 180, 255, 0.2),
                        )),
                        _ => Some(iced::Background::Color(Color::from_rgba8(
                            100, 180, 255, 0.08,
                        ))),
                    };
                    button::Style {
                        background: bg,
                        text_color: Color::from_rgb8(100, 180, 255),
                        border: iced::Border {
                            radius: 4.0.into(),
                            color: Color::from_rgba8(100, 180, 255, 0.4),
                            width: 1.0,
                        },
                        shadow: iced::Shadow::default(),
                    }
                }),
            ]
            .spacing(8)
            .align_y(Center)
            .width(Fill)
            .into()
        } else {
            column![].into()
        };

        // ── Canvas sparkline chart ─────────────────────────────────────
        // Build active drag tuple for highlight overlay
        let active_drag = self
            .zoom_drag_start
            .zip(self.zoom_drag_end)
            .map(|(s, e)| (s.min(e), s.max(e)));

        let chart_canvas = canvas::Canvas::new(TimelineChart {
            data: td,
            visibility: vis,
            shared_y: self.timeline_shared_y,
            hover_idx: self.timeline_hover,
            drag: active_drag,
            zoom: self.zoom_range,
        })
        .width(Fill)
        .height(220);

        // ── Alive count sparkline ──────────────────────────────────────
        let alive_section: Element<ViewerMessage> = if vis.show_alive {
            let alive_label = row![
                text("Alive").size(11).color(theme::TIMELINE_ALIVE),
                horizontal_space(),
                text(format!("{} max", td.raid_count))
                    .size(10)
                    .color(theme::TEXT_MUTED),
            ]
            .width(Fill);

            let alive_canvas = canvas::Canvas::new(AliveChart {
                data: td,
                zoom: self.zoom_range,
            })
            .width(Fill)
            .height(40);

            column![alive_label, alive_canvas]
                .spacing(4)
                .width(Fill)
                .into()
        } else {
            column![].into()
        };

        // ── Hover tooltip ──────────────────────────────────────────────
        let tooltip: Element<ViewerMessage> = if let Some(idx) = self.timeline_hover {
            if let Some(bucket) = td.buckets.get(idx) {
                let time_str = format_encounter_time(bucket.offset);
                let mut parts = vec![time_str];
                if vis.show_dps {
                    parts.push(format!("DPS: {}", theme::format_number(bucket.damage)));
                }
                if vis.show_dtps {
                    parts.push(format!(
                        "DTPS: {}",
                        theme::format_number(bucket.damage_taken)
                    ));
                }
                if vis.show_hps {
                    parts.push(format!("HPS: {}", theme::format_number(bucket.healing)));
                }
                if vis.show_boss_heals && bucket.boss_healing > 0 {
                    parts.push(format!(
                        "Boss HPS: {}",
                        theme::format_number(bucket.boss_healing)
                    ));
                }
                if vis.show_alive {
                    parts.push(format!("Alive: {}", bucket.alive_count));
                }
                container(text(parts.join("  |  ")).size(12).color(Color::WHITE))
                    .padding([4, 8])
                    .style(|_theme: &iced::Theme| container::Style {
                        background: Some(iced::Background::Color(Color::from_rgba8(
                            30, 30, 40, 0.9,
                        ))),
                        border: iced::Border {
                            color: theme::SURFACE_BORDER,
                            width: 1.0,
                            radius: 4.0.into(),
                        },
                        ..Default::default()
                    })
                    .into()
            } else {
                column![].into()
            }
        } else {
            column![].into()
        };

        // ── Aura picker dropdown ───────────────────────────────────────
        let aura_picker: Element<ViewerMessage> = if self.aura_picker_open {
            let search_input = text_input("Search auras...", &self.aura_search)
                .on_input(ViewerMessage::SetAuraSearch)
                .size(11)
                .padding(4)
                .width(Fill);

            let search_lower = self.aura_search.to_lowercase();
            let filtered_auras: Vec<&String> = td
                .available_auras
                .iter()
                .filter(|name| {
                    search_lower.is_empty() || name.to_lowercase().contains(&search_lower)
                })
                .collect();

            let mut aura_list = Column::new().spacing(1);
            for aura_name in filtered_auras {
                let is_tracked = self.tracked_auras.contains(aura_name);
                let name_clone = aura_name.clone();
                let check_label = if is_tracked {
                    format!("[x] {aura_name}")
                } else {
                    format!("[ ] {aura_name}")
                };
                let text_color = if is_tracked {
                    Color::WHITE
                } else {
                    theme::TEXT_SECONDARY
                };
                aura_list = aura_list.push(
                    button(text(check_label).size(10).color(text_color))
                        .on_press(ViewerMessage::ToggleAura(name_clone))
                        .padding([2, 6])
                        .width(Fill)
                        .style(move |_theme: &iced::Theme, status| {
                            let bg = match status {
                                button::Status::Hovered => Some(iced::Background::Color(
                                    Color::from_rgba8(255, 255, 255, 0.08),
                                )),
                                _ => {
                                    if is_tracked {
                                        Some(iced::Background::Color(Color::from_rgba8(
                                            180, 140, 255, 0.08,
                                        )))
                                    } else {
                                        None
                                    }
                                }
                            };
                            button::Style {
                                background: bg,
                                text_color: if is_tracked {
                                    Color::WHITE
                                } else {
                                    theme::TEXT_SECONDARY
                                },
                                border: iced::Border {
                                    radius: 2.0.into(),
                                    ..Default::default()
                                },
                                shadow: iced::Shadow::default(),
                            }
                        }),
                );
            }

            let scrollable_list = scrollable(aura_list).height(Length::Fixed(200.0));

            // Preset buttons
            let preset_buttons: Vec<Element<ViewerMessage>> = log_data::AURA_PRESETS
                .iter()
                .enumerate()
                .map(|(idx, preset)| {
                    button(text(preset.label).size(9).color(theme::TEXT_SECONDARY))
                        .on_press(ViewerMessage::ApplyAuraPreset(idx))
                        .padding([2, 6])
                        .style(|_theme: &iced::Theme, status| {
                            let bg = match status {
                                button::Status::Hovered => Some(iced::Background::Color(
                                    Color::from_rgba8(180, 140, 255, 0.15),
                                )),
                                _ => Some(iced::Background::Color(Color::from_rgba8(
                                    255, 255, 255, 0.04,
                                ))),
                            };
                            button::Style {
                                background: bg,
                                text_color: theme::TEXT_SECONDARY,
                                border: iced::Border {
                                    radius: 3.0.into(),
                                    color: Color::from_rgba8(180, 140, 255, 0.2),
                                    width: 1.0,
                                },
                                shadow: iced::Shadow::default(),
                            }
                        })
                        .into()
                })
                .collect();
            let preset_row = Row::with_children(preset_buttons).spacing(4).wrap();

            // Clear button (only show if auras are selected)
            let clear_btn: Element<ViewerMessage> = if self.tracked_auras.is_empty() {
                column![].into()
            } else {
                button(text("Clear All").size(9).color(theme::TEXT_MUTED))
                    .on_press(ViewerMessage::ClearAuras)
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
                        }
                    })
                    .into()
            };

            container(
                column![
                    text("Presets").size(10).color(theme::TEXT_MUTED),
                    preset_row,
                    horizontal_rule(1),
                    search_input,
                    scrollable_list,
                    clear_btn,
                ]
                .spacing(4)
                .width(Fill),
            )
            .padding(8)
            .width(Length::Fixed(300.0))
            .style(|_theme: &iced::Theme| container::Style {
                background: Some(iced::Background::Color(Color::from_rgba8(25, 27, 35, 0.95))),
                border: iced::Border {
                    color: Color::from_rgba8(180, 140, 255, 0.3),
                    width: 1.0,
                    radius: 6.0.into(),
                },
                ..Default::default()
            })
            .into()
        } else {
            column![].into()
        };

        // ── Aura waterfall chart ────────────────────────────────────────
        let aura_layout = build_aura_layout(td, &self.tracked_auras);
        let aura_section: Element<ViewerMessage> = if aura_layout.is_empty() {
            column![].into()
        } else {
            let chart_height = aura_chart_height(&aura_layout);

            // Hover tooltip: show which players have each aura at the hovered time
            let aura_tooltip: Element<ViewerMessage> = if let Some(second) = self.aura_hover_second
            {
                let mut parts: Vec<String> = Vec::new();
                for group in &aura_layout {
                    if let Some(intervals) = td.aura_intervals.get(group.aura_name) {
                        let mut active: Vec<&str> = intervals
                            .iter()
                            .filter(|iv| iv.start <= second && second <= iv.end)
                            .map(|iv| iv.player.as_str())
                            .collect();
                        active.sort_unstable();
                        active.dedup();
                        if !active.is_empty() {
                            parts.push(format!("{}: {}", group.aura_name, active.join(", ")));
                        }
                    }
                }
                if parts.is_empty() {
                    column![].into()
                } else {
                    container(text(parts.join("  |  ")).size(11).color(Color::WHITE))
                        .padding([3, 8])
                        .style(|_theme: &iced::Theme| container::Style {
                            background: Some(iced::Background::Color(Color::from_rgba8(
                                30, 30, 40, 0.9,
                            ))),
                            border: iced::Border {
                                color: Color::from_rgba8(180, 140, 255, 0.3),
                                width: 1.0,
                                radius: 4.0.into(),
                            },
                            ..Default::default()
                        })
                        .into()
                }
            } else {
                column![].into()
            };

            let aura_canvas = canvas::Canvas::new(AuraChart {
                data: td,
                layout: aura_layout,
                hover_second: self.aura_hover_second,
                zoom: self.zoom_range,
            })
            .width(Fill)
            .height(chart_height);

            // Cap the aura waterfall height and make it scrollable when tall
            let max_aura_height: f32 = 200.0;
            let aura_content: Element<ViewerMessage> = if chart_height > max_aura_height {
                scrollable(aura_canvas)
                    .height(max_aura_height)
                    .width(Fill)
                    .into()
            } else {
                aura_canvas.into()
            };

            column![aura_content, aura_tooltip]
                .spacing(2)
                .width(Fill)
                .into()
        };

        // ── Dispel waterfall chart ─────────────────────────────────────
        let dispel_section: Element<ViewerMessage> =
            if vis.show_dispels && !td.dispel_casters.is_empty() {
                let caster_count = td.dispel_casters.len();
                let chart_height = caster_count as f32 * DISPEL_LANE_HEIGHT;
                let total_dispels = td.dispel_marks.len();

                let dispel_header = row![
                    text("Dispel Activity")
                        .size(11)
                        .color(theme::TIMELINE_DISPEL),
                    horizontal_space(),
                    text(format!("{total_dispels} total"))
                        .size(10)
                        .color(theme::TEXT_MUTED),
                ]
                .width(Fill);

                let dispel_canvas = canvas::Canvas::new(DispelChart {
                    data: td,
                    combatants: &self.log_data.combatants,
                    hover_second: self.aura_hover_second,
                    zoom: self.zoom_range,
                })
                .width(Fill)
                .height(chart_height);

                // Cap height and make scrollable when many casters
                let max_dispel_height: f32 = 200.0;
                let dispel_content: Element<ViewerMessage> = if chart_height > max_dispel_height {
                    scrollable(dispel_canvas)
                        .height(max_dispel_height)
                        .width(Fill)
                        .into()
                } else {
                    dispel_canvas.into()
                };

                column![dispel_header, dispel_content]
                    .spacing(4)
                    .width(Fill)
                    .into()
            } else {
                column![].into()
            };

        // ── Fixed top section: charts ──────────────────────────────────
        // Wrap tracker sections (aura/dispel/alive) in a scrollable capped
        // at 300px so they remain accessible when all three are active,
        // without pushing the event log off-screen.
        let tracker_content = column![aura_section, dispel_section, alive_section,]
            .spacing(6)
            .width(Fill);
        let tracker_panel: Element<'_, ViewerMessage> =
            container(scrollable(tracker_content).width(Fill))
                .max_height(300)
                .width(Fill)
                .into();

        let charts_panel = container(
            column![
                header,
                legend,
                zoom_bar,
                aura_picker,
                horizontal_rule(1),
                chart_canvas,
                tooltip,
                horizontal_rule(1),
                tracker_panel,
            ]
            .spacing(6)
            .width(Fill),
        )
        .padding(12)
        .width(Fill)
        .style(panel_style);

        // ── Event log filter bar ────────────────────────────────────────
        let mode_buttons: Element<ViewerMessage> = {
            let modes = [
                EventLogMode::AllEvents,
                EventLogMode::KeyEvents,
                EventLogMode::DeathLog,
            ];
            let mut r = Row::new().spacing(4).align_y(Center);
            for mode in modes {
                let active = self.event_log_mode == mode;
                let label_text = mode.to_string();
                r = r.push(
                    button(text(label_text).size(10).color(if active {
                        Color::WHITE
                    } else {
                        theme::TEXT_MUTED
                    }))
                    .on_press(ViewerMessage::SetEventLogMode(mode))
                    .padding([3, 8])
                    .style(move |_theme: &iced::Theme, status| {
                        let bg = if active {
                            Some(iced::Background::Color(Color::from_rgba8(
                                80, 140, 220, 0.4,
                            )))
                        } else {
                            match status {
                                button::Status::Hovered => Some(iced::Background::Color(
                                    Color::from_rgba8(255, 255, 255, 0.06),
                                )),
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
                                radius: 4.0.into(),
                                color: if active {
                                    Color::from_rgba8(80, 140, 220, 0.6)
                                } else {
                                    Color::TRANSPARENT
                                },
                                width: if active { 1.0 } else { 0.0 },
                            },
                            shadow: iced::Shadow::default(),
                        }
                    }),
                );
            }
            r.into()
        };

        // Death Log window-size picker — only visible in Death Log mode
        let window_picker: Element<ViewerMessage> = if self.event_log_mode == EventLogMode::DeathLog
        {
            pick_list(
                DeathLogWindow::ALL,
                Some(self.death_log_window),
                ViewerMessage::SetDeathLogWindow,
            )
            .text_size(11)
            .padding(3)
            .into()
        } else {
            horizontal_space().width(0).into()
        };

        let type_toggles: Element<ViewerMessage> = {
            let types = [
                (
                    "Dmg",
                    self.event_log_types.show_damage,
                    EventLogTypeKind::Damage,
                    Color::from_rgb8(200, 200, 200),
                ),
                (
                    "Heal",
                    self.event_log_types.show_healing,
                    EventLogTypeKind::Healing,
                    theme::TIMELINE_HPS,
                ),
                (
                    "Death",
                    self.event_log_types.show_deaths,
                    EventLogTypeKind::Deaths,
                    theme::TIMELINE_DEATH,
                ),
                (
                    "Dispel",
                    self.event_log_types.show_dispels,
                    EventLogTypeKind::Dispels,
                    theme::TIMELINE_DISPEL,
                ),
                (
                    "Intr",
                    self.event_log_types.show_interrupts,
                    EventLogTypeKind::Interrupts,
                    theme::TIMELINE_INTERRUPT,
                ),
            ];
            let mut r = Row::new().spacing(4).align_y(Center);
            for (label, active, kind, color) in types {
                let text_color = if active { color } else { theme::TEXT_MUTED };
                r = r.push(
                    button(text(label).size(10).color(text_color))
                        .on_press(ViewerMessage::ToggleEventLogType(kind))
                        .padding([2, 6])
                        .style(move |_theme: &iced::Theme, status| {
                            let bg = match status {
                                button::Status::Hovered => Some(iced::Background::Color(
                                    Color::from_rgba8(255, 255, 255, 0.06),
                                )),
                                _ => {
                                    if active {
                                        Some(iced::Background::Color(Color { a: 0.08, ..color }))
                                    } else {
                                        None
                                    }
                                }
                            };
                            button::Style {
                                background: bg,
                                text_color,
                                border: iced::Border {
                                    radius: 3.0.into(),
                                    ..Default::default()
                                },
                                shadow: iced::Shadow::default(),
                            }
                        }),
                );
            }
            r.into()
        };

        // Player filter — build list of players from combatants
        let mut player_names: Vec<String> = vec!["All Players".to_string()];
        player_names.extend(
            self.log_data
                .combatants
                .keys()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>(),
        );
        let selected_player = if self.event_log_player.is_empty() {
            "All Players".to_string()
        } else {
            self.event_log_player.clone()
        };

        let player_picker = pick_list(player_names, Some(selected_player), |name| {
            if name == "All Players" {
                ViewerMessage::SetEventLogPlayer(String::new())
            } else {
                ViewerMessage::SetEventLogPlayer(name)
            }
        })
        .text_size(11)
        .padding(3);

        let copy_btn = button(text("Copy").size(10).color(theme::TEXT_SECONDARY))
            .on_press(ViewerMessage::CopyEventLog)
            .padding([3, 10])
            .style(transparent_button_style);

        let filter_bar = container(
            row![
                mode_buttons,
                window_picker,
                type_toggles,
                horizontal_space(),
                player_picker,
                copy_btn,
            ]
            .spacing(12)
            .align_y(Center)
            .width(Fill),
        )
        .padding([6, 12])
        .width(Fill)
        .style(|_theme: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(Color::from_rgba8(20, 22, 28, 0.8))),
            border: iced::Border {
                color: theme::SURFACE_BORDER,
                width: 1.0,
                radius: 4.0.into(),
            },
            ..Default::default()
        });

        // ── Scrollable bottom section: event log ───────────────────────
        let player_filter = if self.event_log_player.is_empty() {
            None
        } else {
            Some(self.event_log_player.as_str())
        };

        let event_log = build_timeline_event_log(
            &self.log_data,
            &self.encounter_filter,
            &self.event_log_types,
            self.event_log_mode,
            player_filter,
            self.timeline_clicked_second,
            self.death_log_window.as_secs(),
        );

        let event_scrollable = scrollable(container(event_log).padding([4, 12]).width(Fill))
            .id(TIMELINE_LOG_ID.clone())
            .height(Fill);

        // Stack: fixed charts on top, filter bar, scrollable log fills remaining space
        column![charts_panel, filter_bar, event_scrollable]
            .spacing(4)
            .width(Fill)
            .height(Fill)
            .into()
    }
}
