# Timeline Bug Fixes & Enhancements

## Bug 1: Duplicate Player Names in Aura Tooltip

### Problem

Hovering over the aura waterfall chart shows a tooltip listing which players have
each tracked aura active at that moment. The tooltip was displaying the same player
name multiple times for a single aura — e.g., `Loatheb's Shadow: Tank, Tank` instead
of `Loatheb's Shadow: Tank`.

This happened because `AuraInterval`s can overlap for the same player and aura.
A buff refresh (fade + immediate re-gain) or a stacking debuff can produce two
intervals whose boundaries overlap at the same second. The tooltip collected every
matching interval's player name without deduplication, so any overlap produced
duplicate names.

### Fix

Added `sort_unstable()` + `dedup()` on the collected player name list before
rendering the tooltip text. This ensures each player appears at most once per aura
in the hover display.

**File:** `src/viewer.rs` — aura hover tooltip builder (~line 1825)

---

## Bug 2: Timeline Charts Not Aligned on the X-Axis

### Problem

The upper sparkline charts (DPS, DTPS, HPS, Alive) and the lower waterfall charts
(Aura bars, Dispel diamonds) were offset by half a second horizontally. An event
at second 30 would appear at slightly different X positions on the sparkline vs the
aura/dispel charts, making cross-referencing between them inaccurate.

The root cause was a coordinate mapping mismatch:

- **Sparklines** (`draw_sparkline_area`, `AliveChart`): Plotted bucket `i` at pixel
  position `(i + 0.5) * x_scale` — centering data points in the middle of their
  1-second bucket window.
- **Waterfalls** (`AuraChart`, `DispelChart`): Plotted events at pixel position
  `seconds / duration * chart_width` — using raw second values with no offset.

The `+ 0.5` centering was cosmetically motivated (placing the dot in the center of
the bucket) but it shifted the entire sparkline half a second to the right relative
to the aura bars and dispel diamonds, breaking visual alignment.

### Fix

Removed the `+ 0.5` offset from all sparkline coordinate calculations:

- `draw_sparkline_area()` — both the line path and the filled area path now use
  `i * x_scale` instead of `(i + 0.5) * x_scale`.
- `AliveChart::draw()` — same removal from both the filled area and stroke paths.
- `TimelineChart` hover line — updated to match.

Event markers (death/big-hit/resurrect dots) and X-axis time labels were already
using raw second coordinates, so they required no changes.

All charts now use the same `seconds → pixels` mapping: `CHART_LEFT_MARGIN + (seconds - view_lo) / view_span * chart_width`.

**File:** `src/viewer.rs` — `draw_sparkline_area()` (~line 4054), `AliveChart::draw()` (~line 4143), `TimelineChart` hover line (~line 3855)

---

## Bug 3: Enabling Dispels Hides the Aura Tracker

### Problem

When the Dispels toggle was activated with auras already tracked, the aura waterfall
chart could disappear or become inaccessible. The three tracker sections — aura
waterfall, dispel waterfall, and alive sparkline — were placed directly in the
charts panel column with no combined height constraint. Their individual max heights
sum to ~460px (200 + 200 + 60), and combined with the main sparkline chart (220px),
header, legend, and tooltip, the total panel height could exceed the window.

Since the charts panel was not scrollable and had no max height, content overflowing
the window was simply clipped. With dispels off (the default), the panel usually fit.
Toggling dispels on added up to 200px of dispel waterfall, pushing the total past
the window boundary and clipping the sections at the bottom.

### Fix

Wrapped the three tracker sections (aura, dispel, alive) in a `scrollable` widget
inside a `container` with `max_height(300)`. This ensures:

- When only one or two trackers are active, the container shrinks to fit naturally.
- When all three are active and their combined height exceeds 300px, a vertical
  scrollbar appears instead of clipping content off-screen.
- The main sparkline chart and event log below always remain visible.

**File:** `src/viewer.rs` — charts panel layout (~line 1931)

---

## Enhancement: DTPS Legend Tooltip

### Problem

The "Raid DTPS" toggle in the timeline legend used an abbreviation that may not be
immediately clear to all users.

### Fix

Wrapped the DTPS legend toggle in an `iced::widget::tooltip` that displays
"Damage Taken Per Second — total raid incoming damage each second" when hovered.
The tooltip appears below the toggle with a styled dark background matching the
application's visual theme.

**File:** `src/viewer.rs` — legend toggles (~line 1419), added `tooltip` to widget imports
