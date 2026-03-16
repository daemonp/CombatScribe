//! Viewer mode — state management, messages, and all UI rendering.
//!
//! This is the native iced implementation of the web-based combat log viewer.
//! Renders damage/healing meters, utility panels, loot tables, and event logs.

// DPS meter values are u64 (max ~18 quintillion); precision loss at 2^52 is
// irrelevant for game damage figures.  Truncation/sign-loss in the bar width
// cast is clamped to [1, 100] before the cast.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use iced::widget::{
    button, canvas, column, container, horizontal_rule, horizontal_space, image, pick_list, row,
    scrollable, stack, text, text_input, Column, Row,
};
use iced::{mouse, Center, Color, Element, Fill, Length, Point, Rectangle, Renderer, Theme};

use std::sync::LazyLock;

use crate::log_data::{
    AbilityStats, AvoidanceStats, BuffStats, Encounter, EncounterFilter, EventLogMode,
    EventLogTypeFilter, EventLogTypeKind, LogData, LogEntry, LootEvent, PlayerEventType,
    ResurrectEvent, TimelineData, TimelineEventKind, TimelineSeriesKind, TimelineVisibility,
};
use crate::theme;

/// Scrollable ID for the timeline event log panel.
static TIMELINE_LOG_ID: LazyLock<scrollable::Id> =
    LazyLock::new(|| scrollable::Id::new("timeline_events"));

// ── State ───────────────────────────────────────────────────────────────────

// ViewerState cannot derive Clone because canvas::Cache is not Clone.
// This is acceptable since ViewerState is only held in a Box by the app.
#[derive(Debug)]
pub struct ViewerState {
    pub log_data: LogData,
    pub current_tab: ViewerTab,
    pub encounter_filter: EncounterFilter,
    pub encounter_names: Vec<String>,
    pub selected_encounter_name: Option<String>,

    // Damage/Healing tab
    pub damage_type: DamageType,

    // Utility tab
    pub dispel_type: DispelSubType,
    pub death_type: DeathSubType,

    // Loot tab
    pub loot_search: String,
    pub collapsed_bosses: HashSet<String>,

    // Events tab
    pub event_player_filter: String,
    pub event_player_names: Vec<String>,

    // Timeline tab
    pub timeline_data: TimelineData,
    pub timeline_visibility: TimelineVisibility,
    /// Whether all three chart series share the same Y-axis scale.
    pub timeline_shared_y: bool,
    /// Hovered bucket index (set by mouse position on the canvas).
    pub timeline_hover: Option<usize>,
    /// Clicked second offset — highlights events at this time in the log.
    pub timeline_clicked_second: Option<usize>,
    /// Event log facet mode (All Events, Key Events, Death Log).
    pub event_log_mode: EventLogMode,
    /// Event type toggles for the log panel.
    pub event_log_types: EventLogTypeFilter,
    /// Player filter for the event log (empty string = all players).
    pub event_log_player: String,
    /// Canvas geometry caches — cleared when data or visibility changes.
    pub timeline_cache: canvas::Cache,
    pub alive_cache: canvas::Cache,

    // Detail overlay
    pub detail: Option<DetailView>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewerTab {
    Meters,
    Utility,
    Timeline,
    Loot,
    Events,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageType {
    Damage,
    DamageWithPets,
    DamageTaken,
}

impl std::fmt::Display for DamageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Damage => write!(f, "Damage done"),
            Self::DamageWithPets => write!(f, "Damage done (incl. pets)"),
            Self::DamageTaken => write!(f, "Damage taken"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispelSubType {
    Dispels,
    Interrupts,
}

impl std::fmt::Display for DispelSubType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dispels => write!(f, "Dispels/Decurses"),
            Self::Interrupts => write!(f, "Interrupts"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeathSubType {
    Deaths,
    Resurrects,
    Absorbs,
    Avoidance,
    Buffs,
    Consumables,
}

impl std::fmt::Display for DeathSubType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Deaths => write!(f, "Deaths"),
            Self::Resurrects => write!(f, "Resurrections"),
            Self::Absorbs => write!(f, "Damage Absorbed"),
            Self::Avoidance => write!(f, "Avoidance (Dodge/Parry)"),
            Self::Buffs => write!(f, "Buff Uptime"),
            Self::Consumables => write!(f, "Consumables"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DetailView {
    pub player_name: String,
    pub detail_type: DetailType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailType {
    Damage,
    Healing,
    Dispels,
    Interrupts,
    Resurrects,
    Avoidance,
    Buffs,
    Consumables,
}

// ── Messages ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum ViewerMessage {
    SwitchTab(ViewerTab),
    SelectEncounter(String),
    SetDamageType(DamageType),
    SetDispelType(DispelSubType),
    SetDeathType(DeathSubType),
    ShowDetail(String, DetailType),
    CloseDetail,
    SetLootSearch(String),
    ToggleBossCollapse(String),
    ExpandAllLoot,
    CollapseAllLoot,
    SetEventPlayerFilter(String),
    ToggleTimelineSeries(TimelineSeriesKind),
    ToggleTimelineYAxis,
    TimelineHover(Option<usize>),
    /// Click on the chart canvas jumps the event log to that second.
    TimelineClick(usize),
    SetEventLogMode(EventLogMode),
    ToggleEventLogType(EventLogTypeKind),
    SetEventLogPlayer(String),
    BackToMain,
    Quit,
}

// ── Construction ────────────────────────────────────────────────────────────

impl ViewerState {
    pub fn new(log_data: LogData) -> Self {
        let encounter_names = build_encounter_names(&log_data);
        let selected = encounter_names.first().cloned();

        let mut event_player_names = vec!["All Players".to_string()];
        let mut player_names: Vec<String> = log_data.combatants.keys().cloned().collect();
        player_names.sort_by_key(|a| a.to_lowercase());
        event_player_names.extend(player_names);

        let timeline_data = log_data.build_timeline(&EncounterFilter::All, BIG_HIT_THRESHOLD);

        Self {
            log_data,
            current_tab: ViewerTab::Meters,
            encounter_filter: EncounterFilter::All,
            encounter_names,
            selected_encounter_name: selected,
            damage_type: DamageType::Damage,
            dispel_type: DispelSubType::Dispels,
            death_type: DeathSubType::Deaths,
            loot_search: String::new(),
            collapsed_bosses: HashSet::new(),
            detail: None,
            event_player_filter: "All Players".to_string(),
            event_player_names,
            timeline_data,
            timeline_visibility: TimelineVisibility::default(),
            timeline_shared_y: true,
            timeline_hover: None,
            timeline_clicked_second: None,
            event_log_mode: EventLogMode::default(),
            event_log_types: EventLogTypeFilter::default(),
            event_log_player: String::new(),
            timeline_cache: canvas::Cache::default(),
            alive_cache: canvas::Cache::default(),
        }
    }

    // ── Update ──────────────────────────────────────────────────────────

    pub fn update(&mut self, message: ViewerMessage) -> iced::Task<ViewerMessage> {
        match message {
            ViewerMessage::SwitchTab(tab) => {
                self.current_tab = tab;
                self.detail = None;
            }
            ViewerMessage::SelectEncounter(name) => {
                // Ignore separator lines — keep current selection
                if name == ENCOUNTER_SEPARATOR {
                    return iced::Task::none();
                }
                self.encounter_filter = parse_encounter_filter(&name, &self.log_data);
                self.selected_encounter_name = Some(name);
                self.detail = None;
                // Rebuild timeline for new encounter selection
                self.timeline_data = self
                    .log_data
                    .build_timeline(&self.encounter_filter, BIG_HIT_THRESHOLD);
                self.timeline_hover = None;
                self.timeline_clicked_second = None;
                self.timeline_cache.clear();
                self.alive_cache.clear();
            }
            ViewerMessage::SetDamageType(dt) => self.damage_type = dt,
            ViewerMessage::SetDispelType(dt) => self.dispel_type = dt,
            ViewerMessage::SetDeathType(dt) => self.death_type = dt,
            ViewerMessage::ShowDetail(name, dtype) => {
                self.detail = Some(DetailView {
                    player_name: name,
                    detail_type: dtype,
                });
            }
            ViewerMessage::CloseDetail => self.detail = None,
            ViewerMessage::SetLootSearch(s) => self.loot_search = s,
            ViewerMessage::ToggleBossCollapse(boss) => {
                if self.collapsed_bosses.contains(&boss) {
                    self.collapsed_bosses.remove(&boss);
                } else {
                    self.collapsed_bosses.insert(boss);
                }
            }
            ViewerMessage::ExpandAllLoot => self.collapsed_bosses.clear(),
            ViewerMessage::CollapseAllLoot => {
                for loot in &self.log_data.loot {
                    self.collapsed_bosses.insert(loot.boss.clone());
                }
            }
            ViewerMessage::SetEventPlayerFilter(name) => self.event_player_filter = name,
            ViewerMessage::ToggleTimelineSeries(kind) => {
                self.timeline_visibility.toggle(kind);
                self.timeline_cache.clear();
                self.alive_cache.clear();
            }
            ViewerMessage::ToggleTimelineYAxis => {
                self.timeline_shared_y = !self.timeline_shared_y;
                self.timeline_cache.clear();
            }
            ViewerMessage::TimelineHover(idx) => {
                self.timeline_hover = idx;
                // Don't clear cache — hover is drawn as an overlay
            }
            ViewerMessage::SetEventLogMode(mode) => self.event_log_mode = mode,
            ViewerMessage::ToggleEventLogType(kind) => {
                let t = &mut self.event_log_types;
                match kind {
                    EventLogTypeKind::Damage => t.show_damage = !t.show_damage,
                    EventLogTypeKind::Healing => t.show_healing = !t.show_healing,
                    EventLogTypeKind::Deaths => t.show_deaths = !t.show_deaths,
                    EventLogTypeKind::Dispels => t.show_dispels = !t.show_dispels,
                    EventLogTypeKind::Interrupts => t.show_interrupts = !t.show_interrupts,
                }
            }
            ViewerMessage::SetEventLogPlayer(name) => self.event_log_player = name,
            ViewerMessage::TimelineClick(second) => {
                self.timeline_clicked_second = Some(second);
                // Snap the event log scrollable to the proportional position
                // corresponding to the clicked second within the encounter.
                let duration = self.timeline_data.duration;
                if duration > 0.0 {
                    let proportion = second as f32 / duration as f32;
                    return scrollable::snap_to(
                        TIMELINE_LOG_ID.clone(),
                        scrollable::RelativeOffset {
                            x: 0.0,
                            y: proportion.clamp(0.0, 1.0),
                        },
                    );
                }
            }
            ViewerMessage::BackToMain | ViewerMessage::Quit => {} // handled by main.rs
        }
        iced::Task::none()
    }

    // ── View ────────────────────────────────────────────────────────────

    pub fn view(&self) -> Element<'_, ViewerMessage> {
        let header = self.view_header();
        let controls = self.view_controls();
        let tab_bar = self.view_tab_bar();

        let content: Element<ViewerMessage> = if let Some(detail) = &self.detail {
            self.view_detail_overlay(detail)
        } else {
            match self.current_tab {
                ViewerTab::Meters => self.view_meters_tab(),
                ViewerTab::Utility => self.view_utility_tab(),
                ViewerTab::Timeline => self.view_timeline_tab(),
                ViewerTab::Loot => self.view_loot_tab(),
                ViewerTab::Events => self.view_events_tab(),
            }
        };

        column![header, controls, tab_bar, content,]
            .spacing(10)
            .padding(20)
            .width(Fill)
            .height(Fill)
            .into()
    }

    // ── Header ──────────────────────────────────────────────────────────

    fn view_header(&self) -> Element<'_, ViewerMessage> {
        // Prefer the zone from the first boss encounter (= the raid instance),
        // falling back to the session-level zone_name (which may just be a city).
        let instance_zone = self
            .log_data
            .encounters
            .iter()
            .filter(|e| e.is_boss)
            .find_map(|e| e.zone.as_deref());
        let zone = instance_zone
            .or_else(|| {
                let z = self.log_data.zone_name.as_str();
                if z.is_empty() {
                    None
                } else {
                    Some(z)
                }
            })
            .unwrap_or("Combat Log");

        let mut header_row = Row::new().spacing(8).align_y(Center).width(Fill);
        header_row = header_row.push(text(zone).size(20).color(Color::WHITE));

        // Raid size badge
        if let Some((_, total)) = self.log_data.raid_size {
            let player_count = self.log_data.combatants.len();
            header_row = header_row.push(
                text(format!("{player_count}/{total} players"))
                    .size(12)
                    .color(theme::TEXT_MUTED),
            );
        } else {
            let player_count = self.log_data.combatants.len();
            if player_count > 0 {
                header_row = header_row.push(
                    text(format!("{player_count} players"))
                        .size(12)
                        .color(theme::TEXT_MUTED),
                );
            }
        }

        // Encounter count
        let boss_kills = self
            .log_data
            .encounters
            .iter()
            .filter(|e| e.is_boss && e.is_kill)
            .count();
        let total_bosses = self
            .log_data
            .encounters
            .iter()
            .filter(|e| e.is_boss)
            .count();
        if total_bosses > 0 {
            header_row = header_row.push(
                text(format!("{boss_kills}/{total_bosses} bosses"))
                    .size(12)
                    .color(theme::TEXT_MUTED),
            );
        }

        header_row = header_row.push(horizontal_space());
        header_row = header_row.push(
            button(text("Change Session").size(11).color(theme::TEXT_SECONDARY))
                .on_press(ViewerMessage::BackToMain)
                .style(transparent_button_style)
                .padding([5, 14]),
        );
        header_row = header_row.push(
            button(text("Quit").size(11).color(theme::TEXT_SECONDARY))
                .on_press(ViewerMessage::Quit)
                .style(transparent_button_style)
                .padding([5, 14]),
        );
        header_row.into()
    }

    // ── Controls ────────────────────────────────────────────────────────

    fn view_controls(&self) -> Element<'_, ViewerMessage> {
        row![pick_list(
            self.encounter_names.clone(),
            self.selected_encounter_name.clone(),
            ViewerMessage::SelectEncounter,
        )
        .width(Length::FillPortion(2))
        .padding(6),]
        .spacing(8)
        .width(Fill)
        .into()
    }

    // ── Tab Bar ─────────────────────────────────────────────────────────

    fn view_tab_bar(&self) -> Element<'_, ViewerMessage> {
        let tabs = [
            ("Damage/Healing", ViewerTab::Meters),
            ("Utility", ViewerTab::Utility),
            ("Timeline", ViewerTab::Timeline),
            ("Loot", ViewerTab::Loot),
            ("Events", ViewerTab::Events),
        ];

        let mut tab_row = Row::new().spacing(4);
        for (label, tab) in tabs {
            let is_active = self.current_tab == tab;
            let label_color = if is_active {
                Color::WHITE
            } else {
                theme::TEXT_SECONDARY
            };

            let btn = button(text(label).size(13).color(label_color).center())
                .on_press(ViewerMessage::SwitchTab(tab))
                .padding([8, 18])
                .style(move |_theme, status| {
                    let bg = if is_active {
                        Some(iced::Background::Color(Color::from_rgba8(
                            90, 130, 255, 0.18,
                        )))
                    } else if matches!(status, button::Status::Hovered) {
                        Some(iced::Background::Color(Color {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: 0.04,
                        }))
                    } else {
                        None
                    };
                    let border = if is_active {
                        iced::Border {
                            color: Color::from_rgba8(90, 130, 255, 0.4),
                            width: 1.0,
                            radius: 6.0.into(),
                        }
                    } else {
                        iced::Border {
                            radius: 6.0.into(),
                            ..Default::default()
                        }
                    };
                    button::Style {
                        background: bg,
                        text_color: label_color,
                        border,
                        shadow: iced::Shadow::default(),
                    }
                });

            tab_row = tab_row.push(btn);
        }

        tab_row.width(Fill).into()
    }

    // ── Meters Tab ──────────────────────────────────────────────────────

    #[allow(clippy::too_many_lines)] // iced UI layout — side-by-side panels with data collection
    fn view_meters_tab(&self) -> Element<'_, ViewerMessage> {
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
                if !self.log_data.combatants.contains_key(name) || ps.healing == 0 {
                    return None;
                }
                Some((
                    name.clone(),
                    self.log_data.player_class(name).to_string(),
                    ps.healing,
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
                Some((name.clone(), DetailType::Damage)),
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
        let heal_header = row![
            text("Effective healing done")
                .size(13)
                .color(theme::TEXT_SECONDARY),
            horizontal_space(),
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

    fn view_utility_tab(&self) -> Element<'_, ViewerMessage> {
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

    // ── Loot Tab ────────────────────────────────────────────────────────

    #[allow(clippy::too_many_lines)] // iced UI layout — grouped loot with boss sections
    fn view_loot_tab(&self) -> Element<'_, ViewerMessage> {
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
            horizontal_space(),
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
                        item_row = item_row.push(horizontal_space());
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
                column![header, horizontal_rule(1), content_col]
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
                if let Some(enc) = self.log_data.encounters.get(*idx) {
                    if let Some(name) = &enc.name {
                        return self
                            .log_data
                            .loot
                            .iter()
                            .filter(|l| l.boss == *name)
                            .collect();
                    }
                }
                Vec::new()
            }
        }
    }

    // ── Timeline Tab ────────────────────────────────────────────────────

    #[allow(clippy::too_many_lines)] // iced UI layout — timeline chart with multiple data lanes
    fn view_timeline_tab(&self) -> Element<'_, ViewerMessage> {
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
            legend_toggle(
                theme::TIMELINE_DTPS,
                "Raid DTPS",
                vis.show_dtps,
                TimelineSeriesKind::Dtps,
            ),
            legend_toggle(
                theme::TIMELINE_HPS,
                "Raid HPS",
                vis.show_hps,
                TimelineSeriesKind::Hps,
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
            horizontal_space(),
            button(text(y_axis_label).size(10).color(theme::TEXT_SECONDARY))
                .on_press(ViewerMessage::ToggleTimelineYAxis)
                .padding([3, 8])
                .style(transparent_button_style),
        ]
        .spacing(8)
        .align_y(Center);

        // ── Canvas sparkline chart ─────────────────────────────────────
        let chart_canvas = canvas::Canvas::new(TimelineChart {
            data: td,
            visibility: vis,
            shared_y: self.timeline_shared_y,
            hover_idx: self.timeline_hover,
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

            let alive_canvas = canvas::Canvas::new(AliveChart { data: td })
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

        // ── Fixed top section: charts ──────────────────────────────────
        let charts_panel = container(
            column![
                header,
                legend,
                horizontal_rule(1),
                chart_canvas,
                tooltip,
                horizontal_rule(1),
                alive_section,
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

        let filter_bar = container(
            row![
                mode_buttons,
                type_toggles,
                horizontal_space(),
                player_picker
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

    // ── Events Tab ──────────────────────────────────────────────────────

    fn view_events_tab(&self) -> Element<'_, ViewerMessage> {
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
                ..
            } => {
                let mut s = format!("{source}'s {spell} hits {target} for {amount}");
                if *absorbed > 0 {
                    let _ = write!(s, " ({absorbed} absorbed)");
                }
                (s, Color::from_rgb8(200, 200, 200))
            }
            LogEntry::Healing {
                source,
                target,
                spell,
                amount,
                ..
            } => (
                format!("{source}'s {spell} heals {target} for {amount}"),
                Color::from_rgb8(100, 255, 100),
            ),
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
        };

        row![
            text(ts).size(11).color([0.4, 0.4, 0.4]),
            text(text_str).size(12).color(color),
        ]
        .spacing(8)
        .into()
    }

    // ── Detail Overlay ──────────────────────────────────────────────────

    fn view_detail_overlay<'a>(&'a self, detail: &'a DetailView) -> Element<'a, ViewerMessage> {
        let close_btn = button(text("X").size(14))
            .on_press(ViewerMessage::CloseDetail)
            .padding([4, 10]);

        let title = match &detail.detail_type {
            DetailType::Damage => format!("{} - Damage Breakdown", detail.player_name),
            DetailType::Healing => format!("{} - Healing Breakdown", detail.player_name),
            DetailType::Dispels => format!("{} - Dispel Breakdown", detail.player_name),
            DetailType::Interrupts => format!("{} - Interrupt Breakdown", detail.player_name),
            DetailType::Resurrects => format!("{} - Resurrection Breakdown", detail.player_name),
            DetailType::Avoidance => format!("{} - Avoidance Breakdown", detail.player_name),
            DetailType::Buffs => format!("{} - Buff Breakdown", detail.player_name),
            DetailType::Consumables => format!("{} - Consumable Usage", detail.player_name),
        };

        // Player info bar (spec/race/guild) — shown for all detail types
        let player_info = self.view_player_info_bar(&detail.player_name);

        let header = row![text(title).size(18), horizontal_space(), close_btn,]
            .spacing(8)
            .align_y(Center)
            .width(Fill);

        let content = match &detail.detail_type {
            DetailType::Damage | DetailType::Healing => {
                self.view_ability_breakdown(&detail.player_name, detail.detail_type)
            }
            DetailType::Dispels => self.view_dispel_detail(&detail.player_name),
            DetailType::Interrupts => self.view_interrupt_detail(&detail.player_name),
            DetailType::Resurrects => self.view_resurrect_detail(&detail.player_name),
            DetailType::Avoidance => self.view_avoidance_detail(&detail.player_name),
            DetailType::Buffs => self.view_buff_detail(&detail.player_name),
            DetailType::Consumables => self.view_consumable_detail(&detail.player_name),
        };

        scrollable(
            container(
                column![header, player_info, horizontal_rule(1), content]
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

        let (abilities, total) = match dtype {
            DetailType::Damage => (&ps.abilities, ps.damage),
            DetailType::Healing => (&ps.healing_abilities, ps.healing),
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

        let summary = text(format!(
            "Total: {} | Per Second: {}/s | Duration: {} | Hits: {} | Crits: {} | Crit Rate: {:.1}%",
            theme::format_number(total),
            theme::format_number_f64(pps),
            theme::format_duration(duration),
            total_hits,
            total_crits,
            crit_rate,
        ))
        .size(12)
        .color([0.5, 0.5, 0.5]);

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
                opener_section.push(text("Opener (first 10s)").size(13).color([0.6, 0.6, 0.6]));
            let mut opener_row = Row::new().spacing(4);
            for (i, o) in opener.iter().enumerate() {
                if i > 0 {
                    opener_row = opener_row.push(text("->").size(11).color([0.4, 0.4, 0.4]));
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
                            .color([0.5, 0.5, 0.5]),
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

        // Ability table
        let table_header = row![
            text("Ability").size(12).width(Length::FillPortion(3)),
            text("Total").size(12).width(Length::FillPortion(1)),
            text("Hits").size(12).width(Length::FillPortion(1)),
            text("Crits").size(12).width(Length::FillPortion(1)),
            text("Crit%").size(12).width(Length::FillPortion(1)),
            text("Avg").size(12).width(Length::FillPortion(1)),
            text("%").size(12).width(Length::FillPortion(1)),
        ]
        .spacing(4)
        .width(Fill);

        let mut table = Column::new().spacing(2);
        table = table.push(text("Ability Breakdown").size(13).color([0.6, 0.6, 0.6]));
        table = table.push(table_header);
        table = table.push(horizontal_rule(1));

        for (spell, ab) in &sorted {
            let percent = if total > 0 {
                (ab.total as f64 / total as f64) * 100.0
            } else {
                0.0
            };
            let avg = ab.total.checked_div(ab.hits).unwrap_or(0);
            let crit_pct = if ab.hits > 0 {
                (ab.crits as f64 / ab.hits as f64) * 100.0
            } else {
                0.0
            };

            table = table.push(
                row![
                    text(spell.clone()).size(12).width(Length::FillPortion(3)),
                    text(theme::format_number(ab.total))
                        .size(12)
                        .width(Length::FillPortion(1)),
                    text(ab.hits.to_string())
                        .size(12)
                        .width(Length::FillPortion(1)),
                    text(ab.crits.to_string())
                        .size(12)
                        .width(Length::FillPortion(1)),
                    text(format!("{crit_pct:.1}%"))
                        .size(12)
                        .width(Length::FillPortion(1)),
                    text(theme::format_number(avg))
                        .size(12)
                        .width(Length::FillPortion(1)),
                    text(format!("{percent:.1}%"))
                        .size(12)
                        .width(Length::FillPortion(1)),
                ]
                .spacing(4)
                .width(Fill),
            );
        }

        column![summary, opener_section, table,]
            .spacing(10)
            .width(Fill)
            .into()
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
        let resurrects: Vec<&ResurrectEvent> = self
            .log_data
            .filtered_resurrects(&self.encounter_filter)
            .into_iter()
            .filter(|r| r.caster == player)
            .collect();

        let summary = text(format!("Total Resurrections: {}", resurrects.len()))
            .size(12)
            .color([0.5, 0.5, 0.5]);

        let mut col = Column::new().spacing(4);
        for r in &resurrects {
            let ts = format_timestamp(r.timestamp);
            col = col.push(
                row![
                    text(&r.target).size(13).width(Length::FillPortion(2)),
                    text(&r.spell).size(13).width(Length::FillPortion(2)),
                    text(ts)
                        .size(12)
                        .color([0.5, 0.5, 0.5])
                        .width(Length::FillPortion(1)),
                ]
                .spacing(4),
            );
        }

        column![summary, col,].spacing(8).width(Fill).into()
    }

    fn view_avoidance_detail(&self, player: &str) -> Element<'_, ViewerMessage> {
        let Some(av) = self.log_data.avoidance.get(player) else {
            return text("No avoidance data").size(14).into();
        };

        let summary = text(format!("Total Attacks Avoided: {}", av.total()))
            .size(12)
            .color([0.5, 0.5, 0.5]);

        let rows = vec![
            ("Dodges", av.dodges),
            ("Parries", av.parries),
            ("Blocks", av.blocks),
            ("Attacks Missed You", av.missed_by),
            ("Your Attacks Missed", av.misses),
        ];

        let mut col = Column::new().spacing(4);
        for (label, count) in rows {
            col = col.push(
                row![
                    text(label).size(13).width(Length::FillPortion(3)),
                    text(count.to_string())
                        .size(13)
                        .width(Length::FillPortion(1)),
                ]
                .spacing(4),
            );
        }

        column![summary, col,].spacing(8).width(Fill).into()
    }

    fn view_buff_detail(&self, player: &str) -> Element<'_, ViewerMessage> {
        let Some(buffs) = self.log_data.buffs.get(player) else {
            return text("No buff data").size(14).into();
        };

        let mut sorted: Vec<(&str, &BuffStats)> = buffs
            .iter()
            .map(|(name, stats)| (name.as_str(), stats))
            .collect();
        sorted.sort_by_key(|s| Reverse(s.1.gains));

        let summary = text(format!("Total Unique Buffs: {}", sorted.len()))
            .size(12)
            .color([0.5, 0.5, 0.5]);

        let mut col = Column::new().spacing(4);
        col = col.push(
            row![
                text("Buff").size(12).width(Length::FillPortion(3)),
                text("Gains").size(12).width(Length::FillPortion(1)),
                text("Fades").size(12).width(Length::FillPortion(1)),
            ]
            .spacing(4),
        );
        col = col.push(horizontal_rule(1));

        for (buff, stats) in &sorted {
            col = col.push(
                row![
                    text(*buff).size(12).width(Length::FillPortion(3)),
                    text(stats.gains.to_string())
                        .size(12)
                        .width(Length::FillPortion(1)),
                    text(stats.fades.to_string())
                        .size(12)
                        .width(Length::FillPortion(1)),
                ]
                .spacing(4),
            );
        }

        column![summary, col,].spacing(8).width(Fill).into()
    }

    fn view_consumable_detail(&self, player: &str) -> Element<'_, ViewerMessage> {
        let consumables = self.log_data.filtered_consumables(&self.encounter_filter);
        let player_cons: Vec<_> = consumables
            .into_iter()
            .filter(|c| c.player == player)
            .collect();

        // Aggregate by consumable name
        let mut by_name: HashMap<&str, u64> = HashMap::new();
        for c in &player_cons {
            *by_name.entry(&c.consumable).or_insert(0) += 1;
        }
        let mut sorted: Vec<(&&str, &u64)> = by_name.iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(a.1));

        let summary = text(format!("Total Consumable Uses: {}", player_cons.len()))
            .size(12)
            .color(theme::TEXT_SECONDARY);

        let mut col = Column::new().spacing(4);
        col = col.push(
            row![
                text("Consumable").size(12).width(Length::FillPortion(3)),
                text("Uses").size(12).width(Length::FillPortion(1)),
            ]
            .spacing(4),
        );
        col = col.push(horizontal_rule(1));

        for (name, count) in &sorted {
            col = col.push(
                row![
                    text(**name)
                        .size(12)
                        .color(theme::BAR_CONSUMABLE)
                        .width(Length::FillPortion(3)),
                    text(count.to_string())
                        .size(12)
                        .width(Length::FillPortion(1)),
                ]
                .spacing(4),
            );
        }

        if sorted.is_empty() {
            col = col.push(empty_state("No consumables used"));
        }

        column![summary, col,].spacing(8).width(Fill).into()
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

    /// Full meter bar row with value + per-second + percentage.
    #[allow(clippy::too_many_arguments, clippy::unused_self)]
    fn meter_bar_row(
        &self,
        rank: usize,
        name: &str,
        class: &str,
        value: u64,
        pps: f64,
        percent: f64,
        bar_color: Color,
        on_click: Option<(String, DetailType)>,
    ) -> Element<'_, ViewerMessage> {
        let value_text = format!(
            "{} - {}/s",
            theme::format_number(value),
            theme::format_number_f64(pps)
        );
        let pct_text = format!("{percent:.1}%");

        build_meter_row(
            rank,
            name,
            class,
            &value_text,
            &pct_text,
            percent,
            bar_color,
            on_click,
        )
    }

    /// Meter bar with custom detail text (for avoidance breakdown).
    #[allow(clippy::too_many_arguments, clippy::unused_self)]
    fn meter_bar_row_with_detail_text(
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
}

// ── Shared Helpers ─────────────────────────────────────────────────────────

/// Unified detail view for spell+target event types (dispels, interrupts).
///
/// Shows total count, "By Spell" breakdown, and "Top Targets" (top 5).
fn view_spell_target_detail<'a>(
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
        .color([0.5, 0.5, 0.5]);

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

/// Shared helper: build meter bars from pre-aggregated players and return total label.
///
/// Converts borrowed `&str` slices to owned data so the result doesn't borrow
/// the caller's local filtered-events vector.
fn view_event_meters_owned<'a>(
    players: &[(&str, &str, u64)],
    bar_color: Color,
    empty_msg: &'a str,
    detail_type: Option<DetailType>,
    unit: &str,
) -> (Element<'a, ViewerMessage>, String) {
    let total: u64 = players.iter().map(|(_, _, v)| *v).sum();
    let max_val = players.first().map_or(1, |(_, _, v)| *v);
    let mut col = Column::new().spacing(2);
    for (rank, (name, class, value)) in players.iter().enumerate() {
        let percent = percent_of(*value, max_val);
        let on_click = detail_type.map(|dt| ((*name).to_string(), dt));
        col = col.push(build_meter_row(
            rank + 1,
            name,
            class,
            &theme::format_number(*value),
            "",
            percent,
            bar_color,
            on_click,
        ));
    }
    if players.is_empty() {
        col = col.push(empty_state(empty_msg));
    }
    (col.into(), format!("{total} {unit}"))
}

/// Per-second rate, guarding against zero duration.
fn per_second(value: u64, duration: f64) -> f64 {
    if duration > 0.0 {
        value as f64 / duration
    } else {
        0.0
    }
}

/// Percentage of total, guarding against zero total.
fn percent_of(value: u64, total: u64) -> f64 {
    if total > 0 {
        (value as f64 / total as f64) * 100.0
    } else {
        0.0
    }
}

/// Build a sorted, aggregated meter bar column from `(name, class, value)` tuples.
///
/// Shared by deaths, resurrects, absorbs, consumables, and dispel/interrupt panels.
#[allow(clippy::too_many_arguments)]
fn build_simple_meters<'a>(
    players: &[(&str, &str, u64)],
    bar_color: Color,
    empty_msg: &'a str,
    detail_type: Option<DetailType>,
) -> Column<'a, ViewerMessage> {
    let max_val = players.first().map_or(1, |(_, _, v)| *v);
    let mut col = Column::new().spacing(2);
    for (rank, (name, class, value)) in players.iter().enumerate() {
        let percent = percent_of(*value, max_val);
        let on_click = detail_type.map(|dt| ((*name).to_string(), dt));
        col = col.push(build_meter_row(
            rank + 1,
            name,
            class,
            &theme::format_number(*value),
            "",
            percent,
            bar_color,
            on_click,
        ));
    }
    if players.is_empty() {
        col = col.push(empty_state(empty_msg));
    }
    col
}

/// Aggregate events by a player field, filter to known combatants, and sort descending.
///
/// The `class_fn` returns a class name whose lifetime is tied to `'a` (typically
/// backed by the `LogData`'s combatant map).
fn aggregate_by_player<'a, T>(
    events: &'a [&T],
    get_player: impl Fn(&'a &T) -> &'a str,
    combatants: &'a HashMap<String, crate::log_data::Combatant>,
    class_fn: impl Fn(&str) -> &'a str,
) -> Vec<(&'a str, &'a str, u64)> {
    let mut by_player: HashMap<&'a str, u64> = HashMap::new();
    for e in events {
        *by_player.entry(get_player(e)).or_insert(0) += 1;
    }
    let mut players: Vec<(&'a str, &'a str, u64)> = by_player
        .into_iter()
        .filter(|(name, _)| combatants.contains_key(*name))
        .map(|(name, count)| (name, class_fn(name), count))
        .collect();
    players.sort_by_key(|p| Reverse(p.2));
    players
}

// ── Free Functions ──────────────────────────────────────────────────────────

/// Visual separator in the encounter `pick_list` between quick filters and bosses.
const ENCOUNTER_SEPARATOR: &str = "-------------------";

/// Build the encounter names list for the `pick_list`.
///
/// Single pass over encounters to collect counts and durations.
fn build_encounter_names(data: &LogData) -> Vec<String> {
    let mut names = Vec::new();

    let total_duration = data.end_time.unwrap_or(0.0) - data.start_time.unwrap_or(0.0);
    names.push(format!(
        "All Combat ({})",
        theme::format_duration(total_duration)
    ));

    // Single pass: accumulate counts and durations for all categories
    let mut kill_count: usize = 0;
    let mut kill_duration: f64 = 0.0;
    let mut wipe_count: usize = 0;
    let mut wipe_duration: f64 = 0.0;
    let mut trash_count: usize = 0;
    let mut trash_duration: f64 = 0.0;

    for enc in &data.encounters {
        if enc.is_boss {
            if enc.is_kill {
                kill_count += 1;
                kill_duration += enc.duration;
            } else {
                wipe_count += 1;
                wipe_duration += enc.duration;
            }
        } else {
            trash_count += 1;
            trash_duration += enc.duration;
        }
    }

    if kill_count > 0 {
        names.push(format!(
            "All Kills ({kill_count}) - {}",
            theme::format_duration(kill_duration)
        ));
    }
    if wipe_count > 0 {
        names.push(format!(
            "All Wipes ({wipe_count}) - {}",
            theme::format_duration(wipe_duration)
        ));
    }
    if trash_count > 0 {
        names.push(format!(
            "All Trash ({trash_count}) - {}",
            theme::format_duration(trash_duration)
        ));
    }

    // Separator between quick filters and individual boss encounters
    if kill_count > 0 || wipe_count > 0 {
        names.push(ENCOUNTER_SEPARATOR.to_string());
    }

    // Individual boss encounters only (trash only accessible via "All Trash")
    for (i, enc) in data.encounters.iter().enumerate() {
        if !enc.is_boss {
            continue;
        }
        names.push(build_single_encounter_name(enc, i));
    }

    names
}

/// Parse an encounter name back into an `EncounterFilter`.
fn parse_encounter_filter(name: &str, data: &LogData) -> EncounterFilter {
    // Ignore separator lines
    if name == ENCOUNTER_SEPARATOR {
        return EncounterFilter::All;
    }
    if name.starts_with("All Combat") {
        return EncounterFilter::All;
    }
    if name.starts_with("All Kills") {
        return EncounterFilter::AllKills;
    }
    if name.starts_with("All Wipes") {
        return EncounterFilter::AllWipes;
    }
    if name.starts_with("All Trash") {
        return EncounterFilter::AllTrash;
    }

    // Try to find matching encounter by name
    for (i, enc) in data.encounters.iter().enumerate() {
        let enc_name = build_single_encounter_name(enc, i);
        if enc_name == name {
            return EncounterFilter::Single(i);
        }
    }

    EncounterFilter::All
}

fn build_single_encounter_name(enc: &Encounter, index: usize) -> String {
    if let Some(boss_name) = &enc.name {
        let result = if enc.is_kill { "Kill" } else { "Wipe" };
        let attempt = enc.attempt.map_or(String::new(), |a| format!(" {a}"));
        format!(
            "{boss_name} - {result}{attempt} - {}",
            theme::format_duration(enc.duration)
        )
    } else {
        format!(
            "Encounter {} - {}",
            index + 1,
            theme::format_duration(enc.duration)
        )
    }
}

/// Format a timestamp (f64 in our internal format) to HH:MM:SS.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn format_timestamp(ts: f64) -> String {
    // Our timestamps are encoded as (month*31+day)*86400 + hour*3600 + min*60 + sec + ms/1000
    let day_secs = ts % 86400.0;
    let hour = (day_secs / 3600.0) as u64;
    let min = ((day_secs % 3600.0) / 60.0) as u64;
    let sec = (day_secs % 60.0) as u64;
    format!("{hour:02}:{min:02}:{sec:02}")
}

/// Build a complete meter row: [icon] [bar with rank+name+stats] [percent]
///
/// Layout matches turtlogs: class icon on the left, a `Stack`-based colored bar
/// in the middle containing rank/name/stats, and percentage pinned to the right.
#[allow(clippy::too_many_arguments)]
fn build_meter_row<'a>(
    rank: usize,
    name: &str,
    class: &str,
    value_text: &str,
    pct_text: &str,
    percent: f64,
    bar_color: Color,
    on_click: Option<(String, DetailType)>,
) -> Element<'a, ViewerMessage> {
    let class_color = theme::class_color(class);

    // Class icon (outside the bar, left side)
    let icon = image(theme::class_icon(class)).width(22).height(22);

    // Inner bar content: rank + name left, stats right
    let inner: Row<'_, ViewerMessage> = row![
        text(format!("{rank}.")).size(12).color(theme::TEXT_MUTED),
        text(name.to_string()).size(12).color(class_color),
        horizontal_space(),
        text(value_text.to_string())
            .size(12)
            .color(Color::from_rgb8(220, 225, 230)),
    ]
    .spacing(6)
    .align_y(Center)
    .width(Fill);

    // The bar stack (colored proportion bar + text overlay)
    let bar = make_bar(inner, percent, bar_color);

    // Percentage label (outside the bar, right side)
    let pct_label: Element<ViewerMessage> = if pct_text.is_empty() {
        horizontal_space().width(0).into()
    } else {
        text(pct_text.to_string())
            .size(12)
            .color(theme::TEXT_MUTED)
            .width(50)
            .align_x(iced::alignment::Horizontal::Right)
            .into()
    };

    // Assemble: [icon] [bar] [percent]
    let full_row: Element<ViewerMessage> = row![icon, bar, pct_label]
        .spacing(6)
        .align_y(Center)
        .width(Fill)
        .into();

    // Wrap in transparent button if clickable
    if let Some((player, dtype)) = on_click {
        button(full_row)
            .on_press(ViewerMessage::ShowDetail(player, dtype))
            .padding(0)
            .width(Fill)
            .style(transparent_button_style)
            .into()
    } else {
        full_row
    }
}

/// Transparent button style with subtle hover highlight.
fn transparent_button_style(_theme: &iced::Theme, status: button::Status) -> button::Style {
    let hover_bg = match status {
        button::Status::Hovered => Some(iced::Background::Color(Color {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 0.06,
        })),
        _ => None,
    };
    button::Style {
        background: hover_bg,
        text_color: Color::WHITE,
        border: iced::Border::default(),
        shadow: iced::Shadow::default(),
    }
}

/// Create the colored meter bar element.
///
/// Uses a `Stack` to layer a partial-width colored bar behind the full-width
/// text content, producing the classic DPS-meter look.
fn make_bar(
    content: Row<'_, ViewerMessage>,
    percent: f64,
    bar_color: Color,
) -> Element<'_, ViewerMessage> {
    let pct = percent.clamp(1.0, 100.0) as u16;
    let remainder = 100_u16.saturating_sub(pct).max(1);

    // Vibrant class-colored bar at ~65% brightness — visible proportion indicator
    let bg_color = Color {
        r: bar_color.r * 0.65,
        g: bar_color.g * 0.65,
        b: bar_color.b * 0.65,
        a: 0.9,
    };

    // Bottom layer: partial-width colored bar + transparent spacer
    let bar_layer: Element<ViewerMessage> = row![
        container("")
            .width(Length::FillPortion(pct))
            .height(Fill)
            .style(move |_theme: &iced::Theme| container::Style {
                background: Some(iced::Background::Color(bg_color)),
                border: iced::Border {
                    radius: 2.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            }),
        container("").width(Length::FillPortion(remainder)),
    ]
    .width(Fill)
    .height(Fill)
    .into();

    // Top layer: full-width text content
    let text_layer: Element<ViewerMessage> = content.padding([4, 8]).into();

    // Fixed height prevents the bar from expanding unboundedly inside a scrollable.
    stack![bar_layer, text_layer].width(Fill).height(28).into()
}

/// Styled container background for card/panel sections.
fn panel_style(_theme: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(theme::SURFACE)),
        border: iced::Border {
            color: theme::SURFACE_BORDER,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..Default::default()
    }
}

/// Big hit threshold: damage-taken events above this value are marked on the timeline.
const BIG_HIT_THRESHOLD: u64 = 2000;

/// Format seconds offset as encounter time (M:SS).
fn format_encounter_time(seconds: f64) -> String {
    let total = seconds as u64;
    let mins = total / 60;
    let secs = total % 60;
    format!("{mins}:{secs:02}")
}

/// Build a clickable legend toggle button.
///
/// Active: full-color dot + white text. Inactive: dimmed dot + muted text.
fn legend_toggle(
    color: Color,
    label: &str,
    active: bool,
    kind: TimelineSeriesKind,
) -> Element<'_, ViewerMessage> {
    let dot_color = if active {
        color
    } else {
        Color { a: 0.25, ..color }
    };
    let text_color = if active {
        Color::WHITE
    } else {
        theme::TEXT_MUTED
    };

    let dot: Element<ViewerMessage> = container("")
        .width(8)
        .height(8)
        .style(move |_theme: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(dot_color)),
            border: iced::Border {
                radius: 4.0.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into();

    let content = row![dot, text(label).size(11).color(text_color)]
        .spacing(4)
        .align_y(Center);

    button(content)
        .on_press(ViewerMessage::ToggleTimelineSeries(kind))
        .padding([2, 6])
        .style(move |_theme: &iced::Theme, status| {
            let bg = if active {
                match status {
                    button::Status::Hovered => Some(iced::Background::Color(Color::from_rgba8(
                        255, 255, 255, 0.08,
                    ))),
                    _ => Some(iced::Background::Color(Color::from_rgba8(
                        255, 255, 255, 0.04,
                    ))),
                }
            } else {
                match status {
                    button::Status::Hovered => Some(iced::Background::Color(Color::from_rgba8(
                        255, 255, 255, 0.04,
                    ))),
                    _ => None,
                }
            };
            button::Style {
                background: bg,
                text_color,
                border: iced::Border {
                    radius: 4.0.into(),
                    ..Default::default()
                },
                shadow: iced::Shadow::default(),
            }
        })
        .into()
}

// ── Timeline Canvas Programs ────────────────────────────────────────────────

/// Canvas program that renders the main DPS/DTPS/HPS sparkline chart.
///
/// Each enabled series is drawn as a filled area with a solid stroke on top.
/// Death and big-hit markers are drawn as vertical lines at their X positions.
struct TimelineChart<'a> {
    data: &'a TimelineData,
    visibility: &'a TimelineVisibility,
    shared_y: bool,
    hover_idx: Option<usize>,
}

impl canvas::Program<ViewerMessage> for TimelineChart<'_> {
    type State = ();

    #[allow(clippy::similar_names)] // dps/dtps/hps are standard WoW combat metrics
    #[allow(clippy::too_many_lines)] // Canvas draw — single rendering pass with multiple layers
    #[allow(clippy::many_single_char_names)] // x/y/w/h/n/t are standard 2D drawing variables
    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let td = self.data;
        let vis = self.visibility;
        let w = bounds.width;
        let h = bounds.height;

        if td.buckets.is_empty() || w < 2.0 || h < 2.0 {
            return vec![];
        }

        let mut frame = canvas::Frame::new(renderer, bounds.size());

        // Background
        frame.fill_rectangle(
            Point::ORIGIN,
            bounds.size(),
            Color::from_rgba8(0, 0, 0, 0.3),
        );

        let n = td.buckets.len();
        let x_scale = w / n.max(1) as f32;

        // Compute Y-axis maximums
        let shared_max = td.max_dps.max(td.max_dtps).max(td.max_hps).max(1) as f32;
        let dps_max = if self.shared_y {
            shared_max
        } else {
            td.max_dps.max(1) as f32
        };
        let dtps_max = if self.shared_y {
            shared_max
        } else {
            td.max_dtps.max(1) as f32
        };
        let hps_max = if self.shared_y {
            shared_max
        } else {
            td.max_hps.max(1) as f32
        };

        // Draw series in back-to-front order: DPS (behind), HPS, DTPS (front).
        // Each call is explicit to avoid a complex generic tuple array.
        if vis.show_dps {
            draw_sparkline_area(
                &mut frame,
                &td.buckets,
                &|b| b.damage,
                x_scale,
                h,
                dps_max,
                theme::TIMELINE_DPS,
            );
        }
        if vis.show_hps {
            draw_sparkline_area(
                &mut frame,
                &td.buckets,
                &|b| b.healing,
                x_scale,
                h,
                hps_max,
                theme::TIMELINE_HPS,
            );
        }
        if vis.show_dtps {
            draw_sparkline_area(
                &mut frame,
                &td.buckets,
                &|b| b.damage_taken,
                x_scale,
                h,
                dtps_max,
                theme::TIMELINE_DTPS,
            );
        }

        // Draw event markers (vertical lines for deaths and big hits)
        for event in &td.events {
            let visible = match event.kind {
                TimelineEventKind::Death | TimelineEventKind::Resurrect => vis.show_deaths,
                TimelineEventKind::BigHit => vis.show_big_hits,
                _ => false, // dispels/interrupts shown in event list only
            };
            if !visible {
                continue;
            }
            let x = (event.offset as f32 / td.duration as f32) * w;
            let marker_color = match event.kind {
                TimelineEventKind::Death => theme::TIMELINE_DEATH,
                TimelineEventKind::BigHit => theme::TIMELINE_BIG_HIT,
                TimelineEventKind::Resurrect => theme::TIMELINE_RESURRECT,
                _ => Color::WHITE,
            };

            let line = canvas::Path::line(Point::new(x, 0.0), Point::new(x, h));
            frame.stroke(
                &line,
                canvas::Stroke::default()
                    .with_color(Color {
                        a: 0.5,
                        ..marker_color
                    })
                    .with_width(1.0),
            );

            // Small circle at top
            let dot = canvas::Path::circle(Point::new(x, 4.0), 3.0);
            frame.fill(&dot, marker_color);
        }

        // Hover line
        if let Some(idx) = self.hover_idx {
            let x = (idx as f32 + 0.5) * x_scale;
            let line = canvas::Path::line(Point::new(x, 0.0), Point::new(x, h));
            frame.stroke(
                &line,
                canvas::Stroke::default()
                    .with_color(Color::from_rgba8(255, 255, 255, 0.6))
                    .with_width(1.0),
            );
        }

        // Y-axis scale labels drawn on canvas
        let y_max_display = if self.shared_y {
            shared_max as u64
        } else {
            // Show the largest visible series max
            let mut m = 0_u64;
            if vis.show_dps {
                m = m.max(td.max_dps);
            }
            if vis.show_dtps {
                m = m.max(td.max_dtps);
            }
            if vis.show_hps {
                m = m.max(td.max_hps);
            }
            m.max(1)
        };

        let y_labels = [
            (0.0, theme::format_number(y_max_display)),
            (h / 2.0, theme::format_number(y_max_display / 2)),
            (h - 10.0, "0".to_string()),
        ];
        for (y, label_text) in &y_labels {
            frame.fill_text(canvas::Text {
                content: label_text.clone(),
                position: Point::new(4.0, *y),
                color: theme::TEXT_MUTED,
                size: 10.0.into(),
                ..canvas::Text::default()
            });
        }

        // X-axis time labels
        let label_interval = if td.duration > 300.0 {
            60.0
        } else if td.duration > 120.0 {
            30.0
        } else {
            15.0
        };

        let mut t = 0.0;
        while t < td.duration {
            let x = (t as f32 / td.duration as f32) * w;
            let label_text = format_encounter_time(t);
            frame.fill_text(canvas::Text {
                content: label_text,
                position: Point::new(x + 2.0, h - 12.0),
                color: Color {
                    a: 0.3,
                    ..Color::WHITE
                },
                size: 9.0.into(),
                ..canvas::Text::default()
            });
            t += label_interval;
        }

        vec![frame.into_geometry()]
    }

    fn update(
        &self,
        _state: &mut (),
        event: canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (canvas::event::Status, Option<ViewerMessage>) {
        match event {
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    let n = self.data.buckets.len();
                    if n > 0 {
                        let x_scale = bounds.width / n as f32;
                        let idx = (pos.x / x_scale) as usize;
                        let clamped = idx.min(n.saturating_sub(1));
                        return (
                            canvas::event::Status::Captured,
                            Some(ViewerMessage::TimelineHover(Some(clamped))),
                        );
                    }
                } else {
                    return (
                        canvas::event::Status::Ignored,
                        Some(ViewerMessage::TimelineHover(None)),
                    );
                }
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    let n = self.data.buckets.len();
                    if n > 0 {
                        let x_scale = bounds.width / n as f32;
                        let idx = (pos.x / x_scale) as usize;
                        let clamped = idx.min(n.saturating_sub(1));
                        return (
                            canvas::event::Status::Captured,
                            Some(ViewerMessage::TimelineClick(clamped)),
                        );
                    }
                }
            }
            _ => {}
        }
        (canvas::event::Status::Ignored, None)
    }
}

/// Draw a single filled sparkline area on the frame.
///
/// Builds a closed path from the bucket values, fills with semi-transparent
/// color, then strokes the top edge with a solid line.
fn draw_sparkline_area(
    frame: &mut canvas::Frame,
    buckets: &[crate::log_data::TimelineBucket],
    get_val: &dyn Fn(&crate::log_data::TimelineBucket) -> u64,
    x_scale: f32,
    height: f32,
    y_max: f32,
    color: Color,
) {
    if buckets.is_empty() {
        return;
    }

    // Build the line path (top edge)
    let line_path = canvas::Path::new(|b| {
        for (i, bucket) in buckets.iter().enumerate() {
            let x = (i as f32 + 0.5) * x_scale;
            let val = get_val(bucket) as f32;
            let y = height - (val / y_max) * height;
            if i == 0 {
                b.move_to(Point::new(x, y));
            } else {
                b.line_to(Point::new(x, y));
            }
        }
    });

    // Build the filled area path (line + close back to baseline)
    let area_path = canvas::Path::new(|b| {
        // Start at bottom-left
        b.move_to(Point::new(0.5 * x_scale, height));

        for (i, bucket) in buckets.iter().enumerate() {
            let x = (i as f32 + 0.5) * x_scale;
            let val = get_val(bucket) as f32;
            let y = height - (val / y_max) * height;
            b.line_to(Point::new(x, y));
        }

        // Close back to bottom-right
        let last_x = ((buckets.len() - 1) as f32 + 0.5) * x_scale;
        b.line_to(Point::new(last_x, height));
        b.close();
    });

    // Fill area with transparency
    frame.fill(&area_path, Color { a: 0.2, ..color });

    // Stroke line on top
    frame.stroke(
        &line_path,
        canvas::Stroke::default()
            .with_color(color)
            .with_width(1.5)
            .with_line_join(canvas::LineJoin::Round),
    );
}

/// Canvas program for the alive-count sparkline below the main chart.
struct AliveChart<'a> {
    data: &'a TimelineData,
}

impl canvas::Program<ViewerMessage> for AliveChart<'_> {
    type State = ();

    #[allow(clippy::many_single_char_names)] // x/y/w/h/n are standard 2D drawing variables
    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let td = self.data;
        let w = bounds.width;
        let h = bounds.height;

        if td.buckets.is_empty() || w < 2.0 || h < 2.0 {
            return vec![];
        }

        let mut frame = canvas::Frame::new(renderer, bounds.size());

        // Background
        frame.fill_rectangle(
            Point::ORIGIN,
            bounds.size(),
            Color::from_rgba8(0, 0, 0, 0.2),
        );

        let n = td.buckets.len();
        let x_scale = w / n.max(1) as f32;
        let y_max = td.raid_count.max(1) as f32;

        // Filled area
        let area_path = canvas::Path::new(|b| {
            b.move_to(Point::new(0.5 * x_scale, h));
            for (i, bucket) in td.buckets.iter().enumerate() {
                let x = (i as f32 + 0.5) * x_scale;
                let y = h - (bucket.alive_count as f32 / y_max) * h;
                b.line_to(Point::new(x, y));
            }
            let last_x = ((n - 1) as f32 + 0.5) * x_scale;
            b.line_to(Point::new(last_x, h));
            b.close();
        });

        frame.fill(
            &area_path,
            Color {
                a: 0.25,
                ..theme::TIMELINE_ALIVE
            },
        );

        // Stroke line
        let line_path = canvas::Path::new(|b| {
            for (i, bucket) in td.buckets.iter().enumerate() {
                let x = (i as f32 + 0.5) * x_scale;
                let y = h - (bucket.alive_count as f32 / y_max) * h;
                if i == 0 {
                    b.move_to(Point::new(x, y));
                } else {
                    b.line_to(Point::new(x, y));
                }
            }
        });

        frame.stroke(
            &line_path,
            canvas::Stroke::default()
                .with_color(theme::TIMELINE_ALIVE)
                .with_width(1.5)
                .with_line_join(canvas::LineJoin::Round),
        );

        vec![frame.into_geometry()]
    }
}

/// Seconds before a death to show in Death Log mode.
const DEATH_LOG_WINDOW: f64 = 10.0;

/// Big hit threshold for Key Events mode.
const KEY_EVENT_BIG_HIT: u64 = 2000;

/// Build the encounter event log for the timeline tab.
///
/// Supports three modes:
/// - `AllEvents`: every entry matching type filters
/// - `KeyEvents`: deaths, big hits, dispels, interrupts, resurrects only
/// - `DeathLog`: for each player death, all events involving that player
///   in the 10 seconds before death
#[allow(clippy::too_many_lines)] // Event log builder — single pass with mode dispatch
#[allow(clippy::too_many_arguments)] // Will be refactored into a struct in future
fn build_timeline_event_log<'a>(
    log_data: &'a LogData,
    filter: &EncounterFilter,
    type_filter: &EventLogTypeFilter,
    mode: EventLogMode,
    player_filter: Option<&str>,
    clicked_second: Option<usize>,
) -> Element<'a, ViewerMessage> {
    let encounters = log_data.selected_encounters(filter);
    if encounters.is_empty() {
        return empty_state("No encounter selected");
    }

    // For Death Log mode, pre-compute death windows
    let death_windows: Vec<(f64, f64, &str)> = if mode == EventLogMode::DeathLog {
        build_death_windows(log_data, &encounters)
    } else {
        Vec::new()
    };

    let mut col = Column::new().spacing(1);

    // Header
    let header_text = match mode {
        EventLogMode::AllEvents => "Event Log — click chart to jump",
        EventLogMode::KeyEvents => "Key Events — deaths, big hits, dispels, interrupts",
        EventLogMode::DeathLog => "Death Log — 10s before each death",
    };
    col = col.push(
        row![
            text("Time").size(10).color(theme::TEXT_MUTED).width(50),
            text(header_text).size(11).color(theme::TEXT_SECONDARY),
        ]
        .spacing(8),
    );
    col = col.push(horizontal_rule(1));

    let mut offset_base: f64 = 0.0;
    let mut row_count: usize = 0;
    let max_rows = 5000;

    // For Death Log mode, insert separator headers before each death window
    let mut last_death_label: Option<String> = None;

    for enc in &encounters {
        let enc_start = enc.start;
        let enc_duration = enc.duration;

        for entry in &log_data.entries {
            let ts = entry.timestamp();
            if ts < enc_start || ts > enc.end {
                continue;
            }

            let relative = ts - enc_start + offset_base;
            let second = relative.floor() as usize;

            // ── Type filter ────────────────────────────────────────────
            if !type_filter.accepts(entry) {
                continue;
            }

            // ── Player filter ──────────────────────────────────────────
            if let Some(player) = player_filter {
                let involves_player =
                    entry.source() == player || entry.target().is_some_and(|t| t == player);
                if !involves_player {
                    continue;
                }
            }

            // ── Mode-specific filter ───────────────────────────────────
            match mode {
                EventLogMode::AllEvents => {}
                EventLogMode::KeyEvents => {
                    if !is_key_event(entry, &log_data.combatants) {
                        continue;
                    }
                }
                EventLogMode::DeathLog => {
                    // Check if this event falls in any death window and involves
                    // the player who is about to die
                    let in_window = death_windows.iter().find(|(start, end, player)| {
                        ts >= *start
                            && ts <= *end
                            && (entry.source() == *player
                                || entry.target().is_some_and(|t| t == *player))
                    });
                    if let Some((_, end_ts, dead_player)) = in_window {
                        // Insert a death header separator when entering a new window
                        let death_label = format!(
                            "{} — died at {}",
                            dead_player,
                            format_encounter_time(*end_ts - enc_start + offset_base)
                        );
                        if last_death_label.as_deref() != Some(&death_label) {
                            if last_death_label.is_some() {
                                col = col.push(container("").height(6));
                            }
                            col = col.push(
                                container(
                                    text(death_label.clone())
                                        .size(11)
                                        .color(theme::TIMELINE_DEATH),
                                )
                                .padding([4, 8])
                                .width(Fill)
                                .style(|_theme: &iced::Theme| container::Style {
                                    background: Some(iced::Background::Color(Color::from_rgba8(
                                        255, 50, 50, 0.08,
                                    ))),
                                    border: iced::Border {
                                        radius: 3.0.into(),
                                        ..Default::default()
                                    },
                                    ..Default::default()
                                }),
                            );
                            last_death_label = Some(death_label);
                        }
                    } else {
                        continue;
                    }
                }
            }

            // ── Render row ─────────────────────────────────────────────
            let (color, label) = format_log_entry(entry);

            let is_highlighted = clicked_second.is_some_and(|cs| second == cs);
            let bg_alpha: f32 = if is_highlighted { 0.15 } else { 0.0 };

            let time_str = format_encounter_time(relative);

            let event_row: Element<ViewerMessage> = container(
                row![
                    text(time_str).size(10).color(theme::TEXT_MUTED).width(50),
                    text(label).size(10).color(color),
                ]
                .spacing(6),
            )
            .padding([1, 4])
            .width(Fill)
            .style(move |_theme: &iced::Theme| container::Style {
                background: if bg_alpha > 0.0 {
                    Some(iced::Background::Color(Color::from_rgba8(
                        100, 180, 255, bg_alpha,
                    )))
                } else {
                    None
                },
                ..Default::default()
            })
            .into();

            col = col.push(event_row);
            row_count += 1;
            if row_count >= max_rows {
                col = col.push(
                    text(format!("... truncated at {max_rows} events"))
                        .size(10)
                        .color(theme::TEXT_MUTED),
                );
                return col.width(Fill).into();
            }
        }

        offset_base += enc_duration;
    }

    if row_count == 0 {
        col = col.push(empty_state("No events matching filters"));
    }

    col.width(Fill).into()
}

/// Format a `LogEntry` into a color + label string for display.
fn format_log_entry(entry: &LogEntry) -> (Color, String) {
    match entry {
        LogEntry::Damage {
            source,
            target,
            spell,
            amount,
            is_crit,
            ..
        } => {
            let crit = if *is_crit { " (crit)" } else { "" };
            (
                Color::from_rgb8(200, 200, 200),
                format!(
                    "{source}'s {spell} hits {target} for {}{crit}",
                    theme::format_number(*amount)
                ),
            )
        }
        LogEntry::Healing {
            source,
            target,
            spell,
            amount,
            is_crit,
            ..
        } => {
            let crit = if *is_crit { " (crit)" } else { "" };
            (
                theme::TIMELINE_HPS,
                format!(
                    "{source}'s {spell} heals {target} for {}{crit}",
                    theme::format_number(*amount)
                ),
            )
        }
        LogEntry::Death { player, .. } => (theme::TIMELINE_DEATH, format!("{player} died")),
        LogEntry::Dispel {
            caster,
            target,
            spell,
            ..
        } => (
            theme::TIMELINE_DISPEL,
            format!("{caster} dispels {spell} on {target}"),
        ),
        LogEntry::Resurrect { caster, target, .. } => (
            theme::TIMELINE_RESURRECT,
            format!("{caster} resurrects {target}"),
        ),
        LogEntry::Interrupt {
            caster,
            target,
            spell,
            ..
        } => (
            theme::TIMELINE_INTERRUPT,
            format!("{caster} interrupts {target} with {spell}"),
        ),
    }
}

/// Check if a `LogEntry` qualifies as a "key event" for the Key Events filter.
fn is_key_event(
    entry: &LogEntry,
    combatants: &HashMap<String, crate::log_data::Combatant>,
) -> bool {
    match entry {
        LogEntry::Death { .. }
        | LogEntry::Dispel { .. }
        | LogEntry::Resurrect { .. }
        | LogEntry::Interrupt { .. } => true,
        LogEntry::Damage { target, amount, .. } => {
            // Big hits on raid members
            *amount >= KEY_EVENT_BIG_HIT && combatants.contains_key(target.as_str())
        }
        LogEntry::Healing { .. } => false,
    }
}

/// Build death windows for Death Log mode.
///
/// For each player death within the selected encounters, creates a window
/// of `(start_ts, death_ts, player_name)` covering the 10 seconds before death.
fn build_death_windows<'a>(
    log_data: &'a LogData,
    encounters: &[&Encounter],
) -> Vec<(f64, f64, &'a str)> {
    let mut windows = Vec::new();
    for enc in encounters {
        for entry in &log_data.entries {
            if let LogEntry::Death { timestamp, player } = entry {
                if *timestamp >= enc.start
                    && *timestamp <= enc.end
                    && log_data.combatants.contains_key(player.as_str())
                {
                    let window_start = (timestamp - DEATH_LOG_WINDOW).max(enc.start);
                    windows.push((window_start, *timestamp, player.as_str()));
                }
            }
        }
    }
    windows
}

/// Create an empty-state text element.
fn empty_state(msg: &str) -> Element<'_, ViewerMessage> {
    container(text(msg).size(13).color(theme::TEXT_MUTED))
        .padding(30)
        .center_x(Fill)
        .into()
}
