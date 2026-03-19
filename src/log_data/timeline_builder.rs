//! Timeline construction: bucketing events into 1-second intervals with aura tracking.

use std::collections::{HashMap, HashSet};

use super::timeline::{
    AuraInterval, ConsumeMark, DispelMark, TimelineBucket, TimelineData, TimelineEvent,
    TimelineEventKind,
};
use super::types::{
    Combatant, ConsumableCategory, Encounter, EncounterFilter, LogData, LogEntry,
};

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
        let (consume_marks, consume_duration, consume_aura_categories, available_consume_categories) =
            Self::build_consume_marks(
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
        }
    }

    /// Build aura intervals and a sorted list of available aura names for the
    /// selected encounter(s).
    ///
    /// Scans `AuraGain`/`AuraFade` entries within encounter windows, pairing
    /// gains with their corresponding fades per player. Unclosed auras are
    /// clamped to the encounter segment end.
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
                    _ => {}
                }
            }

            // Close any unclosed auras at the end of this encounter segment
            let segment_end = offset_base + enc_duration;
            for ((player, aura), starts) in &mut open {
                for start in starts.drain(..) {
                    intervals
                        .entry(aura.clone())
                        .or_default()
                        .push(AuraInterval {
                            player: player.clone(),
                            start,
                            end: segment_end.min(total_duration),
                        });
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
        /// Pre-pull buffer: include consumables used up to 5 minutes before
        /// an encounter starts to capture pre-potting and food buffs.
        const PRE_PULL_BUFFER: f64 = 300.0;

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
            let earliest = encounters
                .iter()
                .map(|e| e.start)
                .fold(f64::INFINITY, f64::min);
            let latest = encounters
                .iter()
                .map(|e| e.end)
                .fold(f64::NEG_INFINITY, f64::max);
            ((earliest - PRE_PULL_BUFFER).max(t_session_start), latest)
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
        const PRE_PULL_BUFFER: f64 = 300.0;

        let Some(t_session_start) = session_start else {
            return Vec::new();
        };

        // Determine the consume timeline's t_start (must match build_consume_marks)
        let t_start = if selected_encounters.is_empty() {
            t_session_start
        } else {
            let earliest = selected_encounters
                .iter()
                .map(|e| e.start)
                .fold(f64::INFINITY, f64::min);
            (earliest - PRE_PULL_BUFFER).max(t_session_start)
        };

        // For "All Encounters", show all boss encounters.
        // For specific filters, show the selected encounters.
        let encounters_to_show: Vec<&Encounter> =
            if matches!(filter, EncounterFilter::All) {
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
}
