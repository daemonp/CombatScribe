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

mod charts;
mod components;
mod detail;
mod events;
mod loot;
mod meters;
mod timeline;

use iced::widget::{
    Column, Row, Space, button, canvas, column, container, float, image, pick_list, row, rule,
    scrollable, text, text_input, tooltip,
};
use iced::{Center, Color, Element, Fill, Length};
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

use std::sync::LazyLock;

use crate::log_data;
use crate::log_data::{
    AbilityStats, AvoidanceStats, BuffStats, DeathEvent, DeathLogWindow, EncounterFilter,
    EventLogMode, EventLogTypeFilter, EventLogTypeKind, LogData, LogEntry, LootEvent,
    PlayerEventType, ResurrectEvent, TimelineData, TimelineSeriesKind, TimelineVisibility,
};
use crate::theme;
use components::build_timeline_event_log_text;
pub use components::transparent_button_style;
#[allow(clippy::wildcard_imports)]
// viewer UI — many shared component functions used throughout
use components::*;

/// Scrollable ID for the timeline event log panel.
static TIMELINE_LOG_ID: LazyLock<iced::widget::Id> =
    LazyLock::new(|| iced::widget::Id::new("timeline_events"));

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

    // Session switching (populated by App after construction)
    pub session_names: Vec<String>,
    pub selected_session_name: Option<String>,

    // Damage/Healing tab
    pub damage_type: DamageType,
    pub healing_type: HealingType,

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
    /// Lookback window for Death Log mode.
    pub death_log_window: DeathLogWindow,
    /// Event type toggles for the log panel.
    pub event_log_types: EventLogTypeFilter,
    /// Player filter for the event log (empty string = all players).
    pub event_log_player: String,
    /// Canvas geometry caches — cleared when data or visibility changes.
    pub timeline_cache: canvas::Cache,
    pub alive_cache: canvas::Cache,
    pub aura_cache: canvas::Cache,
    pub dispel_cache: canvas::Cache,

    // Aura overlay on the timeline chart
    /// Which aura names the user has checked for display.
    pub tracked_auras: HashSet<String>,
    /// Whether the aura picker dropdown is open.
    pub aura_picker_open: bool,
    /// Search text for filtering the aura picker list.
    pub aura_search: String,
    /// Hovered second offset on the aura chart (for tooltip).
    pub aura_hover_second: Option<f64>,

    // Timeline zoom (click-drag to select a time range)
    /// Drag start second (set on mouse-down, cleared on release).
    pub zoom_drag_start: Option<f64>,
    /// Current drag end second (updated on mouse-move while dragging).
    pub zoom_drag_end: Option<f64>,
    /// Committed zoom range — `Some((start, end))` when zoomed in.
    pub zoom_range: Option<(f64, f64)>,

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
pub enum HealingType {
    Effective,
    Raw,
    Overhealing,
}

impl std::fmt::Display for HealingType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Effective => write!(f, "Effective healing"),
            Self::Raw => write!(f, "Raw healing"),
            Self::Overhealing => write!(f, "Overhealing"),
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
    DamageTaken,
    Healing,
    Dispels,
    Interrupts,
    Resurrects,
    Avoidance,
    Buffs,
    Consumables,
    Deaths,
}

// ── Messages ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum ViewerMessage {
    SwitchTab(ViewerTab),
    SelectEncounter(String),
    SetDamageType(DamageType),
    SetHealingType(HealingType),
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
    SetDeathLogWindow(DeathLogWindow),
    ToggleEventLogType(EventLogTypeKind),
    SetEventLogPlayer(String),
    /// Copy the current event log contents to the system clipboard.
    CopyEventLog,
    /// Toggle an aura name on/off for display on the `AuraChart`.
    ToggleAura(String),
    /// Open/close the aura picker dropdown.
    ToggleAuraPicker,
    /// Update the aura picker search text.
    SetAuraSearch(String),
    /// Hover on the aura chart — stores the hovered second offset.
    AuraHover(Option<f64>),
    /// Apply an aura preset (adds all auras from the preset to tracked set).
    ApplyAuraPreset(usize),
    /// Clear all tracked auras.
    ClearAuras,
    /// Begin a click-drag zoom selection at the given second.
    ZoomDragStart(f64),
    /// Update the drag endpoint as the cursor moves.
    ZoomDragUpdate(f64),
    /// Commit the zoom range on mouse release.
    ZoomDragEnd(f64),
    /// Reset zoom back to the full encounter.
    ZoomReset,
    /// Request to load a new file (handled by App).
    LoadFile,
    /// Request to open the export modal (handled by App).
    ShowExport,
    /// Request to switch to a different session (handled by App).
    SwitchSession(String),
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
            session_names: Vec::new(),
            selected_session_name: None,
            damage_type: DamageType::Damage,
            healing_type: HealingType::Effective,
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
            death_log_window: DeathLogWindow::default(),
            event_log_types: EventLogTypeFilter::default(),
            event_log_player: String::new(),
            timeline_cache: canvas::Cache::default(),
            alive_cache: canvas::Cache::default(),
            aura_cache: canvas::Cache::default(),
            dispel_cache: canvas::Cache::default(),
            tracked_auras: HashSet::new(),
            aura_picker_open: false,
            aura_search: String::new(),
            aura_hover_second: None,
            zoom_drag_start: None,
            zoom_drag_end: None,
            zoom_range: None,
        }
    }

    /// Clear all chart canvas geometry caches.
    ///
    /// Called when the underlying data or visible range changes (encounter
    /// selection, zoom, etc.) so that the canvas programs redraw.
    fn clear_all_chart_caches(&mut self) {
        self.timeline_cache.clear();
        self.alive_cache.clear();
        self.aura_cache.clear();
        self.dispel_cache.clear();
    }

    // ── Update ──────────────────────────────────────────────────────────

    #[allow(clippy::too_many_lines)] // iced message dispatch — one arm per variant
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
                self.zoom_range = None;
                self.zoom_drag_start = None;
                self.zoom_drag_end = None;
                self.clear_all_chart_caches();
            }
            ViewerMessage::SetDamageType(dt) => self.damage_type = dt,
            ViewerMessage::SetHealingType(ht) => self.healing_type = ht,
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
                self.dispel_cache.clear();
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
            ViewerMessage::SetDeathLogWindow(w) => self.death_log_window = w,
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
            ViewerMessage::CopyEventLog => {
                let player_filter = if self.event_log_player.is_empty() {
                    None
                } else {
                    Some(self.event_log_player.as_str())
                };
                let text = build_timeline_event_log_text(
                    &self.log_data,
                    &self.encounter_filter,
                    &self.event_log_types,
                    self.event_log_mode,
                    player_filter,
                    self.death_log_window.as_secs(),
                );
                match arboard::Clipboard::new() {
                    Ok(mut clipboard) => {
                        if let Err(e) = clipboard.set_text(text) {
                            eprintln!("Failed to set clipboard text: {e}");
                        }
                    }
                    Err(e) => eprintln!("Failed to open clipboard: {e}"),
                }
            }
            ViewerMessage::TimelineClick(second) => {
                self.timeline_clicked_second = Some(second);
                // Snap the event log scrollable to the proportional position
                // corresponding to the clicked second within the encounter.
                let duration = self.timeline_data.duration;
                if duration > 0.0 {
                    let proportion = second as f32 / duration as f32;
                    return iced::widget::operation::snap_to(
                        TIMELINE_LOG_ID.clone(),
                        scrollable::RelativeOffset {
                            x: 0.0,
                            y: proportion.clamp(0.0, 1.0),
                        },
                    );
                }
            }
            ViewerMessage::ToggleAura(name) => {
                if self.tracked_auras.contains(&name) {
                    self.tracked_auras.remove(&name);
                } else {
                    self.tracked_auras.insert(name);
                }
                self.aura_cache.clear();
            }
            ViewerMessage::ToggleAuraPicker => {
                self.aura_picker_open = !self.aura_picker_open;
                if !self.aura_picker_open {
                    self.aura_search.clear();
                }
            }
            ViewerMessage::SetAuraSearch(s) => self.aura_search = s,
            ViewerMessage::AuraHover(second) => self.aura_hover_second = second,
            ViewerMessage::ApplyAuraPreset(idx) => {
                if let Some(preset) = log_data::AURA_PRESETS.get(idx) {
                    for &aura_name in preset.auras {
                        self.tracked_auras.insert(aura_name.to_string());
                    }
                    self.aura_cache.clear();
                }
            }
            ViewerMessage::ClearAuras => {
                self.tracked_auras.clear();
                self.aura_cache.clear();
            }
            ViewerMessage::ZoomDragStart(second) => {
                self.zoom_drag_start = Some(second);
                self.zoom_drag_end = Some(second);
            }
            ViewerMessage::ZoomDragUpdate(second) => {
                if self.zoom_drag_start.is_some() {
                    self.zoom_drag_end = Some(second);
                }
            }
            ViewerMessage::ZoomDragEnd(second) => {
                if let Some(start) = self.zoom_drag_start {
                    let end = second;
                    let lo = start.min(end);
                    let hi = start.max(end);
                    // Only commit if the selection is at least 2 seconds wide
                    if hi - lo >= 2.0 {
                        self.zoom_range = Some((lo, hi));
                        self.clear_all_chart_caches();
                    }
                }
                self.zoom_drag_start = None;
                self.zoom_drag_end = None;
            }
            ViewerMessage::ZoomReset => {
                self.zoom_range = None;
                self.zoom_drag_start = None;
                self.zoom_drag_end = None;
                self.clear_all_chart_caches();
            }
            // These are intercepted and handled by App::update() in main.rs
            ViewerMessage::LoadFile
            | ViewerMessage::ShowExport
            | ViewerMessage::SwitchSession(_)
            | ViewerMessage::Quit => {}
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
        // ── Left side: action buttons + session dropdown ────────────
        let load_btn = button(text("Load File").size(12).color(Color::WHITE))
            .on_press(ViewerMessage::LoadFile)
            .padding([6, 16])
            .style(header_action_button_style);

        let export_btn = button(text("Export").size(12).color(Color::WHITE))
            .on_press(ViewerMessage::ShowExport)
            .padding([6, 16])
            .style(header_action_button_style);

        let mut left_row = Row::new().spacing(6).align_y(Center);
        left_row = left_row.push(load_btn);
        left_row = left_row.push(export_btn);

        // Session dropdown (only if multiple sessions)
        if self.session_names.len() > 1 {
            left_row = left_row.push(
                pick_list(
                    self.session_names.clone(),
                    self.selected_session_name.clone(),
                    ViewerMessage::SwitchSession,
                )
                .padding(4)
                .text_size(12),
            );
        }

        // ── Right side: zone info + quit ────────────────────────────
        let instance_zone = self
            .log_data
            .encounters
            .iter()
            .filter(|e| e.is_boss)
            .find_map(|e| e.zone.as_deref());
        let zone_raw = instance_zone
            .or_else(|| {
                let z = self.log_data.zone_name.as_str();
                if z.is_empty() { None } else { Some(z) }
            })
            .unwrap_or("Combat Log");
        let zone = crate::parser::format_zone_name(zone_raw);

        let mut right_row = Row::new().spacing(8).align_y(Center);
        right_row = right_row.push(text(zone).size(20).color(Color::WHITE));

        // Raid size badge
        if let Some((_, total)) = self.log_data.raid_size {
            let player_count = self.log_data.combatants.len();
            right_row = right_row.push(
                text(format!("{player_count}/{total} players"))
                    .size(12)
                    .color(theme::TEXT_MUTED),
            );
        } else {
            let player_count = self.log_data.combatants.len();
            if player_count > 0 {
                right_row = right_row.push(
                    text(format!("{player_count} players"))
                        .size(12)
                        .color(theme::TEXT_MUTED),
                );
            }
        }

        // Boss kill count
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
            right_row = right_row.push(
                text(format!("{boss_kills}/{total_bosses} bosses"))
                    .size(12)
                    .color(theme::TEXT_MUTED),
            );
        }

        right_row = right_row.push(
            button(text("Quit").size(11).color(theme::TEXT_SECONDARY))
                .on_press(ViewerMessage::Quit)
                .style(transparent_button_style)
                .padding([5, 14]),
        );

        // ── Assemble header row ─────────────────────────────────────
        row![left_row, Space::new().width(Fill), right_row,]
            .spacing(12)
            .align_y(Center)
            .width(Fill)
            .into()
    }

    // ── Controls ────────────────────────────────────────────────────────

    fn view_controls(&self) -> Element<'_, ViewerMessage> {
        row![
            pick_list(
                self.encounter_names.clone(),
                self.selected_encounter_name.clone(),
                ViewerMessage::SelectEncounter,
            )
            .width(Length::FillPortion(2))
            .padding(6),
        ]
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
                        snap: true,
                    }
                });

            tab_row = tab_row.push(btn);
        }

        tab_row.width(Fill).into()
    }
}
