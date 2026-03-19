mod cli;
mod config;
mod consumable_data;
mod export;
mod file_io;
mod formatter;
mod log_data;
mod log_parser;
mod parser;
mod raid_data;
mod theme;
mod update;
mod viewer;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use iced::keyboard;
use iced::widget::{
    Column, Space, button, center, checkbox, column, container, row, rule, stack, text,
};
use iced::{Center, Color, Element, Fill, Length, Subscription, Task, Theme};

use crate::export::{BatchExportResult, DoneInfo, ExportOptions};
use crate::file_io::{load_file, pick_file};

fn main() -> iced::Result {
    let args: Vec<String> = std::env::args().collect();

    // Quick benchmark mode: `combat-scribe --bench <file>`
    if args.len() >= 3 && args[1] == "--bench" {
        cli::run_bench(&args[2]);
        return Ok(());
    }

    // Debug session detection: `combat-scribe --debug-sessions <file>`
    if args.len() >= 3 && args[1] == "--debug-sessions" {
        cli::run_debug_sessions(&args[2]);
        return Ok(());
    }

    // Debug wipe detection: `combat-scribe --debug-wipes <file> [session#]`
    if args.len() >= 3 && args[1] == "--debug-wipes" {
        let session_num = args.get(3).and_then(|s| s.parse::<usize>().ok());
        cli::run_debug_wipes(&args[2], session_num);
        return Ok(());
    }

    // Batch export: `combat-scribe --export <file> [output_dir] [--zero]`
    if args.len() >= 3 && args[1] == "--export" {
        let zero = args.iter().any(|a| a == "--zero");
        let output_dir = args.get(3).and_then(|a| {
            if a.starts_with('-') {
                None
            } else {
                Some(a.as_str())
            }
        });
        cli::run_export(&args[2], output_dir, zero);
        return Ok(());
    }

    // If a file path is passed as the first argument, load it immediately
    let initial_file: Option<PathBuf> = if args.len() >= 2 && !args[1].starts_with('-') {
        Some(PathBuf::from(&args[1]))
    } else {
        None
    };

    iced::application(
        move || {
            let app = App::new();
            // Always check for updates on startup (runs in background)
            let update_task = Task::perform(
                async { update::check_for_update() },
                Message::UpdateCheckResult,
            );
            let init_task = if let Some(ref path) = initial_file {
                Task::done(Message::FileSelected(Some(path.clone())))
            } else {
                Task::none()
            };
            (app, Task::batch([update_task, init_task]))
        },
        App::update,
        App::view,
    )
    .subscription(App::subscription)
    .title("WoW Log Viewer")
    .theme(|_: &App| Theme::Dark)
    .window_size((1200.0, 800.0))
    .run()
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

    // Loading indicator
    loading_phase: &'static str,
    spinner_tick: usize,

    // Export options
    create_zip: bool,
    zero_log: bool,
    rename_output: bool,
    export_all: bool,

    // Export modal
    show_export_modal: bool,
    export_result: Option<Result<DoneInfo, String>>,
    batch_export_result: Option<Result<BatchExportResult, String>>,

    // Update notification
    update_available: Option<update::NewRelease>,
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
            loading_phase: "",
            spinner_tick: 0,
            create_zip: true,
            zero_log: false,
            rename_output: true,
            export_all: false,
            show_export_modal: false,
            export_result: None,
            batch_export_result: None,
            update_available: None,
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
    ToggleExportAll(bool),
    Export,
    ExportComplete(Result<DoneInfo, String>),
    BatchExportComplete(Result<BatchExportResult, String>),
    DismissExportResult,

    // Update notification
    UpdateCheckResult(Option<update::NewRelease>),
    OpenReleasePage,
    DismissUpdate,
    DismissUpdateForever,

    // Loading spinner
    SpinnerTick,

    // Keyboard
    EscapePressed,

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
                    self.loading_phase = "Reading file...";

                    Task::perform(async move { load_file(p).await }, Message::FileLoaded)
                }
                None => Task::none(), // User cancelled
            },

            Message::FileLoaded(result) => match result {
                Ok(lines) => {
                    self.lines = Arc::clone(&lines);
                    self.state = AppState::Loading;
                    self.loading_phase = "Detecting sessions...";

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
                    self.loading_phase = "Formatting and parsing log...";
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
                self.loading_phase = "Formatting and parsing log...";
                self.parse_session()
            }

            Message::ViewerParsed(log_data) => {
                let mut vs = viewer::ViewerState::new(*log_data, self.config.view.as_ref());
                // Provide session list to the viewer for the header dropdown
                vs.session_names.clone_from(&self.session_names);
                vs.selected_session_name.clone_from(&self.selected_session);
                // Restore tracked auras from config
                vs.tracked_auras.clone_from(&self.config.tracked_auras);
                self.state = AppState::Viewing(Box::new(vs));
                Task::none()
            }

            // ── Keyboard shortcuts ───────────────────────────────────────
            Message::EscapePressed => {
                // Close the topmost overlay: export modal takes priority over detail view
                if self.show_export_modal {
                    self.show_export_modal = false;
                    self.export_result = None;
                    self.batch_export_result = None;
                    self.export_all = false;
                } else if let AppState::Viewing(ref mut vs) = self.state {
                    vs.detail = None;
                }
                Task::none()
            }

            // ── Export modal ────────────────────────────────────────────
            Message::CloseExportModal | Message::DismissExportResult => {
                self.show_export_modal = false;
                self.export_result = None;
                self.batch_export_result = None;
                self.export_all = false;
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

            Message::ToggleExportAll(val) => {
                self.export_all = val;
                Task::none()
            }

            Message::Export => {
                let Some(file_path) = self.file_path.clone() else {
                    self.export_result = Some(Err("No file selected".to_string()));
                    return Task::none();
                };

                let lines = Arc::clone(&self.lines);
                let sessions = self.sessions.clone();

                // Batch export: all raid sessions as individual zips
                if self.export_all {
                    let output_dir = file_path
                        .parent()
                        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
                    let zero_log = self.zero_log;
                    return Task::perform(
                        async move {
                            export::do_batch_export(
                                &lines,
                                &sessions,
                                &output_dir,
                                zero_log,
                                Some(&file_path),
                            )
                        },
                        Message::BatchExportComplete,
                    );
                }

                // Single session export
                let selected = self.selected_session.clone();

                // Look up session metadata for descriptive filename.
                // Search unfiltered `sessions` directly (session_names is raid-filtered).
                let session_idx = self
                    .selected_session
                    .as_ref()
                    .and_then(|sel| self.sessions.iter().position(|s| s.to_string() == *sel));
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
                    async move { export::do_export(&lines, &sessions, selected.as_deref(), &opts) },
                    Message::ExportComplete,
                )
            }

            Message::ExportComplete(result) => {
                self.export_result = Some(result);
                Task::none()
            }

            Message::BatchExportComplete(result) => {
                self.batch_export_result = Some(result);
                Task::none()
            }

            // ── Update notification ──────────────────────────────────────
            Message::UpdateCheckResult(release) => {
                if let Some(ref info) = release {
                    // Don't show if user dismissed this exact version
                    if self.config.dismissed_version.as_deref() == Some(&info.version) {
                        return Task::none();
                    }
                }
                self.update_available = release;
                Task::none()
            }

            Message::OpenReleasePage => {
                update::open_release_page();
                Task::none()
            }

            Message::DismissUpdate => {
                self.update_available = None;
                Task::none()
            }

            Message::DismissUpdateForever => {
                if let Some(ref info) = self.update_available {
                    self.config.dismissed_version = Some(info.version.clone());
                    self.config.save();
                }
                self.update_available = None;
                Task::none()
            }

            // ── Loading spinner ──────────────────────────────────────────
            Message::SpinnerTick => {
                self.spinner_tick = self.spinner_tick.wrapping_add(1);
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
                        self.loading_phase = "Switching session...";
                        self.parse_session()
                    }
                    viewer::ViewerMessage::SaveViewPrefs => {
                        if let AppState::Viewing(ref mut viewer_state) = self.state {
                            // Snapshot current view prefs into config
                            self.config.view = Some(config::ViewPrefs {
                                damage_type: viewer_state.damage_type.to_config_key().to_string(),
                                healing_type: viewer_state.healing_type.to_config_key().to_string(),
                                damage_per_second: false,
                                healing_per_second: false,
                                default_tab: viewer_state.current_tab.to_config_key().to_string(),
                            });
                            self.config.save();
                            // Clear dirty flag in viewer
                            viewer_state.update(viewer_msg).map(Message::Viewer)
                        } else {
                            Task::none()
                        }
                    }
                    viewer::ViewerMessage::Quit => iced::exit(),
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

    // ── Subscription ────────────────────────────────────────────────────────

    fn subscription(&self) -> Subscription<Message> {
        let spinner = if matches!(self.state, AppState::Loading) {
            iced::time::every(std::time::Duration::from_millis(100)).map(|_| Message::SpinnerTick)
        } else {
            Subscription::none()
        };

        // Escape key closes the topmost overlay (export modal > detail view)
        let keys = keyboard::listen().filter_map(|event| match event {
            keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::Escape),
                ..
            } => Some(Message::EscapePressed),
            _ => None,
        });

        Subscription::batch([spinner, keys])
    }

    // ── View ────────────────────────────────────────────────────────────────

    fn view(&self) -> Element<'_, Message> {
        let content: Element<Message> = match &self.state {
            AppState::Empty => self.view_empty(),
            AppState::Loading => self.view_loading(),
            AppState::Viewing(viewer_state) => viewer_state.view().map(Message::Viewer),
            AppState::Error(err) => self.view_error(err),
        };

        // Prepend update banner if a new version is available
        let content = if let Some(ref release) = self.update_available {
            column![self.view_update_banner(release), content]
                .width(Fill)
                .height(Fill)
                .into()
        } else {
            content
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
            Space::new().width(Fill),
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

    fn view_loading(&self) -> Element<'_, Message> {
        let header_row = row![
            button(text("Load File").size(12).color(theme::TEXT_MUTED))
                .padding([6, 16])
                .style(header_button_style),
            Space::new().width(Fill),
        ]
        .spacing(8)
        .align_y(Center)
        .width(Fill);

        let spinner_frames = ["   ", ".  ", ".. ", "..."];
        let frame = spinner_frames[self.spinner_tick % spinner_frames.len()];

        let file_hint = if self.file_name.is_empty() {
            String::new()
        } else {
            self.file_name.clone()
        };

        let loading = column![
            text(format!("Loading{frame}")).size(20).color(Color::WHITE),
            text(self.loading_phase)
                .size(13)
                .color(theme::TEXT_SECONDARY),
            text(file_hint).size(11).color(theme::TEXT_MUTED),
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
            Space::new().width(Fill),
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

    // ── Update Banner ────────────────────────────────────────────────────

    #[allow(clippy::unused_self)] // iced view pattern — method for consistency
    fn view_update_banner(&self, release: &update::NewRelease) -> Element<'_, Message> {
        let label = text(format!(
            "CombatScribe v{} is available  (you have v{})",
            release.version,
            update::CURRENT_VERSION,
        ))
        .size(13)
        .color(Color::WHITE);

        let download_btn = button(text("Download").size(12).color(Color::WHITE))
            .on_press(Message::OpenReleasePage)
            .padding([4, 14])
            .style(update_download_button_style);

        let dismiss_btn = button(text("Dismiss").size(11).color(theme::TEXT_SECONDARY))
            .on_press(Message::DismissUpdate)
            .style(viewer::transparent_button_style)
            .padding([4, 10]);

        let skip_btn = button(text("Skip this version").size(11).color(theme::TEXT_MUTED))
            .on_press(Message::DismissUpdateForever)
            .style(viewer::transparent_button_style)
            .padding([4, 10]);

        let banner = row![
            label,
            Space::new().width(Fill),
            download_btn,
            dismiss_btn,
            skip_btn,
        ]
        .spacing(8)
        .align_y(Center)
        .width(Fill);

        container(banner)
            .padding([8, 16])
            .width(Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(Color::from_rgb(0.15, 0.25, 0.45))),
                border: iced::Border {
                    color: Color::from_rgb(0.25, 0.40, 0.85),
                    width: 1.0,
                    radius: 4.0.into(),
                },
                ..Default::default()
            })
            .into()
    }

    /// Build a preview string for the rename checkbox label.
    fn export_filename_preview(&self) -> String {
        // Search unfiltered `sessions` directly (session_names is raid-filtered).
        let session_idx = self
            .selected_session
            .as_ref()
            .and_then(|sel| self.sessions.iter().position(|s| s.to_string() == *sel));

        if let Some(idx) = session_idx {
            let s = &self.sessions[idx];
            let player_part = if s.you_players.is_empty() {
                "Unknown".to_string()
            } else {
                s.you_players.join("-")
            };
            let raid_part = export::sanitize_zone_for_filename(&s.name);
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
        let title = if self.export_all {
            text("Export All Sessions").size(18).color(Color::WHITE)
        } else {
            text("Export Session").size(18).color(Color::WHITE)
        };

        let session_label: Element<Message> = if self.export_all {
            let raid_count = self.sessions.iter().filter(|s| s.is_raid).count();
            text(format!(
                "{raid_count} raid sessions will be exported as individual zip files"
            ))
            .size(12)
            .color(theme::TEXT_SECONDARY)
            .into()
        } else if let Some(ref sel) = self.selected_session {
            text(sel.as_str())
                .size(12)
                .color(theme::TEXT_SECONDARY)
                .into()
        } else {
            text("No session selected")
                .size(12)
                .color(theme::TEXT_SECONDARY)
                .into()
        };

        let rename_label = self.export_filename_preview();

        let export_all_label = format!(
            "Export all raid sessions ({} sessions)",
            self.sessions.iter().filter(|s| s.is_raid).count()
        );

        let mut options = Column::new().spacing(10);

        // "Export all" checkbox (only shown when multiple raid sessions exist)
        if self.sessions.iter().filter(|s| s.is_raid).count() > 1 {
            options = options.push(
                checkbox(self.export_all)
                    .label(export_all_label.clone())
                    .on_toggle(Message::ToggleExportAll)
                    .size(18),
            );
        }

        // Single-session options (hidden when export_all is active)
        if !self.export_all {
            options = options.push(
                checkbox(self.create_zip)
                    .label("Create ZIP archive")
                    .on_toggle(Message::ToggleZip)
                    .size(18),
            );
            options = options.push(
                checkbox(self.rename_output)
                    .label(rename_label.clone())
                    .on_toggle(Message::ToggleRename)
                    .size(18),
            );
        }

        options = options.push(
            checkbox(self.zero_log)
                .label("Zero (clear) original log file after export")
                .on_toggle(Message::ToggleZeroLog)
                .size(18),
        );

        let warning: Element<Message> = if self.zero_log {
            column![
                text("The original log file will be emptied after export.")
                    .size(12)
                    .color(Color::from_rgb(1.0, 0.6, 0.2)),
                text("Only use this for the live WoWCombatLog.txt so WoW starts a fresh log.")
                    .size(11)
                    .color(theme::TEXT_MUTED),
            ]
            .spacing(2)
            .into()
        } else {
            column![].into()
        };

        // Show batch result, single result, or action buttons
        let bottom_section: Element<Message> = if let Some(ref result) = self.batch_export_result {
            Self::view_batch_result(result)
        } else if let Some(ref result) = self.export_result {
            Self::view_single_result(result)
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
            rule::horizontal(1),
            options,
            warning,
            rule::horizontal(1),
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
                snap: true,
            });

        stack![backdrop, center(modal_card).width(Fill).height(Fill),]
            .width(Fill)
            .height(Fill)
            .into()
    }

    /// Render the result section for a single-session export.
    fn view_single_result(result: &Result<DoneInfo, String>) -> Element<'_, Message> {
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
    }

    /// Render the result section for a batch export.
    fn view_batch_result(result: &Result<BatchExportResult, String>) -> Element<'_, Message> {
        match result {
            Ok(info) => {
                let mut details = Column::new().spacing(4);
                details = details.push(
                    text(format!(
                        "Exported {} sessions ({} total lines)",
                        info.sessions_exported, info.total_lines
                    ))
                    .size(13)
                    .color(Color::from_rgb(0.3, 0.8, 0.5)),
                );
                for f in &info.files {
                    details = details.push(text(f).size(11).color(theme::TEXT_SECONDARY));
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
        snap: true,
    }
}

/// Download button style for the update banner (green accent).
fn update_download_button_style(_theme: &iced::Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => Color::from_rgb(0.20, 0.65, 0.40),
        _ => Color::from_rgb(0.15, 0.55, 0.35),
    };
    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: Color::WHITE,
        border: iced::Border {
            color: Color::from_rgba(0.20, 0.70, 0.45, 0.4),
            width: 1.0,
            radius: 4.0.into(),
        },
        shadow: iced::Shadow::default(),
        snap: true,
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
        snap: true,
    }
}
