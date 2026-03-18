//! Shared UI components: meter bars, encounter name badges, event log builders, and styles.

// ── Shared Helpers & Free Functions ──────────────────────────────────────────

use std::collections::HashMap;

use iced::widget::{Column, Row, Space, button, container, image, row, rule, stack, text};
use iced::{Center, Color, Element, Fill, Length};

use crate::log_data::{
    Combatant, Encounter, EncounterFilter, EventLogMode, EventLogTypeFilter, LogData, LogEntry,
    TimelineSeriesKind,
};
use crate::theme;

use super::{DetailType, ViewerMessage};

// ── Shared Helpers ──────────────────────────────────────────────────────────

/// Shared helper: build meter bars from pre-aggregated players and return total label.
///
/// Converts borrowed `&str` slices to owned data so the result doesn't borrow
/// the caller's local filtered-events vector.
pub(super) fn view_event_meters_owned<'a>(
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
pub(super) fn per_second(value: u64, duration: f64) -> f64 {
    if duration > 0.0 {
        value as f64 / duration
    } else {
        0.0
    }
}

/// Percentage of total, guarding against zero total.
pub(super) fn percent_of(value: u64, total: u64) -> f64 {
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
pub(super) fn build_simple_meters<'a>(
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
pub(super) fn aggregate_by_player<'a, T>(
    events: &'a [&T],
    get_player: impl Fn(&'a &T) -> &'a str,
    combatants: &'a HashMap<String, Combatant>,
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
    players.sort_by_key(|p| std::cmp::Reverse(p.2));
    players
}

// ── Free Functions ──────────────────────────────────────────────────────────

/// Visual separator in the encounter `pick_list` between quick filters and bosses.
pub(super) const ENCOUNTER_SEPARATOR: &str = "-------------------";

/// Build the encounter names list for the `pick_list`.
///
/// Single pass over encounters to collect counts and durations.
pub(super) fn build_encounter_names(data: &LogData) -> Vec<String> {
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
pub(super) fn parse_encounter_filter(name: &str, data: &LogData) -> EncounterFilter {
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
pub(super) fn format_timestamp(ts: f64) -> String {
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
pub(super) fn build_meter_row<'a>(
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
        Space::new().width(Fill),
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
        Space::new().width(0).into()
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

/// Header action button style (subtle background with hover).
pub(super) fn header_action_button_style(
    _theme: &iced::Theme,
    status: button::Status,
) -> button::Style {
    let bg = match status {
        button::Status::Hovered => Color::from_rgba(1.0, 1.0, 1.0, 0.10),
        _ => Color::from_rgba(1.0, 1.0, 1.0, 0.05),
    };
    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: Color::WHITE,
        border: iced::Border {
            color: Color::from_rgba(1.0, 1.0, 1.0, 0.08),
            width: 1.0,
            radius: 4.0.into(),
        },
        shadow: iced::Shadow::default(),
        snap: true,
    }
}

/// Transparent button style with subtle hover highlight.
pub fn transparent_button_style(_theme: &iced::Theme, status: button::Status) -> button::Style {
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
        snap: true,
    }
}

/// Create the colored meter bar element.
///
/// Uses a `Stack` to layer a partial-width colored bar behind the full-width
/// text content, producing the classic DPS-meter look.
pub(super) fn make_bar(
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
pub(super) fn panel_style(_theme: &iced::Theme) -> container::Style {
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
pub(super) const BIG_HIT_THRESHOLD: u64 = 2000;

/// Format seconds offset as encounter time (M:SS).
pub(super) fn format_encounter_time(seconds: f64) -> String {
    let total = seconds as u64;
    let mins = total / 60;
    let secs = total % 60;
    format!("{mins}:{secs:02}")
}

/// Build a clickable legend toggle button.
///
/// Active: full-color dot + white text. Inactive: dimmed dot + muted text.
pub(super) fn legend_toggle(
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
                snap: true,
            }
        })
        .into()
}

// ── Event Log Functions ─────────────────────────────────────────────────────

/// Big hit threshold for Key Events mode.
const KEY_EVENT_BIG_HIT: u64 = 2000;

/// Build the encounter event log for the timeline tab.
///
/// Supports three modes:
/// - `AllEvents`: every entry matching type filters
/// - `KeyEvents`: deaths, big hits, dispels, interrupts, resurrects only
/// - `DeathLog`: for each player death, all events involving that player
///   in the configurable window before death
#[allow(clippy::too_many_lines)] // Event log builder — single pass with mode dispatch
#[allow(clippy::too_many_arguments)] // Will be refactored into a struct in future
pub(super) fn build_timeline_event_log<'a>(
    log_data: &'a LogData,
    filter: &EncounterFilter,
    type_filter: &EventLogTypeFilter,
    mode: EventLogMode,
    player_filter: Option<&str>,
    clicked_second: Option<usize>,
    death_window_secs: f64,
) -> Element<'a, ViewerMessage> {
    let encounters = log_data.selected_encounters(filter);
    if encounters.is_empty() {
        return empty_state("No encounter selected");
    }

    // For Death Log mode, pre-compute death windows
    let death_windows: Vec<(f64, f64, &str)> = if mode == EventLogMode::DeathLog {
        build_death_windows(log_data, &encounters, death_window_secs)
    } else {
        Vec::new()
    };

    let mut col = Column::new().spacing(1);

    // Header
    #[allow(clippy::cast_possible_truncation)] // window seconds always ≤ 30
    let header_text = match mode {
        EventLogMode::AllEvents => "Event Log — click chart to jump".to_string(),
        EventLogMode::KeyEvents => "Key Events — deaths, big hits, dispels, interrupts".to_string(),
        EventLogMode::DeathLog => {
            format!(
                "Death Log — {}s before each death",
                death_window_secs as u32
            )
        }
    };
    col = col.push(
        row![
            text("Time").size(10).color(theme::TEXT_MUTED).width(50),
            text(header_text).size(11).color(theme::TEXT_SECONDARY),
        ]
        .spacing(8),
    );
    col = col.push(rule::horizontal(1));

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
            // In DeathLog mode, track the death timestamp for time-to-death display
            let mut current_death_ts: Option<f64> = None;
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
                        current_death_ts = Some(*end_ts);
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

            // In Death Log mode, show time relative to death (e.g. "-8.2s", "0.0s")
            let time_str = if let Some(death_ts) = current_death_ts {
                let delta = ts - death_ts;
                if delta.abs() < 0.05 {
                    " 0.0s".to_string()
                } else {
                    format!("{delta:+.1}s")
                }
            } else {
                format_encounter_time(relative)
            };

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

/// Build the event log as plain text for clipboard copy.
///
/// Mirrors the filtering logic of `build_timeline_event_log` but produces a
/// newline-separated string instead of UI elements.
pub(super) fn build_timeline_event_log_text(
    log_data: &LogData,
    filter: &EncounterFilter,
    type_filter: &EventLogTypeFilter,
    mode: EventLogMode,
    player_filter: Option<&str>,
    death_window_secs: f64,
) -> String {
    let encounters = log_data.selected_encounters(filter);
    if encounters.is_empty() {
        return String::new();
    }

    let death_windows: Vec<(f64, f64, &str)> = if mode == EventLogMode::DeathLog {
        build_death_windows(log_data, &encounters, death_window_secs)
    } else {
        Vec::new()
    };

    let mut lines: Vec<String> = Vec::new();
    let mut offset_base: f64 = 0.0;
    let mut row_count: usize = 0;
    let max_rows = 5000;
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

            if !type_filter.accepts(entry) {
                continue;
            }

            if let Some(player) = player_filter {
                let involves_player =
                    entry.source() == player || entry.target().is_some_and(|t| t == player);
                if !involves_player {
                    continue;
                }
            }

            let mut current_death_ts: Option<f64> = None;
            match mode {
                EventLogMode::AllEvents => {}
                EventLogMode::KeyEvents => {
                    if !is_key_event(entry, &log_data.combatants) {
                        continue;
                    }
                }
                EventLogMode::DeathLog => {
                    let in_window = death_windows.iter().find(|(start, end, player)| {
                        ts >= *start
                            && ts <= *end
                            && (entry.source() == *player
                                || entry.target().is_some_and(|t| t == *player))
                    });
                    if let Some((_, end_ts, dead_player)) = in_window {
                        current_death_ts = Some(*end_ts);
                        let death_label = format!(
                            "{} — died at {}",
                            dead_player,
                            format_encounter_time(*end_ts - enc_start + offset_base)
                        );
                        if last_death_label.as_deref() != Some(&death_label) {
                            if last_death_label.is_some() {
                                lines.push(String::new());
                            }
                            lines.push(format!("--- {death_label} ---"));
                            last_death_label = Some(death_label);
                        }
                    } else {
                        continue;
                    }
                }
            }

            // In Death Log mode, show time relative to death (e.g. "-8.2s", "0.0s")
            let time_str = if let Some(death_ts) = current_death_ts {
                let delta = ts - death_ts;
                if delta.abs() < 0.05 {
                    " 0.0s".to_string()
                } else {
                    format!("{delta:+.1}s")
                }
            } else {
                format_encounter_time(relative)
            };
            let (_color, label) = format_log_entry(entry);
            lines.push(format!("{time_str}  {label}"));
            row_count += 1;
            if row_count >= max_rows {
                lines.push(format!("... truncated at {max_rows} events"));
                break;
            }
        }

        offset_base += enc_duration;
    }

    lines.join("\n")
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
            effective_heal,
            overheal,
            is_crit,
            ..
        } => {
            let crit = if *is_crit { " (crit)" } else { "" };
            let oh = if *overheal > 0 {
                format!(" ({} OH)", theme::format_number(*overheal))
            } else {
                String::new()
            };
            (
                theme::TIMELINE_HPS,
                format!(
                    "{source}'s {spell} heals {target} for {}{crit}{oh}",
                    theme::format_number(*effective_heal)
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
        LogEntry::AuraGain {
            player,
            aura,
            stacks,
            ..
        } => (
            Color::from_rgb8(180, 140, 255),
            format!("{player} gains {aura} ({stacks})"),
        ),
        LogEntry::AuraFade { player, aura, .. } => (
            Color::from_rgb8(140, 100, 200),
            format!("{aura} fades from {player}"),
        ),
    }
}

/// Check if a `LogEntry` qualifies as a "key event" for the Key Events filter.
fn is_key_event(entry: &LogEntry, combatants: &HashMap<String, Combatant>) -> bool {
    match entry {
        LogEntry::Death { .. }
        | LogEntry::Dispel { .. }
        | LogEntry::Resurrect { .. }
        | LogEntry::Interrupt { .. } => true,
        LogEntry::Damage { target, amount, .. } => {
            // Big hits on raid members
            *amount >= KEY_EVENT_BIG_HIT && combatants.contains_key(target.as_str())
        }
        LogEntry::Healing { .. } | LogEntry::AuraGain { .. } | LogEntry::AuraFade { .. } => false,
    }
}

/// Build death windows for Death Log mode.
///
/// For each player death within the selected encounters, creates a window
/// of `(start_ts, death_ts, player_name)` covering `window_secs` before death.
fn build_death_windows<'a>(
    log_data: &'a LogData,
    encounters: &[&Encounter],
    window_secs: f64,
) -> Vec<(f64, f64, &'a str)> {
    let mut windows = Vec::new();
    for enc in encounters {
        for entry in &log_data.entries {
            if let LogEntry::Death { timestamp, player } = entry
                && *timestamp >= enc.start
                && *timestamp <= enc.end
                && log_data.combatants.contains_key(player.as_str())
            {
                let window_start = (timestamp - window_secs).max(enc.start);
                windows.push((window_start, *timestamp, player.as_str()));
            }
        }
    }
    windows
}

/// Create an empty-state text element.
pub(super) fn empty_state(msg: &str) -> Element<'_, ViewerMessage> {
    container(text(msg).size(13).color(theme::TEXT_MUTED))
        .padding(30)
        .center_x(Fill)
        .into()
}
