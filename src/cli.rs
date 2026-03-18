// ── CLI Diagnostic Modes ────────────────────────────────────────────────────
//
// Non-GUI commands: --bench, --debug-sessions, --debug-wipes.
// These read a file, run the format/parse pipeline, and print to stdout/stderr.

use std::fs;
use std::time::Instant;

use crate::file_io::{is_zip_file, read_text_from_zip_bytes};
use crate::{formatter, log_parser, parser};

/// Read a combat log (plain text or zip) and return its lines.
fn read_log_lines(path: &str) -> Vec<String> {
    let file_path = std::path::Path::new(path);
    let content = if is_zip_file(file_path) {
        let bytes = fs::read(path).expect("read zip file");
        read_text_from_zip_bytes(&bytes).expect("extract txt from zip")
    } else {
        fs::read_to_string(path).expect("read file")
    };
    content.lines().map(str::to_string).collect()
}

pub fn run_bench(path: &str) {
    eprintln!("Reading file...");
    let t0 = Instant::now();
    let lines = read_log_lines(path);
    let read_time = t0.elapsed();
    eprintln!("  Read {} lines in {read_time:.2?}", lines.len());

    eprintln!("Running formatter...");
    let t1 = Instant::now();
    let (formatted, player_names) = formatter::format_log(lines);
    let fmt_time = t1.elapsed();
    eprintln!(
        "  Formatted {} lines in {fmt_time:.2?} ({} players: {})",
        formatted.len(),
        player_names.len(),
        player_names.join(", ")
    );

    eprintln!("Running parser...");
    let t2 = Instant::now();
    let data = log_parser::parse_log(&formatted);
    let parse_time = t2.elapsed();
    eprintln!(
        "  Parsed in {parse_time:.2?}: {} encounters, {} combatants, {} entries",
        data.encounters.len(),
        data.combatants.len(),
        data.entries.len()
    );

    // Show boss encounters
    for enc in &data.encounters {
        if enc.is_boss {
            let result = if enc.is_kill { "Kill" } else { "Wipe" };
            let name = enc.name.as_deref().unwrap_or("Unknown");
            eprintln!("  {name} - {result} - {:.0}s", enc.duration);
        }
    }

    let total = t0.elapsed();
    eprintln!("Total: {total:.2?}");
}

/// Debug session detection — prints session boundaries, line ranges, and per-session parse results.
#[allow(clippy::too_many_lines)] // diagnostic output needs many prints
pub fn run_debug_sessions(path: &str) {
    eprintln!("Reading file...");
    let lines = read_log_lines(path);
    eprintln!("  {} total lines in file\n", lines.len());

    let sessions = parser::detect_sessions(&lines);
    eprintln!("Detected {} sessions:\n", sessions.len());

    // Print session details
    for (i, s) in sessions.iter().enumerate() {
        eprintln!("  Session {}: \"{}\"", i + 1, s.name);
        eprintln!(
            "    Scan window: lines {}..{} ({} lines)",
            s.start_line,
            s.end_line,
            s.end_line.saturating_sub(s.start_line) + 1
        );
        eprintln!(
            "    Time: {:.0}..{:.0} ({:.0}s)",
            s.start_time, s.end_time, s.duration_secs
        );
        eprintln!("    Combat count: {}", s.combat_count);
        if !s.you_players.is_empty() {
            eprintln!("    You: {}", s.you_players.join(", "));
        }
        eprintln!();
    }

    // Parse each session and show details
    for (i, session) in sessions.iter().enumerate() {
        eprintln!("--- Session {} parse details ---", i + 1);
        let session_lines = parser::extract_session_lines(&lines, session, &sessions);
        eprintln!("  Extracted {} lines", session_lines.len());

        let (formatted, player_names) = formatter::format_log(session_lines);
        eprintln!(
            "  Formatted {} lines, players: [{}]",
            formatted.len(),
            player_names.join(", ")
        );

        let data = log_parser::parse_log(&formatted);
        eprintln!("  Zone: \"{}\"", data.zone_name);
        eprintln!(
            "  Combatants: {} (all: {})",
            data.combatants.len(),
            data.all_combatants.len()
        );
        if let Some((in_combat, total)) = data.raid_size {
            eprintln!("  PLAYERS_IN_COMBAT: {in_combat}/{total}");
        }
        eprintln!("  Encounters: {}", data.encounters.len());

        for enc in &data.encounters {
            if enc.is_boss {
                let result = if enc.is_kill { "Kill" } else { "Wipe" };
                let name = enc.name.as_deref().unwrap_or("Unknown");
                let attempt = enc.attempt.map_or(String::new(), |a| format!(" #{a}"));
                let zone = enc.zone.as_deref().unwrap_or("?");
                eprintln!(
                    "    {name} - {result}{attempt} - {:.0}s [zone: {zone}]",
                    enc.duration
                );
            }
        }
        let boss_count = data.encounters.iter().filter(|e| e.is_boss).count();
        let trash_count = data.encounters.iter().filter(|e| !e.is_boss).count();
        eprintln!("  Boss encounters: {boss_count}, Trash: {trash_count}");
        eprintln!();
    }
}

/// Debug encounter/wipe detection for a specific session.
///
/// Usage: `combat-scribe --debug-wipes <file> [session#]`
///
/// If no session number is given, uses the first raid session.
/// Shows every encounter with timestamps, gap to previous, boss detection
/// method, and merge decisions.
#[allow(clippy::too_many_lines)] // diagnostic output
pub fn run_debug_wipes(path: &str, session_num: Option<usize>) {
    eprintln!("Reading file...");
    let lines = read_log_lines(path);

    let sessions = parser::detect_sessions(&lines);
    if sessions.is_empty() {
        eprintln!("No sessions found.");
        return;
    }

    // Pick session: user-specified or first with boss encounters
    let idx = session_num.map_or(0, |n| n.saturating_sub(1));
    if idx >= sessions.len() {
        eprintln!("Session {idx} not found (have {} sessions)", sessions.len());
        return;
    }
    let session = &sessions[idx];
    eprintln!("Analyzing session {}: \"{}\"\n", idx + 1, session.name,);

    let session_lines = parser::extract_session_lines(&lines, session, &sessions);
    let (formatted, _) = formatter::format_log(session_lines);
    let data = log_parser::parse_log(&formatted);

    eprintln!(
        "Zone: \"{}\", {} encounters ({} boss, {} trash)\n",
        data.zone_name,
        data.encounters.len(),
        data.encounters.iter().filter(|e| e.is_boss).count(),
        data.encounters.iter().filter(|e| !e.is_boss).count(),
    );

    let mut prev_end: Option<f64> = None;
    for (i, enc) in data.encounters.iter().enumerate() {
        let gap = prev_end.map_or_else(
            || "  (first)".to_string(),
            |pe| format!("  gap: {:.0}s from prev", enc.start - pe),
        );

        let result = if enc.is_kill { "KILL" } else { "WIPE" };
        let boss_tag = if enc.is_boss { " [BOSS]" } else { "" };
        let name = enc.name.as_deref().unwrap_or("(unnamed)");
        let zone = enc.zone.as_deref().unwrap_or("?");
        let attempt = enc.attempt.map_or(String::new(), |a| format!(" #{a}"));

        eprintln!("  {:>3}. {name} — {result}{attempt}{boss_tag}", i + 1,);
        eprintln!(
            "       duration: {:.0}s, start: {:.0}, end: {:.0}",
            enc.duration, enc.start, enc.end,
        );
        eprintln!(
            "       zone: {zone}, deaths: {}/{} active{gap}",
            enc.player_deaths, enc.active_players
        );

        // Check if this encounter should have merged with the previous one
        if let Some(pe) = prev_end {
            let gap_secs = enc.start - pe;
            if enc.is_boss && gap_secs < 60.0 {
                // Check if the previous encounter was the same boss
                if i > 0 {
                    let prev = &data.encounters[i - 1];
                    if prev.is_boss {
                        let same = prev
                            .name
                            .as_ref()
                            .zip(enc.name.as_ref())
                            .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b));
                        if same {
                            eprintln!(
                                "       *** SHOULD MERGE with prev (same boss, {gap_secs:.0}s gap < 60s) ***"
                            );
                        }
                    }
                }
            }
        }

        prev_end = Some(enc.end);
        eprintln!();
    }
}
