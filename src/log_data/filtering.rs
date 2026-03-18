use std::borrow::Cow;
use std::collections::HashMap;

use super::types::{
    ConsumableUse, DeathEvent, DispelEvent, Encounter, EncounterFilter, InterruptEvent, LogData,
    LogEntry, OpenerSpell, PlayerEventType, PlayerStats, ResurrectEvent,
};

// ── Filtering Methods ───────────────────────────────────────────────────────

impl LogData {
    /// Get encounters matching the current filter.
    pub fn selected_encounters(&self, filter: &EncounterFilter) -> Vec<&Encounter> {
        match filter {
            EncounterFilter::All => self.encounters.iter().collect(),
            EncounterFilter::AllKills => self
                .encounters
                .iter()
                .filter(|e| e.is_boss && e.is_kill)
                .collect(),
            EncounterFilter::AllWipes => self
                .encounters
                .iter()
                .filter(|e| e.is_boss && !e.is_kill)
                .collect(),
            EncounterFilter::AllTrash => self.encounters.iter().filter(|e| !e.is_boss).collect(),
            EncounterFilter::Single(idx) => self.encounters.get(*idx).into_iter().collect(),
        }
    }

    /// Check if a timestamp falls within any selected encounter.
    pub fn is_in_selection(&self, timestamp: f64, filter: &EncounterFilter) -> bool {
        if matches!(filter, EncounterFilter::All) {
            return self.start_time.is_some_and(|s| timestamp >= s)
                && self.end_time.is_some_and(|e| timestamp <= e);
        }
        self.selected_encounters(filter)
            .iter()
            .any(|enc| timestamp >= enc.start && timestamp <= enc.end)
    }

    /// Get total combat duration for selected encounters.
    pub fn selected_duration(&self, filter: &EncounterFilter) -> f64 {
        self.selected_encounters(filter)
            .iter()
            .map(|e| e.duration)
            .sum()
    }

    /// Recalculate player stats filtered to selected encounters.
    ///
    /// Returns `(stats_map, total_duration)`. For `EncounterFilter::All`, borrows
    /// the existing stats to avoid a full clone.
    pub fn filtered_stats(
        &self,
        filter: &EncounterFilter,
    ) -> (Cow<'_, HashMap<String, PlayerStats>>, f64) {
        let duration = self.selected_duration(filter);

        if matches!(filter, EncounterFilter::All) {
            return (Cow::Borrowed(&self.player_stats), duration);
        }

        let mut stats: HashMap<String, PlayerStats> = HashMap::new();

        for entry in &self.entries {
            if !self.is_in_selection(entry.timestamp(), filter) {
                continue;
            }

            match entry {
                LogEntry::Damage {
                    source,
                    target,
                    spell,
                    amount,
                    absorbed,
                    resisted,
                    blocked,
                    is_crit,
                    is_crushing,
                    is_glancing,
                    ..
                } => {
                    let ps = stats.entry(source.clone()).or_default();
                    ps.damage += amount;
                    let ab = ps.abilities.entry(spell.clone()).or_default();
                    ab.total += amount;
                    ab.hits += 1;
                    if *is_crit {
                        ab.crits += 1;
                    }

                    // Damage taken — total and per-source per-ability breakdown
                    let ts = stats.entry(target.clone()).or_default();
                    ts.damage_taken += amount;
                    let dt_ab = ts
                        .damage_taken_breakdown
                        .entry(source.clone())
                        .or_default()
                        .entry(spell.clone())
                        .or_default();
                    dt_ab.total += amount;
                    dt_ab.hits += 1;
                    dt_ab.absorbed += absorbed;
                    dt_ab.resisted += resisted;
                    dt_ab.blocked += blocked;
                    if *is_crit {
                        dt_ab.crits += 1;
                    }
                    if *is_crushing {
                        dt_ab.crushing_hits += 1;
                    }
                    if *is_glancing {
                        dt_ab.glancing_hits += 1;
                    }
                }
                LogEntry::Healing {
                    source,
                    target,
                    spell,
                    amount,
                    effective_heal,
                    overheal,
                    is_crit,
                    ..
                } => {
                    // Only credit player healing stats when the target is a
                    // known player.  Boss-targeted heals (Shadow of Ebonroc,
                    // Blood Siphon, etc.) are excluded — matching the gate
                    // in parse_healing_events().
                    if !self.all_combatants.contains_key(target.as_str()) {
                        continue;
                    }
                    let ps = stats.entry(source.clone()).or_default();
                    ps.healing += amount;
                    ps.effective_healing += effective_heal;
                    ps.overhealing += overheal;
                    let ab = ps.healing_abilities.entry(spell.clone()).or_default();
                    ab.total += amount;
                    ab.effective += effective_heal;
                    ab.overheal += overheal;
                    ab.hits += 1;
                    if *is_crit {
                        ab.crits += 1;
                    }
                }
                _ => {}
            }
        }

        (Cow::Owned(stats), duration)
    }

    /// Filter any timestamped event collection to the current encounter selection.
    fn filtered_by_time<'a, T>(
        &self,
        events: &'a [T],
        get_ts: impl Fn(&T) -> f64,
        filter: &EncounterFilter,
    ) -> Vec<&'a T> {
        events
            .iter()
            .filter(|e| self.is_in_selection(get_ts(e), filter))
            .collect()
    }

    /// Filter deaths to selection.
    pub fn filtered_deaths(&self, filter: &EncounterFilter) -> Vec<&DeathEvent> {
        self.filtered_by_time(&self.deaths, |d| d.timestamp, filter)
    }

    /// Filter dispels to selection.
    pub fn filtered_dispels(&self, filter: &EncounterFilter) -> Vec<&DispelEvent> {
        self.filtered_by_time(&self.dispels, |d| d.timestamp, filter)
    }

    /// Filter resurrects to selection.
    pub fn filtered_resurrects(&self, filter: &EncounterFilter) -> Vec<&ResurrectEvent> {
        self.filtered_by_time(&self.resurrects, |r| r.timestamp, filter)
    }

    /// Filter interrupts to selection.
    pub fn filtered_interrupts(&self, filter: &EncounterFilter) -> Vec<&InterruptEvent> {
        self.filtered_by_time(&self.interrupts, |i| i.timestamp, filter)
    }

    /// Get the opener sequence for a player (first N abilities in first 10s).
    pub fn opener_sequence(
        &self,
        player: &str,
        event_type: PlayerEventType,
        filter: &EncounterFilter,
    ) -> Vec<OpenerSpell> {
        let limit = 8;
        let window_secs = 10.0;

        // Collect matching entries
        let mut matching: Vec<(&str, f64, u64, bool)> = Vec::new();
        for entry in &self.entries {
            if !self.is_in_selection(entry.timestamp(), filter) {
                continue;
            }
            match entry {
                LogEntry::Damage {
                    source,
                    spell,
                    amount,
                    is_crit,
                    timestamp,
                    ..
                } if event_type == PlayerEventType::Damage && source == player => {
                    matching.push((spell, *timestamp, *amount, *is_crit));
                }
                LogEntry::Healing {
                    source,
                    spell,
                    amount,
                    is_crit,
                    timestamp,
                    ..
                } if event_type == PlayerEventType::Healing && source == player => {
                    matching.push((spell, *timestamp, *amount, *is_crit));
                }
                _ => {}
            }
        }

        if matching.is_empty() {
            return Vec::new();
        }

        matching.sort_by(|a, b| a.1.total_cmp(&b.1));
        let first_time = matching[0].1;

        matching
            .iter()
            .filter(|(_, ts, _, _)| *ts <= first_time + window_secs)
            .take(limit)
            .map(|(spell, ts, amount, is_crit)| OpenerSpell {
                spell: spell.to_string(),
                delay: if (*ts - first_time).abs() < f64::EPSILON {
                    0.0
                } else {
                    *ts - first_time
                },
                amount: *amount,
                is_crit: *is_crit,
            })
            .collect()
    }

    /// Filter consumable uses to selection.
    pub fn filtered_consumables(&self, filter: &EncounterFilter) -> Vec<&ConsumableUse> {
        self.filtered_by_time(&self.consumables, |c| c.timestamp, filter)
    }

    /// Get the class of a player, or "UNKNOWN".
    pub fn player_class(&self, name: &str) -> &str {
        self.combatants
            .get(name)
            .map_or("UNKNOWN", |c| c.class.as_str())
    }
}
