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
            View::Dashboard => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => false,
                KeyCode::Char('j') | KeyCode::Down => {
                    if !self.repos.is_empty() {
                        let next = self
                            .dashboard_state
                            .selected()
                            .map_or(0, |i| i.saturating_add(1).min(self.repos.len() - 1));
                        self.dashboard_state.select(Some(next));
                    }
                    true
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    if !self.repos.is_empty() {
                        let prev = self
                            .dashboard_state
                            .selected()
                            .map_or(0, |i| i.saturating_sub(1));
                        self.dashboard_state.select(Some(prev));
                    }
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
                                let prev = self
                                    .file_state
                                    .selected()
                                    .map_or(0, |i| i.saturating_sub(1));
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
                        KeyCode::Char('b') => {
                            self.enter_branches();
                            true
                        }
                        _ => true,
                    }
                }
            }
            View::Diff => match key.code {
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
            // Branch panel: modal input / delete-confirm take precedence over
            // list navigation; Esc returns to the view it was opened from.
            View::Branches => {
                if self.show_branch_input {
                    match key.code {
                        KeyCode::Esc => {
                            self.cancel_branch_create();
                            true
                        }
                        KeyCode::Enter => {
                            self.submit_branch_create();
                            true
                        }
                        KeyCode::Backspace => {
                            self.branch_input.pop();
                            true
                        }
                        KeyCode::Char(c)
                            if !key
                                .modifiers
                                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                        {
                            self.branch_input.push(c);
                            true
                        }
                        _ => true,
                    }
                } else if self.show_branch_delete_confirm {
                    match key.code {
                        KeyCode::Char('y') | KeyCode::Char('Y') => {
                            self.confirm_branch_delete();
                            true
                        }
                        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                            self.cancel_branch_delete();
                            true
                        }
                        _ => true,
                    }
                } else {
                    match key.code {
                        KeyCode::Esc => {
                            self.leave_branches();
                            true
                        }
                        KeyCode::Char('q') => false,
                        KeyCode::Char('j') | KeyCode::Down => {
                            if !self.branches.is_empty() {
                                let next = self.branch_state.selected().map_or(0, |i| {
                                    i.saturating_add(1).min(self.branches.len() - 1)
                                });
                                self.branch_state.select(Some(next));
                            }
                            true
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            if !self.branches.is_empty() {
                                let prev = self
                                    .branch_state
                                    .selected()
                                    .map_or(0, |i| i.saturating_sub(1));
                                self.branch_state.select(Some(prev));
                            }
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
}
