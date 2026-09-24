//! Application state and the TUI event loop.

use std::io::{self, Stdout};
use std::time::Duration;

use anyhow::Result;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::crossterm::{cursor::Show, execute};
use ratatui::widgets::ListState;
use ratatui::Terminal;
use tokio::sync::mpsc;

use crate::config::Config;
use crate::git::{self, FileEntry, Progress, RepoInfo};

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusLine {
    /// Message text.
    pub message: String,
    /// `true` for errors, which render red and persist until the next operation.
    pub is_error: bool,
}

/// Push/pull feedback rendered on the repo-view status line (convention 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteStatus {
    /// Operation in flight; carries the latest progress update.
    Progress(Progress),
    /// Operation finished successfully.
    Success,
    /// Operation failed; the reason persists until the next operation.
    Error(String),
}

/// Message sent from a background push/pull task over the progress channel.
enum RemoteEvent {
    /// Intermediate progress update.
    Progress(Progress),
    /// Terminal result of the operation.
    Done(Result<(), String>),
}

/// Application state shared across views.
pub struct App {
    /// Current view; key presses are dispatched per view in [`App::handle_key`].
    pub view: View,
    /// Loaded configuration; repo discovery consumes it (M1 subtask 3).
    config: Config,
    /// Bottom status line (convention 3); rendered with the dashboard (M1 subtask 4).
    pub status: Option<StatusLine>,
    /// Discovered git repositories (M1 subtask 3).
    pub repos: Vec<RepoInfo>,
    /// Dashboard selection cursor, consumed by the dashboard renderer (M1 subtask 4).
    pub dashboard_state: ListState,
    /// Index into `repos` for the currently opened single-repo view.
    pub current_repo: Option<usize>,
    /// File status entries for the current single-repo view.
    pub file_entries: Vec<FileEntry>,
    /// Selection cursor for the file status list in single-repo view.
    pub file_state: ListState,
    /// Scroll offset for the diff viewer.
    pub diff_scroll: u16,
    /// Whether the diff viewer is active (set when entering Diff view from Repo).
    pub show_diff: bool,
    /// Whether the discard confirmation dialog is shown.
    pub show_discard_confirm: bool,
    /// Current diff content for the selected file.
    pub diff_content: String,
    /// Single-line commit message being typed in the Commit view (convention 9).
    pub commit_message: String,
    /// Push/pull feedback; takes precedence over the normal status line.
    pub remote_status: Option<RemoteStatus>,
    /// Label of the running/last remote operation (`push` / `pull`).
    pub remote_op: &'static str,
    /// Receiver for background push/pull progress and completion events.
    remote_rx: Option<mpsc::UnboundedReceiver<RemoteEvent>>,
}

impl App {
    /// Create the application around a loaded configuration.
    pub fn new(config: Config) -> Self {
        Self {
            view: View::Dashboard,
            config,
            status: None,
            repos: Vec::new(),
            dashboard_state: ListState::default(),
            current_repo: None,
            file_entries: Vec::new(),
            file_state: ListState::default(),
            diff_scroll: 0,
            show_diff: false,
            show_discard_confirm: false,
            diff_content: String::new(),
            commit_message: String::new(),
            remote_status: None,
            remote_op: "",
            remote_rx: None,
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

    /// Re-run repo discovery and update the dashboard (manual refresh, convention 5).
    ///
    /// The selection cursor is clamped so a shrinking list never points past
    /// the end of `repos`.
    pub fn refresh_repos(&mut self) {
        self.load_repos();
        if self.repos.is_empty() {
            self.dashboard_state.select(None);
        } else if let Some(i) = self.dashboard_state.selected() {
            if i >= self.repos.len() {
                self.dashboard_state
                    .select(Some(self.repos.len().saturating_sub(1)));
            }
        }
    }

    /// Enter the single-repo view for the selected dashboard row.
    ///
    /// No-op when nothing (valid) is selected. Tracking the current repo for
    /// the repo view lands with M1 subtask 5.
    pub fn enter_repo(&mut self) {
        if let Some(i) = self.dashboard_state.selected() {
            if i < self.repos.len() {
                self.view = View::Repo;
                self.current_repo = Some(i);
                self.load_file_entries();
                self.file_state.select(Some(0));
                self.show_diff = false;
                self.clear_error();
            }
        }
    }

    /// Load file status entries for the current repo.
    pub fn load_file_entries(&mut self) {
        if let Some(i) = self.current_repo {
            if i < self.repos.len() {
                let repo_path = self.repos[i].path.clone();
                match git::list_file_status(&repo_path) {
                    Ok(entries) => {
                        self.file_entries = entries;
                        self.clear_error();
                        if self.file_entries.is_empty() {
                            self.file_state.select(None);
                        } else {
                            let sel = self.file_state.selected().unwrap_or(0);
                            let new_sel = sel.min(self.file_entries.len() - 1);
                            self.file_state.select(Some(new_sel));
                        }
                    }
                    Err(e) => {
                        self.set_error(format!("加载文件列表失败: {e}"));
                        self.file_entries.clear();
                        self.file_state.select(None);
                    }
                }
            }
        }
    }

    /// View the diff of the currently selected file.
    pub fn view_file_diff(&mut self) {
        if let (Some(repo_idx), Some(file_idx)) = (self.current_repo, self.file_state.selected()) {
            if repo_idx < self.repos.len() && file_idx < self.file_entries.len() {
                let repo_path = self.repos[repo_idx].path.clone();
                let file_path = self.file_entries[file_idx].path.clone();
                match git::diff_file(&repo_path, &file_path) {
                    Ok(diff) => {
                        self.diff_content = diff;
                        self.show_diff = true;
                        self.view = View::Diff;
                        self.diff_scroll = 0;
                    }
                    Err(e) => self.set_error(format!("加载 diff 失败: {e}")),
                }
            }
        }
    }

    /// Stage the currently selected file.
    pub fn stage_current(&mut self) {
        if let (Some(repo_idx), Some(file_idx)) = (self.current_repo, self.file_state.selected()) {
            if repo_idx < self.repos.len() && file_idx < self.file_entries.len() {
                let repo_path = self.repos[repo_idx].path.clone();
                let file_path = self.file_entries[file_idx].path.clone();
                match git::stage(&repo_path, &file_path) {
                    Ok(()) => {
                        self.clear_error();
                        self.load_file_entries();
                    }
                    Err(e) => self.set_error(format!("stage 失败: {e}")),
                }
            }
        }
    }

    /// Unstage the currently selected file.
    pub fn unstage_current(&mut self) {
        if let (Some(repo_idx), Some(file_idx)) = (self.current_repo, self.file_state.selected()) {
            if repo_idx < self.repos.len() && file_idx < self.file_entries.len() {
                let repo_path = self.repos[repo_idx].path.clone();
                let file_path = self.file_entries[file_idx].path.clone();
                match git::unstage(&repo_path, &file_path) {
                    Ok(()) => {
                        self.clear_error();
                        self.load_file_entries();
                    }
                    Err(e) => self.set_error(format!("unstage 失败: {e}")),
                }
            }
        }
    }

    /// Show the discard confirmation dialog.
    pub fn start_discard(&mut self) {
        self.show_discard_confirm = true;
    }

    /// Execute the discard for the currently selected file.
    pub fn confirm_discard(&mut self) {
        self.show_discard_confirm = false;
        if let (Some(repo_idx), Some(file_idx)) = (self.current_repo, self.file_state.selected()) {
            if repo_idx < self.repos.len() && file_idx < self.file_entries.len() {
                let repo_path = self.repos[repo_idx].path.clone();
                let file_path = self.file_entries[file_idx].path.clone();
                match git::discard(&repo_path, &file_path) {
                    Ok(()) => {
                        self.clear_error();
                        self.load_file_entries();
                    }
                    Err(e) => self.set_error(format!("discard 失败: {e}")),
                }
            }
        }
    }

    /// Cancel the discard confirmation dialog.
    pub fn cancel_discard(&mut self) {
        self.show_discard_confirm = false;
    }

    /// Record a short success message on the status line (convention 3).
    pub fn set_info(&mut self, message: impl Into<String>) {
        self.status = Some(StatusLine {
            message: message.into(),
            is_error: false,
        });
    }

    /// Open the single-line commit message input (convention 9).
    pub fn open_commit(&mut self) {
        if self.remote_rx.is_some() {
            return;
        }
        self.commit_message.clear();
        self.view = View::Commit;
    }

    /// Cancel the commit input and return to the repo view.
    pub fn cancel_commit(&mut self) {
        self.view = View::Repo;
        self.commit_message.clear();
    }

    /// Stage-aware commit of `self.commit_message`; returns to the repo view.
    ///
    /// Success clears the input and refreshes status (convention 5); failure
    /// keeps the typed message and shows the reason on the status line.
    pub fn submit_commit(&mut self) {
        self.view = View::Repo;
        let message = self.commit_message.clone();
        let Some(repo_idx) = self.current_repo else {
            self.set_error("commit 失败: 未打开仓库");
            return;
        };
        if repo_idx >= self.repos.len() {
            self.set_error("commit 失败: 仓库索引失效");
            return;
        }
        let repo_path = self.repos[repo_idx].path.clone();
        self.remote_status = None;
        match git::commit(&repo_path, &message) {
            Ok(oid) => {
                self.commit_message.clear();
                self.set_info(format!("commit 成功 {}", short_oid(&oid)));
                self.refresh_repo_status(repo_idx);
            }
            Err(e) => {
                self.set_error(format!("commit 失败: {e}"));
            }
        }
    }

    /// Start a background push (`op == "push"`) or pull (`op == "pull"`).
    ///
    /// The task runs off the UI thread; progress arrives on an mpsc channel
    /// consumed each frame (convention 1), so the UI stays interactive while
    /// the operation runs. No-op while another remote op is in flight or
    /// outside the repo view.
    pub fn start_remote(&mut self, op: &'static str) {
        if self.view != View::Repo || self.remote_rx.is_some() {
            return;
        }
        let Some(repo_idx) = self.current_repo else {
            return;
        };
        if repo_idx >= self.repos.len() {
            return;
        }
        let repo_path = self.repos[repo_idx].path.clone();
        let (tx, rx) = mpsc::unbounded_channel();
        self.remote_rx = Some(rx);
        self.remote_op = op;
        self.remote_status = Some(RemoteStatus::Progress(Progress {
            percent: 0,
            message: if op == "push" {
                "正在 push…".to_string()
            } else {
                "正在 pull…".to_string()
            },
        }));
        self.status = None;
        tokio::spawn(async move {
            let result = match op {
                "push" => {
                    let sink = tx.clone();
                    git::push(&repo_path, move |p| {
                        let _ = sink.send(RemoteEvent::Progress(p));
                    })
                    .await
                }
                _ => {
                    let sink = tx.clone();
                    git::pull(&repo_path, move |p| {
                        let _ = sink.send(RemoteEvent::Progress(p));
                    })
                    .await
                }
            };
            let _ = tx.send(RemoteEvent::Done(result.map_err(|e| e.to_string())));
        });
    }

    /// Drain pending progress events; called once per frame (convention 1).
    pub fn poll_remote(&mut self) {
        let Some(rx) = self.remote_rx.as_mut() else {
            return;
        };
        loop {
            match rx.try_recv() {
                Ok(RemoteEvent::Progress(p)) => {
                    self.remote_status = Some(RemoteStatus::Progress(p));
                }
                Ok(RemoteEvent::Done(Ok(()))) => {
                    self.remote_status = Some(RemoteStatus::Success);
                    self.remote_rx = None;
                    if let Some(idx) = self.current_repo {
                        if idx < self.repos.len() {
                            self.refresh_repo_status(idx);
                        }
                    }
                    return;
                }
                Ok(RemoteEvent::Done(Err(err))) => {
                    self.remote_status = Some(RemoteStatus::Error(err));
                    self.remote_rx = None;
                    return;
                }
                Err(mpsc::error::TryRecvError::Empty) => return,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    self.remote_status =
                        Some(RemoteStatus::Error("远端任务意外退出".to_string()));
                    self.remote_rx = None;
                    return;
                }
            }
        }
    }

    /// Re-query status for `idx` after a commit/push/pull (convention 5).
    fn refresh_repo_status(&mut self, idx: usize) {
        let repo_path = self.repos[idx].path.clone();
        match git::query_status(&repo_path) {
            Ok(info) => self.repos[idx] = info,
            Err(e) => self.set_error(format!("刷新状态失败: {e}")),
        }
        self.load_file_entries();
    }

    /// Dispatch a key press for the current view; returns `false` when the app should exit.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        if key.kind == KeyEventKind::Release {
            return true;
        }
        match self.view {
            View::Dashboard => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => false,
                KeyCode::Char('j') | KeyCode::Down => {
                    if !self.repos.is_empty() {
                        let next = self.dashboard_state.selected().map_or(0, |i| {
                            i.saturating_add(1).min(self.repos.len() - 1)
                        });
                        self.dashboard_state.select(Some(next));
                    }
                    true
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    if !self.repos.is_empty() {
                        let prev = self.dashboard_state.selected().map_or(0, |i| {
                            i.saturating_sub(1)
                        });
                        self.dashboard_state.select(Some(prev));
                    }
                    true
                }
                KeyCode::Char('r') => {
                    self.refresh_repos();
                    true
                }
                KeyCode::Enter => {
                    self.enter_repo();
                    true
                }
                _ => true,
            },
            // Entered from the dashboard by later subtasks; Esc steps back one level.
            View::Repo => {
                if self.show_discard_confirm {
                    match key.code {
                        KeyCode::Char('y') | KeyCode::Char('Y') => {
                            self.confirm_discard();
                            true
                        }
                        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                            self.cancel_discard();
                            true
                        }
                        _ => true,
                    }
                } else {
                    match key.code {
                        KeyCode::Esc => {
                            self.view = View::Dashboard;
                            true
                        }
                        KeyCode::Char('q') => false,
                        KeyCode::Char('j') | KeyCode::Down => {
                            if !self.file_entries.is_empty() {
                                let next = self.file_state.selected().map_or(0, |i| {
                                    i.saturating_add(1).min(self.file_entries.len() - 1)
                                });
                                self.file_state.select(Some(next));
                            }
                            true
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            if !self.file_entries.is_empty() {
                                let prev = self.file_state.selected().map_or(0, |i| {
                                    i.saturating_sub(1)
                                });
                                self.file_state.select(Some(prev));
                            }
                            true
                        }
                        KeyCode::Char('d') => {
                            self.view_file_diff();
                            true
                        }
                        KeyCode::Char('s') => {
                            self.stage_current();
                            true
                        }
                        KeyCode::Char('u') => {
                            self.unstage_current();
                            true
                        }
                        KeyCode::Char('x') => {
                            self.start_discard();
                            true
                        }
                        KeyCode::Char('c') => {
                            self.open_commit();
                            true
                        }
                        KeyCode::Char('p') => {
                            self.start_remote("push");
                            true
                        }
                        KeyCode::Char('l') => {
                            self.start_remote("pull");
                            true
                        }
                        _ => true,
                    }
                }
            }
            View::Diff => match key.code {
                KeyCode::Esc => {
                    self.view = View::Repo;
                    self.show_diff = false;
                    true
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    self.diff_scroll = self.diff_scroll.saturating_add(1);
                    true
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.diff_scroll = self.diff_scroll.saturating_sub(1);
                    true
                }
                KeyCode::Char('q') => false,
                _ => true,
            },
            // Single-line commit input (convention 9): Enter submits, Esc cancels.
            View::Commit => match key.code {
                KeyCode::Esc => {
                    self.cancel_commit();
                    true
                }
                KeyCode::Enter => {
                    self.submit_commit();
                    true
                }
                KeyCode::Backspace => {
                    self.commit_message.pop();
                    true
                }
                KeyCode::Char(c)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    self.commit_message.push(c);
                    true
                }
                _ => true,
            },
            // Fallback for Branches: Esc steps back to Dashboard.
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

/// Render the first 7 hex characters of a commit id for the status line.
fn short_oid(oid: &git2::Oid) -> String {
    oid.to_string().chars().take(7).collect()
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
    app.refresh_repos();
    let run_result = event_loop(&mut terminal, &mut app);
    let restore_result: Result<()> = restore_terminal().map_err(Into::into);
    run_result.and(restore_result)
}

/// Event loop: draw, wait up to 200ms for input, then drain background
/// push/pull progress so the status line stays live during remote ops
/// (convention 1: the UI never blocks on an in-flight operation).
fn event_loop(terminal: &mut Tui, app: &mut App) -> Result<()> {
    loop {
        terminal.draw(|f| crate::ui::render(f, app))?;
        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if !app.handle_key(key) {
                    return Ok(());
                }
            }
        }
        app.poll_remote();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::RepoInfo;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, ratatui::crossterm::event::KeyModifiers::NONE)
    }

    fn app_with_repos(count: usize) -> App {
        let mut app = App::new(Config::default());
        app.repos = (0..count)
            .map(|i| RepoInfo {
                name: format!("repo-{i}"),
                path: std::path::PathBuf::from(format!("/tmp/repo-{i}")),
                branch: "main".to_string(),
                dirty: Default::default(),
                ahead: 0,
                behind: 0,
                last_commit: Default::default(),
            })
            .collect();
        app
    }

    #[test]
    fn dashboard_j_k_moves_selection() {
        let mut app = app_with_repos(3);
        assert_eq!(app.dashboard_state.selected(), None);

        assert!(app.handle_key(key(KeyCode::Char('j'))));
        assert_eq!(app.dashboard_state.selected(), Some(0));

        assert!(app.handle_key(key(KeyCode::Char('j'))));
        assert_eq!(app.dashboard_state.selected(), Some(1));

        assert!(app.handle_key(key(KeyCode::Char('k'))));
        assert_eq!(app.dashboard_state.selected(), Some(0));
    }

    #[test]
    fn dashboard_j_stops_at_last_repo() {
        let mut app = app_with_repos(2);
        app.dashboard_state.select(Some(1));
        assert!(app.handle_key(key(KeyCode::Char('j'))));
        assert_eq!(app.dashboard_state.selected(), Some(1));
    }

    #[test]
    fn dashboard_enter_opens_selected_repo() {
        let mut app = app_with_repos(1);
        // Nothing selected: Enter is a no-op.
        assert!(app.handle_key(key(KeyCode::Enter)));
        assert_eq!(app.view, View::Dashboard);

        app.dashboard_state.select(Some(0));
        assert!(app.handle_key(key(KeyCode::Enter)));
        assert_eq!(app.view, View::Repo);
    }

    #[test]
    fn dashboard_q_quits() {
        let mut app = app_with_repos(1);
        assert!(!app.handle_key(key(KeyCode::Char('q'))));
    }

    #[test]
    fn dashboard_r_refreshes_and_clears_error() {
        let mut app = app_with_repos(1);
        app.set_error("boom".to_string());
        assert!(app.handle_key(key(KeyCode::Char('r'))));
        assert_eq!(app.status, None);
        // Default config has empty scan_dirs, so a refresh yields no repos.
        assert!(app.repos.is_empty());
        assert_eq!(app.dashboard_state.selected(), None);
    }

    #[test]
    fn refresh_clamps_out_of_range_selection() {
        let mut app = app_with_repos(3);
        app.dashboard_state.select(Some(2));
        app.repos.pop();
        app.repos.pop();
        app.refresh_repos();
        // scan_dirs empty: repos cleared and selection dropped.
        assert_eq!(app.dashboard_state.selected(), None);
    }
}