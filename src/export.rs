// ── Export Pipeline ──────────────────────────────────────────────────────────
//
// Exports a parsed session to a formatted .txt file (optionally zipped),
// with optional log zeroing and descriptive filename renaming.

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use crate::{formatter, parser};

/// Options passed to the export pipeline.
#[derive(Debug, Clone)]
pub struct ExportOptions {
    pub file_path: PathBuf,
    pub create_zip: bool,
    pub zero_log: bool,
    pub rename_output: bool,
    /// Session metadata for descriptive filename (empty when exporting entire log).
    pub session_player_names: Vec<String>,
    pub session_zone_name: String,
    pub session_start_time: f64,
    pub session_start_year: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct DoneInfo {
    pub output_path: String,
    pub player_names: Vec<String>,
    pub line_count: usize,
    pub zipped: bool,
    pub zeroed: bool,
}

/// Strip session name qualifiers and make it filename-safe.
///
/// Removes trailing ` Full Clear`, ` (3/10)`, ` (wipes)` added by
/// `finalize_sessions()`, then drops spaces and non-alphanumeric characters.
pub fn sanitize_zone_for_filename(session_name: &str) -> String {
    let base = session_name
        .trim_end_matches(" Full Clear")
        .split(" (")
        .next()
        .unwrap_or(session_name)
        .trim();

    base.chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

/// Export pipeline — runs synchronous I/O (no async needed).
pub fn do_export(
    all_lines: &[String],
    sessions: &[parser::Session],
    selected: Option<&str>,
    opts: &ExportOptions,
) -> Result<DoneInfo, String> {
    // Determine which lines to process.
    // Match selected name against all sessions (not just filtered names).
    let lines_to_process = selected
        .and_then(|sel| sessions.iter().position(|s| s.to_string() == sel))
        .map_or_else(
            || all_lines.to_vec(),
            |idx| parser::extract_session_lines(all_lines, &sessions[idx], sessions),
        );

    // Format the log (takes ownership, applies all replacements)
    let (formatted_lines, player_names) = formatter::format_log(lines_to_process);
    let line_count = formatted_lines.len();

    // Create backup of original file
    let file_stem = opts
        .file_path
        .file_stem()
        .map_or_else(|| "log".to_string(), |s| s.to_string_lossy().to_string());
    let parent = opts
        .file_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));

    let timestamp = chrono::Local::now().format("%s").to_string();
    let backup_name = format!("{file_stem}.original.{timestamp}.txt");
    let backup_path = parent.join(&backup_name);

    fs::copy(&opts.file_path, &backup_path).map_err(|e| format!("Failed to create backup: {e}"))?;

    // Determine output filename
    let output_path = if opts.rename_output {
        let player_part = if opts.session_player_names.is_empty() {
            "Unknown".to_string()
        } else {
            opts.session_player_names.join("-")
        };

        let raid_part = sanitize_zone_for_filename(&opts.session_zone_name);
        let raid_part = if raid_part.is_empty() {
            "Export".to_string()
        } else {
            raid_part
        };

        let date_part =
            parser::date_from_session_timestamp(opts.session_start_time, opts.session_start_year);

        parent.join(format!("{player_part}-{raid_part}-{date_part}-export.txt"))
    } else {
        opts.file_path.clone()
    };

    // Write formatted output
    let content = formatted_lines.join("\n");
    fs::write(&output_path, &content).map_err(|e| format!("Failed to write output: {e}"))?;

    // Optionally create zip
    let zipped = if opts.create_zip {
        let zip_path = output_path.with_extension("txt.zip");
        create_zip_file(&output_path, &zip_path, content.as_bytes())
            .map_err(|e| format!("Failed to create zip: {e}"))?;
        true
    } else {
        false
    };

    // Optionally zero the original log (File::create truncates to zero)
    let zeroed = if opts.zero_log {
        fs::File::create(&opts.file_path).map_err(|e| format!("Failed to zero log: {e}"))?;
        true
    } else {
        false
    };

    Ok(DoneInfo {
        output_path: output_path.display().to_string(),
        player_names,
        line_count,
        zipped,
        zeroed,
    })
}

fn create_zip_file(
    source: &std::path::Path,
    zip_path: &std::path::Path,
    content: &[u8],
) -> Result<(), String> {
    let file = fs::File::create(zip_path).map_err(|e| e.to_string())?;
    let mut zip_writer = zip::ZipWriter::new(file);

    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let source_name = source.file_name().map_or_else(
        || "log.txt".to_string(),
        |n| n.to_string_lossy().to_string(),
    );

    zip_writer
        .start_file(&source_name, options)
        .map_err(|e| e.to_string())?;

    zip_writer.write_all(content).map_err(|e| e.to_string())?;
    zip_writer.finish().map_err(|e| e.to_string())?;

    Ok(())
}
