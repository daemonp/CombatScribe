//! Canvas drawing programs: `TimelineChart`, `AliveChart`, `DispelChart`, and `AuraChart`.

use std::collections::{HashMap, HashSet};

use iced::widget::Action;
use iced::widget::canvas;
use iced::{Color, Event, Point, Rectangle, Renderer, Theme, mouse};

use crate::log_data::{
    Combatant, TimelineBucket, TimelineData, TimelineEventKind, TimelineVisibility,
};
use crate::theme;

use super::ViewerMessage;
use super::components::format_encounter_time;

// ── Timeline Canvas Programs ────────────────────────────────────────────────

/// Canvas program that renders the main DPS/DTPS/HPS sparkline chart.
///
/// Each enabled series is drawn as a filled area with a solid stroke on top.
/// Death and big-hit markers are drawn as vertical lines at their X positions.
pub(super) struct TimelineChart<'a> {
    pub data: &'a TimelineData,
    pub visibility: &'a TimelineVisibility,
    pub shared_y: bool,
    pub hover_idx: Option<usize>,
    /// Active drag selection `(start_second, end_second)` for highlight overlay.
    pub drag: Option<(f64, f64)>,
    /// Committed zoom range `(start_second, end_second)`.
    pub zoom: Option<(f64, f64)>,
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

        let chart_w = (w - CHART_LEFT_MARGIN - CHART_RIGHT_MARGIN).max(1.0);

        // Zoom: if a zoom range is active, scale to show only that window.
        let (view_lo, view_hi) = self.zoom.map_or((0.0_f32, td.duration as f32), |(lo, hi)| {
            (lo as f32, hi as f32)
        });
        let view_span = (view_hi - view_lo).max(0.001);
        let x_scale = chart_w / view_span;
        let x_offset = CHART_LEFT_MARGIN - view_lo * x_scale;

        // Compute Y-axis maximums
        let shared_max = td
            .max_dps
            .max(td.max_dtps)
            .max(td.max_hps)
            .max(td.max_boss_hps)
            .max(1) as f32;
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
        let boss_hps_max = if self.shared_y {
            shared_max
        } else {
            td.max_boss_hps.max(1) as f32
        };

        // Draw series in back-to-front order: DPS, Boss Heals, HPS, DTPS (front).
        // Each call is explicit to avoid a complex generic tuple array.
        if vis.show_dps {
            draw_sparkline_area(
                &mut frame,
                &td.buckets,
                &|b| b.damage,
                x_scale,
                x_offset,
                h,
                dps_max,
                theme::TIMELINE_DPS,
            );
        }
        if vis.show_boss_heals {
            draw_sparkline_area(
                &mut frame,
                &td.buckets,
                &|b| b.boss_healing,
                x_scale,
                x_offset,
                h,
                boss_hps_max,
                theme::TIMELINE_BOSS_HEAL,
            );
        }
        if vis.show_hps {
            draw_sparkline_area(
                &mut frame,
                &td.buckets,
                &|b| b.healing,
                x_scale,
                x_offset,
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
                x_offset,
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
            let x = x_offset + event.offset as f32 * x_scale;
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
            let x = x_offset + idx as f32 * x_scale;
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
            if vis.show_boss_heals {
                m = m.max(td.max_boss_hps);
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
        let view_duration = f64::from(view_span);
        let label_interval = if view_duration > 300.0 {
            60.0
        } else if view_duration > 120.0 {
            30.0
        } else if view_duration > 30.0 {
            15.0
        } else {
            5.0
        };

        // Start labels from the first whole interval at or after view_lo
        let t_start = (f64::from(view_lo) / label_interval).ceil() * label_interval;
        let mut t = t_start;
        while t < f64::from(view_hi) {
            let x = x_offset + t as f32 * x_scale;
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

        // ── Drag selection highlight ────────────────────────────────────
        if let Some((drag_lo, drag_hi)) = self.drag {
            let x_lo = x_offset + drag_lo as f32 * x_scale;
            let x_hi = x_offset + drag_hi as f32 * x_scale;
            let sel_x = x_lo.max(CHART_LEFT_MARGIN);
            let sel_w = (x_hi - x_lo).abs().min(chart_w);
            frame.fill_rectangle(
                Point::new(sel_x, 0.0),
                iced::Size::new(sel_w, h),
                Color::from_rgba8(100, 180, 255, 0.15),
            );
            // Selection border lines
            for &edge_x in &[x_lo, x_hi] {
                if edge_x >= CHART_LEFT_MARGIN && edge_x <= CHART_LEFT_MARGIN + chart_w {
                    let edge = canvas::Path::line(Point::new(edge_x, 0.0), Point::new(edge_x, h));
                    frame.stroke(
                        &edge,
                        canvas::Stroke::default()
                            .with_color(Color::from_rgba8(100, 180, 255, 0.7))
                            .with_width(1.0),
                    );
                }
            }
        }

        vec![frame.into_geometry()]
    }

    fn update(
        &self,
        _state: &mut (),
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<Action<ViewerMessage>> {
        let td = self.data;
        let chart_w = (bounds.width - CHART_LEFT_MARGIN - CHART_RIGHT_MARGIN).max(1.0);
        let (view_lo, view_hi) = self.zoom.map_or((0.0, td.duration), |(lo, hi)| (lo, hi));
        let view_span = (view_hi - view_lo).max(0.001);

        // Convert pixel x → seconds within the current view
        let px_to_second = |px: f32| -> f64 {
            let frac = f64::from((px - CHART_LEFT_MARGIN) / chart_w).clamp(0.0, 1.0);
            view_lo + frac * view_span
        };

        match event {
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    let second = px_to_second(pos.x);
                    let n = td.buckets.len();
                    if n > 0 {
                        let idx = (second.floor() as usize).min(n.saturating_sub(1));
                        // If dragging, update the drag endpoint
                        if self.drag.is_some() {
                            return Some(
                                Action::publish(ViewerMessage::ZoomDragUpdate(second))
                                    .and_capture(),
                            );
                        }
                        return Some(
                            Action::publish(ViewerMessage::TimelineHover(Some(idx))).and_capture(),
                        );
                    }
                } else {
                    return Some(Action::publish(ViewerMessage::TimelineHover(None)));
                }
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    let second = px_to_second(pos.x);
                    return Some(
                        Action::publish(ViewerMessage::ZoomDragStart(second)).and_capture(),
                    );
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    let second = px_to_second(pos.x);
                    return Some(Action::publish(ViewerMessage::ZoomDragEnd(second)).and_capture());
                }
            }
            _ => {}
        }
        None
    }
}

/// Left margin reserved for labels across all timeline charts.
const CHART_LEFT_MARGIN: f32 = 80.0;
/// Right margin reserved for count labels across all timeline charts.
const CHART_RIGHT_MARGIN: f32 = 30.0;

/// Draw a single filled sparkline area on the frame.
///
/// Builds a closed path from the bucket values, fills with semi-transparent
/// color, then strokes the top edge with a solid line. Data is drawn within
/// `[x_offset .. x_offset + chart_w]` to align with shared chart margins.
#[allow(clippy::too_many_arguments)] // Drawing helper — x_offset needed for chart margin alignment
fn draw_sparkline_area(
    frame: &mut canvas::Frame,
    buckets: &[TimelineBucket],
    get_val: &dyn Fn(&TimelineBucket) -> u64,
    x_scale: f32,
    x_offset: f32,
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
            let x = x_offset + i as f32 * x_scale;
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
        b.move_to(Point::new(x_offset, height));

        for (i, bucket) in buckets.iter().enumerate() {
            let x = x_offset + i as f32 * x_scale;
            let val = get_val(bucket) as f32;
            let y = height - (val / y_max) * height;
            b.line_to(Point::new(x, y));
        }

        // Close back to bottom-right
        let last_x = x_offset + (buckets.len() - 1) as f32 * x_scale;
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
pub(super) struct AliveChart<'a> {
    pub data: &'a TimelineData,
    pub zoom: Option<(f64, f64)>,
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
        let chart_w = (w - CHART_LEFT_MARGIN - CHART_RIGHT_MARGIN).max(1.0);
        let (view_lo, view_hi) = self
            .zoom
            .map_or((0.0_f32, n as f32), |(lo, hi)| (lo as f32, hi as f32));
        let view_span = (view_hi - view_lo).max(0.001);
        let x_scale = chart_w / view_span;
        let x_offset = CHART_LEFT_MARGIN - view_lo * x_scale;
        let y_max = td.raid_count.max(1) as f32;

        // Filled area
        let area_path = canvas::Path::new(|b| {
            b.move_to(Point::new(x_offset, h));
            for (i, bucket) in td.buckets.iter().enumerate() {
                let x = x_offset + i as f32 * x_scale;
                let y = h - (bucket.alive_count as f32 / y_max) * h;
                b.line_to(Point::new(x, y));
            }
            let last_x = x_offset + (n - 1) as f32 * x_scale;
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
                let x = x_offset + i as f32 * x_scale;
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

// ── Dispel Waterfall Chart ──────────────────────────────────────────────────

/// Height of each caster lane in the dispel waterfall.
pub(super) const DISPEL_LANE_HEIGHT: f32 = 18.0;

/// Dispel waterfall chart — one lane per caster with class-colored diamond marks
/// at each dispel timestamp.  Casters are pre-sorted by count descending so the
/// most active dispeller appears on top.
pub(super) struct DispelChart<'a> {
    pub data: &'a TimelineData,
    pub combatants: &'a HashMap<String, Combatant>,
    pub hover_second: Option<f64>,
    pub zoom: Option<(f64, f64)>,
}

/// Draw a small diamond marker on the canvas at (`cx`, `cy`) with the given
/// radius and color.  Used for dispel tick marks on the waterfall chart.
fn draw_diamond(frame: &mut canvas::Frame, cx: f32, cy: f32, r: f32, color: Color) {
    let path = canvas::Path::new(|b| {
        b.move_to(Point::new(cx, cy - r)); // top
        b.line_to(Point::new(cx + r, cy)); // right
        b.line_to(Point::new(cx, cy + r)); // bottom
        b.line_to(Point::new(cx - r, cy)); // left
        b.close();
    });
    frame.fill(&path, Color { a: 0.6, ..color });
    frame.stroke(
        &path,
        canvas::Stroke::default().with_color(color).with_width(1.0),
    );
}

impl canvas::Program<ViewerMessage> for DispelChart<'_> {
    type State = ();

    #[allow(clippy::many_single_char_names)] // x/y/w/h are standard 2D drawing variables
    #[allow(clippy::too_many_lines)] // Canvas draw — waterfall with per-caster lanes
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

        if td.duration <= 0.0 || w < 2.0 || h < 2.0 || td.dispel_casters.is_empty() {
            return vec![];
        }

        let mut frame = canvas::Frame::new(renderer, bounds.size());

        // Background
        frame.fill_rectangle(
            Point::ORIGIN,
            bounds.size(),
            Color::from_rgba8(0, 0, 0, 0.2),
        );

        let chart_w = (w - CHART_LEFT_MARGIN - CHART_RIGHT_MARGIN).max(1.0);
        let (view_lo, view_hi) = self.zoom.map_or((0.0, td.duration), |(lo, hi)| (lo, hi));
        let view_span = (view_hi - view_lo).max(0.001) as f32;

        for (lane_idx, caster) in td.dispel_casters.iter().enumerate() {
            let lane_y = lane_idx as f32 * DISPEL_LANE_HEIGHT;

            // Alternating row background for readability
            if lane_idx % 2 == 0 {
                frame.fill_rectangle(
                    Point::new(0.0, lane_y),
                    iced::Size::new(w, DISPEL_LANE_HEIGHT),
                    Color::from_rgba8(255, 255, 255, 0.02),
                );
            }

            // Look up class color for this caster
            let class_color = self
                .combatants
                .get(caster.as_str())
                .map_or(Color::from_rgb8(128, 128, 128), |c| {
                    theme::class_color(&c.class)
                });

            // Player name label on the left (class-colored)
            frame.fill_text(canvas::Text {
                content: caster.clone(),
                position: Point::new(4.0, lane_y + 2.0),
                color: Color {
                    a: 0.8,
                    ..class_color
                },
                size: 9.0.into(),
                ..canvas::Text::default()
            });

            // Count the marks for this caster and draw diamonds
            let mut count: usize = 0;
            let lane_center_y = lane_y + DISPEL_LANE_HEIGHT / 2.0;
            for mark in &td.dispel_marks {
                if mark.caster == *caster {
                    count += 1;
                    let x = CHART_LEFT_MARGIN
                        + ((mark.offset as f32 - view_lo as f32) / view_span) * chart_w;
                    draw_diamond(&mut frame, x, lane_center_y, 3.0, class_color);
                }
            }

            // Count label on the right edge
            frame.fill_text(canvas::Text {
                content: count.to_string(),
                position: Point::new(w - CHART_RIGHT_MARGIN + 6.0, lane_y + 2.0),
                color: Color {
                    a: 0.6,
                    ..class_color
                },
                size: 9.0.into(),
                ..canvas::Text::default()
            });
        }

        // Hover line (shared time cursor with aura chart)
        if let Some(second) = self.hover_second {
            let x = CHART_LEFT_MARGIN + ((second as f32 - view_lo as f32) / view_span) * chart_w;
            if x >= CHART_LEFT_MARGIN && x <= CHART_LEFT_MARGIN + chart_w {
                let line = canvas::Path::line(Point::new(x, 0.0), Point::new(x, h));
                frame.stroke(
                    &line,
                    canvas::Stroke::default()
                        .with_color(Color::from_rgba8(255, 255, 255, 0.6))
                        .with_width(1.0),
                );
            }
        }

        vec![frame.into_geometry()]
    }

    fn update(
        &self,
        _state: &mut (),
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<Action<ViewerMessage>> {
        let chart_w = (bounds.width - CHART_LEFT_MARGIN - CHART_RIGHT_MARGIN).max(1.0);
        let (view_lo, view_hi) = self
            .zoom
            .map_or((0.0, self.data.duration), |(lo, hi)| (lo, hi));
        let view_span = (view_hi - view_lo).max(0.001);
        match event {
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    if self.data.duration > 0.0 {
                        let second = view_lo
                            + f64::from((pos.x - CHART_LEFT_MARGIN) / chart_w).clamp(0.0, 1.0)
                                * view_span;
                        return Some(
                            Action::publish(ViewerMessage::AuraHover(Some(second))).and_capture(),
                        );
                    }
                } else {
                    return Some(Action::publish(ViewerMessage::AuraHover(None)));
                }
            }
            Event::Mouse(mouse::Event::CursorLeft) => {
                return Some(Action::publish(ViewerMessage::AuraHover(None)));
            }
            _ => {}
        }
        None
    }
}

// ── Aura Waterfall Chart ───────────────────────────────────────────────────

///
/// Waterfall layout: one lane per player per tracked aura, grouped under aura
/// name headers.  Each player row shows horizontal bars for every gain→fade
/// interval, colored per-aura.  Mouse hover reports the time offset for
/// tooltip rendering.
pub(super) struct AuraChart<'a> {
    pub data: &'a TimelineData,
    /// Pre-computed layout: (`aura_name`, color, players) in display order.
    pub layout: Vec<AuraLaneGroup<'a>>,
    pub hover_second: Option<f64>,
    pub zoom: Option<(f64, f64)>,
}

/// A group of player lanes for one aura.
pub(super) struct AuraLaneGroup<'a> {
    pub aura_name: &'a str,
    pub color: Color,
    /// Unique players (sorted) who have intervals for this aura.
    pub players: Vec<&'a str>,
}

/// Height of the aura name header row.
const AURA_HEADER_HEIGHT: f32 = 16.0;
/// Height of each player lane within an aura group.
const AURA_LANE_HEIGHT: f32 = 18.0;

/// Compute the waterfall layout for the currently tracked auras.
///
/// Returns the layout groups and total canvas height. Extracted as a free
/// function so both `view_timeline_tab` (for canvas height) and `AuraChart`
/// (for rendering) use the same layout.
pub(super) fn build_aura_layout<'a>(
    td: &'a TimelineData,
    tracked: &'a HashSet<String>,
) -> Vec<AuraLaneGroup<'a>> {
    let mut aura_names: Vec<&String> = tracked
        .iter()
        .filter(|name| td.aura_intervals.contains_key(name.as_str()))
        .collect();
    aura_names.sort_by_key(|n| n.to_lowercase());

    aura_names
        .iter()
        .enumerate()
        .map(|(idx, aura_name)| {
            let color = theme::AURA_COLORS[idx % theme::AURA_COLORS.len()];
            let mut players: Vec<&str> = td.aura_intervals[aura_name.as_str()]
                .iter()
                .map(|iv| iv.player.as_str())
                .collect::<HashSet<_>>()
                .into_iter()
                .collect();
            players.sort_unstable();
            AuraLaneGroup {
                aura_name: aura_name.as_str(),
                color,
                players,
            }
        })
        .collect()
}

/// Compute the total canvas height for the waterfall layout.
pub(super) fn aura_chart_height(layout: &[AuraLaneGroup<'_>]) -> f32 {
    layout
        .iter()
        .map(|g| AURA_HEADER_HEIGHT + g.players.len() as f32 * AURA_LANE_HEIGHT)
        .sum::<f32>()
        .max(AURA_LANE_HEIGHT)
}

impl canvas::Program<ViewerMessage> for AuraChart<'_> {
    type State = ();

    #[allow(clippy::many_single_char_names)] // x/y/w/h are standard 2D drawing variables
    #[allow(clippy::too_many_lines)] // Canvas draw — waterfall with grouped player lanes
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

        if td.duration <= 0.0 || w < 2.0 || h < 2.0 {
            return vec![];
        }

        let mut frame = canvas::Frame::new(renderer, bounds.size());

        // Background
        frame.fill_rectangle(
            Point::ORIGIN,
            bounds.size(),
            Color::from_rgba8(0, 0, 0, 0.2),
        );

        let aura_chart_w = (w - CHART_LEFT_MARGIN - CHART_RIGHT_MARGIN).max(1.0);
        let (view_lo, view_hi) = self.zoom.map_or((0.0, td.duration), |(lo, hi)| (lo, hi));
        let view_span = (view_hi - view_lo).max(0.001) as f32;

        let mut y_cursor: f32 = 0.0;

        for group in &self.layout {
            let color = group.color;

            // Aura name header
            frame.fill_text(canvas::Text {
                content: group.aura_name.to_string(),
                position: Point::new(2.0, y_cursor + 1.0),
                color: Color { a: 0.7, ..color },
                size: 11.0.into(),
                ..canvas::Text::default()
            });

            // Thin separator line under the header
            let sep = canvas::Path::line(
                Point::new(0.0, y_cursor + AURA_HEADER_HEIGHT - 1.0),
                Point::new(w, y_cursor + AURA_HEADER_HEIGHT - 1.0),
            );
            frame.stroke(
                &sep,
                canvas::Stroke::default()
                    .with_color(Color { a: 0.15, ..color })
                    .with_width(1.0),
            );

            y_cursor += AURA_HEADER_HEIGHT;

            // Player lanes
            if let Some(intervals) = td.aura_intervals.get(group.aura_name) {
                for (player_idx, &player) in group.players.iter().enumerate() {
                    let lane_y = y_cursor + player_idx as f32 * AURA_LANE_HEIGHT;

                    // Alternating row background for readability
                    if player_idx % 2 == 0 {
                        frame.fill_rectangle(
                            Point::new(0.0, lane_y),
                            iced::Size::new(w, AURA_LANE_HEIGHT),
                            Color::from_rgba8(255, 255, 255, 0.02),
                        );
                    }

                    // Player name label on the left
                    frame.fill_text(canvas::Text {
                        content: player.to_string(),
                        position: Point::new(4.0, lane_y + 2.0),
                        color: theme::TEXT_MUTED,
                        size: 9.0.into(),
                        ..canvas::Text::default()
                    });

                    // Draw bars for this player's intervals
                    for interval in intervals.iter().filter(|iv| iv.player == player) {
                        let x_start = CHART_LEFT_MARGIN
                            + ((interval.start as f32 - view_lo as f32) / view_span) * aura_chart_w;
                        let x_end = CHART_LEFT_MARGIN
                            + ((interval.end as f32 - view_lo as f32) / view_span) * aura_chart_w;
                        let bar_w = (x_end - x_start).max(2.0);

                        // Filled bar
                        frame.fill_rectangle(
                            Point::new(x_start, lane_y + 2.0),
                            iced::Size::new(bar_w, AURA_LANE_HEIGHT - 4.0),
                            Color { a: 0.55, ..color },
                        );

                        // Border
                        let bar_rect = canvas::Path::rectangle(
                            Point::new(x_start, lane_y + 2.0),
                            iced::Size::new(bar_w, AURA_LANE_HEIGHT - 4.0),
                        );
                        frame.stroke(
                            &bar_rect,
                            canvas::Stroke::default()
                                .with_color(Color { a: 0.8, ..color })
                                .with_width(1.0),
                        );
                    }
                }
            }

            y_cursor += group.players.len() as f32 * AURA_LANE_HEIGHT;
        }

        // Hover line (shared time cursor with main chart)
        if let Some(second) = self.hover_second {
            let x =
                CHART_LEFT_MARGIN + ((second as f32 - view_lo as f32) / view_span) * aura_chart_w;
            let line = canvas::Path::line(Point::new(x, 0.0), Point::new(x, h));
            frame.stroke(
                &line,
                canvas::Stroke::default()
                    .with_color(Color::from_rgba8(255, 255, 255, 0.6))
                    .with_width(1.0),
            );
        }

        vec![frame.into_geometry()]
    }

    fn update(
        &self,
        _state: &mut (),
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<Action<ViewerMessage>> {
        let chart_w = (bounds.width - CHART_LEFT_MARGIN - CHART_RIGHT_MARGIN).max(1.0);
        let (view_lo, view_hi) = self
            .zoom
            .map_or((0.0, self.data.duration), |(lo, hi)| (lo, hi));
        let view_span = (view_hi - view_lo).max(0.001);
        match event {
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    if self.data.duration > 0.0 {
                        let second = view_lo
                            + f64::from((pos.x - CHART_LEFT_MARGIN) / chart_w).clamp(0.0, 1.0)
                                * view_span;
                        return Some(
                            Action::publish(ViewerMessage::AuraHover(Some(second))).and_capture(),
                        );
                    }
                } else {
                    return Some(Action::publish(ViewerMessage::AuraHover(None)));
                }
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(pos) = cursor.position_in(bounds)
                    && self.data.duration > 0.0
                {
                    let second = (view_lo
                        + f64::from((pos.x - CHART_LEFT_MARGIN) / chart_w).clamp(0.0, 1.0)
                            * view_span) as usize;
                    return Some(
                        Action::publish(ViewerMessage::TimelineClick(second)).and_capture(),
                    );
                }
            }
            _ => {}
        }
        None
    }
}
