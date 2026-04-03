//! Timeline construction: bucketing events into 1-second intervals with aura tracking.

use std::collections::{HashMap, HashSet};

use super::timeline::{
    AuraInterval, ConsumeMark, DispelMark, TimelineBucket, TimelineData, TimelineEvent,
    TimelineEventKind,
};
use super::types::{Combatant, ConsumableCategory, Encounter, EncounterFilter, LogData, LogEntry};

/// Pre-pull buffer in seconds (5 minutes) for consume timeline.
const PRE_PULL_BUFFER: f64 = 300.0;

/// Compute the start of the consume timeline window.
///
/// Returns `(earliest_encounter_start - PRE_PULL_BUFFER).max(session_start)`,
/// or `session_start` if there are no encounters.
fn consume_timeline_start(session_start: f64, encounters: &[&Encounter]) -> f64 {
    let earliest = encounters
        .iter()
        .map(|e| e.start)
        .fold(f64::INFINITY, f64::min);
    (earliest - PRE_PULL_BUFFER).max(session_start)
}

// ── Timeline Builder ────────────────────────────────────────────────────────

impl LogData {
    /// Build timeline data for the selected encounter filter.
    ///
    /// Buckets all events into 1-second intervals relative to the encounter start,
    /// and collects discrete events (deaths, big hits, dispels) for overlay markers.
    /// The `big_hit_threshold` marks any single damage-taken event above this value.
    #[allow(clippy::too_many_lines)] // Timeline builder — single cohesive pass over events
    #[allow(clippy::cast_possible_truncation)] // Timestamps/durations never approach usize limits
    #[allow(clippy::cast_sign_loss)] // Duration and offsets are always non-negative
    #[allow(clippy::cast_precision_loss)] // Bucket indices never approach 2^52
    #[allow(clippy::similar_names)] // dps/dtps/hps are standard WoW combat log metrics
    pub fn build_timeline(&self, filter: &EncounterFilter, big_hit_threshold: u64) -> TimelineData {
        let encounters = self.selected_encounters(filter);
        if encounters.is_empty() {
            return TimelineData::default();
        }

        // For Single encounters, use the encounter's own start/end.
        // For multi-encounter filters, concatenate them sequentially.
        let total_duration: f64 = encounters.iter().map(|e| e.duration).sum();
        if total_duration <= 0.0 {
            return TimelineData::default();
        }

        let bucket_count = total_duration.ceil() as usize + 1;
        let mut buckets: Vec<TimelineBucket> = (0..bucket_count)
            .map(|i| TimelineBucket {
                offset: i as f64,
                ..TimelineBucket::default()
            })
            .collect();
        let mut events: Vec<TimelineEvent> = Vec::new();
        let mut dispel_marks: Vec<DispelMark> = Vec::new();

        // Track alive count: start with all combatants, decrement on death,
        // increment on resurrect.
        let raid_count = self.combatants.len() as u32;
        let mut alive: u32;

        // Offset accumulator for multi-encounter concatenation
        let mut offset_base: f64 = 0.0;

        for enc in &encounters {
            let enc_start = enc.start;
            let enc_duration = enc.duration;

            // Reset alive count for each encounter segment
            alive = raid_count;
            // Anchor the first bucket of this segment to the fresh alive count
            let first_bucket = offset_base.floor() as usize;
            if first_bucket < buckets.len() {
                buckets[first_bucket].alive_count = alive;
            }

            for entry in &self.entries {
                let ts = entry.timestamp();
                if ts < enc_start || ts > enc.end {
                    continue;
                }

                let relative = ts - enc_start + offset_base;
                let bucket_idx = relative.floor() as usize;
                if bucket_idx >= buckets.len() {
                    continue;
                }

                match entry {
                    LogEntry::Damage {
                        target,
                        amount,
                        spell,
                        source,
                        ..
                    } => {
                        // Damage done (from raid members)
                        if self.combatants.contains_key(source.as_str()) {
                            buckets[bucket_idx].damage += amount;
                        }
                        // Damage taken (by raid members)
                        if self.combatants.contains_key(target.as_str()) {
                            buckets[bucket_idx].damage_taken += amount;
                            // Big hit marker
                            if *amount >= big_hit_threshold {
                                events.push(TimelineEvent {
                                    offset: relative,
                                    kind: TimelineEventKind::BigHit,
                                    label: format!(
                                        "{target} takes {amount} from {source}'s {spell}"
                                    ),
                                });
                            }
                        }
                    }
                    LogEntry::Healing {
                        source,
                        target,
                        effective_heal,
                        ..
                    } => {
                        if self.combatants.contains_key(source.as_str()) {
                            if self.combatants.contains_key(target.as_str()) {
                                buckets[bucket_idx].healing += effective_heal;
                            } else if !is_pet_target(target, &self.combatants) {
                                buckets[bucket_idx].boss_healing += effective_heal;
                            }
                            // Pet heals (player healing their own pet) are dropped
                            // from both sparklines.
                        }
                    }
                    LogEntry::Death { player, .. } => {
                        if self.combatants.contains_key(player.as_str()) {
                            alive = alive.saturating_sub(1);
                            events.push(TimelineEvent {
                                offset: relative,
                                kind: TimelineEventKind::Death,
                                label: format!("{player} died"),
                            });
                        }
                    }
                    LogEntry::Dispel {
                        caster,
                        target,
                        spell,
                        ..
                    } => {
                        events.push(TimelineEvent {
                            offset: relative,
                            kind: TimelineEventKind::Dispel,
                            label: format!("{caster} dispels {spell} on {target}"),
                        });
                        dispel_marks.push(DispelMark {
                            caster: caster.clone(),
                            target: target.clone(),
                            spell: spell.clone(),
                            offset: relative,
                        });
                    }
                    LogEntry::Resurrect { caster, target, .. } => {
                        if self.combatants.contains_key(target.as_str()) {
                            alive = alive.saturating_add(1).min(raid_count);
                            events.push(TimelineEvent {
                                offset: relative,
                                kind: TimelineEventKind::Resurrect,
                                label: format!("{caster} resurrects {target}"),
                            });
                        }
                    }
                    LogEntry::Interrupt {
                        caster,
                        target,
                        spell,
                        ..
                    } => {
                        events.push(TimelineEvent {
                            offset: relative,
                            kind: TimelineEventKind::Interrupt,
                            label: format!("{caster} interrupts {target} with {spell}"),
                        });
                    }
                    // Aura events are rendered by AuraChart, not the main timeline.
                    LogEntry::AuraGain { .. } | LogEntry::AuraFade { .. } => {}
                }

                buckets[bucket_idx].alive_count = alive;
            }

            offset_base += enc_duration;
        }

        // Forward-fill alive counts for empty buckets
        let mut last_alive = raid_count;
        for bucket in &mut buckets {
            if bucket.alive_count == 0
                && bucket.damage == 0
                && bucket.healing == 0
                && bucket.damage_taken == 0
            {
                bucket.alive_count = last_alive;
            } else {
                last_alive = bucket.alive_count;
            }
        }

        let max_dps = buckets.iter().map(|b| b.damage).max().unwrap_or(0);
        let max_dtps = buckets.iter().map(|b| b.damage_taken).max().unwrap_or(0);
        let max_hps = buckets.iter().map(|b| b.healing).max().unwrap_or(0);
        let max_boss_hps = buckets.iter().map(|b| b.boss_healing).max().unwrap_or(0);

        // ── Collect aura intervals ──────────────────────────────────
        let (aura_intervals, available_auras) =
            self.build_aura_intervals(&encounters, total_duration);

        // ── Collect consumable use marks ────────────────────────────
        // For "All Encounters", show the full session.  For specific encounter
        // filters, extend each encounter window by 5 minutes before the pull
        // to capture pre-pull potions, food, and elixirs.
        let (
            consume_marks,
            consume_duration,
            consume_aura_categories,
            available_consume_categories,
        ) = Self::build_consume_marks(
            &self.consumables,
            self.start_time,
            self.end_time,
            &encounters,
            &available_auras,
        );

        // ── Encounter boundaries for the consumable chart ───────────
        // Show all boss encounters as faint vertical lines so the user can
        // see where fights are relative to consumable usage.
        let consume_encounter_bounds = Self::build_consume_encounter_bounds(
            &self.encounters,
            self.start_time,
            &encounters,
            filter,
        );

        // ── Compute dispel casters sorted by count descending ───────
        let mut caster_counts: HashMap<&str, usize> = HashMap::new();
        for mark in &dispel_marks {
            *caster_counts.entry(mark.caster.as_str()).or_default() += 1;
        }
        let mut dispel_casters: Vec<String> =
            caster_counts.keys().map(|s| (*s).to_owned()).collect();
        dispel_casters.sort_by(|a, b| {
            caster_counts[b.as_str()]
                .cmp(&caster_counts[a.as_str()])
                .then_with(|| a.cmp(b))
        });

        // ── Aura-to-consume offset translation ─────────────────────
        // Aura intervals use encounter-relative offsets (0 to duration,
        // concatenated with offset_base for multi-encounter). The consume
        // chart uses session-relative offsets with a 5-min pre-pull buffer.
        // Build translation segments so the consume chart renderer can map
        // aura interval offsets to consume-timeline pixel coordinates.
        let consume_aura_offset_segments =
            Self::build_consume_aura_segments(self.start_time, &encounters);

        TimelineData {
            buckets,
            events,
            max_dps,
            max_dtps,
            max_hps,
            max_boss_hps,
            duration: total_duration,
            raid_count,
            aura_intervals,
            available_auras,
            dispel_marks,
            dispel_casters,
            consume_marks,
            consume_duration,
            consume_aura_categories,
            available_consume_categories,
            consume_encounter_bounds,
            consume_aura_offset_segments,
        }
    }

    /// Build aura intervals and a sorted list of available aura names for the
    /// selected encounter(s).
    ///
    /// Scans `AuraGain`/`AuraFade` entries within encounter windows, pairing
    /// gains with their corresponding fades per player. Unclosed auras are
    /// clamped to the player's death time (if any) or to the encounter end.
    /// Flasks and Zanzas (death-persistent buffs) are always clamped to the
    /// encounter end, ignoring death.
    ///
    /// Pre-encounter lookback: scans entries before each encounter to detect
    /// buffs that were already active when the fight started (e.g. Spirit of
    /// Zanza applied 30 minutes pre-pull).
    #[allow(clippy::too_many_lines)] // Aura interval builder — pre-encounter lookback + death-aware clamping
    fn build_aura_intervals(
        &self,
        encounters: &[&Encounter],
        total_duration: f64,
    ) -> (HashMap<String, Vec<AuraInterval>>, Vec<String>) {
        // Key: (player, aura) -> stack of unclosed gain offsets
        let mut open: HashMap<(String, String), Vec<f64>> = HashMap::new();
        let mut intervals: HashMap<String, Vec<AuraInterval>> = HashMap::new();
        let mut aura_names: HashSet<String> = HashSet::new();

        let mut offset_base: f64 = 0.0;

        for enc in encounters {
            let enc_start = enc.start;
            let enc_duration = enc.duration;

            // ── Pre-encounter lookback ──────────────────────────────────
            // Find buffs that were already active when the encounter started.
            // Scan all entries before enc_start, tracking the last AuraGain
            // and AuraFade per (player, aura). If the last event for a pair
            // is an AuraGain (no subsequent fade), the buff was active at
            // encounter start → open an interval at offset 0.
            let mut pre_state: HashMap<(String, String), bool> = HashMap::new();
            for entry in &self.entries {
                let ts = entry.timestamp();
                if ts >= enc_start {
                    break; // entries are sorted by timestamp
                }
                match entry {
                    LogEntry::AuraGain { player, aura, .. } => {
                        if self.all_combatants.contains_key(player.as_str()) {
                            pre_state.insert((player.clone(), aura.clone()), true);
                        }
                    }
                    LogEntry::AuraFade { player, aura, .. } => {
                        pre_state.insert((player.clone(), aura.clone()), false);
                    }
                    _ => {}
                }
            }
            // Inject pre-existing buffs as open intervals at encounter start
            for ((player, aura), was_active) in &pre_state {
                if *was_active {
                    aura_names.insert(aura.clone());
                    open.entry((player.clone(), aura.clone()))
                        .or_default()
                        .push(offset_base); // offset 0 relative to this encounter
                }
            }

            // ── Main encounter scan ─────────────────────────────────────
            // Collect death timestamps within this encounter segment
            let mut deaths: HashMap<&str, f64> = HashMap::new();

            for entry in &self.entries {
                let ts = entry.timestamp();
                if ts < enc_start || ts > enc.end {
                    continue;
                }
                let relative = ts - enc_start + offset_base;

                match entry {
                    LogEntry::AuraGain { player, aura, .. } => {
                        aura_names.insert(aura.clone());
                        open.entry((player.clone(), aura.clone()))
                            .or_default()
                            .push(relative);
                    }
                    LogEntry::AuraFade { player, aura, .. } => {
                        aura_names.insert(aura.clone());
                        // Pop the most recent open gain for this player+aura
                        if let Some(starts) = open.get_mut(&(player.clone(), aura.clone()))
                            && let Some(start) = starts.pop()
                        {
                            intervals
                                .entry(aura.clone())
                                .or_default()
                                .push(AuraInterval {
                                    player: player.clone(),
                                    start,
                                    end: relative,
                                });
                        }
                    }
                    LogEntry::Death { player, .. } => {
                        // Record first death only (ignore subsequent deaths from
                        // combat log quirks); resurrect creates new AuraGain events.
                        deaths.entry(player.as_str()).or_insert(relative);
                    }
                    _ => {}
                }
            }

            // ── Close unclosed auras ────────────────────────────────────
            // Clamp to death time UNLESS the aura is death-persistent
            let segment_end = offset_base + enc_duration;
            for ((player, aura), starts) in &mut open {
                let clamp = if crate::consumable_data::is_death_persistent(aura) {
                    // Flasks, Zanzas → clamp to encounter end (persist through death)
                    segment_end.min(total_duration)
                } else {
                    // Normal buffs (including world buffs, elixirs) → clamp to death time
                    deaths
                        .get(player.as_str())
                        .copied()
                        .unwrap_or(segment_end)
                        .min(segment_end)
                        .min(total_duration)
                };

                for start in starts.drain(..) {
                    if clamp > start {
                        intervals
                            .entry(aura.clone())
                            .or_default()
                            .push(AuraInterval {
                                player: player.clone(),
                                start,
                                end: clamp,
                            });
                    }
                }
            }

            offset_base += enc_duration;
        }

        // Sort intervals within each aura by start time
        for ivs in intervals.values_mut() {
            ivs.sort_by(|a, b| a.start.total_cmp(&b.start));
        }

        let mut sorted_names: Vec<String> = aura_names.into_iter().collect();
        sorted_names.sort_by_key(|n| n.to_lowercase());

        (intervals, sorted_names)
    }

    /// Compute per-player per-buff uptime fractions and encounter-filtered
    /// gains/fades for selected encounters.
    ///
    /// Returns `(uptimes, total_duration)` where `uptimes` maps
    /// `player -> buff_name -> BuffUptime`.
    ///
    /// Reuses `build_aura_intervals()` for encounter-aware interval pairing,
    /// then merges overlapping intervals per (player, buff) before computing
    /// the fraction of total encounter duration each buff was active.
    /// Also counts `AuraGain`/`AuraFade` events within encounter windows for
    /// encounter-filtered gains/fades.
    pub fn compute_buff_uptimes(
        &self,
        filter: &EncounterFilter,
    ) -> (HashMap<String, HashMap<String, BuffUptime>>, f64) {
        let encounters = self.selected_encounters(filter);
        let total_duration: f64 = encounters.iter().map(|e| e.duration).sum();

        if total_duration <= 0.0 {
            return (HashMap::new(), 0.0);
        }

        let (aura_intervals, _available) = self.build_aura_intervals(&encounters, total_duration);

        // Count encounter-filtered gains/fades
        let mut gains_fades: HashMap<(String, String), (u64, u64)> = HashMap::new();
        for enc in &encounters {
            for entry in &self.entries {
                let ts = entry.timestamp();
                if ts < enc.start || ts > enc.end {
                    continue;
                }
                match entry {
                    LogEntry::AuraGain { player, aura, .. } => {
                        gains_fades
                            .entry((player.clone(), aura.clone()))
                            .or_default()
                            .0 += 1;
                    }
                    LogEntry::AuraFade { player, aura, .. } => {
                        gains_fades
                            .entry((player.clone(), aura.clone()))
                            .or_default()
                            .1 += 1;
                    }
                    _ => {}
                }
            }
        }

        // Reorganize: aura_name -> Vec<AuraInterval>  into  player -> aura -> BuffUptime
        let mut uptimes: HashMap<String, HashMap<String, BuffUptime>> = HashMap::new();

        for (aura_name, intervals) in &aura_intervals {
            // Group intervals by player
            let mut by_player: HashMap<&str, Vec<(f64, f64)>> = HashMap::new();
            for iv in intervals {
                by_player
                    .entry(iv.player.as_str())
                    .or_default()
                    .push((iv.start, iv.end));
            }

            for (player, mut spans) in by_player {
                let merged_duration = merge_and_sum(&mut spans);
                let fraction = (merged_duration / total_duration).min(1.0);
                let (gains, fades) = gains_fades
                    .get(&(player.to_string(), aura_name.clone()))
                    .copied()
                    .unwrap_or((0, 0));

                uptimes.entry(player.to_string()).or_default().insert(
                    aura_name.clone(),
                    BuffUptime {
                        fraction,
                        gains,
                        fades,
                    },
                );
            }
        }

        // Add entries for buffs that have gains/fades but no intervals
        // (e.g., buff gained and faded at the same timestamp)
        for ((player, aura), (gains, fades)) in &gains_fades {
            uptimes
                .entry(player.clone())
                .or_default()
                .entry(aura.clone())
                .or_insert(BuffUptime {
                    fraction: 0.0,
                    gains: *gains,
                    fades: *fades,
                });
        }

        (uptimes, total_duration)
    }

    /// Build consumable use marks and category metadata.
    ///
    /// For `EncounterFilter::All`, spans the full session so all consumable
    /// uses are visible.  For specific encounter filters, extends each
    /// encounter window by 5 minutes before the pull to capture pre-pull
    /// potions, food, and elixirs.
    ///
    /// Returns:
    /// - `consume_marks`: point-in-time consumable events with time-relative offsets
    /// - `consume_duration`: total time span in seconds (X-axis duration)
    /// - `consume_aura_categories`: aura name → category mapping (exact match + TOML overrides)
    /// - `available_consume_categories`: sorted unique categories present
    fn build_consume_marks(
        consumables: &[super::types::ConsumableUse],
        session_start: Option<f64>,
        session_end: Option<f64>,
        encounters: &[&Encounter],
        aura_names: &[String],
    ) -> (
        Vec<ConsumeMark>,
        f64,
        HashMap<String, ConsumableCategory>,
        Vec<ConsumableCategory>,
    ) {
        let Some(t_session_start) = session_start else {
            return (Vec::new(), 0.0, HashMap::new(), Vec::new());
        };
        let t_session_end = session_end.unwrap_or(t_session_start);

        // Determine the effective time window based on the encounter filter.
        // For all filters, use encounter boundaries (with pre-pull buffer) to
        // trim dead whitespace from the timeline ends.
        let (t_start, t_end) = if encounters.is_empty() {
            (t_session_start, t_session_end)
        } else {
            let latest = encounters
                .iter()
                .map(|e| e.end)
                .fold(f64::NEG_INFINITY, f64::max);
            (consume_timeline_start(t_session_start, encounters), latest)
        };

        let consume_duration = (t_end - t_start).max(0.0);

        let mut marks = Vec::new();
        let mut aura_categories: HashMap<String, ConsumableCategory> = HashMap::new();
        let mut category_set: HashSet<ConsumableCategory> = HashSet::new();

        for cu in consumables {
            if cu.timestamp < t_start || cu.timestamp > t_end {
                continue;
            }
            let relative = cu.timestamp - t_start;

            marks.push(ConsumeMark {
                player: cu.player.clone(),
                consumable: cu.consumable.clone(),
                category: cu.category,
                offset: relative,
            });

            category_set.insert(cu.category);

            // Record the item name as a potential aura match. Many consumable
            // buff names match the item name exactly (e.g. "Elixir of the
            // Mongoose"). This mapping enables hybrid bar/tick rendering.
            aura_categories
                .entry(cu.consumable.clone())
                .or_insert(cu.category);
        }

        marks.sort_by(|a, b| a.offset.total_cmp(&b.offset));

        // Scan aura interval names for buff-name overrides from consumables.toml.
        // This handles cases where the combat log buff name differs from the item
        // name (e.g. "Fire Protection" from "Greater Fire Protection Potion").
        for aura_name in aura_names {
            if aura_categories.contains_key(aura_name) {
                continue;
            }
            if let Some(category) = crate::consumable_data::classify_buff(aura_name) {
                aura_categories.insert(aura_name.clone(), category);
            }
        }

        let mut categories: Vec<ConsumableCategory> = category_set.into_iter().collect();
        categories.sort();

        (marks, consume_duration, aura_categories, categories)
    }

    /// Compute encounter boundary offsets for the consumable chart background.
    ///
    /// Returns `(start_offset, end_offset, boss_name, is_kill)` for each boss
    /// encounter, using the same time base as consume marks.
    fn build_consume_encounter_bounds(
        all_encounters: &[Encounter],
        session_start: Option<f64>,
        selected_encounters: &[&Encounter],
        filter: &EncounterFilter,
    ) -> Vec<(f64, f64, String, bool)> {
        let Some(t_session_start) = session_start else {
            return Vec::new();
        };

        // Determine the consume timeline's t_start (must match build_consume_marks)
        let t_start = if selected_encounters.is_empty() {
            t_session_start
        } else {
            consume_timeline_start(t_session_start, selected_encounters)
        };

        // For "All Encounters", show all boss encounters.
        // For specific filters, show the selected encounters.
        let encounters_to_show: Vec<&Encounter> = if matches!(filter, EncounterFilter::All) {
            all_encounters.iter().filter(|e| e.is_boss).collect()
        } else {
            selected_encounters.to_vec()
        };

        encounters_to_show
            .iter()
            .map(|enc| {
                let name = enc.name.as_deref().unwrap_or("Trash");
                let start = enc.start - t_start;
                let end = enc.end - t_start;
                (start, end, name.to_string(), enc.is_kill)
            })
            .collect()
    }

    /// Build translation segments for mapping encounter-relative aura interval
    /// offsets to consume-timeline-relative offsets.
    ///
    /// Aura intervals use concatenated encounter-relative offsets (0 to dur1,
    /// dur1 to dur1+dur2, …). The consume chart uses session-relative offsets
    /// starting from `t_start` (earliest encounter − 5 min pre-pull buffer).
    ///
    /// Returns `(aura_offset_start, aura_offset_end, consume_timeline_start)`
    /// for each encounter segment.
    fn build_consume_aura_segments(
        session_start: Option<f64>,
        encounters: &[&Encounter],
    ) -> Vec<(f64, f64, f64)> {
        let Some(t_session_start) = session_start else {
            return Vec::new();
        };

        if encounters.is_empty() {
            return Vec::new();
        }

        // Compute t_start using the same formula as build_consume_marks()
        let t_start = consume_timeline_start(t_session_start, encounters);

        let mut segments = Vec::with_capacity(encounters.len());
        let mut aura_offset = 0.0;
        for enc in encounters {
            let consume_start = enc.start - t_start;
            segments.push((aura_offset, aura_offset + enc.duration, consume_start));
            aura_offset += enc.duration;
        }
        segments
    }
}

// ── Buff Uptime Types ───────────────────────────────────────────────────────

/// Per-buff uptime data for a single aura on a single player.
#[derive(Debug, Clone)]
pub struct BuffUptime {
    /// Uptime fraction in \[0.0, 1.0\].
    pub fraction: f64,
    /// Gains count within selected encounter(s).
    pub gains: u64,
    /// Fades count within selected encounter(s).
    pub fades: u64,
}

/// Merge overlapping time spans and return total covered duration.
///
/// Input spans are `(start, end)` pairs. The function sorts by start time,
/// merges overlapping/adjacent intervals, and sums the merged durations.
/// This prevents double-counting when a buff is refreshed before fading.
fn merge_and_sum(spans: &mut [(f64, f64)]) -> f64 {
    if spans.is_empty() {
        return 0.0;
    }
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut total = 0.0;
    let mut current_start = spans[0].0;
    let mut current_end = spans[0].1;

    for &(start, end) in &spans[1..] {
        if start <= current_end {
            // Overlapping — extend current interval
            current_end = current_end.max(end);
        } else {
            // Gap — close current interval, start new one
            total += current_end - current_start;
            current_start = start;
            current_end = end;
        }
    }
    total += current_end - current_start;
    total
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Check if a heal target is a player's pet (e.g. `"Nymeria (Phair)"`).
///
/// Returns `true` when the parenthesized name is a known combatant whose
/// `pet_name` matches the name before the parentheses.  This lets us
/// distinguish pet heals (dropped from sparklines) from MC'd-mob heals
/// (counted as boss/enemy healing).
fn is_pet_target(target: &str, combatants: &HashMap<String, Combatant>) -> bool {
    if let Some(paren_start) = target.find('(') {
        let owner = target[paren_start + 1..].trim_end_matches(')').trim();
        let pet_part = target[..paren_start].trim();
        if let Some(combatant) = combatants.get(owner) {
            return combatant.pet_name.as_deref() == Some(pet_part);
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn combatants_with_pet(owner: &str, pet: &str) -> HashMap<String, Combatant> {
        let mut map = HashMap::new();
        map.insert(
            owner.to_string(),
            Combatant {
                pet_name: Some(pet.to_string()),
                ..Combatant::default()
            },
        );
        map
    }

    #[test]
    fn test_is_pet_target_true() {
        let combatants = combatants_with_pet("Phair", "Nymeria");
        assert!(is_pet_target("Nymeria (Phair)", &combatants));
    }

    #[test]
    fn test_is_pet_target_wrong_pet_name() {
        let combatants = combatants_with_pet("Hunter", "Cat");
        assert!(!is_pet_target("Wolf (Hunter)", &combatants));
    }

    #[test]
    fn test_is_pet_target_unknown_owner() {
        let combatants = combatants_with_pet("Hunter", "Cat");
        assert!(!is_pet_target("Wolf (Unknown)", &combatants));
    }

    #[test]
    fn test_is_pet_target_no_parens() {
        let combatants = combatants_with_pet("Hunter", "Cat");
        assert!(!is_pet_target("Ragnaros", &combatants));
    }

    // ── merge_and_sum tests ─────────────────────────────────────────

    #[test]
    fn test_merge_and_sum_single_span() {
        let mut spans = vec![(0.0, 10.0)];
        assert!((merge_and_sum(&mut spans) - 10.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_merge_and_sum_no_overlap() {
        let mut spans = vec![(0.0, 5.0), (10.0, 15.0)];
        assert!((merge_and_sum(&mut spans) - 10.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_merge_and_sum_overlapping() {
        let mut spans = vec![(0.0, 8.0), (5.0, 12.0)];
        assert!((merge_and_sum(&mut spans) - 12.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_merge_and_sum_nested() {
        let mut spans = vec![(0.0, 20.0), (5.0, 10.0)];
        assert!((merge_and_sum(&mut spans) - 20.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_merge_and_sum_empty() {
        let mut spans: Vec<(f64, f64)> = vec![];
        assert!((merge_and_sum(&mut spans)).abs() < f64::EPSILON);
    }

    #[test]
    fn test_merge_and_sum_adjacent() {
        // Spans that touch exactly (end == start of next)
        let mut spans = vec![(0.0, 5.0), (5.0, 10.0)];
        assert!((merge_and_sum(&mut spans) - 10.0).abs() < f64::EPSILON);
    }

    // ── compute_buff_uptimes tests ──────────────────────────────────

    /// Helper to build a minimal encounter for testing.
    fn test_encounter(name: &str, start: f64, end: f64) -> Encounter {
        Encounter {
            name: Some(name.to_string()),
            start,
            end,
            duration: end - start,
            is_boss: true,
            is_kill: true,
            zone: None,
            attempt: None,
            player_deaths: 0,
            active_players: 1,
        }
    }

    #[test]
    fn test_buff_uptime_basic() {
        // Aura gain at 10s, fade at 60s, encounter 0-100s → 50% uptime
        let mut data = LogData::default();
        data.encounters.push(test_encounter("Boss", 0.0, 100.0));
        data.all_combatants
            .insert("Warrior".to_string(), Combatant::default());
        data.entries.push(LogEntry::AuraGain {
            timestamp: 10.0,
            player: "Warrior".to_string(),
            aura: "Elixir of the Mongoose".to_string(),
            stacks: 1,
        });
        data.entries.push(LogEntry::AuraFade {
            timestamp: 60.0,
            player: "Warrior".to_string(),
            aura: "Elixir of the Mongoose".to_string(),
        });

        let (uptimes, duration) = data.compute_buff_uptimes(&EncounterFilter::All);
        assert!((duration - 100.0).abs() < f64::EPSILON);
        let warrior = uptimes.get("Warrior").expect("warrior should have uptimes");
        let mongoose = warrior
            .get("Elixir of the Mongoose")
            .expect("mongoose should have uptime");
        assert!((mongoose.fraction - 0.5).abs() < 0.01);
        assert_eq!(mongoose.gains, 1);
        assert_eq!(mongoose.fades, 1);
    }

    #[test]
    fn test_buff_uptime_unclosed_aura() {
        // Aura gained at 20s, never fades → clamped to encounter end (100s)
        // Expected uptime: 80/100 = 80%
        let mut data = LogData::default();
        data.encounters.push(test_encounter("Boss", 0.0, 100.0));
        data.all_combatants
            .insert("Mage".to_string(), Combatant::default());
        data.entries.push(LogEntry::AuraGain {
            timestamp: 20.0,
            player: "Mage".to_string(),
            aura: "Arcane Power".to_string(),
            stacks: 1,
        });

        let (uptimes, _) = data.compute_buff_uptimes(&EncounterFilter::All);
        let mage = uptimes.get("Mage").expect("mage should have uptimes");
        let ap = mage.get("Arcane Power").expect("AP should have uptime");
        assert!((ap.fraction - 0.8).abs() < 0.01);
        assert_eq!(ap.gains, 1);
        assert_eq!(ap.fades, 0);
    }

    #[test]
    fn test_buff_uptime_death_clamp() {
        // Aura gained at 10s, player dies at 40s, encounter lasts 100s
        // Expected uptime: 30/100 = 30% (clamped at death, not encounter end)
        let mut data = LogData::default();
        let mut enc = test_encounter("Boss", 0.0, 100.0);
        enc.player_deaths = 1;
        data.encounters.push(enc);
        data.all_combatants
            .insert("Rogue".to_string(), Combatant::default());
        data.entries.push(LogEntry::AuraGain {
            timestamp: 10.0,
            player: "Rogue".to_string(),
            aura: "Slice and Dice".to_string(),
            stacks: 1,
        });
        data.entries.push(LogEntry::Death {
            timestamp: 40.0,
            player: "Rogue".to_string(),
        });

        let (uptimes, _) = data.compute_buff_uptimes(&EncounterFilter::All);
        let rogue = uptimes.get("Rogue").expect("rogue should have uptimes");
        let snd = rogue.get("Slice and Dice").expect("SnD should have uptime");
        assert!((snd.fraction - 0.3).abs() < 0.01);
    }

    #[test]
    fn test_buff_uptime_encounter_filtered_gains() {
        // Two encounters, buff gained in first only.
        // When filtering to second encounter, gains should be 0.
        let mut data = LogData::default();
        data.encounters.push(test_encounter("Boss A", 0.0, 100.0));
        data.encounters.push(test_encounter("Boss B", 200.0, 300.0));
        data.all_combatants
            .insert("Priest".to_string(), Combatant::default());
        data.entries.push(LogEntry::AuraGain {
            timestamp: 10.0,
            player: "Priest".to_string(),
            aura: "Power Infusion".to_string(),
            stacks: 1,
        });
        data.entries.push(LogEntry::AuraFade {
            timestamp: 25.0,
            player: "Priest".to_string(),
            aura: "Power Infusion".to_string(),
        });

        // Filter to second encounter only — should have no buff data
        let filter = EncounterFilter::Single(1);
        let (uptimes, _) = data.compute_buff_uptimes(&filter);
        assert!(
            uptimes.get("Priest").is_none()
                || uptimes.get("Priest").expect("checked above").is_empty()
        );
    }

    #[test]
    fn test_buff_uptime_death_persistent_flask() {
        // Flask gained at 5s, player dies at 30s, encounter lasts 100s
        // Flasks persist through death → uptime should be 95/100 = 95%
        let mut data = LogData::default();
        let mut enc = test_encounter("Boss", 0.0, 100.0);
        enc.player_deaths = 1;
        data.encounters.push(enc);
        data.all_combatants
            .insert("Mage".to_string(), Combatant::default());
        data.entries.push(LogEntry::AuraGain {
            timestamp: 5.0,
            player: "Mage".to_string(),
            aura: "Flask of Supreme Power".to_string(),
            stacks: 1,
        });
        data.entries.push(LogEntry::Death {
            timestamp: 30.0,
            player: "Mage".to_string(),
        });

        let (uptimes, _) = data.compute_buff_uptimes(&EncounterFilter::All);
        let mage = uptimes.get("Mage").expect("mage should have uptimes");
        let flask = mage
            .get("Flask of Supreme Power")
            .expect("flask should have uptime");
        // Flask persists through death: 95s / 100s = 95%
        assert!((flask.fraction - 0.95).abs() < 0.01);
    }

    #[test]
    fn test_buff_uptime_pre_encounter_lookback() {
        // Spirit of Zanza gained at timestamp 50 (before encounter)
        // Encounter runs from 100-200
        // No AuraFade in the log → buff active entire fight
        // Expected uptime: 100%
        let mut data = LogData::default();
        data.encounters.push(test_encounter("Boss", 100.0, 200.0));
        data.all_combatants
            .insert("Warrior".to_string(), Combatant::default());
        data.entries.push(LogEntry::AuraGain {
            timestamp: 50.0,
            player: "Warrior".to_string(),
            aura: "Spirit of Zanza".to_string(),
            stacks: 1,
        });

        let (uptimes, duration) = data.compute_buff_uptimes(&EncounterFilter::All);
        assert!((duration - 100.0).abs() < f64::EPSILON);
        let warrior = uptimes.get("Warrior").expect("warrior should have uptimes");
        let zanza = warrior
            .get("Spirit of Zanza")
            .expect("zanza should have uptime");
        assert!((zanza.fraction - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_buff_uptime_pre_encounter_with_mid_fight_fade() {
        // Elixir gained at timestamp 50 (before encounter), fades at 150 (mid-fight)
        // Encounter runs from 100-200
        // Expected uptime: 50/100 = 50% (active from enc start to fade)
        let mut data = LogData::default();
        data.encounters.push(test_encounter("Boss", 100.0, 200.0));
        data.all_combatants
            .insert("Rogue".to_string(), Combatant::default());
        data.entries.push(LogEntry::AuraGain {
            timestamp: 50.0,
            player: "Rogue".to_string(),
            aura: "Elixir of the Mongoose".to_string(),
            stacks: 1,
        });
        data.entries.push(LogEntry::AuraFade {
            timestamp: 150.0,
            player: "Rogue".to_string(),
            aura: "Elixir of the Mongoose".to_string(),
        });

        let (uptimes, _) = data.compute_buff_uptimes(&EncounterFilter::All);
        let rogue = uptimes.get("Rogue").expect("rogue should have uptimes");
        let mongoose = rogue
            .get("Elixir of the Mongoose")
            .expect("mongoose should have uptime");
        assert!((mongoose.fraction - 0.5).abs() < 0.01);
    }
}
