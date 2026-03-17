mod config;
mod formatter;
mod log_data;
mod log_parser;
mod parser;
mod raid_data;
mod theme;
mod viewer;

use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

use iced::widget::{
    button, center, checkbox, column, container, horizontal_rule, horizontal_space, row, stack,
    text, Column,
};
use iced::{Center, Color, Element, Fill, Length, Task, Theme};

fn main() -> iced::Result {
    let args: Vec<String> = std::env::args().collect();

    // Quick benchmark mode: `combat-scribe --bench <file>`
    if args.len() >= 3 && args[1] == "--bench" {
        run_bench(&args[2]);
        return Ok(());
    }

    // Debug session detection: `combat-scribe --debug-sessions <file>`
    if args.len() >= 3 && args[1] == "--debug-sessions" {
        run_debug_sessions(&args[2]);
        return Ok(());
    }

    // Debug wipe detection: `combat-scribe --debug-wipes <file> [session#]`
    if args.len() >= 3 && args[1] == "--debug-wipes" {
        let session_num = args.get(3).and_then(|s| s.parse::<usize>().ok());
        run_debug_wipes(&args[2], session_num);
        return Ok(());
    }

    // If a file path is passed as the first argument, load it immediately
    let initial_file: Option<PathBuf> = if args.len() >= 2 && !args[1].starts_with('-') {
        Some(PathBuf::from(&args[1]))
    } else {
        None
    };

    iced::application("WoW Log Viewer", App::update, App::view)
        .theme(|_| Theme::Dark)
        .window_size((1200.0, 800.0))
        .run_with(move || {
            let app = App::new();
            if let Some(path) = initial_file {
                let p = path.clone();
                (app, Task::done(Message::FileSelected(Some(p))))
            } else {
                (app, Task::none())
            }
        })
}

fn run_bench(path: &str) {
    use std::time::Instant;

    let file_path = std::path::Path::new(path);
    eprintln!("Reading file...");
    let t0 = Instant::now();
    let content = if is_zip_file(file_path) {
        let bytes = fs::read(path).expect("read zip file");
        read_text_from_zip_bytes(&bytes).expect("extract txt from zip")
    } else {
        fs::read_to_string(path).expect("read file")
    };
    let lines: Vec<String> = content.lines().map(str::to_string).collect();
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
fn run_debug_sessions(path: &str) {
    let file_path = std::path::Path::new(path);
    eprintln!("Reading file...");
    let content = if is_zip_file(file_path) {
        let bytes = fs::read(path).expect("read zip file");
        read_text_from_zip_bytes(&bytes).expect("extract txt from zip")
    } else {
        fs::read_to_string(path).expect("read file")
    };
    let lines: Vec<String> = content.lines().map(str::to_string).collect();
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
fn run_debug_wipes(path: &str, session_num: Option<usize>) {
    let file_path = std::path::Path::new(path);
    eprintln!("Reading file...");
    let content = if is_zip_file(file_path) {
        let bytes = fs::read(path).expect("read zip file");
        read_text_from_zip_bytes(&bytes).expect("extract txt from zip")
    } else {
        fs::read_to_string(path).expect("read file")
    };
    let lines: Vec<String> = content.lines().map(str::to_string).collect();

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

// ── Application State ───────────────────────────────────────────────────────

#[derive(Debug)]
enum AppState {
    /// No data loaded — shows welcome screen with Load button.
    Empty,
    /// Loading/parsing a file or switching sessions.
    Loading,
    /// Viewing parsed log data.
    Viewing(Box<viewer::ViewerState>),
    /// Error occurred.
    Error(String),
}

#[derive(Debug, Clone)]
struct DoneInfo {
    output_path: String,
    player_names: Vec<String>,
    line_count: usize,
    zipped: bool,
    zeroed: bool,
}

/// Options passed to the export pipeline.
#[derive(Debug, Clone)]
struct ExportOptions {
    file_path: PathBuf,
    create_zip: bool,
    zero_log: bool,
    rename_output: bool,
    /// Session metadata for descriptive filename (empty when exporting entire log).
    session_player_names: Vec<String>,
    session_zone_name: String,
    session_start_time: f64,
    session_start_year: Option<i32>,
}

#[allow(clippy::struct_excessive_bools)] // export options are independent toggles
struct App {
    state: AppState,
    config: config::AppConfig,
    file_path: Option<PathBuf>,
    file_name: String,
    lines: Arc<Vec<String>>,
    sessions: Vec<parser::Session>,
    session_names: Vec<String>,
    selected_session: Option<String>,

    // Export options
    create_zip: bool,
    zero_log: bool,
    rename_output: bool,

    // Export modal
    show_export_modal: bool,
    export_result: Option<Result<DoneInfo, String>>,
}

impl App {
    fn new() -> Self {
        Self {
            state: AppState::Empty,
            config: config::AppConfig::load(),
            file_path: None,
            file_name: String::new(),
            lines: Arc::new(Vec::new()),
            sessions: Vec::new(),
            session_names: Vec::new(),
            selected_session: None,
            create_zip: true,
            zero_log: false,
            rename_output: true,
            show_export_modal: false,
            export_result: None,
        }
    }

    /// Parse the selected session (or the first session if none selected).
    /// Returns a Task that yields `Message::ViewerParsed`.
    fn parse_session(&self) -> Task<Message> {
        let lines = Arc::clone(&self.lines);
        let sessions = self.sessions.clone();
        let selected_name = self.selected_session.clone();

        Task::perform(
            async move {
                // Find the session matching the selected name.
                // Session names are display strings (Session::to_string()),
                // so match against all sessions regardless of filtering.
                let session_idx = selected_name
                    .and_then(|sel| sessions.iter().position(|s| s.to_string() == sel));
                let lines_to_extract = session_idx.map_or_else(
                    || lines.as_ref().clone(),
                    |idx| parser::extract_session_lines(&lines, &sessions[idx], &sessions),
                );
                // Format lines (You/Your replacement, pet attribution,
                // apostrophe normalization) before parsing
                let (formatted_lines, _player_names) = formatter::format_log(lines_to_extract);
                Box::new(log_parser::parse_log(&formatted_lines))
            },
            Message::ViewerParsed,
        )
    }
}

// ── Messages ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum Message {
    // File loading
    OpenFile,
    FileSelected(Option<PathBuf>),
    FileLoaded(Result<Arc<Vec<String>>, String>),
    SessionsDetected(Vec<parser::Session>),

    // Viewer parsed result
    ViewerParsed(Box<log_data::LogData>),

    // Export modal
    CloseExportModal,
    ToggleZip(bool),
    ToggleZeroLog(bool),
    ToggleRename(bool),
    Export,
    ExportComplete(Result<DoneInfo, String>),
    DismissExportResult,

    // Viewer messages
    Viewer(viewer::ViewerMessage),
}

// ── Update ──────────────────────────────────────────────────────────────────

impl App {
    #[allow(clippy::too_many_lines)] // iced update pattern — one match arm per message
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::OpenFile => {
                let dir = self.config.last_directory.clone();
                Task::perform(pick_file(dir), Message::FileSelected)
            }

            Message::FileSelected(path) => match path {
                Some(p) => {
                    self.config.set_last_directory_from_file(&p);
                    self.config.save();

                    self.file_name = p
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    self.file_path = Some(p.clone());
                    self.state = AppState::Loading;

                    Task::perform(async move { load_file(p).await }, Message::FileLoaded)
                }
                None => Task::none(), // User cancelled
            },

            Message::FileLoaded(result) => match result {
                Ok(lines) => {
                    self.lines = Arc::clone(&lines);
                    self.state = AppState::Loading;

                    Task::perform(
                        async move { parser::detect_sessions(&lines) },
                        Message::SessionsDetected,
                    )
                }
                Err(e) => {
                    self.state = AppState::Error(e);
                    Task::none()
                }
            },

            Message::SessionsDetected(sessions) => {
                self.sessions = sessions;
                // Only show raid/instance sessions in the dropdown —
                // overworld sessions (Orgrimmar, Stranglethorn Vale, etc.)
                // are noise for a combat log viewer.
                self.session_names = self
                    .sessions
                    .iter()
                    .filter(|s| s.is_raid)
                    .map(std::string::ToString::to_string)
                    .collect();

                // Auto-select first raid session
                if let Some(first) = self.session_names.first() {
                    self.selected_session = Some(first.clone());
                }

                if self.session_names.is_empty() {
                    // No sessions detected — parse entire log as one session
                    let lines = Arc::clone(&self.lines);
                    self.state = AppState::Loading;
                    return Task::perform(
                        async move {
                            let (formatted_lines, _player_names) =
                                formatter::format_log(lines.as_ref().clone());
                            Box::new(log_parser::parse_log(&formatted_lines))
                        },
                        Message::ViewerParsed,
                    );
                }

                // Parse the first session automatically
                self.state = AppState::Loading;
                self.parse_session()
            }

            Message::ViewerParsed(log_data) => {
                let mut vs = viewer::ViewerState::new(*log_data);
                // Provide session list to the viewer for the header dropdown
                vs.session_names.clone_from(&self.session_names);
                vs.selected_session_name.clone_from(&self.selected_session);
                // Restore tracked auras from config
                vs.tracked_auras.clone_from(&self.config.tracked_auras);
                self.state = AppState::Viewing(Box::new(vs));
                Task::none()
            }

            // ── Export modal ────────────────────────────────────────────
            Message::CloseExportModal | Message::DismissExportResult => {
                self.show_export_modal = false;
                self.export_result = None;
                Task::none()
            }

            Message::ToggleZip(val) => {
                self.create_zip = val;
                Task::none()
            }

            Message::ToggleZeroLog(val) => {
                self.zero_log = val;
                Task::none()
            }

            Message::ToggleRename(val) => {
                self.rename_output = val;
                Task::none()
            }

            Message::Export => {
                let Some(file_path) = self.file_path.clone() else {
                    self.export_result = Some(Err("No file selected".to_string()));
                    return Task::none();
                };

                let lines = Arc::clone(&self.lines);
                let sessions = self.sessions.clone();
                let selected = self.selected_session.clone();

                // Look up session metadata for descriptive filename.
                let session_idx = self
                    .selected_session
                    .as_ref()
                    .and_then(|sel| self.session_names.iter().position(|n| n == sel));
                let (players, zone, start_time, start_year) = session_idx.map_or_else(
                    || (Vec::new(), String::new(), 0.0, None),
                    |idx| {
                        let s = &self.sessions[idx];
                        (
                            s.you_players.clone(),
                            s.name.clone(),
                            s.start_time,
                            s.start_year,
                        )
                    },
                );

                let opts = ExportOptions {
                    file_path,
                    create_zip: self.create_zip,
                    zero_log: self.zero_log,
                    rename_output: self.rename_output,
                    session_player_names: players,
                    session_zone_name: zone,
                    session_start_time: start_time,
                    session_start_year: start_year,
                };

                Task::perform(
                    async move { do_export(&lines, &sessions, selected.as_deref(), &opts) },
                    Message::ExportComplete,
                )
            }

            Message::ExportComplete(result) => {
                self.export_result = Some(result);
                Task::none()
            }

            // ── Viewer messages ─────────────────────────────────────────
            Message::Viewer(viewer_msg) => {
                // Intercept viewer messages that need app-level handling
                match viewer_msg {
                    viewer::ViewerMessage::LoadFile => {
                        let dir = self.config.last_directory.clone();
                        Task::perform(pick_file(dir), Message::FileSelected)
                    }
                    viewer::ViewerMessage::ShowExport => {
                        self.show_export_modal = true;
                        self.export_result = None;
                        Task::none()
                    }
                    viewer::ViewerMessage::SwitchSession(name) => {
                        self.selected_session = Some(name);
                        self.state = AppState::Loading;
                        self.parse_session()
                    }
                    viewer::ViewerMessage::Quit => {
                        iced::window::get_latest().and_then(iced::window::close)
                    }
                    viewer::ViewerMessage::ToggleAura(_)
                    | viewer::ViewerMessage::ApplyAuraPreset(_)
                    | viewer::ViewerMessage::ClearAuras => {
                        if let AppState::Viewing(ref mut viewer_state) = self.state {
                            let task = viewer_state.update(viewer_msg).map(Message::Viewer);
                            // Persist tracked auras to config after change
                            self.config
                                .tracked_auras
                                .clone_from(&viewer_state.tracked_auras);
                            self.config.save();
                            task
                        } else {
                            Task::none()
                        }
                    }
                    other => {
                        if let AppState::Viewing(ref mut viewer_state) = self.state {
                            viewer_state.update(other).map(Message::Viewer)
                        } else {
                            Task::none()
                        }
                    }
                }
            }
        }
    }

    // ── View ────────────────────────────────────────────────────────────────

    fn view(&self) -> Element<'_, Message> {
        let content: Element<Message> = match &self.state {
            AppState::Empty => self.view_empty(),
            AppState::Loading => self.view_loading(),
            AppState::Viewing(viewer_state) => viewer_state.view().map(Message::Viewer),
            AppState::Error(err) => self.view_error(err),
        };

        // Wrap with export modal overlay if needed
        if self.show_export_modal {
            stack![content, self.view_export_modal()]
                .width(Fill)
                .height(Fill)
                .into()
        } else {
            content
        }
    }

    // ── Empty State (Welcome Screen) ────────────────────────────────────

    #[allow(clippy::unused_self)] // iced view pattern
    fn view_empty(&self) -> Element<'_, Message> {
        let header_row = row![
            button(text("Load File").size(12).color(Color::WHITE))
                .on_press(Message::OpenFile)
                .padding([6, 16])
                .style(header_button_style),
            horizontal_space(),
            button(text("Quit").size(11).color(theme::TEXT_SECONDARY))
                .on_press(Message::Viewer(viewer::ViewerMessage::Quit))
                .style(viewer::transparent_button_style)
                .padding([5, 14]),
        ]
        .spacing(8)
        .align_y(Center)
        .width(Fill);

        let welcome = column![
            text("CombatScribe").size(32).color(Color::WHITE),
            text("WoW Vanilla Combat Log Viewer")
                .size(14)
                .color(theme::TEXT_SECONDARY),
            container(
                button(
                    text("Load Combat Log")
                        .size(16)
                        .center()
                        .color(Color::WHITE)
                )
                .on_press(Message::OpenFile)
                .padding([14, 40])
                .style(primary_button_style)
            )
            .padding([20, 0]),
            text("Supports WoWCombatLog.txt and .zip files")
                .size(12)
                .color(theme::TEXT_MUTED),
        ]
        .spacing(8)
        .align_x(Center);

        column![header_row, center(welcome).width(Fill).height(Fill),]
            .spacing(10)
            .padding(20)
            .width(Fill)
            .height(Fill)
            .into()
    }

    // ── Loading State ───────────────────────────────────────────────────

    #[allow(clippy::unused_self)] // iced view pattern
    fn view_loading(&self) -> Element<'_, Message> {
        let header_row = row![
            button(text("Load File").size(12).color(theme::TEXT_MUTED))
                .padding([6, 16])
                .style(header_button_style),
            horizontal_space(),
        ]
        .spacing(8)
        .align_y(Center)
        .width(Fill);

        let loading = column![
            text("Loading...").size(20).color(Color::WHITE),
            text("Parsing combat log data...")
                .size(13)
                .color(theme::TEXT_SECONDARY),
        ]
        .spacing(8)
        .align_x(Center);

        column![header_row, center(loading).width(Fill).height(Fill),]
            .spacing(10)
            .padding(20)
            .width(Fill)
            .height(Fill)
            .into()
    }

    // ── Error State ─────────────────────────────────────────────────────

    #[allow(clippy::unused_self)] // iced view pattern
    fn view_error<'a>(&self, err: &'a str) -> Element<'a, Message> {
        let header_row = row![
            button(text("Load File").size(12).color(Color::WHITE))
                .on_press(Message::OpenFile)
                .padding([6, 16])
                .style(header_button_style),
            horizontal_space(),
            button(text("Quit").size(11).color(theme::TEXT_SECONDARY))
                .on_press(Message::Viewer(viewer::ViewerMessage::Quit))
                .style(viewer::transparent_button_style)
                .padding([5, 14]),
        ]
        .spacing(8)
        .align_y(Center)
        .width(Fill);

        let error_content = column![
            text("Error").size(24).color(Color::from_rgb(1.0, 0.3, 0.3)),
            text(err).size(14).color(Color::from_rgb(1.0, 0.5, 0.5)),
            button(
                text("Load Another File")
                    .size(14)
                    .center()
                    .color(Color::WHITE)
            )
            .on_press(Message::OpenFile)
            .padding([10, 28])
            .style(primary_button_style),
        ]
        .spacing(12)
        .align_x(Center);

        column![header_row, center(error_content).width(Fill).height(Fill),]
            .spacing(10)
            .padding(20)
            .width(Fill)
            .height(Fill)
            .into()
    }

    /// Build a preview string for the rename checkbox label.
    fn export_filename_preview(&self) -> String {
        let session_idx = self
            .selected_session
            .as_ref()
            .and_then(|sel| self.session_names.iter().position(|n| n == sel));

        if let Some(idx) = session_idx {
            let s = &self.sessions[idx];
            let player_part = if s.you_players.is_empty() {
                "Unknown".to_string()
            } else {
                s.you_players.join("-")
            };
            let raid_part = sanitize_zone_for_filename(&s.name);
            let raid_part = if raid_part.is_empty() {
                "Export".to_string()
            } else {
                raid_part
            };
            let date_part = parser::date_from_session_timestamp(s.start_time, s.start_year);
            format!("Rename to {player_part}-{raid_part}-{date_part}-export.txt")
        } else {
            "Rename output to Player-Raid-Date-export.txt".to_string()
        }
    }

    // ── Export Modal Overlay ─────────────────────────────────────────────

    #[allow(clippy::too_many_lines)] // iced UI layout — modal with conditional result display
    fn view_export_modal(&self) -> Element<'_, Message> {
        let title = text("Export Session").size(18).color(Color::WHITE);

        let session_label = if let Some(ref sel) = self.selected_session {
            text(sel.as_str()).size(12).color(theme::TEXT_SECONDARY)
        } else {
            text("No session selected")
                .size(12)
                .color(theme::TEXT_SECONDARY)
        };

        let rename_label = self.export_filename_preview();

        let options = column![
            checkbox("Create ZIP archive", self.create_zip)
                .on_toggle(Message::ToggleZip)
                .size(18),
            checkbox(rename_label.as_str(), self.rename_output)
                .on_toggle(Message::ToggleRename)
                .size(18),
            checkbox("Zero (clear) original log file after export", self.zero_log)
                .on_toggle(Message::ToggleZeroLog)
                .size(18),
        ]
        .spacing(10);

        let warning: Element<Message> = if self.zero_log {
            text("The original log file will be emptied after export.")
                .size(12)
                .color(Color::from_rgb(1.0, 0.6, 0.2))
                .into()
        } else {
            column![].into()
        };

        // Show export result or action buttons
        let bottom_section: Element<Message> = if let Some(ref result) = self.export_result {
            match result {
                Ok(info) => {
                    let mut details = Column::new().spacing(4);
                    details = details.push(
                        text(format!("Exported {} lines", info.line_count))
                            .size(13)
                            .color(Color::from_rgb(0.3, 0.8, 0.5)),
                    );
                    details = details.push(
                        text(&info.output_path)
                            .size(11)
                            .color(theme::TEXT_SECONDARY),
                    );
                    if !info.player_names.is_empty() {
                        details = details.push(
                            text(format!("Players: {}", info.player_names.join(", ")))
                                .size(11)
                                .color(theme::TEXT_SECONDARY),
                        );
                    }
                    if info.zipped {
                        details = details.push(
                            text("ZIP archive created")
                                .size(11)
                                .color(theme::TEXT_SECONDARY),
                        );
                    }
                    if info.zeroed {
                        details = details.push(
                            text("Original log file cleared")
                                .size(11)
                                .color(Color::from_rgb(1.0, 0.6, 0.2)),
                        );
                    }
                    column![
                        details,
                        button(text("Done").size(14).center().color(Color::WHITE))
                            .on_press(Message::DismissExportResult)
                            .padding([8, 24])
                            .width(Fill)
                            .style(primary_button_style),
                    ]
                    .spacing(12)
                    .into()
                }
                Err(e) => column![
                    text(format!("Export failed: {e}"))
                        .size(13)
                        .color(Color::from_rgb(1.0, 0.3, 0.3)),
                    row![
                        button(text("Cancel").size(14).center())
                            .on_press(Message::CloseExportModal)
                            .padding([8, 24])
                            .width(Fill),
                        button(text("Retry").size(14).center().color(Color::WHITE))
                            .on_press(Message::Export)
                            .padding([8, 24])
                            .width(Fill)
                            .style(primary_button_style),
                    ]
                    .spacing(8),
                ]
                .spacing(12)
                .into(),
            }
        } else {
            row![
                button(text("Cancel").size(14).center())
                    .on_press(Message::CloseExportModal)
                    .padding([8, 24])
                    .width(Fill),
                button(text("Export").size(14).center().color(Color::WHITE))
                    .on_press(Message::Export)
                    .padding([8, 24])
                    .width(Fill)
                    .style(primary_button_style),
            ]
            .spacing(8)
            .into()
        };

        let modal_content = column![
            title,
            session_label,
            horizontal_rule(1),
            options,
            warning,
            horizontal_rule(1),
            bottom_section,
        ]
        .spacing(12)
        .padding(24)
        .width(Length::Fixed(420.0));

        let modal_card = container(modal_content).style(|_theme| container::Style {
            background: Some(iced::Background::Color(Color::from_rgb(0.12, 0.13, 0.15))),
            border: iced::Border {
                color: theme::SURFACE_BORDER,
                width: 1.0,
                radius: 8.0.into(),
            },
            ..Default::default()
        });

        // Semi-transparent backdrop
        let backdrop = button(container("").width(Fill).height(Fill))
            .on_press(Message::CloseExportModal)
            .width(Fill)
            .height(Fill)
            .style(|_theme, _status| button::Style {
                background: Some(iced::Background::Color(Color::from_rgba(
                    0.0, 0.0, 0.0, 0.6,
                ))),
                text_color: Color::TRANSPARENT,
                border: iced::Border::default(),
                shadow: iced::Shadow::default(),
            });

        stack![backdrop, center(modal_card).width(Fill).height(Fill),]
            .width(Fill)
            .height(Fill)
            .into()
    }
}

// ── Button Styles ───────────────────────────────────────────────────────────

/// Header action button style (subtle background with hover).
fn header_button_style(_theme: &iced::Theme, status: button::Status) -> button::Style {
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
    }
}

/// Primary action button style (blue).
fn primary_button_style(_theme: &iced::Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => Color::from_rgb(0.30, 0.45, 0.90),
        _ => Color::from_rgb(0.25, 0.40, 0.85),
    };
    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: Color::WHITE,
        border: iced::Border {
            color: Color::from_rgba(0.35, 0.50, 1.0, 0.3),
            width: 1.0,
            radius: 6.0.into(),
        },
        shadow: iced::Shadow::default(),
    }
}

// ── Async helpers ───────────────────────────────────────────────────────────

async fn pick_file(start_dir: Option<PathBuf>) -> Option<PathBuf> {
    let mut dialog = rfd::AsyncFileDialog::new()
        .add_filter("Combat Log", &["txt", "zip"])
        .add_filter("All Files", &["*"])
        .set_title("Select WoW Combat Log");

    if let Some(dir) = start_dir {
        dialog = dialog.set_directory(dir);
    }

    dialog.pick_file().await.map(|f| f.path().to_path_buf())
}

async fn load_file(path: PathBuf) -> Result<Arc<Vec<String>>, String> {
    let content = if is_zip_file(&path) {
        // Read the raw bytes and extract the first .txt from the zip
        let bytes = tokio::fs::read(&path)
            .await
            .map_err(|e| format!("Failed to read zip file: {e}"))?;
        read_text_from_zip_bytes(&bytes)?
    } else {
        tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| format!("Failed to read file: {e}"))?
    };

    let lines: Vec<String> = content.lines().map(str::to_string).collect();
    Ok(Arc::new(lines))
}

/// Check if a path looks like a zip file (by extension).
fn is_zip_file(path: &std::path::Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
}

/// Extract the first `.txt` file from a zip archive's raw bytes.
fn read_text_from_zip_bytes(bytes: &[u8]) -> Result<String, String> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| format!("Failed to open zip archive: {e}"))?;

    // Find the first .txt entry
    let txt_index = (0..archive.len())
        .find(|&i| {
            archive
                .by_index(i)
                .is_ok_and(|f| f.name().to_lowercase().ends_with(".txt"))
        })
        .ok_or_else(|| "No .txt file found inside zip archive".to_string())?;

    let mut file = archive
        .by_index(txt_index)
        .map_err(|e| format!("Failed to read file from zip: {e}"))?;

    let mut content = String::new();
    file.read_to_string(&mut content)
        .map_err(|e| format!("Failed to read text from zip entry '{}': {e}", file.name()))?;

    Ok(content)
}

/// Strip session name qualifiers and make it filename-safe.
///
/// Removes trailing ` Full Clear`, ` (3/10)`, ` (wipes)` added by
/// `finalize_sessions()`, then drops spaces and non-alphanumeric characters.
fn sanitize_zone_for_filename(session_name: &str) -> String {
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
fn do_export(
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
