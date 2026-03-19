//! Encounter-aware query methods on `LogData` for stats, deaths, dispels, and openers.

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
    ///
    /// Uses lazy iterators to avoid the `Vec` allocation that
    /// `selected_encounters()` would perform on every call.
    pub fn is_in_selection(&self, timestamp: f64, filter: &EncounterFilter) -> bool {
        let in_range = |enc: &Encounter| timestamp >= enc.start && timestamp <= enc.end;
        match filter {
            EncounterFilter::All => {
                self.start_time.is_some_and(|s| timestamp >= s)
                    && self.end_time.is_some_and(|e| timestamp <= e)
            }
            EncounterFilter::Single(idx) => self
                .encounters
                .get(*idx)
                .is_some_and(|enc| timestamp >= enc.start && timestamp <= enc.end),
            EncounterFilter::AllKills => self
                .encounters
                .iter()
                .filter(|e| e.is_boss && e.is_kill)
                .any(in_range),
            EncounterFilter::AllWipes => self
                .encounters
                .iter()
                .filter(|e| e.is_boss && !e.is_kill)
                .any(in_range),
            EncounterFilter::AllTrash => {
                self.encounters.iter().filter(|e| !e.is_boss).any(in_range)
            }
        }
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
                    is_pet_spell,
                    ..
                } => {
                    let ps = stats.entry(source.clone()).or_default();
                    ps.accumulate_damage(spell, *amount, *is_crit);

                    // Mirror the add_damage() logic: inherit pet status from
                    // the base spell for (dot) variants of summoned-entity spells.
                    let effective_pet = *is_pet_spell || {
                        let base = spell.strip_suffix(" (dot)").unwrap_or(spell);
                        base != spell.as_str() && ps.abilities.get(base).is_some_and(|a| a.is_pet)
                    };
                    if effective_pet {
                        ps.pet_damage += amount;
                        ps.abilities.entry(spell.clone()).or_default().is_pet = true;
                        if *is_pet_spell {
                            let dot_key = format!("{spell} (dot)");
                            if let Some(dot_ab) = ps.abilities.get_mut(&dot_key)
                                && !dot_ab.is_pet
                            {
                                dot_ab.is_pet = true;
                                ps.pet_damage += dot_ab.total;
                            }
                        }
                    }

                    stats
                        .entry(target.clone())
                        .or_default()
                        .accumulate_damage_taken(
                            source,
                            spell,
                            *amount,
                            *absorbed,
                            *resisted,
                            *blocked,
                            *is_crit,
                            *is_crushing,
                            *is_glancing,
                        );
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
                    stats.entry(source.clone()).or_default().accumulate_healing(
                        spell,
                        *amount,
                        *effective_heal,
                        *overheal,
                        *is_crit,
                    );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log_data::types::Combatant;

    /// Build a minimal `LogData` with the given encounters.
    fn make_log_data(encounters: Vec<Encounter>) -> LogData {
        let start = encounters.first().map(|e| e.start);
        let end = encounters.last().map(|e| e.end);
        LogData {
            encounters,
            start_time: start,
            end_time: end,
            ..LogData::default()
        }
    }

    fn boss_kill(name: &str, start: f64, end: f64) -> Encounter {
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
            active_players: 0,
        }
    }

    fn boss_wipe(name: &str, start: f64, end: f64) -> Encounter {
        Encounter {
            is_kill: false,
            ..boss_kill(name, start, end)
        }
    }

    fn trash(start: f64, end: f64) -> Encounter {
        Encounter {
            name: None,
            start,
            end,
            duration: end - start,
            is_boss: false,
            is_kill: false,
            zone: None,
            attempt: None,
            player_deaths: 0,
            active_players: 0,
        }
    }

    fn damage_entry(ts: f64, source: &str, target: &str, spell: &str, amount: u64) -> LogEntry {
        LogEntry::Damage {
            timestamp: ts,
            source: source.to_string(),
            target: target.to_string(),
            spell: spell.to_string(),
            amount,
            absorbed: 0,
            resisted: 0,
            blocked: 0,
            is_crit: false,
            is_glancing: false,
            is_crushing: false,
            school: None,
            is_pet_spell: false,
            is_fully_resisted: false,
            is_fully_absorbed: false,
            is_fully_blocked: false,
        }
    }

    fn healing_entry(ts: f64, source: &str, target: &str, spell: &str, amount: u64) -> LogEntry {
        LogEntry::Healing {
            timestamp: ts,
            source: source.to_string(),
            target: target.to_string(),
            spell: spell.to_string(),
            amount,
            effective_heal: amount,
            overheal: 0,
            is_crit: false,
        }
    }

    // ── selected_encounters ─────────────────────────────────────────────

    #[test]
    fn test_selected_encounters_filters() {
        let data = make_log_data(vec![
            boss_kill("Ragnaros", 100.0, 200.0),
            boss_wipe("Vael", 300.0, 350.0),
            trash(400.0, 420.0),
        ]);

        assert_eq!(data.selected_encounters(&EncounterFilter::All).len(), 3);
        assert_eq!(
            data.selected_encounters(&EncounterFilter::AllKills).len(),
            1
        );
        assert_eq!(
            data.selected_encounters(&EncounterFilter::AllWipes).len(),
            1
        );
        assert_eq!(
            data.selected_encounters(&EncounterFilter::AllTrash).len(),
            1
        );
        assert_eq!(
            data.selected_encounters(&EncounterFilter::Single(0)).len(),
            1
        );
        assert_eq!(
            data.selected_encounters(&EncounterFilter::Single(0))[0]
                .name
                .as_deref(),
            Some("Ragnaros")
        );
    }

    #[test]
    fn test_selected_encounters_single_out_of_bounds() {
        let data = make_log_data(vec![boss_kill("Ragnaros", 100.0, 200.0)]);
        assert!(data
            .selected_encounters(&EncounterFilter::Single(99))
            .is_empty());
    }

    // ── is_in_selection ─────────────────────────────────────────────────

    #[test]
    fn test_is_in_selection_all_filter() {
        let data = make_log_data(vec![boss_kill("Rag", 100.0, 200.0)]);
        let filter = EncounterFilter::All;
        assert!(data.is_in_selection(100.0, &filter));
        assert!(data.is_in_selection(150.0, &filter));
        assert!(data.is_in_selection(200.0, &filter));
        assert!(!data.is_in_selection(99.9, &filter));
        assert!(!data.is_in_selection(200.1, &filter));
    }

    #[test]
    fn test_is_in_selection_single_encounter() {
        let data = make_log_data(vec![
            boss_kill("Rag", 100.0, 150.0),
            boss_kill("Ony", 200.0, 250.0),
        ]);
        let filter = EncounterFilter::Single(1);
        assert!(!data.is_in_selection(125.0, &filter));
        assert!(data.is_in_selection(225.0, &filter));
        assert!(!data.is_in_selection(260.0, &filter));
    }

    // ── selected_duration ───────────────────────────────────────────────

    #[test]
    fn test_selected_duration() {
        let data = make_log_data(vec![
            boss_kill("Rag", 100.0, 130.0),
            boss_kill("Ony", 200.0, 245.0),
            trash(300.0, 360.0),
        ]);
        let all_dur = data.selected_duration(&EncounterFilter::All);
        assert!((all_dur - 135.0).abs() < f64::EPSILON);

        let single_dur = data.selected_duration(&EncounterFilter::Single(1));
        assert!((single_dur - 45.0).abs() < f64::EPSILON);
    }

    // ── filtered_stats ──────────────────────────────────────────────────

    #[test]
    fn test_filtered_stats_all_borrows() {
        let mut data = make_log_data(vec![boss_kill("Rag", 100.0, 200.0)]);
        data.player_stats
            .insert("Warrior".to_string(), PlayerStats::default());

        let (stats, _dur) = data.filtered_stats(&EncounterFilter::All);
        assert!(matches!(stats, std::borrow::Cow::Borrowed(_)));
    }

    #[test]
    fn test_filtered_stats_single_encounter() {
        let mut data = make_log_data(vec![
            boss_kill("Rag", 100.0, 200.0),
            boss_kill("Ony", 300.0, 400.0),
        ]);
        data.entries = vec![
            damage_entry(150.0, "Warrior", "Rag", "Mortal Strike", 1000),
            damage_entry(350.0, "Warrior", "Ony", "Mortal Strike", 2000),
        ];
        // Add Warrior as a combatant for healing gate
        data.all_combatants
            .insert("Warrior".to_string(), Combatant::default());

        let (stats, _dur) = data.filtered_stats(&EncounterFilter::Single(0));
        let warrior = stats.get("Warrior").expect("warrior should have stats");
        assert_eq!(warrior.damage, 1000);
    }

    #[test]
    fn test_filtered_stats_skips_boss_heals() {
        let mut data = make_log_data(vec![boss_kill("Rag", 100.0, 200.0)]);
        // "Priest" is a combatant, "Boss" is not
        data.all_combatants
            .insert("Priest".to_string(), Combatant::default());
        data.entries = vec![
            healing_entry(150.0, "Priest", "Priest", "Flash Heal", 500),
            healing_entry(160.0, "Priest", "Boss", "Shadow of Ebonroc", 1000),
        ];

        let (stats, _dur) = data.filtered_stats(&EncounterFilter::Single(0));
        let priest = stats.get("Priest").expect("priest should have stats");
        // Only the self-heal should count (target is a combatant)
        assert_eq!(priest.healing, 500);
    }

    // ── opener_sequence ─────────────────────────────────────────────────

    #[test]
    fn test_opener_sequence_basic() {
        let mut data = make_log_data(vec![boss_kill("Rag", 100.0, 200.0)]);
        data.entries = vec![
            damage_entry(100.0, "Mage", "Rag", "Frostbolt", 300),
            damage_entry(101.5, "Mage", "Rag", "Frostbolt", 320),
            damage_entry(103.0, "Mage", "Rag", "Fireblast", 200),
            damage_entry(115.0, "Mage", "Rag", "Frostbolt", 310), // outside 10s window
        ];

        let opener =
            data.opener_sequence("Mage", PlayerEventType::Damage, &EncounterFilter::Single(0));
        assert_eq!(opener.len(), 3);
        assert_eq!(opener[0].spell, "Frostbolt");
        assert!((opener[0].delay - 0.0).abs() < f64::EPSILON);
        assert!((opener[1].delay - 1.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_opener_sequence_empty_for_unknown_player() {
        let data = make_log_data(vec![boss_kill("Rag", 100.0, 200.0)]);
        let opener = data.opener_sequence(
            "Nobody",
            PlayerEventType::Damage,
            &EncounterFilter::Single(0),
        );
        assert!(opener.is_empty());
    }

    #[test]
    fn test_opener_sequence_limit() {
        let mut data = make_log_data(vec![boss_kill("Rag", 100.0, 200.0)]);
        // 12 entries all within 10s window
        data.entries = (0..12)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)] // test index values are tiny
                let ts = 100.0 + f64::from(i);
                damage_entry(ts, "Mage", "Rag", "Frostbolt", 300)
            })
            .collect();

        let opener =
            data.opener_sequence("Mage", PlayerEventType::Damage, &EncounterFilter::Single(0));
        assert_eq!(opener.len(), 8); // limit is 8
    }

    // ── player_class ────────────────────────────────────────────────────

    #[test]
    fn test_player_class_known_and_unknown() {
        let mut data = LogData::default();
        data.combatants.insert(
            "Warrior".to_string(),
            Combatant {
                class: "WARRIOR".to_string(),
                ..Combatant::default()
            },
        );

        assert_eq!(data.player_class("Warrior"), "WARRIOR");
        assert_eq!(data.player_class("Nobody"), "UNKNOWN");
    }

    // ── filtered_deaths ─────────────────────────────────────────────────

    #[test]
    fn test_filtered_deaths_time_range() {
        let mut data = make_log_data(vec![boss_kill("Rag", 150.0, 200.0)]);
        data.deaths = vec![
            super::DeathEvent {
                timestamp: 100.0,
                player: "Tank".to_string(),
                killer: None,
                killing_blow: None,
                damage_amount: None,
            },
            super::DeathEvent {
                timestamp: 175.0,
                player: "Healer".to_string(),
                killer: None,
                killing_blow: None,
                damage_amount: None,
            },
            super::DeathEvent {
                timestamp: 300.0,
                player: "DPS".to_string(),
                killer: None,
                killing_blow: None,
                damage_amount: None,
            },
        ];

        let deaths = data.filtered_deaths(&EncounterFilter::Single(0));
        assert_eq!(deaths.len(), 1);
        assert_eq!(deaths[0].player, "Healer");
    }
}
