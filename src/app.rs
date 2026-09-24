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
use crate::git::{self, BranchInfo, FileEntry, Progress, RepoInfo};

/// Terminal type used by the TUI.
type Tui = Terminal<CrosstermBackend<Stdout>>;

/// Top-level view of the view state machine; stack depth is 2 (Dashboard ↔ other views).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
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
    /// Scan-dir configuration panel.
    Config,
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

/// One line of the dashboard batch progress log: one repository (convention 6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchLogLine {
    /// Repository name, resolved when the event was sent.
    pub name: String,
    /// Status text from `git::batch`: `进行中` / `成功` / `失败: <reason>`.
    pub message: String,
    /// `true` when the message reports a failure.
    pub is_error: bool,
    /// `true` when the repository finished successfully.
    pub is_success: bool,
}

impl BatchLogLine {
    /// Build a log line, deriving the style flags from the message prefixes
    /// documented in `git::batch`.
    fn new(name: String, message: String) -> Self {
        let is_error = message.starts_with("失败");
        let is_success = message.starts_with("成功");
        Self {
            name,
            message,
            is_error,
            is_success,
        }
    }
}

/// Message sent from a background batch pull/fetch task over its channel.
enum BatchEvent {
    /// Status update for the repository at `index` (positional key: distinct
    /// repositories may share the same display name).
    Line {
        index: usize,
        name: String,
        message: String,
    },
    /// Terminal per-repository results of the batch run.
    Done(Vec<(String, Result<()>)>),
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
    /// Rolling per-repository batch progress log (convention 6).
    pub batch_log: Vec<BatchLogLine>,
    /// Label of the running/last batch operation (`pull` / `fetch`).
    pub batch_op: &'static str,
    /// Receiver for background batch pull/fetch progress and completion events.
    batch_rx: Option<mpsc::UnboundedReceiver<BatchEvent>>,
    /// Local branches shown in the branch panel.
    pub branches: Vec<BranchInfo>,
    /// Selection cursor for the branch list.
    pub branch_state: ListState,
    /// Index into `repos` whose branches the branch panel edits.
    branch_repo: Option<usize>,
    /// View the branch panel returns to on Esc.
    branches_from: View,
    /// Single-line branch name being typed in create mode.
    pub branch_input: String,
    /// Whether the branch-create input overlay is shown.
    pub show_branch_input: bool,
    /// Whether the branch-delete confirmation dialog is shown.
    pub show_branch_delete_confirm: bool,
    /// Selection cursor for the scan-dir list in the config panel.
    pub config_state: ListState,
    /// Whether the add-scan-dir input overlay is shown.
    pub show_config_input: bool,
    /// Directory path being typed in the add-scan-dir input.
    pub config_input: String,
    /// Whether the delete-scan-dir confirmation dialog is shown.
    pub show_config_delete_confirm: bool,
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
            show_discard_confirm: false,
            diff_content: String::new(),
            commit_message: String::new(),
            remote_status: None,
            remote_op: "",
            remote_rx: None,
            batch_log: Vec::new(),
            batch_op: "",
            batch_rx: None,
            branches: Vec::new(),
            branch_state: ListState::default(),
            branch_repo: None,
            branches_from: View::Dashboard,
            branch_input: String::new(),
            show_branch_input: false,
            show_branch_delete_confirm: false,
            config_state: ListState::default(),
            show_config_input: false,
            config_input: String::new(),
            show_config_delete_confirm: false,
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
                // Refresh first: it clears the status line, so the success
                // message must be written after, otherwise it never shows.
                self.refresh_repo_status(repo_idx);
                self.set_info_unless_error(format!("commit 成功 {}", short_oid(&oid)));
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
    /// the operation runs. No-op while another remote or batch op is in
    /// flight (they would contend on the same repository locks), or outside
    /// the repo view.
    pub fn start_remote(&mut self, op: &'static str) {
        if self.view != View::Repo || self.remote_rx.is_some() || self.batch_rx.is_some() {
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
                    self.remote_status = Some(RemoteStatus::Error("远端任务意外退出".to_string()));
                    self.remote_rx = None;
                    return;
                }
            }
        }
    }

    /// Re-query status for `idx` after a commit/push/pull/branch change
    /// (convention 5). File entries reload only when the repo view shows
    /// this repository, so unrelated views are left untouched.
    fn refresh_repo_status(&mut self, idx: usize) {
        let repo_path = self.repos[idx].path.clone();
        match git::query_status(&repo_path) {
            Ok(info) => self.repos[idx] = info,
            Err(e) => self.set_error(format!("刷新状态失败: {e}")),
        }
        if self.current_repo == Some(idx) {
            self.load_file_entries();
        }
    }

    /// Start a background batch `pull` (`op == "pull"`) or `fetch`
    /// (`op == "fetch"`) across every discovered repository.
    ///
    /// Progress arrives on an mpsc channel consumed each frame (convention 1)
    /// and renders as a rolling per-repository log at the bottom of the
    /// dashboard (convention 6). No-op while a batch or remote op is in
    /// flight, or when there are no repositories.
    pub fn start_batch(&mut self, op: &'static str) {
        if self.view != View::Dashboard || self.batch_rx.is_some() || self.remote_rx.is_some() {
            return;
        }
        if self.repos.is_empty() {
            self.set_info("没有仓库，无法执行批量操作");
            return;
        }
        let repos = self.repos.clone();
        let names: Vec<String> = repos.iter().map(|r| r.name.clone()).collect();
        let (tx, rx) = mpsc::unbounded_channel();
        self.batch_rx = Some(rx);
        self.batch_op = op;
        self.batch_log.clear();
        self.status = None;
        tokio::spawn(async move {
            let progress = {
                let sink = tx.clone();
                move |index: usize, message: &str| {
                    let name = names.get(index).cloned().unwrap_or_default();
                    let _ = sink.send(BatchEvent::Line {
                        index,
                        name,
                        message: message.to_string(),
                    });
                }
            };
            let results = match op {
                "fetch" => git::batch_fetch(&repos, progress).await,
                _ => git::batch_pull(&repos, progress).await,
            };
            let _ = tx.send(BatchEvent::Done(results));
        });
    }

    /// Drain pending batch events; called once per frame (convention 1).
    ///
    /// Each repository keeps a single log line that advances from `进行中`
    /// to `成功` / `失败: <reason>`; on completion every repository status is
    /// refreshed (convention 5) before the summary is written.
    pub fn poll_batch(&mut self) {
        let Some(rx) = self.batch_rx.as_mut() else {
            return;
        };
        loop {
            match rx.try_recv() {
                Ok(BatchEvent::Line {
                    index,
                    name,
                    message,
                }) => {
                    // Positional key: each repository keeps one advancing line
                    // even when two repositories share the same display name.
                    let line = BatchLogLine::new(name, message);
                    if index < self.batch_log.len() {
                        self.batch_log[index] = line;
                    } else {
                        self.batch_log.push(line);
                    }
                }
                Ok(BatchEvent::Done(results)) => {
                    self.batch_rx = None;
                    let total = results.len();
                    let failed = results.iter().filter(|(_, r)| r.is_err()).count();
                    self.refresh_repos();
                    if failed == 0 {
                        self.set_info(format!("批量 {} 完成: {total} 个仓库成功", self.batch_op));
                    } else {
                        self.set_error(format!(
                            "批量 {} 完成: {} 成功 / {} 失败",
                            self.batch_op,
                            total - failed,
                            failed
                        ));
                    }
                    return;
                }
                Err(mpsc::error::TryRecvError::Empty) => return,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    self.set_error("批量任务意外退出".to_string());
                    self.batch_rx = None;
                    return;
                }
            }
        }
    }

    /// Enter the branch panel for the selected (dashboard) or current (repo)
    /// repository.
    ///
    /// The origin view is recorded so Esc returns there; the branch list is
    /// loaded immediately, with failures surfacing as an error status rather
    /// than an empty list masquerading as success (checkpoint 2).
    pub fn enter_branches(&mut self) {
        let origin = self.view;
        let idx = match origin {
            View::Dashboard => self.dashboard_state.selected(),
            View::Repo => self.current_repo,
            _ => return,
        };
        let Some(idx) = idx.filter(|i| *i < self.repos.len()) else {
            return;
        };
        self.branch_repo = Some(idx);
        self.branches_from = origin;
        self.branch_state.select(None);
        self.branch_input.clear();
        self.show_branch_input = false;
        self.show_branch_delete_confirm = false;
        self.view = View::Branches;
        self.load_branches();
    }

    /// Return from the branch panel to the view it was opened from.
    pub fn leave_branches(&mut self) {
        self.view = self.branches_from;
        self.show_branch_input = false;
        self.show_branch_delete_confirm = false;
        self.branch_input.clear();
        self.clear_error();
    }

    /// Enter the scan-dir configuration panel (dashboard `c`).
    ///
    /// The selection lands on the first configured directory — or is cleared
    /// when none is configured — so `j/k` navigation always starts valid,
    /// and a stale cursor from a previous visit is re-clamped.
    pub fn enter_config(&mut self) {
        self.view = View::Config;
        if self.config.scan_dirs.is_empty() {
            self.config_state.select(None);
        } else {
            let selected = self
                .config_state
                .selected()
                .filter(|i| *i < self.config.scan_dirs.len())
                .unwrap_or(0);
            self.config_state.select(Some(selected));
        }
        self.clear_error();
    }

    /// Leave the configuration panel back to the dashboard (`Esc`).
    pub fn leave_config(&mut self) {
        self.view = View::Dashboard;
        self.show_config_input = false;
        self.config_input.clear();
        self.show_config_delete_confirm = false;
        self.clear_error();
    }

    /// Configured scan directories, read by the config panel renderer.
    pub fn scan_dirs(&self) -> &[std::path::PathBuf] {
        &self.config.scan_dirs
    }

    /// Open the add-scan-dir input overlay (config panel `a`).
    pub fn start_config_add(&mut self) {
        self.config_input.clear();
        self.show_config_input = true;
        self.show_config_delete_confirm = false;
        self.status = None;
    }

    /// Cancel the add-scan-dir input, discarding the typed path (convention 9).
    pub fn cancel_config_add(&mut self) {
        self.show_config_input = false;
        self.config_input.clear();
    }

    /// Validate the typed path, persist it, and rescan (convention 5).
    ///
    /// Each validation failure keeps the input open with the typed text so it
    /// can be corrected: empty / missing / not a directory / duplicate.
    /// On success the config is saved, repos refreshed, and the new entry
    /// selected; the success message is written after the refresh, which
    /// would otherwise clear it.
    pub fn submit_config_add(&mut self) {
        let path = self.config_input.trim().to_string();
        if path.is_empty() {
            self.set_error("目录不能为空");
            return;
        }
        if !std::path::Path::new(&path).exists() {
            self.set_error(format!("目录不存在: {path}"));
            return;
        }
        if !std::path::Path::new(&path).is_dir() {
            self.set_error(format!("不是目录: {path}"));
            return;
        }
        let dir = std::path::PathBuf::from(&path);
        if self.config.scan_dirs.contains(&dir) {
            self.set_error("目录已在列表中");
            return;
        }

        self.config.scan_dirs.push(dir);
        if let Err(err) = self.config.save() {
            // Keep memory and disk consistent, and keep the typed text so the
            // save can be retried (convention 9).
            self.config.scan_dirs.pop();
            self.set_error(format!("保存配置失败: {err:#}"));
            return;
        }
        self.cancel_config_add();
        self.refresh_repos();
        self.config_state
            .select(Some(self.config.scan_dirs.len() - 1));
        self.set_info_unless_error(format!("已添加扫描目录: {path}"));
    }

    /// Open the delete confirmation for the selected scan dir (config `d`).
    ///
    /// An empty list reports an error instead of opening a dialog that could
    /// not name a target; a stale selection falls back to the first entry.
    pub fn start_config_delete(&mut self) {
        if self.config.scan_dirs.is_empty() {
            self.set_error("没有可删除的目录");
            return;
        }
        let selected = self
            .config_state
            .selected()
            .filter(|i| *i < self.config.scan_dirs.len())
            .unwrap_or(0);
        self.config_state.select(Some(selected));
        self.show_config_input = false;
        self.show_config_delete_confirm = true;
    }

    /// Cancel the delete-scan-dir confirmation.
    pub fn cancel_config_delete(&mut self) {
        self.show_config_delete_confirm = false;
    }

    /// Remove the confirmed scan dir, persist, and rescan (convention 5).
    ///
    /// The selection is clamped so a shrinking list never points past the
    /// end (and resets to none when the list empties); a failed save rolls
    /// the removal back so memory stays consistent with disk.
    pub fn confirm_config_delete(&mut self) {
        self.show_config_delete_confirm = false;
        let Some(i) = self
            .config_state
            .selected()
            .filter(|i| *i < self.config.scan_dirs.len())
        else {
            return;
        };
        let removed = self.config.scan_dirs.remove(i);
        if let Err(err) = self.config.save() {
            self.config.scan_dirs.insert(i, removed);
            self.set_error(format!("保存配置失败: {err:#}"));
            return;
        }
        self.refresh_repos();
        if self.config.scan_dirs.is_empty() {
            self.config_state.select(None);
        } else {
            self.config_state
                .select(Some(i.min(self.config.scan_dirs.len() - 1)));
        }
        self.set_info_unless_error(format!("已删除扫描目录: {}", removed.display()));
    }

    /// Whether a background batch operation is currently running.
    pub fn batch_running(&self) -> bool {
        self.batch_rx.is_some()
    }

    /// Name of the repository the branch panel edits (empty when invalid).
    pub fn branch_repo_name(&self) -> &str {
        self.branch_repo
            .filter(|i| *i < self.repos.len())
            .map_or("", |i| &self.repos[i].name)
    }

    /// Path of the repository the branch panel edits, when still valid.
    fn branch_repo_path(&self) -> Option<std::path::PathBuf> {
        self.branch_repo
            .filter(|i| *i < self.repos.len())
            .map(|i| self.repos[i].path.clone())
    }

    /// Name of the currently selected branch, if any.
    fn selected_branch(&self) -> Option<String> {
        self.branch_state
            .selected()
            .and_then(|i| self.branches.get(i))
            .map(|b| b.name.clone())
    }

    /// Load (or reload) the branch list for the branch panel.
    ///
    /// The cursor keeps its position when still valid and otherwise falls
    /// back to the HEAD branch. On failure the list is cleared and the error
    /// is shown so a failed load never looks like an empty repository
    /// (checkpoint 2).
    pub fn load_branches(&mut self) {
        let Some(path) = self.branch_repo_path() else {
            self.set_error("分支面板: 仓库索引失效".to_string());
            self.branches.clear();
            self.branch_state.select(None);
            return;
        };
        match git::list_branches(&path) {
            Ok(branches) => {
                let head = branches.iter().position(|b| b.is_head).unwrap_or(0);
                let keep = self
                    .branch_state
                    .selected()
                    .filter(|i| *i < branches.len())
                    .unwrap_or(head);
                let selected = if branches.is_empty() {
                    None
                } else {
                    Some(keep.min(branches.len() - 1))
                };
                self.branches = branches;
                self.branch_state.select(selected);
                self.clear_error();
            }
            Err(e) => {
                self.set_error(format!("加载分支列表失败: {e}"));
                self.branches.clear();
                self.branch_state.select(None);
            }
        }
    }

    /// Switch to the selected branch, then refresh the affected views
    /// (convention 5).
    pub fn switch_to_selected_branch(&mut self) {
        let (Some(path), Some(name)) = (self.branch_repo_path(), self.selected_branch()) else {
            return;
        };
        match git::switch_branch(&path, &name) {
            Ok(()) => {
                self.after_branch_change();
                self.set_info_unless_error(format!("已切换到 {name}"));
            }
            Err(e) => self.set_error(format!("切换分支失败: {e}")),
        }
    }

    /// Open the single-line branch name input (convention 9).
    pub fn start_branch_create(&mut self) {
        if self.branch_repo_path().is_none() {
            return;
        }
        self.branch_input.clear();
        self.show_branch_input = true;
        self.show_branch_delete_confirm = false;
        self.status = None;
    }

    /// Cancel the branch-create input, keeping the branch list as it was.
    pub fn cancel_branch_create(&mut self) {
        self.show_branch_input = false;
        self.branch_input.clear();
    }

    /// Create the branch named in the input and select it in the list.
    ///
    /// An empty name keeps the input open with an error; a git-side failure
    /// also keeps the typed text so it can be corrected (convention 9).
    pub fn submit_branch_create(&mut self) {
        let name = self.branch_input.trim().to_string();
        if name.is_empty() {
            self.set_error("分支名不能为空".to_string());
            return;
        }
        let Some(path) = self.branch_repo_path() else {
            self.set_error("创建分支失败: 仓库索引失效".to_string());
            return;
        };
        match git::create_branch(&path, &name) {
            Ok(()) => {
                self.show_branch_input = false;
                self.branch_input.clear();
                self.after_branch_change();
                if let Some(i) = self.branches.iter().position(|b| b.name == name) {
                    self.branch_state.select(Some(i));
                }
                self.set_info_unless_error(format!("已创建分支 {name}"));
            }
            Err(e) => self.set_error(format!("创建分支失败: {e}")),
        }
    }

    /// Open the delete confirmation for the selected branch (discard-style
    /// confirmation, convention 8).
    pub fn start_branch_delete(&mut self) {
        if self.branch_repo_path().is_some() && self.selected_branch().is_some() {
            self.show_branch_delete_confirm = true;
            self.show_branch_input = false;
        }
    }

    /// Cancel the branch-delete confirmation.
    pub fn cancel_branch_delete(&mut self) {
        self.show_branch_delete_confirm = false;
    }

    /// Delete the confirmed branch, then refresh the affected views
    /// (convention 5).
    pub fn confirm_branch_delete(&mut self) {
        self.show_branch_delete_confirm = false;
        let (Some(path), Some(name)) = (self.branch_repo_path(), self.selected_branch()) else {
            return;
        };
        match git::delete_branch(&path, &name) {
            Ok(()) => {
                self.after_branch_change();
                self.set_info_unless_error(format!("已删除分支 {name}"));
            }
            Err(e) => self.set_error(format!("删除分支失败: {e}")),
        }
    }

    /// Refresh repo status and the branch list after a branch mutation
    /// (convention 5: auto-refresh the affected views).
    fn after_branch_change(&mut self) {
        if let Some(idx) = self.branch_repo.filter(|i| *i < self.repos.len()) {
            self.refresh_repo_status(idx);
        }
        self.load_branches();
    }

    /// Record a success message unless an error already surfaced while
    /// refreshing; errors keep priority on the status line (convention 3).
    fn set_info_unless_error(&mut self, message: impl Into<String>) {
        let has_error = self.status.as_ref().is_some_and(|s| s.is_error);
        if !has_error {
            self.set_info(message);
        }
    }

    /// Dispatch a key press for the current view; returns `false` when the app should exit.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        if key.kind == KeyEventKind::Release {
            return true;
        }
        match self.view {
            View::Dashboard => self.handle_dashboard_key(key),
            View::Repo => self.handle_repo_key(key),
            View::Diff => self.handle_diff_key(key),
            View::Commit => self.handle_commit_key(key),
            View::Branches => self.handle_branches_key(key),
            View::Config => self.handle_config_key(key),
        }
    }

    /// Key handler for the dashboard: list navigation and view entry points.
    fn handle_dashboard_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => false,
            KeyCode::Char('j') | KeyCode::Down => {
                move_selection(&mut self.dashboard_state, self.repos.len(), true);
                true
            }
            KeyCode::Char('k') | KeyCode::Up => {
                move_selection(&mut self.dashboard_state, self.repos.len(), false);
                true
            }
            KeyCode::Char('r') => {
                self.refresh_repos();
                true
            }
            KeyCode::Char('l') => {
                self.start_batch("pull");
                true
            }
            KeyCode::Char('f') => {
                self.start_batch("fetch");
                true
            }
            KeyCode::Char('b') => {
                self.enter_branches();
                true
            }
            KeyCode::Char('c') => {
                self.enter_config();
                true
            }
            KeyCode::Enter => {
                self.enter_repo();
                true
            }
            _ => true,
        }
    }

    /// Key handler for the single-repo view. Entered from the dashboard by
    /// later subtasks; Esc steps back one level.
    fn handle_repo_key(&mut self, key: KeyEvent) -> bool {
        if self.show_discard_confirm {
            match confirm_modal_key(key) {
                ConfirmOutcome::Yes => self.confirm_discard(),
                ConfirmOutcome::No => self.cancel_discard(),
                ConfirmOutcome::Ignored => {}
            }
            true
        } else {
            match key.code {
                KeyCode::Esc => {
                    self.view = View::Dashboard;
                    true
                }
                KeyCode::Char('q') => false,
                KeyCode::Char('j') | KeyCode::Down => {
                    move_selection(&mut self.file_state, self.file_entries.len(), true);
                    true
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    move_selection(&mut self.file_state, self.file_entries.len(), false);
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
                KeyCode::Char('b') => {
                    self.enter_branches();
                    true
                }
                _ => true,
            }
        }
    }

    /// Key handler for the diff viewer: vertical scroll and exit.
    fn handle_diff_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => {
                self.view = View::Repo;
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
        }
    }

    /// Key handler for the single-line commit input (convention 9): Enter
    /// submits, Esc cancels.
    fn handle_commit_key(&mut self, key: KeyEvent) -> bool {
        match input_modal_key(key, &mut self.commit_message) {
            InputOutcome::Cancel => self.cancel_commit(),
            InputOutcome::Submit => self.submit_commit(),
            InputOutcome::Consumed => {}
        }
        true
    }

    /// Key handler for the branch panel: modal input / delete-confirm take
    /// precedence over list navigation; Esc returns to the view it was
    /// opened from.
    fn handle_branches_key(&mut self, key: KeyEvent) -> bool {
        // The input overlay and the delete confirm are mutually exclusive
        // (`start_branch_*` always clears the other flag); the input is
        // checked first, matching the config panel.
        if self.show_branch_input {
            match input_modal_key(key, &mut self.branch_input) {
                InputOutcome::Cancel => self.cancel_branch_create(),
                InputOutcome::Submit => self.submit_branch_create(),
                InputOutcome::Consumed => {}
            }
            true
        } else if self.show_branch_delete_confirm {
            match confirm_modal_key(key) {
                ConfirmOutcome::Yes => self.confirm_branch_delete(),
                ConfirmOutcome::No => self.cancel_branch_delete(),
                ConfirmOutcome::Ignored => {}
            }
            true
        } else {
            match key.code {
                KeyCode::Esc => {
                    self.leave_branches();
                    true
                }
                KeyCode::Char('q') => false,
                KeyCode::Char('j') | KeyCode::Down => {
                    move_selection(&mut self.branch_state, self.branches.len(), true);
                    true
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    move_selection(&mut self.branch_state, self.branches.len(), false);
                    true
                }
                KeyCode::Enter => {
                    self.switch_to_selected_branch();
                    true
                }
                KeyCode::Char('n') => {
                    self.start_branch_create();
                    true
                }
                KeyCode::Char('d') => {
                    self.start_branch_delete();
                    true
                }
                KeyCode::Char('r') => {
                    self.load_branches();
                    true
                }
                _ => true,
            }
        }
    }

    /// Key handler for the config panel: modal input / delete-confirm take
    /// precedence over list navigation; Esc returns to the dashboard, q
    /// exits the program.
    fn handle_config_key(&mut self, key: KeyEvent) -> bool {
        // The input overlay and the delete confirm are mutually exclusive
        // (`start_config_*` always clears the other flag); the input is
        // checked first, matching the branch panel.
        if self.show_config_input {
            match input_modal_key(key, &mut self.config_input) {
                InputOutcome::Cancel => self.cancel_config_add(),
                InputOutcome::Submit => self.submit_config_add(),
                InputOutcome::Consumed => {}
            }
            true
        } else if self.show_config_delete_confirm {
            match confirm_modal_key(key) {
                ConfirmOutcome::Yes => self.confirm_config_delete(),
                ConfirmOutcome::No => self.cancel_config_delete(),
                ConfirmOutcome::Ignored => {}
            }
            true
        } else {
            match key.code {
                KeyCode::Esc => {
                    self.leave_config();
                    true
                }
                KeyCode::Char('q') => false,
                KeyCode::Char('a') => {
                    self.start_config_add();
                    true
                }
                KeyCode::Char('d') => {
                    self.start_config_delete();
                    true
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    move_selection(&mut self.config_state, self.config.scan_dirs.len(), true);
                    true
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    move_selection(&mut self.config_state, self.config.scan_dirs.len(), false);
                    true
                }
                _ => true,
            }
        }
    }
}

/// Move the selection in `state` one row within a list of `len` items:
/// `down` clamps at the last row, otherwise at the first. An empty list
/// never selects (the cursor stays cleared).
fn move_selection(state: &mut ListState, len: usize, down: bool) {
    if len == 0 {
        return;
    }
    let next = if down {
        state
            .selected()
            .map_or(0, |i| i.saturating_add(1).min(len - 1))
    } else {
        state.selected().map_or(0, |i| i.saturating_sub(1))
    };
    state.select(Some(next));
}

/// Outcome of routing one key press through [`input_modal_key`].
enum InputOutcome {
    /// Esc: the caller cancels the overlay.
    Cancel,
    /// Enter: the caller submits the overlay.
    Submit,
    /// The key was consumed editing the buffer (or swallowed): the modal
    /// stays open and the caller does nothing further.
    Consumed,
}

/// Route one key press through the shared single-line input modal
/// (convention 9): Esc cancels, Enter submits; Backspace and plain
/// characters edit `buf` in place, while control/alt chords and any other
/// key are swallowed. Every key is consumed while the modal is open.
fn input_modal_key(key: KeyEvent, buf: &mut String) -> InputOutcome {
    match key.code {
        KeyCode::Esc => InputOutcome::Cancel,
        KeyCode::Enter => InputOutcome::Submit,
        KeyCode::Backspace => {
            buf.pop();
            InputOutcome::Consumed
        }
        KeyCode::Char(c)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            buf.push(c);
            InputOutcome::Consumed
        }
        _ => InputOutcome::Consumed,
    }
}

/// Outcome of routing one key press through [`confirm_modal_key`].
enum ConfirmOutcome {
    /// y/Y: the caller runs the confirmed action.
    Yes,
    /// n/N/Esc: the caller dismisses the dialog.
    No,
    /// Any other key, swallowed while the dialog is open.
    Ignored,
}

/// Route one key press through the shared y/n confirmation modal
/// (discard-style, convention 8): y/Y confirms, n/N/Esc cancels, and every
/// other key is swallowed while the dialog is open.
fn confirm_modal_key(key: KeyEvent) -> ConfirmOutcome {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => ConfirmOutcome::Yes,
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => ConfirmOutcome::No,
        _ => ConfirmOutcome::Ignored,
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
            app.set_error(format!("加载配置失败: {err:#}"));
            app
        }
    };
    app.refresh_repos();
    let run_result = event_loop(&mut terminal, &mut app);
    let restore_result: Result<()> = restore_terminal().map_err(Into::into);
    run_result.and(restore_result)
}

/// Event loop: draw, wait up to 200ms for input, then drain background
/// push/pull and batch progress so status and the dashboard batch log stay
/// live during remote ops (convention 1: the UI never blocks on an
/// in-flight operation).
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
        app.poll_batch();
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

    // ---------- helpers for temp-repository tests ----------

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("gitpulse-app-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn init_repo_with_commit(dir: &std::path::Path) {
        let repo = git2::Repository::init(dir).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "GitPulse Test").unwrap();
        config.set_str("user.email", "test@gitpulse.local").unwrap();
        drop(config);
        std::fs::write(dir.join("a.txt"), "v1\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("a.txt")).unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = git2::Signature::now("GitPulse Test", "test@gitpulse.local").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "v1", &tree, &[])
            .unwrap();
    }

    fn app_with_repo_at(dir: &std::path::Path) -> App {
        let mut app = App::new(Config::default());
        app.repos.push(git::query_status(dir).unwrap());
        app
    }

    fn index_has_staged(dir: &std::path::Path, rel: &str) -> bool {
        let repo = git2::Repository::open(dir).unwrap();
        let mut opts = git2::StatusOptions::new();
        let statuses = repo.statuses(Some(&mut opts)).unwrap();
        statuses.iter().any(|e| {
            e.path() == Some(rel)
                && (e.status().contains(git2::Status::INDEX_MODIFIED)
                    || e.status().contains(git2::Status::INDEX_NEW))
        })
    }

    // ---------- A: pure state machine ----------

    #[test]
    fn status_line_error_info_lifecycle() {
        let mut app = App::new(Config::default());
        assert!(app.status.is_none());

        app.set_error("bad");
        assert_eq!(
            app.status,
            Some(StatusLine {
                message: "bad".to_string(),
                is_error: true
            })
        );

        app.clear_error();
        assert!(app.status.is_none());

        app.set_info("good");
        assert!(!app.status.as_ref().unwrap().is_error);

        // set_info overwrites an error; the guard lives in set_info_unless_error.
        app.set_error("bad again");
        app.set_info("good again");
        assert!(!app.status.as_ref().unwrap().is_error);

        app.set_error("kept");
        app.set_info_unless_error("ignored");
        assert!(app.status.as_ref().unwrap().is_error);
        assert_eq!(app.status.as_ref().unwrap().message, "kept");

        app.clear_error();
        app.set_info_unless_error("applied");
        let st = app.status.as_ref().unwrap();
        assert!(!st.is_error);
        assert_eq!(st.message, "applied");
    }

    #[test]
    fn load_repos_discovers_temp_git_repo() {
        let dir = temp_dir("load-scan");
        init_repo_with_commit(&dir);

        let mut app = App::new(Config::default());
        app.config.scan_dirs = vec![dir.clone()];
        app.set_error("stale".to_string());
        app.load_repos();

        assert_eq!(app.repos.len(), 1);
        assert_eq!(app.repos[0].path, dir);
        assert!(app.status.is_none()); // success clears the prior error

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_repos_missing_scan_dir_yields_empty_without_error() {
        // discover_repos swallows per-directory I/O errors, so a missing scan
        // dir yields Ok([]) and load_repos clears the status instead of erroring.
        let missing =
            std::env::temp_dir().join(format!("gitpulse-app-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&missing);

        let mut app = App::new(Config::default());
        app.config.scan_dirs = vec![missing];
        app.load_repos();

        assert!(app.repos.is_empty());
        assert!(app.status.is_none());
    }

    #[test]
    fn refresh_repos_on_empty_list_clears_selection_without_panic() {
        let mut app = App::new(Config::default());
        app.dashboard_state.select(Some(4));
        app.refresh_repos();
        assert_eq!(app.dashboard_state.selected(), None);
        app.refresh_repos();
        assert_eq!(app.dashboard_state.selected(), None);
    }

    #[test]
    fn enter_repo_requires_valid_selection_and_loads_entries() {
        let dir = temp_dir("enter-repo");
        init_repo_with_commit(&dir);
        std::fs::write(dir.join("a.txt"), "dirty\n").unwrap();

        let mut app = app_with_repo_at(&dir);

        // No selection: no-op.
        app.enter_repo();
        assert_eq!(app.view, View::Dashboard);
        assert_eq!(app.current_repo, None);

        // Out-of-range selection: no-op.
        app.dashboard_state.select(Some(9));
        app.enter_repo();
        assert_eq!(app.view, View::Dashboard);

        // Valid selection: enters the repo view with entries loaded.
        app.dashboard_state.select(Some(0));
        app.set_error("stale".to_string());
        app.enter_repo();
        assert_eq!(app.view, View::Repo);
        assert_eq!(app.current_repo, Some(0));
        assert_eq!(app.file_entries.len(), 1);
        assert_eq!(app.file_state.selected(), Some(0));
        assert!(app.status.is_none());

        // load_file_entries: no-op without a current repo or with a stale index.
        let snapshot = app.file_entries.len();
        app.current_repo = None;
        app.load_file_entries();
        assert_eq!(app.file_entries.len(), snapshot);
        app.current_repo = Some(7);
        app.load_file_entries();
        assert_eq!(app.file_entries.len(), snapshot);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn commit_flow_open_submit_and_error_paths() {
        let mut app = App::new(Config::default());

        // open_commit switches to the commit input...
        app.view = View::Repo;
        app.open_commit();
        assert_eq!(app.view, View::Commit);
        assert!(app.commit_message.is_empty());

        // ...but is blocked while a remote operation is in flight.
        let (_tx, rx) = mpsc::unbounded_channel();
        app.remote_rx = Some(rx);
        app.view = View::Repo;
        app.open_commit();
        assert_eq!(app.view, View::Repo);
        app.remote_rx = None;

        // Submit without an open repo: error, returns to the repo view,
        // typed message kept.
        app.view = View::Commit;
        app.commit_message = "msg".to_string();
        app.submit_commit();
        assert_eq!(app.view, View::Repo);
        let st = app.status.as_ref().unwrap();
        assert!(st.is_error);
        assert!(
            st.message.contains("commit 失败: 未打开仓库"),
            "{}",
            st.message
        );
        assert_eq!(app.commit_message, "msg");

        // Stale repo index: distinct error.
        let mut app = App::new(Config::default());
        app.view = View::Commit;
        app.current_repo = Some(3);
        app.submit_commit();
        assert_eq!(app.view, View::Repo);
        let st = app.status.as_ref().unwrap();
        assert!(st.is_error);
        assert!(st.message.contains("仓库索引失效"), "{}", st.message);

        // Empty message with an open repo: git rejects it before opening.
        let dir = temp_dir("commit-empty");
        init_repo_with_commit(&dir);
        let mut app = app_with_repo_at(&dir);
        app.current_repo = Some(0);
        app.view = View::Commit;
        app.submit_commit();
        assert_eq!(app.view, View::Repo);
        let st = app.status.as_ref().unwrap();
        assert!(st.is_error);
        assert!(
            st.message.contains("commit message 不能为空"),
            "{}",
            st.message
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discard_dialog_flag_lifecycle_without_selection() {
        let mut app = App::new(Config::default());
        app.start_discard();
        assert!(app.show_discard_confirm);
        app.cancel_discard();
        assert!(!app.show_discard_confirm);

        app.start_discard();
        app.confirm_discard(); // no repo/selection: only closes the dialog
        assert!(!app.show_discard_confirm);
        assert!(app.status.is_none());
    }

    #[test]
    fn dashboard_keys_with_empty_repo_list_do_not_panic() {
        let mut app = App::new(Config::default());
        assert!(app.handle_key(key(KeyCode::Char('j'))));
        assert_eq!(app.dashboard_state.selected(), None);
        assert!(app.handle_key(key(KeyCode::Char('k'))));
        assert_eq!(app.dashboard_state.selected(), None);
        assert!(app.handle_key(key(KeyCode::Down)));
        assert!(app.handle_key(key(KeyCode::Up)));
        assert_eq!(app.dashboard_state.selected(), None);

        // Enter and b are no-ops without a selection.
        assert!(app.handle_key(key(KeyCode::Enter)));
        assert_eq!(app.view, View::Dashboard);
        assert!(app.handle_key(key(KeyCode::Char('b'))));
        assert_eq!(app.view, View::Dashboard);

        // l with no repos reports info and never starts a batch.
        assert!(app.handle_key(key(KeyCode::Char('l'))));
        let st = app.status.as_ref().unwrap();
        assert!(!st.is_error);
        assert!(st.message.contains("没有仓库"), "{}", st.message);
        assert!(!app.batch_running());

        assert!(!app.handle_key(key(KeyCode::Esc)));
    }

    #[test]
    fn repo_view_keys_route_to_commit_branches_and_dashboard() {
        let dir = temp_dir("repo-keys");
        init_repo_with_commit(&dir);
        let mut app = app_with_repo_at(&dir);
        app.dashboard_state.select(Some(0));
        app.enter_repo();
        assert_eq!(app.view, View::Repo);

        // c opens the commit input; Esc cancels back to the repo view.
        assert!(app.handle_key(key(KeyCode::Char('c'))));
        assert_eq!(app.view, View::Commit);
        app.commit_message = "draft".to_string();
        assert!(app.handle_key(key(KeyCode::Esc)));
        assert_eq!(app.view, View::Repo);
        assert!(app.commit_message.is_empty());

        // x opens the discard dialog, n cancels it.
        assert!(app.handle_key(key(KeyCode::Char('x'))));
        assert!(app.show_discard_confirm);
        assert!(app.handle_key(key(KeyCode::Char('n'))));
        assert!(!app.show_discard_confirm);

        // File-list keys on an empty list are safe no-ops.
        assert!(app.file_entries.is_empty());
        let sel = app.file_state.selected();
        assert!(app.handle_key(key(KeyCode::Char('j'))));
        assert_eq!(app.file_state.selected(), sel);
        assert!(app.handle_key(key(KeyCode::Char('k'))));
        assert_eq!(app.file_state.selected(), sel);

        // b opens the branch panel with the repo view as origin; Esc returns.
        assert!(app.handle_key(key(KeyCode::Char('b'))));
        assert_eq!(app.view, View::Branches);
        assert!(!app.branches.is_empty());
        assert!(app.handle_key(key(KeyCode::Esc)));
        assert_eq!(app.view, View::Repo);

        // p / l are no-ops without a current repo (nothing is spawned).
        app.current_repo = None;
        assert!(app.handle_key(key(KeyCode::Char('p'))));
        assert!(app.remote_rx.is_none());
        assert!(app.handle_key(key(KeyCode::Char('l'))));
        assert!(app.remote_rx.is_none());
        app.current_repo = Some(0);

        // Esc steps back to the dashboard; q exits from the repo view.
        assert!(app.handle_key(key(KeyCode::Esc)));
        assert_eq!(app.view, View::Dashboard);
        app.view = View::Repo;
        assert!(!app.handle_key(key(KeyCode::Char('q'))));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn diff_view_keys_scroll_and_exit() {
        let dir = temp_dir("diff-keys");
        init_repo_with_commit(&dir);
        std::fs::write(dir.join("a.txt"), "changed\n").unwrap();

        let mut app = app_with_repo_at(&dir);
        app.dashboard_state.select(Some(0));
        app.enter_repo();

        // d opens the diff of the modified tracked file.
        assert!(app.handle_key(key(KeyCode::Char('d'))));
        assert_eq!(app.view, View::Diff);
        assert!(!app.diff_content.is_empty());
        assert_eq!(app.diff_scroll, 0);

        assert!(app.handle_key(key(KeyCode::Char('j'))));
        assert_eq!(app.diff_scroll, 1);
        assert!(app.handle_key(key(KeyCode::Down)));
        assert_eq!(app.diff_scroll, 2);
        assert!(app.handle_key(key(KeyCode::Char('k'))));
        assert_eq!(app.diff_scroll, 1);
        assert!(app.handle_key(key(KeyCode::Up)));
        assert_eq!(app.diff_scroll, 0);
        assert!(app.handle_key(key(KeyCode::Char('k')))); // saturates at 0
        assert_eq!(app.diff_scroll, 0);

        assert!(!app.handle_key(key(KeyCode::Char('q'))));
        assert!(app.handle_key(key(KeyCode::Esc)));
        assert_eq!(app.view, View::Repo);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn view_file_diff_reports_error_for_untracked_file() {
        let dir = temp_dir("diff-untracked");
        init_repo_with_commit(&dir);
        std::fs::write(dir.join("new.txt"), "x\n").unwrap();

        let mut app = app_with_repo_at(&dir);
        app.dashboard_state.select(Some(0));
        app.enter_repo();
        assert_eq!(app.file_entries.len(), 1);

        // Untracked files produce no HEAD→workdir diff: error, stay in Repo.
        assert!(app.handle_key(key(KeyCode::Char('d'))));
        assert_eq!(app.view, View::Repo);
        let st = app.status.as_ref().unwrap();
        assert!(st.is_error);
        assert!(st.message.contains("加载 diff 失败"), "{}", st.message);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn commit_view_keys_edit_submit_and_cancel() {
        let mut app = App::new(Config::default());
        app.view = View::Commit;

        assert!(app.handle_key(key(KeyCode::Char('h'))));
        assert!(app.handle_key(key(KeyCode::Char('i'))));
        assert_eq!(app.commit_message, "hi");

        // Ctrl+char does not type into the input.
        let ctrl = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL);
        assert!(app.handle_key(ctrl));
        assert_eq!(app.commit_message, "hi");

        assert!(app.handle_key(key(KeyCode::Backspace)));
        assert_eq!(app.commit_message, "h");

        // Esc cancels: back to the repo view with a cleared input.
        assert!(app.handle_key(key(KeyCode::Esc)));
        assert_eq!(app.view, View::Repo);
        assert!(app.commit_message.is_empty());

        // Enter submits (no repo here → error, returns to the repo view).
        app.view = View::Commit;
        app.commit_message = "msg".to_string();
        assert!(app.handle_key(key(KeyCode::Enter)));
        assert_eq!(app.view, View::Repo);
        assert!(app.status.as_ref().unwrap().is_error);
    }

    #[test]
    fn stage_and_unstage_keys_update_the_index() {
        let dir = temp_dir("stage");
        init_repo_with_commit(&dir);
        std::fs::write(dir.join("a.txt"), "v2\n").unwrap();

        let mut app = app_with_repo_at(&dir);
        app.dashboard_state.select(Some(0));
        app.enter_repo();
        assert_eq!(app.file_entries.len(), 1);
        assert!(!index_has_staged(&dir, "a.txt"));

        assert!(app.handle_key(key(KeyCode::Char('s'))));
        assert!(index_has_staged(&dir, "a.txt"));
        assert!(app.status.is_none());

        assert!(app.handle_key(key(KeyCode::Char('u'))));
        assert!(!index_has_staged(&dir, "a.txt"));
        assert!(app.status.is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stage_unstage_discard_on_broken_repo_report_errors() {
        let mut app = app_with_repos(1); // fake path: every git call fails
        app.current_repo = Some(0);
        app.file_state.select(Some(0));
        app.file_entries.push(FileEntry {
            path: std::path::PathBuf::from("a.txt"),
            state: crate::git::FileState::Modified,
        });

        app.stage_current();
        let st = app.status.as_ref().unwrap();
        assert!(st.is_error);
        assert!(st.message.contains("stage 失败"), "{}", st.message);

        app.unstage_current();
        let st = app.status.as_ref().unwrap();
        assert!(st.is_error);
        assert!(st.message.contains("unstage 失败"), "{}", st.message);

        app.start_discard();
        app.confirm_discard();
        assert!(!app.show_discard_confirm);
        let st = app.status.as_ref().unwrap();
        assert!(st.is_error);
        assert!(st.message.contains("discard 失败"), "{}", st.message);
    }

    #[test]
    fn batch_running_tracks_receiver_presence() {
        let mut app = App::new(Config::default());
        assert!(!app.batch_running());
        let (_tx, rx) = mpsc::unbounded_channel();
        app.batch_rx = Some(rx);
        assert!(app.batch_running());
    }

    #[test]
    fn branch_repo_name_reports_empty_for_invalid_index() {
        let mut app = App::new(Config::default());
        assert_eq!(app.branch_repo_name(), "");
        app.branch_repo = Some(0);
        assert_eq!(app.branch_repo_name(), "");

        let dir = temp_dir("branch-name");
        init_repo_with_commit(&dir);
        let mut app = app_with_repo_at(&dir);
        app.branch_repo = Some(0);
        assert_eq!(
            app.branch_repo_name(),
            dir.file_name().unwrap().to_str().unwrap()
        );
        app.branch_repo = Some(5);
        assert_eq!(app.branch_repo_name(), "");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn handle_key_ignores_release_events() {
        let mut app = app_with_repos(2);
        let mut k = key(KeyCode::Char('j'));
        k.kind = KeyEventKind::Release;
        assert!(app.handle_key(k));
        assert_eq!(app.dashboard_state.selected(), None);
    }

    // ---------- config panel skeleton ----------

    #[test]
    fn dashboard_c_enters_config_and_esc_returns() {
        let mut app = App::new(Config::default());
        app.set_error("stale".to_string());

        assert!(app.handle_key(key(KeyCode::Char('c'))));
        assert_eq!(app.view, View::Config);
        assert!(app.status.is_none()); // entering clears the stale status

        assert!(app.handle_key(key(KeyCode::Esc)));
        assert_eq!(app.view, View::Dashboard);
    }

    #[test]
    fn config_keys_navigate_dirs_and_quit() {
        let mut app = App::new(Config::default());
        app.config.scan_dirs = vec![
            std::path::PathBuf::from("/a"),
            std::path::PathBuf::from("/b"),
        ];
        app.enter_config();
        assert_eq!(app.config_state.selected(), Some(0));

        assert!(app.handle_key(key(KeyCode::Char('j'))));
        assert_eq!(app.config_state.selected(), Some(1));
        assert!(app.handle_key(key(KeyCode::Char('j')))); // clamps at the end
        assert_eq!(app.config_state.selected(), Some(1));
        assert!(app.handle_key(key(KeyCode::Down)));
        assert_eq!(app.config_state.selected(), Some(1));
        assert!(app.handle_key(key(KeyCode::Char('k'))));
        assert_eq!(app.config_state.selected(), Some(0));
        assert!(app.handle_key(key(KeyCode::Char('k')))); // clamps at the start
        assert_eq!(app.config_state.selected(), Some(0));

        // q exits the program from the config panel.
        assert!(!app.handle_key(key(KeyCode::Char('q'))));
    }

    #[test]
    fn config_enter_leave_handles_empty_list_and_stale_selection() {
        let mut app = App::new(Config::default());

        // Empty list: no selection, Esc returns to the dashboard.
        app.enter_config();
        assert_eq!(app.view, View::Config);
        assert_eq!(app.config_state.selected(), None);
        assert!(app.handle_key(key(KeyCode::Char('j'))));
        assert_eq!(app.config_state.selected(), None);
        app.leave_config();
        assert_eq!(app.view, View::Dashboard);

        // Non-empty list: a stale out-of-range cursor is re-clamped to the
        // first entry; a valid cursor survives a re-entry.
        app.config.scan_dirs = vec![std::path::PathBuf::from("/a")];
        app.config_state.select(Some(7));
        app.enter_config();
        assert_eq!(app.config_state.selected(), Some(0));
        app.leave_config();
        app.enter_config();
        assert_eq!(app.config_state.selected(), Some(0));
    }

    #[test]
    fn config_add_validation_reports_each_failure() {
        let mut app = App::new(Config::default());
        app.enter_config();

        // a opens a clean input; keys type and edit the path.
        assert!(app.handle_key(key(KeyCode::Char('a'))));
        assert!(app.show_config_input);
        assert!(app.config_input.is_empty());
        for c in "ab".chars() {
            assert!(app.handle_key(key(KeyCode::Char(c))));
        }
        assert_eq!(app.config_input, "ab");
        assert!(app.handle_key(key(KeyCode::Backspace)));
        assert_eq!(app.config_input, "a");
        // Ctrl+char does not type into the input.
        let ctrl = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL);
        assert!(app.handle_key(ctrl));
        assert_eq!(app.config_input, "a");

        // Empty path: error, input stays open.
        app.cancel_config_add();
        app.start_config_add();
        app.submit_config_add();
        let st = app.status.as_ref().unwrap();
        assert!(st.is_error);
        assert_eq!(st.message, "目录不能为空");
        assert!(app.show_config_input);

        // Path does not exist: error, input stays open with the typed text.
        let missing =
            std::env::temp_dir().join(format!("gitpulse-config-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&missing);
        app.config_input = missing.display().to_string();
        app.submit_config_add();
        let st = app.status.as_ref().unwrap();
        assert!(st.is_error);
        assert_eq!(st.message, format!("目录不存在: {}", missing.display()));
        assert!(app.show_config_input);
        assert_eq!(app.config_input, missing.display().to_string());

        // Path exists but is not a directory: distinct error.
        let file =
            std::env::temp_dir().join(format!("gitpulse-config-file-{}.txt", std::process::id()));
        std::fs::write(&file, "x").unwrap();
        app.config_input = file.display().to_string();
        app.submit_config_add();
        let st = app.status.as_ref().unwrap();
        assert!(st.is_error);
        assert_eq!(st.message, format!("不是目录: {}", file.display()));

        // Duplicate: rejected without touching the list.
        let dup = temp_dir("config-dup");
        app.config.scan_dirs = vec![dup.clone()];
        app.config_input = dup.display().to_string();
        app.submit_config_add();
        let st = app.status.as_ref().unwrap();
        assert!(st.is_error);
        assert_eq!(st.message, "目录已在列表中");
        assert_eq!(app.config.scan_dirs, vec![dup.clone()]);

        // Esc cancels the input but stays in the config panel.
        assert!(app.handle_key(key(KeyCode::Esc)));
        assert!(!app.show_config_input);
        assert!(app.config_input.is_empty());
        assert_eq!(app.view, View::Config);

        let _ = std::fs::remove_file(&file);
        let _ = std::fs::remove_dir_all(&dup);
    }

    #[test]
    fn config_add_success_saves_persists_and_refreshes() {
        // Holds the I/O guard: the save below writes the redirected config.
        let _io = crate::config::config_io_guard();

        let dir = temp_dir("config-add");
        init_repo_with_commit(&dir);

        let mut app = App::new(Config::default());
        app.enter_config();
        app.handle_key(key(KeyCode::Char('a')));
        for c in dir.display().to_string().chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        assert!(app.handle_key(key(KeyCode::Enter)));

        // Input closed and cleared, list updated, new entry selected.
        assert!(!app.show_config_input);
        assert!(app.config_input.is_empty());
        assert_eq!(app.config.scan_dirs, vec![dir.clone()]);
        assert_eq!(app.config_state.selected(), Some(0));

        // Persisted to the config file (restart would load the same dirs).
        let loaded = Config::load().expect("load saved config");
        assert_eq!(loaded.scan_dirs, vec![dir.clone()]);

        // Auto-rescan discovered the repository under the new scan dir.
        assert_eq!(app.repos.len(), 1);
        assert_eq!(app.repos[0].path, dir);

        // Success message survives the refresh (which clears the status).
        let st = app.status.as_ref().unwrap();
        assert!(!st.is_error, "{}", st.message);
        assert_eq!(st.message, format!("已添加扫描目录: {}", dir.display()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---------- B: branch panel ----------

    #[test]
    fn enter_branches_loads_list_and_head_selection() {
        let dir = temp_dir("enter-branches");
        init_repo_with_commit(&dir);
        git::create_branch(&dir, "feature").unwrap();

        let mut app = app_with_repo_at(&dir);
        app.dashboard_state.select(Some(0));
        assert!(app.handle_key(key(KeyCode::Char('b'))));
        assert_eq!(app.view, View::Branches);
        assert_eq!(app.branches.len(), 2);
        let sel = app.branch_state.selected().unwrap();
        assert!(app.branches[sel].is_head);
        assert_eq!(app.branch_repo, Some(0));
        assert_eq!(app.branches_from, View::Dashboard);

        assert!(app.handle_key(key(KeyCode::Esc)));
        assert_eq!(app.view, View::Dashboard);
        assert!(app.status.is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn enter_branches_is_noop_without_valid_selection() {
        // Nothing selected at all.
        let mut app = App::new(Config::default());
        app.enter_branches();
        assert_eq!(app.view, View::Dashboard);

        // Selection past the end of the repo list.
        let mut app = app_with_repos(1);
        app.dashboard_state.select(Some(5));
        app.enter_branches();
        assert_eq!(app.view, View::Dashboard);

        // Unsupported origin view.
        app.view = View::Diff;
        app.enter_branches();
        assert_eq!(app.view, View::Diff);

        // Repo view without a current repo.
        let mut app = app_with_repos(1);
        app.view = View::Repo;
        app.enter_branches();
        assert_eq!(app.view, View::Repo);
    }

    #[test]
    fn branches_view_keys_navigate_and_open_create_input() {
        let dir = temp_dir("branch-keys");
        init_repo_with_commit(&dir);
        git::create_branch(&dir, "feature").unwrap();

        let mut app = app_with_repo_at(&dir);
        app.dashboard_state.select(Some(0));
        app.enter_branches();
        assert_eq!(app.branches.len(), 2);

        let head_idx = app.branch_state.selected().unwrap();
        assert!(app.branches[head_idx].is_head);

        // j / k move within the list.
        assert!(app.handle_key(key(KeyCode::Char('j'))));
        assert_eq!(app.branch_state.selected(), Some(head_idx + 1));
        assert!(app.handle_key(key(KeyCode::Char('k'))));
        assert_eq!(app.branch_state.selected(), Some(head_idx));

        // n opens the create input; Esc closes it but stays in the panel.
        assert!(app.handle_key(key(KeyCode::Char('n'))));
        assert!(app.show_branch_input);
        assert!(app.handle_key(key(KeyCode::Esc)));
        assert!(!app.show_branch_input);
        assert_eq!(app.view, View::Branches);

        // q exits from the list; Esc leaves the panel back to the dashboard.
        assert!(!app.handle_key(key(KeyCode::Char('q'))));
        assert!(app.handle_key(key(KeyCode::Esc)));
        assert_eq!(app.view, View::Dashboard);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn switch_to_selected_branch_updates_repo_and_status() {
        let dir = temp_dir("switch");
        init_repo_with_commit(&dir);
        git::create_branch(&dir, "feature").unwrap();

        let mut app = app_with_repo_at(&dir);
        app.dashboard_state.select(Some(0));
        app.enter_branches();

        // No selection: no-op.
        app.branch_state.select(None);
        app.switch_to_selected_branch();
        assert!(app.status.is_none());

        let idx = app
            .branches
            .iter()
            .position(|b| b.name == "feature")
            .unwrap();
        app.branch_state.select(Some(idx));
        app.switch_to_selected_branch();

        let st = app.status.as_ref().unwrap();
        assert!(!st.is_error, "{}", st.message);
        assert_eq!(st.message, "已切换到 feature");
        assert_eq!(app.repos[0].branch, "feature");
        let head = app.branches.iter().find(|b| b.is_head).unwrap();
        assert_eq!(head.name, "feature");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn branch_create_flow_start_submit_and_errors() {
        let dir = temp_dir("branch-create");
        init_repo_with_commit(&dir);

        let mut app = app_with_repo_at(&dir);
        app.dashboard_state.select(Some(0));
        app.enter_branches();
        let initial = app.branches.len();

        // start_branch_create opens a clean input.
        app.start_branch_create();
        assert!(app.show_branch_input);
        assert!(!app.show_branch_delete_confirm);
        assert!(app.branch_input.is_empty());

        // Empty name: error, input stays open.
        app.submit_branch_create();
        let st = app.status.as_ref().unwrap();
        assert!(st.is_error);
        assert!(st.message.contains("分支名不能为空"), "{}", st.message);
        assert!(app.show_branch_input);

        app.cancel_branch_create();
        assert!(!app.show_branch_input);
        assert!(app.branch_input.is_empty());

        // Create via keys: n, type, backspace, Enter.
        assert!(app.handle_key(key(KeyCode::Char('n'))));
        for c in "feature".chars() {
            assert!(app.handle_key(key(KeyCode::Char(c))));
        }
        assert_eq!(app.branch_input, "feature");
        assert!(app.handle_key(key(KeyCode::Backspace)));
        assert_eq!(app.branch_input, "featur");
        assert!(app.handle_key(key(KeyCode::Char('e'))));
        assert!(app.handle_key(key(KeyCode::Enter)));

        assert!(!app.show_branch_input);
        assert_eq!(app.branches.len(), initial + 1);
        assert!(app.branches.iter().any(|b| b.name == "feature"));
        let sel = app.branch_state.selected().unwrap();
        assert_eq!(app.branches[sel].name, "feature");
        let st = app.status.as_ref().unwrap();
        assert!(!st.is_error, "{}", st.message);
        assert!(st.message.contains("已创建分支 feature"), "{}", st.message);

        // Duplicate name: error, typed text kept for correction.
        app.start_branch_create();
        app.branch_input = "feature".to_string();
        app.submit_branch_create();
        let st = app.status.as_ref().unwrap();
        assert!(st.is_error);
        assert!(st.message.contains("创建分支失败"), "{}", st.message);
        assert!(app.show_branch_input);
        assert_eq!(app.branch_input, "feature");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn branch_delete_flow_refuses_head_removes_feature() {
        let dir = temp_dir("branch-delete");
        init_repo_with_commit(&dir);
        git::create_branch(&dir, "feature").unwrap();

        let mut app = app_with_repo_at(&dir);
        app.dashboard_state.select(Some(0));
        app.enter_branches();

        // No selection: start is a no-op.
        app.branch_state.select(None);
        app.start_branch_delete();
        assert!(!app.show_branch_delete_confirm);

        // Head branch: d opens, Esc cancels...
        let head_idx = app.branches.iter().position(|b| b.is_head).unwrap();
        app.branch_state.select(Some(head_idx));
        assert!(app.handle_key(key(KeyCode::Char('d'))));
        assert!(app.show_branch_delete_confirm);
        assert!(app.handle_key(key(KeyCode::Esc)));
        assert!(!app.show_branch_delete_confirm);
        assert_eq!(app.view, View::Branches);

        // ...d + n cancels as well...
        assert!(app.handle_key(key(KeyCode::Char('d'))));
        assert!(app.handle_key(key(KeyCode::Char('n'))));
        assert!(!app.show_branch_delete_confirm);

        // ...and d + Y on the head branch is refused: branch kept, error shown.
        assert!(app.handle_key(key(KeyCode::Char('d'))));
        assert!(app.handle_key(key(KeyCode::Char('Y'))));
        assert!(!app.show_branch_delete_confirm);
        let st = app.status.as_ref().unwrap();
        assert!(st.is_error);
        assert!(st.message.contains("删除分支失败"), "{}", st.message);
        assert_eq!(app.branches.len(), 2);

        // Non-head branch: confirmed delete removes it and refreshes the list.
        let idx = app
            .branches
            .iter()
            .position(|b| b.name == "feature")
            .unwrap();
        app.branch_state.select(Some(idx));
        app.start_branch_delete();
        assert!(app.show_branch_delete_confirm);
        app.confirm_branch_delete();
        assert!(!app.show_branch_delete_confirm);
        assert!(!app.branches.iter().any(|b| b.name == "feature"));
        assert!(app.branches.iter().any(|b| b.is_head));
        let st = app.status.as_ref().unwrap();
        assert!(!st.is_error, "{}", st.message);
        assert!(st.message.contains("已删除分支 feature"), "{}", st.message);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn branch_panel_index_edges_report_errors() {
        let mut app = App::new(Config::default());
        app.branch_repo = Some(0); // stale index: no such repo
        app.load_branches();
        let st = app.status.as_ref().unwrap();
        assert!(st.is_error);
        assert!(
            st.message.contains("分支面板: 仓库索引失效"),
            "{}",
            st.message
        );
        assert!(app.branches.is_empty());
        assert_eq!(app.branch_state.selected(), None);

        app.branch_input = "x".to_string();
        app.submit_branch_create();
        let st = app.status.as_ref().unwrap();
        assert!(st.is_error);
        assert!(
            st.message.contains("创建分支失败: 仓库索引失效"),
            "{}",
            st.message
        );

        app.start_branch_create(); // no valid repo path: stays closed
        assert!(!app.show_branch_input);
    }

    // ---------- C: async paths (tokio) and temp-repo side effects ----------

    #[tokio::test]
    async fn start_remote_guards_and_valid_start() {
        let dir = temp_dir("remote-start");
        init_repo_with_commit(&dir);

        // Outside the repo view: no-op.
        let mut app = app_with_repo_at(&dir);
        app.start_remote("push");
        assert!(app.remote_rx.is_none());

        // Repo view without a current repo: no-op.
        app.view = View::Repo;
        app.start_remote("push");
        assert!(app.remote_rx.is_none());

        // Batch operation in flight: no-op.
        let mut app = app_with_repo_at(&dir);
        app.view = View::Repo;
        app.current_repo = Some(0);
        let (btx, brx) = mpsc::unbounded_channel();
        app.batch_rx = Some(brx);
        app.start_remote("push");
        assert!(app.remote_rx.is_none());
        app.batch_rx = None;
        drop(btx);

        // Remote already running: label untouched.
        let (rtx, rrx) = mpsc::unbounded_channel::<RemoteEvent>();
        app.remote_rx = Some(rrx);
        app.remote_op = "push";
        app.start_remote("pull");
        assert_eq!(app.remote_op, "push");
        app.remote_rx = None;
        drop(rtx);

        // Valid start: arms the receiver, initial progress, clears the status.
        app.status = Some(StatusLine {
            message: "old".to_string(),
            is_error: true,
        });
        app.start_remote("pull");
        assert!(app.remote_rx.is_some());
        assert_eq!(app.remote_op, "pull");
        assert!(matches!(app.remote_status, Some(RemoteStatus::Progress(_))));
        assert!(app.status.is_none());
    }

    #[test]
    fn poll_remote_drains_progress_done_and_disconnect() {
        let mut app = App::new(Config::default());

        // No receiver: no-op.
        app.poll_remote();

        let (tx, rx) = mpsc::unbounded_channel();
        app.remote_rx = Some(rx);
        tx.send(RemoteEvent::Progress(Progress {
            percent: 40,
            message: "40%".to_string(),
        }))
        .unwrap();
        app.poll_remote();
        match &app.remote_status {
            Some(RemoteStatus::Progress(p)) => {
                assert_eq!(p.percent, 40);
                assert_eq!(p.message, "40%");
            }
            other => panic!("expected progress, got {other:?}"),
        }
        assert!(app.remote_rx.is_some()); // still in flight

        tx.send(RemoteEvent::Done(Err("boom".to_string()))).unwrap();
        app.poll_remote();
        assert_eq!(
            app.remote_status,
            Some(RemoteStatus::Error("boom".to_string()))
        );
        assert!(app.remote_rx.is_none());

        // Done(Ok) closes the receiver; a refresh failure on a non-repo path
        // surfaces as a status error.
        let broken = temp_dir("remote-refresh"); // exists but is not a git repo
        let mut app = App::new(Config::default());
        app.repos.push(RepoInfo {
            name: "broken".to_string(),
            path: broken.clone(),
            branch: "main".to_string(),
            dirty: Default::default(),
            ahead: 0,
            behind: 0,
            last_commit: Default::default(),
        });
        app.current_repo = Some(0);
        let (tx, rx) = mpsc::unbounded_channel();
        app.remote_rx = Some(rx);
        tx.send(RemoteEvent::Done(Ok(()))).unwrap();
        app.poll_remote();
        assert_eq!(app.remote_status, Some(RemoteStatus::Success));
        assert!(app.remote_rx.is_none());
        let st = app.status.as_ref().expect("refresh failure must surface");
        assert!(st.is_error, "{}", st.message);
        let _ = std::fs::remove_dir_all(&broken);

        // Sender dropped without Done: explicit disconnect error.
        let mut app = App::new(Config::default());
        let (tx, rx) = mpsc::unbounded_channel::<RemoteEvent>();
        app.remote_rx = Some(rx);
        drop(tx);
        app.poll_remote();
        assert_eq!(
            app.remote_status,
            Some(RemoteStatus::Error("远端任务意外退出".to_string()))
        );
        assert!(app.remote_rx.is_none());
    }

    #[tokio::test]
    async fn start_batch_guards_info_and_valid_start() {
        // No repos: info message, nothing armed.
        let mut app = App::new(Config::default());
        app.start_batch("pull");
        assert!(!app.batch_running());
        let st = app.status.as_ref().unwrap();
        assert!(!st.is_error);
        assert!(st.message.contains("没有仓库"), "{}", st.message);

        // Outside the dashboard: no-op.
        let mut app = app_with_repos(2);
        app.view = View::Repo;
        app.start_batch("pull");
        assert!(!app.batch_running());
        assert!(app.status.is_none());

        // Batch already running: label and log untouched.
        let mut app = app_with_repos(2);
        app.batch_op = "pull";
        app.batch_log
            .push(BatchLogLine::new("stale".to_string(), "进行中".to_string()));
        let (btx, brx) = mpsc::unbounded_channel();
        app.batch_rx = Some(brx);
        app.start_batch("fetch");
        assert_eq!(app.batch_op, "pull");
        assert_eq!(app.batch_log.len(), 1);
        app.batch_rx = None;
        drop(btx);

        // Remote in flight: no-op.
        let mut app = app_with_repos(2);
        let (rtx, rrx) = mpsc::unbounded_channel();
        app.remote_rx = Some(rrx);
        app.start_batch("fetch");
        assert!(!app.batch_running());
        app.remote_rx = None;
        drop(rtx);

        // Valid start: arms the receiver and resets log/status.
        let mut app = app_with_repos(2);
        app.batch_log
            .push(BatchLogLine::new("stale".to_string(), "进行中".to_string()));
        app.set_error("stale error".to_string());
        app.start_batch("fetch");
        assert!(app.batch_running());
        assert_eq!(app.batch_op, "fetch");
        assert!(app.batch_log.is_empty());
        assert!(app.status.is_none());
    }

    #[test]
    fn poll_batch_line_dedup_done_summaries_and_disconnect() {
        let mut app = App::new(Config::default());

        // No receiver: no-op.
        app.poll_batch();

        app.batch_op = "pull";
        let (tx, rx) = mpsc::unbounded_channel();
        app.batch_rx = Some(rx);

        // Same index keeps one advancing line; a new index appends.
        tx.send(BatchEvent::Line {
            index: 0,
            name: "r0".to_string(),
            message: "进行中".to_string(),
        })
        .unwrap();
        tx.send(BatchEvent::Line {
            index: 0,
            name: "r0".to_string(),
            message: "成功".to_string(),
        })
        .unwrap();
        tx.send(BatchEvent::Line {
            index: 1,
            name: "r1".to_string(),
            message: "进行中".to_string(),
        })
        .unwrap();
        app.poll_batch();
        assert_eq!(app.batch_log.len(), 2);
        assert_eq!(app.batch_log[0].message, "成功");
        assert!(app.batch_log[0].is_success);
        assert!(!app.batch_log[0].is_error);
        assert_eq!(app.batch_log[1].message, "进行中");
        assert!(!app.batch_log[1].is_success);
        assert!(app.batch_running());

        // Done with all successes → info summary.
        tx.send(BatchEvent::Done(vec![
            ("r0".to_string(), Ok(())),
            ("r1".to_string(), Ok(())),
        ]))
        .unwrap();
        app.poll_batch();
        assert!(!app.batch_running());
        let st = app.status.as_ref().unwrap();
        assert!(!st.is_error);
        assert!(
            st.message.contains("批量 pull 完成: 2 个仓库成功"),
            "{}",
            st.message
        );

        // Done with a partial failure → error summary.
        let (tx, rx) = mpsc::unbounded_channel();
        app.batch_rx = Some(rx);
        app.batch_op = "fetch";
        tx.send(BatchEvent::Done(vec![
            ("a".to_string(), Ok(())),
            ("b".to_string(), Err(anyhow::anyhow!("nope"))),
        ]))
        .unwrap();
        app.poll_batch();
        assert!(!app.batch_running());
        let st = app.status.as_ref().unwrap();
        assert!(st.is_error);
        assert!(
            st.message.contains("批量 fetch 完成: 1 成功 / 1 失败"),
            "{}",
            st.message
        );

        // Sender dropped without Done: explicit disconnect error.
        let (tx, rx) = mpsc::unbounded_channel::<BatchEvent>();
        app.batch_rx = Some(rx);
        drop(tx);
        app.poll_batch();
        assert!(!app.batch_running());
        let st = app.status.as_ref().unwrap();
        assert!(st.is_error);
        assert!(st.message.contains("批量任务意外退出"), "{}", st.message);
    }

    #[test]
    fn submit_commit_success_creates_commit_and_refreshes() {
        let dir = temp_dir("commit-ok");
        init_repo_with_commit(&dir);
        std::fs::write(dir.join("b.txt"), "second\n").unwrap();
        git::stage(&dir, std::path::Path::new("b.txt")).unwrap();

        let mut app = app_with_repo_at(&dir);
        app.dashboard_state.select(Some(0));
        app.enter_repo();
        app.open_commit();
        assert_eq!(app.view, View::Commit);
        app.commit_message = "add b.txt".to_string();
        app.submit_commit();

        assert_eq!(app.view, View::Repo);
        assert!(app.commit_message.is_empty());
        assert!(app.file_entries.is_empty());
        assert_eq!(app.repos[0].last_commit.summary, "add b.txt");
        let d = &app.repos[0].dirty;
        assert_eq!((d.added, d.modified, d.deleted, d.untracked), (0, 0, 0, 0));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn confirm_discard_restores_tracked_file_and_removes_untracked() {
        let dir = temp_dir("discard");
        init_repo_with_commit(&dir);
        std::fs::write(dir.join("a.txt"), "changed\n").unwrap();

        let mut app = app_with_repo_at(&dir);
        app.dashboard_state.select(Some(0));
        app.enter_repo();
        assert_eq!(app.file_entries.len(), 1);

        app.start_discard();
        assert!(app.show_discard_confirm);
        app.confirm_discard();
        assert!(!app.show_discard_confirm);
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "v1\n");
        assert!(app.file_entries.is_empty());
        assert!(app.status.is_none());

        // Untracked file: discard removes it from the working tree.
        std::fs::write(dir.join("u.txt"), "junk\n").unwrap();
        app.load_file_entries();
        assert_eq!(app.file_entries.len(), 1);
        assert_eq!(app.file_state.selected(), Some(0));
        app.start_discard();
        app.confirm_discard();
        assert!(!dir.join("u.txt").exists());
        assert!(app.file_entries.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---------- config delete confirmation ----------

    #[test]
    fn config_delete_opens_and_esc_cancels_without_removing() {
        let mut app = App::new(Config::default());
        app.config.scan_dirs = vec![
            std::path::PathBuf::from("/a"),
            std::path::PathBuf::from("/b"),
        ];
        app.enter_config();
        app.config_state.select(Some(1));

        // d opens the confirmation on the selected entry.
        assert!(app.handle_key(key(KeyCode::Char('d'))));
        assert!(app.show_config_delete_confirm);
        assert!(!app.show_config_input);
        assert_eq!(app.config_state.selected(), Some(1));

        // Other keys are swallowed while the dialog is open.
        assert!(app.handle_key(key(KeyCode::Char('j'))));
        assert!(app.show_config_delete_confirm);
        assert_eq!(app.config_state.selected(), Some(1));

        // Esc (before y) cancels: nothing removed, still in the panel.
        assert!(app.handle_key(key(KeyCode::Esc)));
        assert!(!app.show_config_delete_confirm);
        assert_eq!(app.config.scan_dirs.len(), 2);
        assert_eq!(app.view, View::Config);

        // n also cancels.
        assert!(app.handle_key(key(KeyCode::Char('d'))));
        assert!(app.show_config_delete_confirm);
        assert!(app.handle_key(key(KeyCode::Char('n'))));
        assert!(!app.show_config_delete_confirm);
        assert_eq!(app.config.scan_dirs.len(), 2);
    }

    #[test]
    fn config_delete_on_empty_list_reports_error() {
        let mut app = App::new(Config::default());
        app.enter_config();

        assert!(app.handle_key(key(KeyCode::Char('d'))));
        assert!(!app.show_config_delete_confirm);
        let st = app.status.as_ref().unwrap();
        assert!(st.is_error);
        assert_eq!(st.message, "没有可删除的目录");
    }

    #[test]
    fn config_delete_confirm_removes_saves_and_refreshes() {
        // Holds the I/O guard: the save below writes the redirected config.
        let _io = crate::config::config_io_guard();

        let keep = temp_dir("config-del-keep");
        init_repo_with_commit(&keep);
        let drop = temp_dir("config-del-drop");

        let mut app = App::new(Config::default());
        app.config.scan_dirs = vec![keep.clone(), drop.clone()];
        app.enter_config();
        app.config_state.select(Some(1));

        // y removes the selected dir, persists, and rescans.
        assert!(app.handle_key(key(KeyCode::Char('d'))));
        assert!(app.handle_key(key(KeyCode::Char('y'))));
        assert!(!app.show_config_delete_confirm);
        assert_eq!(app.config.scan_dirs, vec![keep.clone()]);
        // Selection clamps to the surviving entry.
        assert_eq!(app.config_state.selected(), Some(0));

        // Persisted to the config file (restart would load the same dirs).
        let loaded = Config::load().expect("load saved config");
        assert_eq!(loaded.scan_dirs, vec![keep.clone()]);

        // Auto-rescan keeps the repository under the surviving scan dir.
        assert_eq!(app.repos.len(), 1);
        assert_eq!(app.repos[0].path, keep);

        // Success message survives the refresh (which clears the status).
        let st = app.status.as_ref().unwrap();
        assert!(!st.is_error, "{}", st.message);
        assert_eq!(st.message, format!("已删除扫描目录: {}", drop.display()));

        let _ = std::fs::remove_dir_all(&keep);
        let _ = std::fs::remove_dir_all(&drop);
    }
}
