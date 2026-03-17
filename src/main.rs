mod formatter;
mod log_data;
mod log_parser;
mod parser;
mod theme;
mod viewer;

use iced::widget::{
    button, checkbox, column, container, horizontal_rule, horizontal_space, pick_list,
    progress_bar, row, scrollable, text, Column,
};
use iced::{Center, Element, Fill, Task, Theme};
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

fn main() -> iced::Result {
    let args: Vec<String> = std::env::args().collect();

    // Quick benchmark mode: `combat-scribe --bench <file>`
    if args.len() >= 3 && args[1] == "--bench" {
        run_bench(&args[2]);
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

// ── Application State ───────────────────────────────────────────────────────

#[derive(Debug)]
enum AppState {
    /// Waiting for user to select a file.
    Idle,
    /// File loaded, showing sessions to pick from.
    FileLoaded,
    /// Processing the log.
    Processing,
    /// Done.
    Done(DoneInfo),
    /// Error.
    Error(String),
    /// Viewing parsed log data.
    Viewing(Box<viewer::ViewerState>),
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
}

struct App {
    state: AppState,
    file_path: Option<PathBuf>,
    file_name: String,
    lines: Arc<Vec<String>>,
    sessions: Vec<parser::Session>,
    session_names: Vec<String>,
    selected_session: Option<String>,
    create_zip: bool,
    zero_log: bool,
    rename_output: bool,
    progress: f32,
    status_message: String,
}

impl App {
    fn new() -> Self {
        Self {
            state: AppState::Idle,
            file_path: None,
            file_name: String::new(),
            lines: Arc::new(Vec::new()),
            sessions: Vec::new(),
            session_names: Vec::new(),
            selected_session: None,
            create_zip: true,
            zero_log: false,
            rename_output: true,
            progress: 0.0,
            status_message: String::from("Select a WoW combat log file to begin."),
        }
    }

    /// Get the `you_players` for the currently selected session (if any).
    fn selected_you_players(&self) -> Option<&[String]> {
        let sel = self.selected_session.as_ref()?;
        let idx = self.session_names.iter().position(|n| n == sel)?;
        if idx == 0 {
            return None; // "Entire Log" — no specific session
        }
        let session = self.sessions.get(idx - 1)?;
        if session.you_players.is_empty() {
            None
        } else {
            Some(&session.you_players)
        }
    }
}

// ── Messages ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum Message {
    OpenFile,
    FileSelected(Option<PathBuf>),
    FileLoaded(Result<Arc<Vec<String>>, String>),
    SessionsDetected(Vec<parser::Session>),
    SessionSelected(String),
    ToggleZip(bool),
    ToggleZeroLog(bool),
    ToggleRename(bool),
    Export,
    ExportComplete(Result<DoneInfo, String>),
    Reset,
    ViewLog,
    ViewerParsed(Box<log_data::LogData>),
    Viewer(viewer::ViewerMessage),
}

// ── Update ──────────────────────────────────────────────────────────────────

impl App {
    #[allow(clippy::too_many_lines)] // iced update pattern — one match arm per message
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::OpenFile => Task::perform(pick_file(), Message::FileSelected),

            Message::FileSelected(path) => match path {
                Some(p) => {
                    self.file_name = p
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    self.status_message = format!("Loading {}...", self.file_name);
                    self.progress = 0.1;
                    self.file_path = Some(p.clone());

                    Task::perform(async move { load_file(p).await }, Message::FileLoaded)
                }
                None => Task::none(), // User cancelled
            },

            Message::FileLoaded(result) => match result {
                Ok(lines) => {
                    self.lines = Arc::clone(&lines);
                    self.status_message = format!(
                        "Loaded {} ({} lines). Detecting sessions...",
                        self.file_name,
                        self.lines.len()
                    );
                    self.progress = 0.3;

                    Task::perform(
                        async move { parser::detect_sessions(&lines) },
                        Message::SessionsDetected,
                    )
                }
                Err(e) => {
                    self.state = AppState::Error(e.clone());
                    self.status_message = format!("Error: {e}");
                    Task::none()
                }
            },

            Message::SessionsDetected(sessions) => {
                self.sessions = sessions;
                self.session_names = Vec::new();

                // Always include "Entire Log" option
                self.session_names
                    .push(format!("Entire Log ({} lines)", self.lines.len()));

                for s in &self.sessions {
                    self.session_names.push(s.to_string());
                }

                // Default to "Entire Log"
                self.selected_session = Some(self.session_names[0].clone());

                self.state = AppState::FileLoaded;
                self.progress = 0.5;

                self.status_message = if self.sessions.is_empty() {
                    format!(
                        "Loaded {}. No sessions detected - will process entire log.",
                        self.file_name
                    )
                } else {
                    format!(
                        "Found {} session(s). Select a segment and options, then export.",
                        self.sessions.len()
                    )
                };

                Task::none()
            }

            Message::SessionSelected(session) => {
                self.selected_session = Some(session);
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
                    self.state = AppState::Error("No file selected".to_string());
                    self.status_message = "Error: No file selected".to_string();
                    return Task::none();
                };

                self.state = AppState::Processing;
                self.status_message = "Processing log...".to_string();
                self.progress = 0.6;

                let lines = Arc::clone(&self.lines);
                let sessions = self.sessions.clone();
                let selected = self.selected_session.clone();
                let session_names = self.session_names.clone();
                let opts = ExportOptions {
                    file_path,
                    create_zip: self.create_zip,
                    zero_log: self.zero_log,
                    rename_output: self.rename_output,
                };

                Task::perform(
                    async move {
                        do_export(
                            &lines,
                            &sessions,
                            selected.as_deref(),
                            &session_names,
                            &opts,
                        )
                    },
                    Message::ExportComplete,
                )
            }

            Message::ExportComplete(result) => {
                match result {
                    Ok(info) => {
                        self.progress = 1.0;
                        let mut msg =
                            format!("Exported {} lines to {}", info.line_count, info.output_path);
                        if !info.player_names.is_empty() {
                            use std::fmt::Write;
                            let _ = write!(msg, ". Players: {}", info.player_names.join(", "));
                        }
                        if info.zipped {
                            msg.push_str(". Zip created.");
                        }
                        if info.zeroed {
                            msg.push_str(". Original log zeroed.");
                        }
                        self.status_message = msg;
                        self.state = AppState::Done(info);
                    }
                    Err(e) => {
                        self.status_message = format!("Export error: {e}");
                        self.state = AppState::Error(e);
                    }
                }
                Task::none()
            }

            Message::Reset => {
                *self = Self::new();
                Task::none()
            }

            Message::ViewLog => {
                let lines = Arc::clone(&self.lines);
                let sessions = self.sessions.clone();
                let selected = self.selected_session.clone();
                let session_names = self.session_names.clone();

                self.state = AppState::Processing;
                self.status_message = "Parsing log for viewer...".to_string();
                self.progress = 0.6;

                Task::perform(
                    async move {
                        // Extract the session lines
                        let lines_to_extract = selected
                            .and_then(|sel| session_names.iter().position(|n| n == &sel))
                            .map_or_else(
                                || lines.as_ref().clone(),
                                |idx| {
                                    if idx == 0 {
                                        lines.as_ref().clone()
                                    } else {
                                        parser::extract_session_lines(&lines, &sessions[idx - 1])
                                    }
                                },
                            );
                        // Format lines (You/Your replacement, pet attribution,
                        // apostrophe normalization) before parsing
                        let (formatted_lines, _player_names) =
                            formatter::format_log(lines_to_extract);
                        Box::new(log_parser::parse_log(&formatted_lines))
                    },
                    Message::ViewerParsed,
                )
            }

            Message::ViewerParsed(log_data) => {
                self.state = AppState::Viewing(Box::new(viewer::ViewerState::new(*log_data)));
                self.progress = 1.0;
                self.status_message = "Log parsed. Viewing.".to_string();
                Task::none()
            }

            Message::Viewer(viewer_msg) => {
                if matches!(viewer_msg, viewer::ViewerMessage::BackToMain) {
                    self.state = AppState::FileLoaded;
                    self.status_message = format!(
                        "Found {} session(s). Select a segment and options, then export or view.",
                        self.sessions.len()
                    );
                    return Task::none();
                }

                if matches!(viewer_msg, viewer::ViewerMessage::Quit) {
                    return iced::window::get_latest().and_then(iced::window::close);
                }

                if let AppState::Viewing(ref mut viewer_state) = self.state {
                    return viewer_state.update(viewer_msg).map(Message::Viewer);
                }
                Task::none()
            }
        }
    }

    // ── View ────────────────────────────────────────────────────────────────

    fn view(&self) -> Element<'_, Message> {
        let header = text("WoW Log Formatter").size(28);

        let subtitle = text("Format combat logs for upload")
            .size(14)
            .color([0.6, 0.6, 0.6]);

        let header_section = column![header, subtitle].spacing(4).align_x(Center);

        let content: Element<Message> = match &self.state {
            AppState::Idle => self.view_idle(),
            AppState::FileLoaded => self.view_file_loaded(),
            AppState::Processing => self.view_processing(),
            AppState::Done(info) => self.view_done(info),
            AppState::Error(err) => self.view_error(err),
            AppState::Viewing(viewer_state) => {
                return viewer_state.view().map(Message::Viewer);
            }
        };

        let status_bar =
            container(text(&self.status_message).size(12).color([0.5, 0.5, 0.5])).padding(8);

        let layout = column![
            header_section,
            horizontal_rule(1),
            content,
            horizontal_rule(1),
            status_bar,
        ]
        .spacing(16)
        .padding(24)
        .width(Fill);

        container(layout)
            .width(Fill)
            .height(Fill)
            .center_x(Fill)
            .into()
    }

    #[allow(clippy::unused_self)] // iced view pattern
    fn view_idle(&self) -> Element<'_, Message> {
        let open_btn = button(text("Select Log File").size(16).center())
            .on_press(Message::OpenFile)
            .padding([12, 32])
            .width(Fill);

        let hint = text("Supports WoWCombatLog.txt files")
            .size(12)
            .color([0.5, 0.5, 0.5]);

        column![container(
            column![
                text("No file selected").size(16).color([0.6, 0.6, 0.6]),
                open_btn,
                hint,
            ]
            .spacing(16)
            .align_x(Center)
            .width(Fill)
        )
        .padding(40)
        .center_x(Fill)
        .width(Fill),]
        .spacing(16)
        .into()
    }

    fn view_file_loaded(&self) -> Element<'_, Message> {
        let file_label = row![
            text("File:").size(14),
            text(&self.file_name).size(14).color([0.3, 0.8, 0.5]),
            horizontal_space(),
            button(text("Change").size(12))
                .on_press(Message::OpenFile)
                .padding([4, 12]),
        ]
        .spacing(8)
        .align_y(Center);

        // Session picker
        let mut session_section = Column::new().spacing(6);
        session_section = session_section.push(text("Session / Segment").size(14));
        session_section = session_section.push(
            pick_list(
                self.session_names.clone(),
                self.selected_session.clone(),
                Message::SessionSelected,
            )
            .width(Fill)
            .padding(8),
        );

        // Show which player(s) "You/Your" maps to for the selected session
        if let Some(players) = self.selected_you_players() {
            session_section = session_section.push(
                text(format!("You/Your: {}", players.join(", ")))
                    .size(12)
                    .color([0.4, 0.7, 1.0]),
            );
        }

        // Options
        let options_section = column![
            text("Export Options").size(14),
            checkbox("Create ZIP archive", self.create_zip)
                .on_toggle(Message::ToggleZip)
                .size(18),
            checkbox(
                "Rename output to TurtLog-{timestamp}.txt",
                self.rename_output
            )
            .on_toggle(Message::ToggleRename)
            .size(18),
            checkbox("Zero (clear) original log file after export", self.zero_log)
                .on_toggle(Message::ToggleZeroLog)
                .size(18),
        ]
        .spacing(10);

        // Action buttons
        let view_btn = button(text("View Log").size(16).center())
            .on_press(Message::ViewLog)
            .padding([12, 32])
            .width(Fill);

        let export_btn = button(text("Export").size(16).center())
            .on_press(Message::Export)
            .padding([12, 32])
            .width(Fill);

        let action_row = row![view_btn, export_btn].spacing(12).width(Fill);

        let warning: Element<Message> = if self.zero_log {
            text("The original log file will be emptied after export.")
                .size(12)
                .color([1.0, 0.6, 0.2])
                .into()
        } else {
            column![].into()
        };

        scrollable(
            column![
                file_label,
                horizontal_rule(1),
                session_section,
                horizontal_rule(1),
                options_section,
                warning,
                horizontal_rule(1),
                action_row,
            ]
            .spacing(16)
            .width(Fill),
        )
        .into()
    }

    fn view_processing(&self) -> Element<'_, Message> {
        column![
            text("Processing...").size(18),
            progress_bar(0.0..=1.0, self.progress).width(Fill),
            text("Formatting log entries and applying replacements...")
                .size(12)
                .color([0.5, 0.5, 0.5]),
        ]
        .spacing(16)
        .padding(40)
        .align_x(Center)
        .width(Fill)
        .into()
    }

    #[allow(clippy::unused_self)] // iced view pattern
    fn view_done<'a>(&self, info: &'a DoneInfo) -> Element<'a, Message> {
        let mut details = Column::new().spacing(6);

        details = details.push(
            row![
                text("Output:").size(14),
                text(&info.output_path).size(14).color([0.3, 0.8, 0.5]),
            ]
            .spacing(8),
        );

        details = details.push(
            row![
                text("Lines:").size(14),
                text(info.line_count.to_string()).size(14),
            ]
            .spacing(8),
        );

        if !info.player_names.is_empty() {
            details = details.push(
                row![
                    text("Players:").size(14),
                    text(info.player_names.join(", "))
                        .size(14)
                        .color([0.4, 0.7, 1.0]),
                ]
                .spacing(8),
            );
        }

        if info.zipped {
            details = details.push(text("ZIP archive created").size(14).color([0.3, 0.8, 0.5]));
        }

        if info.zeroed {
            details = details.push(
                text("Original log file cleared")
                    .size(14)
                    .color([1.0, 0.6, 0.2]),
            );
        }

        let reset_btn = button(text("Process Another File").size(16).center())
            .on_press(Message::Reset)
            .padding([12, 32])
            .width(Fill);

        column![
            text("Export Complete!").size(22).color([0.3, 0.8, 0.5]),
            horizontal_rule(1),
            details,
            horizontal_rule(1),
            reset_btn,
        ]
        .spacing(16)
        .padding(20)
        .width(Fill)
        .into()
    }

    #[allow(clippy::unused_self)] // iced view pattern
    fn view_error<'a>(&self, err: &'a str) -> Element<'a, Message> {
        let reset_btn = button(text("Start Over").size(16).center())
            .on_press(Message::Reset)
            .padding([12, 32])
            .width(Fill);

        column![
            text("Error").size(22).color([1.0, 0.3, 0.3]),
            text(err).size(14).color([1.0, 0.5, 0.5]),
            reset_btn,
        ]
        .spacing(16)
        .padding(20)
        .width(Fill)
        .into()
    }
}

// ── Async helpers ───────────────────────────────────────────────────────────

async fn pick_file() -> Option<PathBuf> {
    let file = rfd::AsyncFileDialog::new()
        .add_filter("Combat Log", &["txt", "zip"])
        .add_filter("All Files", &["*"])
        .set_title("Select WoW Combat Log")
        .pick_file()
        .await;

    file.map(|f| f.path().to_path_buf())
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

/// Export pipeline — runs synchronous I/O (no async needed).
fn do_export(
    all_lines: &[String],
    sessions: &[parser::Session],
    selected: Option<&str>,
    session_names: &[String],
    opts: &ExportOptions,
) -> Result<DoneInfo, String> {
    // Determine which lines to process — format_log takes ownership
    let lines_to_process = selected
        .and_then(|sel| session_names.iter().position(|n| n == sel))
        .map_or_else(
            || all_lines.to_vec(),
            |idx| {
                if idx == 0 {
                    // "Entire Log" selected
                    all_lines.to_vec()
                } else {
                    // Session index is idx - 1 — only copies the session's lines
                    parser::extract_session_lines(all_lines, &sessions[idx - 1])
                }
            },
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
        let ts = chrono::Local::now().format("%Y-%m-%dT%H-%M").to_string();
        parent.join(format!("TurtLog-{ts}.txt"))
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
