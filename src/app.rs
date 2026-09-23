//! Application state and the TUI event loop.

use std::io::{self, Stdout};

use anyhow::Result;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::crossterm::{cursor::Show, execute};
use ratatui::{Frame, Terminal};

use crate::config::Config;
use crate::git::{self, RepoInfo};

/// Terminal type used by the TUI.
type Tui = Terminal<CrosstermBackend<Stdout>>;

/// Top-level view of the view state machine; stack depth is 2 (Dashboard ↔ other views).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum View {
    /// Multi-repo overview and entry point.
    #[default]
    Dashboard,
    /// Single repository workflow.
    Repo,
    /// File diff viewer.
    Diff,
    /// Commit message input.
    Commit,
    /// Branch panel.
    Branches,
}

/// Bottom status line content: short message or error (convention 3).
// Fields are read once the status line is rendered (M1 subtask 4).
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub struct StatusLine {
    /// Message text.
    pub message: String,
    /// `true` for errors, which render red and persist until the next operation.
    pub is_error: bool,
}

/// Application state shared across views.
pub struct App {
    /// Current view; key presses are dispatched per view in [`App::handle_key`].
    view: View,
    /// Loaded configuration; repo discovery consumes it (M1 subtask 3).
    #[allow(dead_code)]
    config: Config,
    /// Bottom status line (convention 3); rendered with the dashboard (M1 subtask 4).
    #[allow(dead_code)]
    status: Option<StatusLine>,
    /// Discovered git repositories (M1 subtask 3).
    pub repos: Vec<RepoInfo>,
}

impl App {
    /// Create the application around a loaded configuration.
    pub fn new(config: Config) -> Self {
        Self {
            view: View::Dashboard,
            config,
            status: None,
            repos: Vec::new(),
        }
    }

    /// Record an error message on the status line (persists until the next operation).
    pub fn set_error(&mut self, message: impl Into<String>) {
        self.status = Some(StatusLine {
            message: message.into(),
            is_error: true,
        });
    }

    /// Clear the status line.
    pub fn clear_error(&mut self) {
        self.status = None;
    }

    /// Discover git repositories from `config.scan_dirs` and load their status.
    ///
    /// On failure, records an error and clears `repos` so the UI shows the
    /// error rather than an empty list masquerading as success (checkpoint 2).
    pub fn load_repos(&mut self) {
        match git::discover_repos(&self.config.scan_dirs) {
            Ok(repos) => {
                self.repos = repos;
                self.clear_error();
            }
            Err(e) => {
                self.set_error(format!("扫描仓库失败: {e}"));
                self.repos = Vec::new();
            }
        }
    }

    /// Dispatch a key press for the current view; returns `false` when the app should exit.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        if key.kind == KeyEventKind::Release {
            return true;
        }
        match self.view {
            View::Dashboard => !matches!(key.code, KeyCode::Char('q') | KeyCode::Esc),
            // Entered from the dashboard by later subtasks; Esc steps back one level.
            _ => match key.code {
                KeyCode::Esc => {
                    self.view = View::Dashboard;
                    true
                }
                KeyCode::Char('q') => false,
                _ => true,
            },
        }
    }
}

/// Run the TUI until the user quits (`q` / `Esc` on the dashboard).
pub async fn run() -> Result<()> {
    install_panic_hook();
    let mut terminal = init_terminal()?;
    let mut app = match Config::load() {
        Ok(config) => App::new(config),
        Err(err) => {
            let mut app = App::new(Config::default());
            app.set_error(format!("failed to load config: {err:#}"));
            app
        }
    };
    app.load_repos();
    let run_result = event_loop(&mut terminal, &mut app);
    let restore_result: Result<()> = restore_terminal().map_err(Into::into);
    run_result.and(restore_result)
}

/// Render one frame: an empty dashboard until data loading lands in later subtasks.
fn render(_frame: &mut Frame) {}

/// Blocking event loop: draw a frame, then wait for the next terminal event.
fn event_loop(terminal: &mut Tui, app: &mut App) -> Result<()> {
    loop {
        terminal.draw(render)?;
        if let Event::Key(key) = event::read()? {
            if !app.handle_key(key) {
                return Ok(());
            }
        }
    }
}

/// Enter raw mode + alternate screen and build the TUI terminal.
fn init_terminal() -> Result<Tui> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    if let Err(err) = execute!(stdout, EnterAlternateScreen) {
        let _ = disable_raw_mode();
        return Err(err.into());
    }
    match Terminal::new(CrosstermBackend::new(stdout)) {
        Ok(terminal) => Ok(terminal),
        Err(err) => {
            let _ = restore_terminal();
            Err(err.into())
        }
    }
}

/// Leave raw mode + alternate screen and show the cursor.
fn restore_terminal() -> io::Result<()> {
    let raw = disable_raw_mode();
    let screen = execute!(io::stdout(), Show, LeaveAlternateScreen);
    raw.and(screen)
}

/// Restore the terminal before the default hook prints the panic.
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore_terminal();
        default_hook(info);
    }));
}