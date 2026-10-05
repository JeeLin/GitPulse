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
use crate::git::{self, show_path, BranchInfo, FileEntry, RepoInfo};
use crate::git::{CommitDetail, CommitRow, TagInfo};
use crate::ui::status::{RemoteStatus, StatusLine};

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
    /// Commit history view (DAG, commit list and detail pane).
    History,
    /// Tag management panel (create / delete / push).
    Tags,
    /// GitHub receipts hub: the open-pull list (entered from the repo view via `g`).
    Pulls,
    /// Pull-request detail panel (opened from the pulls hub by `Enter`).
    PullDetail,
    /// CI checks panel (head-commit checks for the current repository HEAD).
    Checks,
    /// GitHub issues panel.
    Issues,
    /// Issue detail panel (opened from the issues hub by `Enter`).
    IssueDetail,
    /// GitHub notifications panel.
    Notifications,
}

/// Every screen the command palette can jump to (DESIGN §6.11). The drill-down
/// states — `Commit`, `Diff`, `PullDetail`, `IssueDetail` — are reached by
/// `Enter` inside their hub, not as direct palette targets, but they are listed
/// anyway so the palette's screen list stays in step with `View`.
pub(crate) const SCREENS: [View; 14] = [
    View::Dashboard,
    View::Repo,
    View::Diff,
    View::Commit,
    View::Branches,
    View::Config,
    View::History,
    View::Tags,
    View::Pulls,
    View::PullDetail,
    View::Checks,
    View::Issues,
    View::IssueDetail,
    View::Notifications,
];

impl View {
    /// Receipt screens — the platform hub and the two detail screens it opens.
    ///
    /// The hub owns feedback ([`App::remote_status`] plus
    /// [`App::receipt_op`]) that only makes sense there, so every view switch
    /// out of one has to drop it through [`App::leave_receipt_domain`] (B2).
    pub(crate) fn is_receipt(self) -> bool {
        matches!(
            self,
            View::Pulls
                | View::PullDetail
                | View::Checks
                | View::Issues
                | View::IssueDetail
                | View::Notifications
        )
    }

    /// The two detail screens a receipt list opens.
    ///
    /// A detail fetch belongs to the detail it was armed for, so leaving one
    /// for its list has to drop the in-flight request even when the tab does
    /// not change — see [`App::switch_github_tab`].
    pub(crate) fn is_receipt_detail(self) -> bool {
        matches!(self, View::PullDetail | View::IssueDetail)
    }
}

/// Active tab within the GitHub receipts hub (`p`/`c`/`i`/`n`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GithubTab {
    /// Open pull requests (the hub entry point).
    #[default]
    Pulls,
    /// Head-commit checks for the current repository HEAD.
    Checks,
    /// Open issues (excluding PRs).
    Issues,
    /// Repository notifications.
    Notifications,
}

/// State of the GitHub receipts spine, owned by [`App`] and rendered directly
/// by the `ui::pulls`/`checks`/`issues`/`notifications` panels (checkpoint 2:
/// no separate mock data source).
///
/// `client` is the readiness gate built once by [`App::enter_github`]; the
/// per-fetch tasks reconstruct a client from `remote` (the token is
/// cached in `resolve_token`, so that is cheap), keeping the field as an
/// `Option<Box<dyn crate::platform::PlatformClient>>`.
#[derive(Default)]
pub struct GithubState {
    /// Active receipts tab.
    pub tab: GithubTab,
    /// Parsed remote URL — platform family, owner and repo — set by
    /// [`App::enter_github`] and the single source of truth for the receipt
    /// copy: [`GithubState::owner`] / [`GithubState::repo`] / the platform
    /// label are all derived from it. `None` while no remote has been
    /// resolved, which every fallback (`—`, 「平台」) keys off.
    pub remote: Option<crate::platform::PlatformRemote>,
    /// Cached client used as a credential-validity gate; `None` until
    /// `enter_github` succeeds in building one.
    pub client: Option<Box<dyn crate::platform::PlatformClient>>,
    /// Index of the highlighted row in the active list.
    pub selected: usize,
    /// Latest open-PR list.
    pub pulls: Vec<crate::platform::PrSummary>,
    /// Detail payload of the highlighted pull request.
    pub detail: Option<crate::platform::PrDetail>,
    /// Latest check-run list for the selected pull's head commit.
    pub checks: Vec<crate::platform::CheckSummary>,
    /// Latest issue list.
    pub issues: Vec<crate::platform::IssueSummary>,
    /// Latest notification list.
    pub notifications: Vec<crate::platform::NotificationSummary>,
    /// Detail of the selected issue (opened from the Issues panel).
    pub issue_detail: Option<crate::platform::IssueDetail>,
}

impl GithubState {
    /// Namespace above the repository, or `None` while no remote has been
    /// resolved. Derived from `remote` so there is no second copy to drift.
    pub fn owner(&self) -> Option<&str> {
        self.remote
            .as_ref()
            .map(|r| r.owner.as_str())
            .filter(|owner| !owner.is_empty())
    }

    /// Repository name, `None` under the same conditions as
    /// [`GithubState::owner`].
    pub fn repo(&self) -> Option<&str> {
        self.remote
            .as_ref()
            .map(|r| r.repo.as_str())
            .filter(|repo| !repo.is_empty())
    }

    /// Platform family of the resolved remote, or `None` while none is known
    /// (the panels fall back to neutral copy in that case).
    pub fn kind(&self) -> Option<crate::platform::PlatformKind> {
        self.remote.as_ref().map(|r| r.kind)
    }

    /// The single readiness gate every receipt panel and every fetch spawner
    /// asks before it renders a live list or arms a request (U1).
    ///
    /// Both halves are required: a client without a remote cannot say which
    /// repository its data belongs to, and a remote without a client has no
    /// credential to fetch with. [`App::enter_github`] resets the two
    /// together, so `client.is_some() && remote.is_none()` is not a state the
    /// app can reach — this predicate is what keeps it that way.
    pub fn connected(&self) -> bool {
        self.client.is_some() && self.remote.is_some()
    }

    /// Drop everything the previous repository left behind: the six receipt
    /// lists, both details, the cached client and the resolved remote.
    ///
    /// [`App::enter_github`] calls this before it resolves the new remote, so
    /// a repository whose remote cannot be parsed renders an empty gate
    /// instead of the previous repository's receipts (U1).
    pub fn reset(&mut self) {
        self.pulls.clear();
        self.checks.clear();
        self.issues.clear();
        self.notifications.clear();
        self.detail = None;
        self.issue_detail = None;
        self.client = None;
        self.remote = None;
        self.selected = 0;
    }
}

/// Message sent from a background GitHub fetch task over its channel.
///
/// A task emits exactly one terminal message, consumed per-frame by
/// [`App::poll_github`] — the same template every remote op shares (convention 1).
#[derive(Debug)]
enum GithubMsg {
    /// Open pull-request list result.
    Pulls(Vec<crate::platform::PrSummary>),
    /// Selected pull-request detail result.
    Detail(crate::platform::PrDetail),
    /// Head-commit check-run list result.
    Checks(Vec<crate::platform::CheckSummary>),
    /// Open issue list result.
    Issues(Vec<crate::platform::IssueSummary>),
    /// Selected issue detail result.
    IssueDetail(crate::platform::IssueDetail),
    /// Repository notification list result.
    Notifications(Vec<crate::platform::NotificationSummary>),
    /// Platform failure, rendered by [`App::poll_github`] through
    /// `platform::render_platform_error` — the single place that decides the
    /// user-facing wording.
    Error(crate::platform::PlatformError),
    /// Pre-rendered failure the platform layer cannot own (the git layer).
    Other(String),
}

/// Op slug for a receipt fetch, stored in `receipt_op` and rendered by
/// `ui::remote::render_remote_status` as `{op} 成功` inside the receipt views
/// only — the push/pull views read [`App::remote_op`] instead (U3).
///
/// A platform slug, not display copy, so it stays lowercase — GitHub keeps its
/// v0.5.0 `github` spelling and the other families get their own slug instead
/// of sharing a hardcoded `github`.
fn platform_op_slug(kind: crate::platform::PlatformKind) -> &'static str {
    use crate::platform::PlatformKind;
    match kind {
        PlatformKind::GitHub => "github",
        PlatformKind::GitLab => "gitlab",
        PlatformKind::Gitea => "gitea",
        PlatformKind::SelfHosted => "self-hosted",
    }
}

/// Number of commits loaded into the history view (`App::enter_history`).
const HISTORY_LIMIT: usize = 200;

/// State of the commit-history view ([`View::History`]).
///
/// Owned by [`App`] and rendered by `ui::history` through a direct borrow
/// (checkpoint 2: no separate mock data source).
#[derive(Debug, Default)]
pub struct HistoryState {
    /// Commits newest-first, as returned by `git::list_commits`.
    pub commits: Vec<CommitRow>,
    /// Index of the highlighted commit; always `0` when `commits` is empty.
    pub selected: usize,
    /// Detail payload of the highlighted commit, reloaded on every move.
    pub detail: Option<CommitDetail>,
    /// One DAG row per entry of `commits`, from `git::layout_graph`: exactly
    /// `git::MAX_LANES` characters, each column already holding its glyph.
    pub graph: Vec<Vec<char>>,
    /// Selection cursor shared across frames, kept in sync with `selected`.
    pub list_state: ListState,
}

impl HistoryState {
    /// Install a freshly loaded history and land on the first row.
    ///
    /// The cursor is reset together with the data so a selection left over
    /// from a previous repository never points past the new list.
    pub fn set_commits(&mut self, commits: Vec<CommitRow>, graph: Vec<Vec<char>>) {
        self.commits = commits;
        self.graph = graph;
        self.detail = None;
        self.select(0);
    }

    /// Drop the loaded history (failed load or empty repository).
    pub fn clear(&mut self) {
        self.commits.clear();
        self.graph.clear();
        self.detail = None;
        // An empty list clears both the index and the cursor.
        self.select(0);
    }

    /// Highlight `idx`, clamped to the last commit; an empty list clears the
    /// cursor and keeps `selected` at `0`.
    pub fn select(&mut self, idx: usize) {
        select_row(
            &mut self.selected,
            &mut self.list_state,
            self.commits.len(),
            idx,
        );
    }

    /// Move the cursor one row: `down` clamps at the last commit, `up` at
    /// the first. An empty list never moves.
    pub fn move_selection(&mut self, down: bool) {
        move_row(
            &mut self.selected,
            &mut self.list_state,
            self.commits.len(),
            down,
        );
    }

    /// The highlighted commit, or `None` for an empty history.
    pub fn selected_commit(&self) -> Option<&CommitRow> {
        self.commits.get(self.selected)
    }
}

/// Modal overlay shown on top of the tag panel (conventions 8 and 9).
#[derive(Debug, PartialEq, Eq)]
pub enum TagsModal {
    /// Single-line create-tag input; carries the buffer being typed.
    Input {
        /// Text typed so far, edited by `input_modal_key`.
        buffer: String,
    },
    /// Delete confirmation naming the tag `y` will remove.
    Confirm {
        /// Name of the tag targeted by the confirmation.
        tag_to_delete: String,
    },
}

/// State of the tag panel ([`View::Tags`]).
///
/// Owned by [`App`] and rendered by `ui::tags` through a direct borrow.
#[derive(Debug, Default)]
pub struct TagsState {
    /// Tags sorted alphabetically by name, as returned by
    /// `git::list_tags`.
    pub tags: Vec<TagInfo>,
    /// Index of the highlighted tag; always `0` when `tags` is empty.
    pub selected: usize,
    /// Open overlay: the create input or the delete confirmation.
    pub modal: Option<TagsModal>,
    /// Selection cursor shared across frames, kept in sync with `selected`.
    pub list_state: ListState,
}

impl TagsState {
    /// Drop the loaded panel (entry, failed load): empty list, no cursor,
    /// no overlay.
    ///
    /// The open overlay is closed as well, so every error path starts from
    /// the same blank slate as a fresh entry.
    pub fn clear(&mut self) {
        self.tags.clear();
        self.modal = None;
        // An empty list clears both the index and the cursor.
        self.select(0);
    }

    /// Install a freshly loaded list and re-clamp the cursor, so a shrinking
    /// list never points past its end (and empties clear the cursor).
    pub fn set_tags(&mut self, tags: Vec<TagInfo>) {
        self.tags = tags;
        self.select(self.selected);
    }

    /// Highlight `idx`, clamped to the last tag; an empty list clears the
    /// cursor and keeps `selected` at `0`.
    pub fn select(&mut self, idx: usize) {
        select_row(
            &mut self.selected,
            &mut self.list_state,
            self.tags.len(),
            idx,
        );
    }

    /// Move the cursor one row: `down` clamps at the last tag, `up` at the
    /// first. An empty list never moves.
    pub fn move_selection(&mut self, down: bool) {
        move_row(
            &mut self.selected,
            &mut self.list_state,
            self.tags.len(),
            down,
        );
    }

    /// The highlighted tag, or `None` for an empty list.
    pub fn selected_tag(&self) -> Option<&TagInfo> {
        self.tags.get(self.selected)
    }

    /// Highlight the tag named `name`, leaving the selection untouched when
    /// the name is not in the list.
    ///
    /// The lookup walks the panel's own list, which [`git::list_tags`]
    /// returns sorted alphabetically by name (G26): the caller relies on
    /// that stable ordering to land on the row the panel shows for `name`
    /// after a reload, rather than on an index handed back by the git
    /// layer. A name missing from the list keeps the current highlight
    /// instead of moving it.
    pub fn select_name(&mut self, name: &str) {
        if let Some(idx) = self.tags.iter().position(|t| t.name == name) {
            self.select(idx);
        }
    }
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
    /// Current view; key presses are dispatched per view in [`App::dispatch_key`].
    pub view: View,
    /// Loaded configuration; repo discovery consumes it (M1 subtask 3).
    /// `pub(crate)` so the config panel can borrow `scan_dirs` by field
    /// instead of cloning them every frame.
    pub(crate) config: Config,
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
    /// Label of the running/last push or pull (`push` / `pull`).
    ///
    /// Receipt fetches label themselves in [`App::receipt_op`] instead (U3):
    /// one field holding both vocabularies let a `gitlab` receipt slug be
    /// rendered as `gitlab 成功` on the repo view's push/pull status row.
    pub remote_op: &'static str,
    /// Label of the running/last receipt fetch — the remote's platform slug
    /// (see `platform_op_slug`). Only the receipt views render it.
    pub receipt_op: &'static str,
    /// Receiver for background push/pull completion events.
    remote_rx: Option<mpsc::UnboundedReceiver<Result<(), String>>>,
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
    /// Whether the interactive directory picker overlay is shown.
    pub show_config_picker: bool,
    /// Directory currently browsed by the directory picker.
    pub picker_dir: std::path::PathBuf,
    /// Subdirectories of `picker_dir`, sorted by name. Row 0 of the picker
    /// list is a virtual "select current directory" row, not an entry here.
    pub picker_entries: Vec<std::path::PathBuf>,
    /// Selection cursor for the directory picker list.
    pub picker_state: ListState,
    /// Commit-history view state (`View::History`).
    pub history: HistoryState,
    /// Tag-panel state (`View::Tags`).
    pub tags_state: TagsState,
    /// GitHub receipts spine state (pulls / detail / checks / issues / notifications).
    pub github_state: GithubState,
    /// Receiver for a single in-flight GitHub fetch task (convention 1).
    github_rx: Option<mpsc::UnboundedReceiver<GithubMsg>>,
    /// `?` help overlay toggle (DESIGN §6.11). The dispatch already owns the
    /// key so it can never be read as navigation; Wave 2 renders the overlay.
    pub help_open: bool,
    /// `Ctrl-P` command-palette toggle (DESIGN §6.11), same prepared-state
    /// contract as [`App::help_open`].
    pub palette_open: bool,
    /// Cursor for the `Ctrl-P` command-palette jump-target list (§6.11). The
    /// palette owns its selection separately from the view beneath it, so the
    /// underlying list keeps its own position while the palette is open.
    pub palette_state: ListState,
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
            receipt_op: "",
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
            show_config_picker: false,
            picker_dir: std::path::PathBuf::new(),
            picker_entries: Vec::new(),
            picker_state: ListState::default(),
            history: HistoryState::default(),
            tags_state: TagsState::default(),
            github_state: GithubState::default(),
            github_rx: None,
            help_open: false,
            palette_open: false,
            palette_state: ListState::default(),
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

    /// Drop the settled push/pull (or receipt) feedback so the next status
    /// message is not outranked by it (G3).
    ///
    /// The panels that render `remote_status` above the plain status line
    /// would otherwise keep showing a stale `Success` / `Error` under (or
    /// over) a message written afterwards. A push still in flight keeps its
    /// receiver and re-reports when it finishes, so its running feedback is
    /// deliberately left alone — this is the one guard the tag panel, its
    /// overlays and the history/config entry points (G19) share instead of
    /// re-typing `if self.remote_rx.is_none()`.
    ///
    /// It is also the guard that keeps a settled receipt fetch out of the
    /// repo view (U3): leaving the receipt hub clears its feedback the same
    /// way, so the repo status row never renders `github 成功` / `gitlab 成功`
    /// for an operation it did not run.
    /// Op label for a receipt view's status row: the platform slug of the
    /// fetch that settled here, falling back to [`App::remote_op`] while no
    /// receipt has run in this visit.
    ///
    /// The fallback keeps a push or pull that succeeded before the hub was
    /// entered readable (`push 成功`) instead of degrading to the op-less
    /// `操作完成`, which is what an empty slug would render (U3).
    pub fn receipt_op_label(&self) -> &str {
        if self.receipt_op.is_empty() {
            self.remote_op
        } else {
            self.receipt_op
        }
    }

    fn leave_receipt_domain(&mut self) {
        // The receipt feedback belongs to the hub: `github 成功` / `gitlab 成功`
        // on the repo view's status row reads as a push or pull the repo never
        // ran, and an empty op would render as `操作完成` — an operation the
        // user never started. A push still in flight keeps its own feedback
        // (G3), which `clear_finished_push_feedback` already guards.
        self.clear_finished_push_feedback();
        self.receipt_op = "";
        // A fetch still in flight would settle *after* this point and paint its
        // verdict onto a view the user already left — `poll_github` keeps
        // draining the receiver every frame, so the feedback would reappear on
        // the repo status row with no op to name it. Dropping the receiver
        // disarms the task: its late result goes to a channel nobody reads.
        // Re-entering the hub refetches (`enter_github` resets the state), so
        // nothing is lost that the user could still act on.
        self.github_rx = None;
    }

    fn clear_finished_push_feedback(&mut self) {
        if self.remote_rx.is_none() {
            self.remote_status = None;
        }
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
    ///
    /// Blocked while a remote operation is in flight — it would contend on
    /// the same repository locks — and the key reports that instead of
    /// dropping silently, as the push/batch entry points do (G21, R7).
    pub fn open_commit(&mut self) {
        if self.remote_rx.is_some() {
            self.set_info("已有远端操作进行中");
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
            self.set_error(invalid_repo_msg("commit 失败"));
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
                self.set_info_unless_error(format!("commit 成功 {}", git::short_id(&oid)));
            }
            Err(e) => {
                self.set_error(format!("commit 失败: {e}"));
            }
        }
    }

    /// Start a background push (`op == "push"`) or pull (`op == "pull"`).
    ///
    /// The task runs off the UI thread; its result arrives on an mpsc channel
    /// consumed each frame (convention 1), so the UI stays interactive while
    /// the operation runs. Outside the repo view, or with no valid repository
    /// open, it stays a silent no-op; the remote/batch mutual-exclusion guard
    /// lives in [`App::spawn_remote_task`] (H8), where every remote op shares
    /// it (they would contend on the same repository locks).
    pub fn start_remote(&mut self, op: &'static str) {
        if self.view != View::Repo {
            return;
        }
        let Some(repo_idx) = self.current_repo else {
            return;
        };
        if repo_idx >= self.repos.len() {
            return;
        }
        let repo_path = self.repos[repo_idx].path.clone();
        let message = if op == "push" {
            "正在 push…".to_string()
        } else {
            "正在 pull…".to_string()
        };
        // `None`: the plain push/pull keys have always ignored a busy remote
        // silently, unlike the tag panel's `p`/`a`.
        self.spawn_remote_task(op, message, None, move || async move {
            match op {
                "push" => git::push(&repo_path).await,
                _ => git::pull(&repo_path).await,
            }
        });
    }

    /// Wire up the shared remote completion channel and spawn `task` as the
    /// background worker behind it (convention 1).
    ///
    /// The receiver is armed on `remote_rx`, `remote_op` labels the running
    /// operation, `message` is published as [`RemoteStatus::Running`] and the
    /// plain status line is cleared. `task` resolves to the operation's
    /// result, which is forwarded to [`App::poll_remote`] as the terminal
    /// event — the template every remote op shares.
    ///
    /// The remote/batch mutual-exclusion guard lives here instead of at each
    /// call site (H8): a second operation would replace the live receiver —
    /// its result could never arrive — and contend on the same repository
    /// locks. The refusal is silent when `busy_notice` is `None` and records
    /// the notice (e.g. `已有远端操作进行中`, R7) when the caller would
    /// otherwise swallow the key.
    fn spawn_remote_task<F>(
        &mut self,
        op: &'static str,
        message: String,
        busy_notice: Option<&'static str>,
        task: impl FnOnce() -> F + Send + 'static,
    ) where
        F: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        if self.remote_rx.is_some() || self.batch_rx.is_some() {
            if let Some(notice) = busy_notice {
                self.set_info(notice);
            }
            return;
        }
        let (tx, rx) = mpsc::unbounded_channel();
        self.remote_rx = Some(rx);
        self.remote_op = op;
        self.remote_status = Some(RemoteStatus::Running(message));
        self.status = None;
        tokio::spawn(async move {
            let _ = tx.send(task().await.map_err(|e| e.to_string()));
        });
    }

    /// Receive the pending completion event; called once per frame
    /// (convention 1). The spawned task sends exactly one terminal result,
    /// so a single receive per frame is enough.
    pub fn poll_remote(&mut self) {
        let Some(rx) = self.remote_rx.as_mut() else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(())) => {
                self.remote_status = Some(RemoteStatus::Success);
                self.remote_rx = None;
                if let Some(idx) = self.current_repo {
                    if idx < self.repos.len() {
                        self.refresh_repo_status(idx);
                    }
                }
            }
            Ok(Err(err)) => {
                self.remote_status = Some(RemoteStatus::Error(err));
                self.remote_rx = None;
            }
            Err(mpsc::error::TryRecvError::Empty) => {}
            Err(mpsc::error::TryRecvError::Disconnected) => {
                self.remote_status = Some(RemoteStatus::Error("远端任务意外退出".to_string()));
                self.remote_rx = None;
            }
        }
    }

    /// Enter the receipts hub from the repo view (`g`).
    ///
    /// Resolves the current repo's remote URL to a [`PlatformRemote`], builds a
    /// platform client once (cached as a credential-validity gate) and lands on
    /// the pulls list. On a missing remote or a missing token the
    /// view still becomes [`View::Pulls`] so the panel can surface the error
    /// and the user can retry with `r`.
    ///
    /// The whole previous-repository state is dropped up front (U1): the six
    /// receipt lists, both details, the cached client, the resolved remote and
    /// the receiver of a fetch still in flight. So a repository whose remote
    /// fails to resolve presents an empty gate rather than the previous
    /// repository's receipts, and a response that arrives after the switch is
    /// dropped instead of landing on the new repository's panel — see
    /// [`App::reset_github_state`] for why dropping the receiver is enough.
    pub fn enter_github(&mut self) {
        self.github_state.tab = GithubTab::Pulls;
        // The label is read before the reset so this failure can still name
        // the platform; a GitHub remote keeps its v0.5.0 wording.
        let platform = self.github_state.kind();
        self.reset_github_state();
        self.view = View::Pulls;
        self.clear_error();

        let Some(repo) = self.current_repo_path() else {
            self.set_error(format!(
                "打开 {} 回执失败: 未打开仓库",
                platform.map_or("平台", |kind| kind.as_str())
            ));
            return;
        };
        let url = match crate::git::remote_url(&repo) {
            Ok(url) => url,
            Err(e) => {
                // The git layer hands back an `anyhow::Error`, which
                // `render_platform_error` cannot take (it renders
                // `PlatformError`); the two platform failures below do go
                // through it, so the two error renderings are not
                // interchangeable but are equally readable.
                self.set_error(format!("解析远端失败: {e}"));
                return;
            }
        };
        let remote = match crate::platform::parse_remote(&url) {
            Ok(r) => r,
            Err(e) => {
                self.set_error(crate::platform::render_platform_error(&e));
                return;
            }
        };
        self.github_state.remote = Some(remote.clone());

        // The gate is built for the remote that was just resolved, never
        // reused across repositories: the reset above dropped the previous
        // one, so a client can no longer certify credentials it never asked
        // for (U1).
        match crate::platform::build_client(&remote) {
            Ok(client) => self.github_state.client = Some(client),
            Err(e) => {
                self.set_error(crate::platform::render_platform_error(&e));
                return;
            }
        }
        self.refresh_github();
    }

    /// Reset the receipts hub to the empty gate, dropping the receiver of a
    /// fetch that is still in flight (U1).
    ///
    /// `github_rx` holds the *only* sender of that fetch, so dropping the
    /// receiver disarms the task for good: its late result is handed to a
    /// channel nobody reads, and `poll_github` finds no receiver and returns.
    /// Each fetch gets a fresh channel and the slot holds at most one, so
    /// there is no other live receiver its result could land in — the reset
    /// is what keeps a previous repository's response off the new panel.
    fn reset_github_state(&mut self) {
        self.github_state.reset();
        self.github_rx = None;
        self.receipt_op = "";
    }

    /// Number of rows in the active receipts list (`0` for an empty list).
    fn github_list_len(&self) -> usize {
        match self.github_state.tab {
            GithubTab::Pulls => self.github_state.pulls.len(),
            GithubTab::Checks => self.github_state.checks.len(),
            GithubTab::Issues => self.github_state.issues.len(),
            GithubTab::Notifications => self.github_state.notifications.len(),
        }
    }

    /// Clamp `selected` to the current list length.
    fn clamp_github_selection(&mut self) {
        let len = self.github_list_len();
        if self.github_state.selected >= len {
            self.github_state.selected = len.saturating_sub(1);
        }
    }

    /// Move the receipts selection one row (`down` = toward the last row).
    fn move_github_selection(&mut self, down: bool) {
        let len = self.github_list_len();
        if len == 0 {
            return;
        }
        if down {
            if self.github_state.selected + 1 < len {
                self.github_state.selected += 1;
            }
        } else if self.github_state.selected > 0 {
            self.github_state.selected -= 1;
        }
    }

    /// Switch the active receipts tab and refresh its list.
    fn switch_github_tab(&mut self, tab: GithubTab) -> bool {
        // A fetch in flight was armed for the screen we are leaving, so its
        // result describes rows the user is no longer looking at. Dropping the
        // receiver lets the new screen's fetch start instead of being refused
        // by the in-flight guard, and stops the old rows from landing under the
        // new tab's title.
        //
        // Two ways to leave, and they need opposite answers:
        //
        // - Another tab: arm unconditionally, or the guard in
        //   [`App::refresh_github`] would refuse the new tab's fetch.
        // - A detail screen for the *same* tab: the in-flight fetch may be
        //   that detail's request, whose rows the list will never render, and
        //   `refresh_github` would then early-return and leave the list on
        //   stale data. Dropping it lets the list load.
        //
        // Re-pressing the current tab from its own list stays idempotent
        // (v0.5.0): arming there would throw away a fetch that is still
        // loading and spend another request per keypress.
        if tab != self.github_state.tab || self.view.is_receipt_detail() {
            self.github_rx = None;
        }
        self.github_state.tab = tab;
        self.github_state.selected = 0;
        self.view = match tab {
            GithubTab::Pulls => View::Pulls,
            GithubTab::Checks => View::Checks,
            GithubTab::Issues => View::Issues,
            GithubTab::Notifications => View::Notifications,
        };
        if self.github_state.connected() {
            self.refresh_github();
        }
        true
    }

    /// Open the detail view for the highlighted pull request.
    fn open_github_detail(&mut self) {
        self.view = View::PullDetail;
        let Some(number) = self
            .github_state
            .pulls
            .get(self.github_state.selected)
            .map(|p| p.number)
        else {
            return;
        };
        let Some(remote) = self.github_state.remote.clone() else {
            self.set_error("未解析到远端，无法加载 PR 详情");
            return;
        };
        // No client yet, or a fetch already in flight: just show the panel
        // (which surfaces the error / empty state) without spawning.
        if !self.github_state.connected() || self.github_rx.is_some() {
            return;
        }
        self.remote_status = Some(RemoteStatus::Running("加载 PR…".to_string()));
        self.receipt_op = platform_op_slug(remote.kind);
        let (tx, rx) = mpsc::unbounded_channel();
        self.github_rx = Some(rx);
        tokio::spawn(async move {
            match crate::platform::build_client(&remote) {
                Ok(client) => match client.get_pull(number).await {
                    Ok(detail) => {
                        let _ = tx.send(GithubMsg::Detail(detail));
                    }
                    Err(e) => {
                        let _ = tx.send(GithubMsg::Error(e));
                    }
                },
                Err(e) => {
                    let _ = tx.send(GithubMsg::Error(e));
                }
            }
        });
    }

    /// Open the detail view for the highlighted issue.
    fn open_issue_detail(&mut self) {
        self.view = View::IssueDetail;
        let Some(number) = self
            .github_state
            .issues
            .get(self.github_state.selected)
            .map(|i| i.number)
        else {
            return;
        };
        let Some(remote) = self.github_state.remote.clone() else {
            self.set_error("未解析到远端，无法加载 Issue 详情");
            return;
        };
        // No client yet, or a fetch already in flight: just show the panel
        // (which surfaces the error / empty state) without spawning.
        if !self.github_state.connected() || self.github_rx.is_some() {
            return;
        }
        self.remote_status = Some(RemoteStatus::Running("加载 Issue…".to_string()));
        self.receipt_op = platform_op_slug(remote.kind);
        let (tx, rx) = mpsc::unbounded_channel();
        self.github_rx = Some(rx);
        tokio::spawn(async move {
            match crate::platform::build_client(&remote) {
                Ok(client) => match client.get_issue(number).await {
                    Ok(detail) => {
                        let _ = tx.send(GithubMsg::IssueDetail(detail));
                    }
                    Err(e) => {
                        let _ = tx.send(GithubMsg::Error(e));
                    }
                },
                Err(e) => {
                    let _ = tx.send(GithubMsg::Error(e));
                }
            }
        });
    }

    /// Refresh the active receipts list (convention 1: background
    /// task, per-frame drain by [`App::poll_github`]).
    ///
    /// The cached `client` plus the resolved `remote` are the readiness gate
    /// ([`GithubState::connected`]); each fetch task rebuilds a client from
    /// `remote` via [`build_client`] because concrete clients are not `Clone`.
    /// The token itself is cached per host (`src/platform/auth.rs` for Gitea /
    /// GitLab, `src/github/auth.rs` for GitHub), so the rebuild is cheap.
    fn refresh_github(&mut self) {
        let Some(remote) = self.github_state.remote.clone() else {
            self.remote_status = Some(RemoteStatus::Error(
                "未解析到远端，请重新进入回执视图".to_string(),
            ));
            return;
        };
        if !self.github_state.connected() {
            return;
        }
        if self.remote_rx.is_some() || self.batch_rx.is_some() || self.github_rx.is_some() {
            return;
        }
        self.remote_status = Some(RemoteStatus::Running("加载回执…".to_string()));
        self.receipt_op = platform_op_slug(remote.kind);
        let tab = self.github_state.tab;
        let repo_path = self.current_repo_path().map(|p| p.to_path_buf());
        self.github_state.detail = None;
        self.github_state.issue_detail = None;
        let (tx, rx) = mpsc::unbounded_channel();
        self.github_rx = Some(rx);
        tokio::spawn(async move {
            let client = match crate::platform::build_client(&remote) {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send(GithubMsg::Error(e));
                    return;
                }
            };
            let send = |msg: GithubMsg| {
                let _ = tx.send(msg);
            };
            match tab {
                GithubTab::Pulls => match client.list_pulls().await {
                    Ok(v) => send(GithubMsg::Pulls(v)),
                    Err(e) => send(GithubMsg::Error(e)),
                },
                GithubTab::Issues => match client.list_issues().await {
                    Ok(v) => send(GithubMsg::Issues(v)),
                    Err(e) => send(GithubMsg::Error(e)),
                },
                GithubTab::Notifications => match client.list_notifications().await {
                    Ok(v) => send(GithubMsg::Notifications(v)),
                    Err(e) => send(GithubMsg::Error(e)),
                },
                GithubTab::Checks => match repo_path {
                    Some(path) => match crate::git::head_sha(&path) {
                        Ok(sha) => match client.list_checks(&sha).await {
                            Ok(v) => send(GithubMsg::Checks(v)),
                            Err(e) => send(GithubMsg::Error(e)),
                        },
                        Err(e) => send(GithubMsg::Other(e.to_string())),
                    },
                    None => send(GithubMsg::Other(
                        "未打开仓库，无法获取 HEAD 提交".to_string(),
                    )),
                },
            }
        });
    }

    /// Drain the single GitHub fetch result per frame (convention 1).
    ///
    /// A result can only ever belong to the repository the hub is currently
    /// on: [`App::enter_github`] drops `github_rx` before it resolves the new
    /// remote, so a fetch that was still in flight for the previous
    /// repository has lost its only receiver by the time it answers and never
    /// reaches this function (U1).
    pub fn poll_github(&mut self) {
        let Some(rx) = self.github_rx.as_mut() else {
            return;
        };
        let msg = match rx.try_recv() {
            Ok(m) => m,
            Err(mpsc::error::TryRecvError::Empty) => return,
            Err(mpsc::error::TryRecvError::Disconnected) => {
                self.github_rx = None;
                let platform = self.github_state.kind();
                self.remote_status = Some(RemoteStatus::Error(format!(
                    "{} 任务意外退出",
                    platform.map_or("平台", |kind| kind.as_str())
                )));
                return;
            }
        };
        self.github_rx = None;
        match msg {
            GithubMsg::Pulls(v) => {
                self.github_state.pulls = v;
                self.clamp_github_selection();
                self.remote_status = Some(RemoteStatus::Success);
            }
            GithubMsg::Detail(d) => {
                self.github_state.detail = Some(d);
                self.remote_status = Some(RemoteStatus::Success);
            }
            GithubMsg::Checks(v) => {
                self.github_state.checks = v;
                self.remote_status = Some(RemoteStatus::Success);
            }
            GithubMsg::Issues(v) => {
                self.github_state.issues = v;
                self.remote_status = Some(RemoteStatus::Success);
            }
            GithubMsg::IssueDetail(d) => {
                self.github_state.issue_detail = Some(d);
                self.remote_status = Some(RemoteStatus::Success);
            }
            GithubMsg::Notifications(v) => {
                self.github_state.notifications = v;
                self.remote_status = Some(RemoteStatus::Success);
            }
            GithubMsg::Error(e) => {
                self.remote_status = Some(RemoteStatus::Error(
                    crate::platform::render_platform_error(&e),
                ));
            }
            GithubMsg::Other(message) => {
                self.remote_status = Some(RemoteStatus::Error(message));
            }
        }
    }

    /// Key handler for the GitHub receipts hub and its detail panels.
    ///
    /// `p`/`c`/`i`/`n` switch tabs, `j`/`k` (and arrows / space) navigate and
    /// `Enter` opens the pull detail from the pulls hub or the issue detail
    /// from the issues hub. The grammar keys belong to
    /// [`App::dispatch_key`]: `Esc` pops (PullDetail → Pulls, IssueDetail →
    /// Issues, any other receipts view → Repo), `r` re-fetches and `q` quits
    /// the app like on every other non-modal screen.
    fn handle_github_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('p') => self.switch_github_tab(GithubTab::Pulls),
            KeyCode::Char('c') => self.switch_github_tab(GithubTab::Checks),
            KeyCode::Char('i') => self.switch_github_tab(GithubTab::Issues),
            KeyCode::Char('n') => self.switch_github_tab(GithubTab::Notifications),
            KeyCode::Char('j') | KeyCode::Down | KeyCode::Char(' ') | KeyCode::Char(',') => {
                self.move_github_selection(true);
                true
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.move_github_selection(false);
                true
            }
            KeyCode::Enter if self.view == View::Pulls => {
                self.open_github_detail();
                true
            }
            KeyCode::Enter if self.view == View::Issues => {
                self.open_issue_detail();
                true
            }
            _ => true,
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

    /// The repository an index into `repos` points at, when that index is
    /// still valid.
    fn repo_of(&self, idx: Option<usize>) -> Option<&RepoInfo> {
        idx.filter(|i| *i < self.repos.len())
            .map(|i| &self.repos[i])
    }

    /// Path of the currently opened repository, when its index into
    /// `repos` is still valid.
    fn current_repo_path(&self) -> Option<std::path::PathBuf> {
        self.repo_of(self.current_repo).map(|r| r.path.clone())
    }

    /// Enter the commit-history view for the current repository (repo `h`).
    ///
    /// The list loads immediately with the first commit selected and its
    /// detail preloaded; a repository without commits shows the empty state.
    /// No-op when no valid repository is open, so a stale cursor can never
    /// open the view against the wrong path.
    pub fn enter_history(&mut self) {
        if self.current_repo_path().is_none() {
            return;
        }
        self.view = View::History;
        // Stale push feedback must not survive into the view; a push still
        // in flight keeps its receiver (G3/G19).
        self.clear_finished_push_feedback();
        self.clear_error();
        self.load_history();
    }

    /// Leave the history view back to the repo view (`Esc`).
    pub fn leave_history(&mut self) {
        self.view = View::Repo;
        self.clear_error();
    }

    /// Load the history list for the current repository and select the first
    /// commit, preloading its detail.
    ///
    /// A failed load clears the list and reports the reason, so a broken
    /// repository never looks like an empty one (checkpoint 2); an empty
    /// result is a valid state and keeps the status line clean.
    pub fn load_history(&mut self) {
        let Some(path) = self.current_repo_path() else {
            self.set_error(invalid_repo_msg("历史视图"));
            self.history.clear();
            return;
        };
        match git::list_commits(&path, HISTORY_LIMIT) {
            Ok(commits) => {
                let graph = git::layout_graph(&commits);
                self.history.set_commits(commits, graph);
                self.clear_error();
                // An empty history resolves to `detail == None` below, so
                // the detail reload is unconditional.
                self.load_history_detail();
            }
            Err(e) => {
                self.set_error(format!("加载提交历史失败: {e}"));
                self.history.clear();
            }
        }
    }

    /// Load the detail payload of the highlighted commit.
    ///
    /// Failures (unresolvable object, repository gone) surface on the status
    /// line instead of panicking; an empty history simply drops the detail.
    /// The error chain is printed in full (`{e:#}`, H2) so the cause from
    /// `git::commit_detail`'s context is not swallowed.
    ///
    /// Known limitation (G38): the whole-tree diff runs synchronously on the
    /// UI thread — a `j`/`k` press pays for it once, and a press that cannot
    /// move the cursor skips it entirely (see
    /// [`App::move_history_selection`]). Moving it off-thread would need an
    /// asynchronous detail slot plus a loading placeholder, and would change
    /// what the entry path synchronously guarantees.
    pub fn load_history_detail(&mut self) {
        // `full_id`, not the 7-character `id` the list renders: an
        // abbreviated prefix can be ambiguous (`GIT_EAMBIGUOUS`) in a large
        // repository, while the full oid always resolves to exactly one
        // object.
        let Some(id) = self.history.selected_commit().map(|c| c.full_id.clone()) else {
            self.history.detail = None;
            return;
        };
        let Some(path) = self.current_repo_path() else {
            self.history.detail = None;
            self.set_error(invalid_repo_msg("历史视图"));
            return;
        };
        match git::commit_detail(&path, &id) {
            Ok(detail) => {
                self.history.detail = Some(detail);
                self.clear_error();
            }
            Err(e) => {
                self.history.detail = None;
                self.set_error(format!("加载提交详情失败: {e:#}"));
            }
        }
    }

    /// Enter the tag panel for the current repository (repo `t`).
    ///
    /// The list loads immediately with the first tag selected; a repository
    /// without tags shows the empty state. No-op when no valid repository is
    /// open, so a stale cursor can never open the panel against the wrong
    /// path.
    pub fn enter_tags(&mut self) {
        if self.current_repo_path().is_none() {
            return;
        }
        self.view = View::Tags;
        self.tags_state.clear();
        // Stale push feedback would outrank the load status below.
        self.clear_finished_push_feedback();
        self.clear_error();
        self.load_tags();
    }

    /// Leave the tag panel back to the repo view (`Esc`).
    pub fn leave_tags(&mut self) {
        self.view = View::Repo;
        self.tags_state.modal = None;
        self.clear_error();
    }

    /// Load (or reload) the tag list for the current repository, keeping the
    /// cursor on a still-valid row.
    ///
    /// A failed load clears the list and reports the reason, so a broken
    /// repository never looks like an empty one (checkpoint 2); an empty
    /// result is a valid state and keeps the status line clean.
    pub fn load_tags(&mut self) {
        let Some(path) = self.current_repo_path() else {
            self.set_error(invalid_repo_msg("标签面板"));
            self.tags_state.clear();
            return;
        };
        match git::list_tags(&path) {
            Ok(tags) => {
                self.tags_state.set_tags(tags);
                self.clear_error();
            }
            Err(e) => {
                self.set_error(format!("加载标签列表失败: {e}"));
                self.tags_state.clear();
            }
        }
    }

    /// Open the single-line tag-name input (convention 9).
    pub fn start_tag_create(&mut self) {
        if self.current_repo_path().is_none() {
            return;
        }
        self.tags_state.modal = Some(TagsModal::Input {
            buffer: String::new(),
        });
        self.status = None;
        // Stale push feedback would outrank whatever the overlay reports.
        self.clear_finished_push_feedback();
    }

    /// Cancel the tag-create input, keeping the list as it was.
    pub fn cancel_tag_create(&mut self) {
        self.tags_state.modal = None;
    }

    /// Create the lightweight tag named in the input and select it
    /// (convention 5).
    ///
    /// An empty name keeps the input open with an error; a git-side failure
    /// (duplicate name, unborn HEAD) also keeps the typed text so it can be
    /// corrected (convention 9). Validation, the git call and the shared
    /// error wording live in [`App::submit_name_create`].
    pub fn submit_tag_create(&mut self) {
        let name = match &self.tags_state.modal {
            Some(TagsModal::Input { buffer }) => buffer.trim().to_string(),
            _ => return,
        };
        let path = self.current_repo_path();
        let created = self.submit_name_create(&name, "标签", path, true, |path| {
            git::create_tag(&path, &name)
        });
        if created {
            self.tags_state.modal = None;
            self.load_tags();
            self.tags_state.select_name(&name);
            self.finish_create("标签", &name);
        }
    }

    /// Open the delete confirmation for the selected tag (discard-style
    /// confirmation, convention 8).
    ///
    /// An empty list reports an error instead of opening a dialog that could
    /// not name a target. The status/feedback cleanup runs first on every
    /// path, including that early return, so the panel always starts an
    /// operation from the same state (H5). A tag whose name is not valid
    /// UTF-8 is refused up front: the panel shows an escaped display form
    /// that cannot be handed back as a ref name, and a dialog promising a
    /// deletion that would then fail with a misleading "不存在" is worse than
    /// a plain refusal (H1/V2).
    pub fn start_tag_delete(&mut self) {
        self.status = None;
        // Stale push feedback would outrank the messages below.
        self.clear_finished_push_feedback();
        let Some(tag) = self.tags_state.selected_tag() else {
            self.set_error("没有可删除的标签");
            return;
        };
        if std::str::from_utf8(&tag.name_bytes).is_err() {
            self.set_error("非 UTF-8 标签名暂不支持删除");
            return;
        }
        let tag_to_delete = tag.name.clone();
        self.tags_state.modal = Some(TagsModal::Confirm { tag_to_delete });
    }

    /// Cancel the tag-delete confirmation.
    pub fn cancel_tag_delete(&mut self) {
        self.tags_state.modal = None;
    }

    /// Delete the confirmed tag, then refresh the list (convention 5).
    ///
    /// The overlay closes first — like the branch and config confirms — so a
    /// failed deletion surfaces on the status line instead of reopening the
    /// dialog it just dismissed.
    pub fn confirm_tag_delete(&mut self) {
        let Some(TagsModal::Confirm { tag_to_delete }) = self.tags_state.modal.take() else {
            return;
        };
        let Some(path) = self.current_repo_path() else {
            self.set_error(invalid_repo_msg("删除标签失败"));
            return;
        };
        // Refresh-first ordering, as in `submit_tag_create`: a stale push
        // result would otherwise outrank the outcome written below.
        self.clear_finished_push_feedback();
        match git::delete_tag(&path, &tag_to_delete) {
            Ok(()) => {
                self.load_tags();
                self.set_info_unless_error(format!("已删除标签 {tag_to_delete}"));
            }
            // `{e:#}` keeps the git layer's cause next to its context (H2).
            Err(e) => self.set_error(format!("删除标签失败: {e:#}")),
        }
    }

    /// Start a background push of the selected tag (`all == false`) or of
    /// every local tag (`all == true`) from the tag panel.
    ///
    /// Reuses the push/pull completion channel (convention 1): the result
    /// arrives on an mpsc receiver consumed each frame and renders through
    /// [`RemoteStatus`] on the panel's status line. Pressing `p` / `a`
    /// while another remote or batch op is in flight (they would contend on
    /// the same repository locks) reports `已有远端操作进行中` instead of
    /// dropping the key silently — that refusal lives inside
    /// [`App::spawn_remote_task`] with every other remote op (H8); outside
    /// the tag panel it stays a no-op. The notice shares the status row with
    /// the running feedback (see [`crate::ui::remote::render_remote_status`]),
    /// so it is never hidden by the very display that triggered it. A
    /// highlighted tag whose name is not valid UTF-8 is refused with an
    /// explicit reason before any task is spawned (H1/V2).
    pub fn start_tag_push(&mut self, all: bool) {
        if self.view != View::Tags {
            return;
        }
        // Every path below writes the status line, so stale feedback is
        // dropped first — including on the early returns (H5).
        self.clear_finished_push_feedback();
        let Some(path) = self.current_repo_path() else {
            self.set_error(invalid_repo_msg("标签推送失败"));
            return;
        };
        let tag = match self.push_tag_target(all) {
            Ok(tag) => tag,
            Err(message) => {
                self.set_error(message);
                return;
            }
        };
        let message = match &tag {
            Some(name) => format!("正在 push 标签 {name}…"),
            None => "正在 push 所有标签…".to_string(),
        };
        self.spawn_remote_task(
            "push",
            message,
            Some("已有远端操作进行中"),
            move || async move {
                match tag {
                    Some(name) => git::push_tag(&path, &name).await,
                    None => git::push_all_tags(&path).await,
                }
            },
        );
    }

    /// The tag a panel push should publish: `Ok(None)` for "every tag"
    /// (`all == true`), `Ok(Some(name))` for the highlighted one, and `Err`
    /// with the status-line text when there is nothing to push (G16).
    ///
    /// Pushing every tag on a repository without any would spawn a task that
    /// reports success while doing nothing, and a push with no highlighted
    /// tag has no target — both refuse before any task is spawned. A
    /// highlighted tag whose name is not valid UTF-8 is refused the same way:
    /// the panel holds an escaped display form, and feeding it back would
    /// build a refspec that can never match the real ref (H1/V2). The
    /// push-all path needs no name at all (`git push --tags`), so it stays
    /// available.
    fn push_tag_target(&self, all: bool) -> Result<Option<String>, &'static str> {
        if all {
            if self.tags_state.tags.is_empty() {
                return Err("没有可推送的标签");
            }
            Ok(None)
        } else {
            match self.tags_state.selected_tag() {
                Some(tag) if std::str::from_utf8(&tag.name_bytes).is_err() => {
                    Err("非 UTF-8 标签名暂不支持推送")
                }
                Some(tag) => Ok(Some(tag.name.clone())),
                None => Err("没有可推送的标签"),
            }
        }
    }

    /// Enter the scan-dir configuration panel (dashboard `c`).
    ///
    /// The selection lands on the first configured directory — or is cleared
    /// when none is configured — so `j/k` navigation always starts valid,
    /// and a stale cursor from a previous visit is re-clamped.
    pub fn enter_config(&mut self) {
        self.view = View::Config;
        clamp_selection(&mut self.config_state, self.config.scan_dirs.len());
        // Stale push feedback must not survive into the view; a push still
        // in flight keeps its receiver (G3/G19).
        self.clear_finished_push_feedback();
        self.clear_error();
    }

    /// Leave the configuration panel back to the dashboard (`Esc`).
    pub fn leave_config(&mut self) {
        self.view = View::Dashboard;
        self.cancel_config_add();
        self.show_config_delete_confirm = false;
        self.clear_error();
    }

    /// Open the interactive directory picker overlay (config panel `a`).
    ///
    /// Browsing starts at the parent of the first configured scan dir when
    /// readable, otherwise the home directory, then `/`, so the picker
    /// always lands somewhere useful on the first open.
    pub fn start_config_add(&mut self) {
        self.cancel_config_add();
        self.status = None;
        self.show_config_picker = true;
        let start = self.picker_start_dir();
        self.set_picker_dir(start);
    }

    /// Open the manual path input overlay (config list / picker `i`).
    ///
    /// Kept as a fallback for paths that are awkward to browse to.
    pub fn start_config_manual(&mut self) {
        self.config_input.clear();
        self.show_config_input = true;
        self.show_config_picker = false;
        self.show_config_delete_confirm = false;
        self.status = None;
    }

    /// Cancel the add-scan-dir overlays (picker and input), discarding the
    /// typed path and the browse state (convention 9).
    pub fn cancel_config_add(&mut self) {
        self.show_config_input = false;
        self.config_input.clear();
        self.show_config_picker = false;
        self.picker_dir = std::path::PathBuf::new();
        self.picker_entries.clear();
        self.picker_state.select(None);
    }

    /// Resolve the directory the picker opens at: the parent of the first
    /// scan dir when readable, else the home directory, else `/`.
    fn picker_start_dir(&self) -> std::path::PathBuf {
        let from_scan = self
            .config
            .scan_dirs
            .first()
            .and_then(|dir| dir.parent())
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(std::path::Path::to_path_buf);
        from_scan
            .into_iter()
            .chain([crate::config::home_dir(), std::path::PathBuf::from("/")])
            .find(|candidate| std::fs::read_dir(candidate).is_ok())
            .unwrap_or_else(|| std::path::PathBuf::from("/"))
    }

    /// Point the picker at `dir` and reload its subdirectory listing.
    ///
    /// A read failure keeps the previous listing and reports the reason on
    /// the status line, so an unreadable directory never looks like an
    /// empty one (checkpoint 2).
    fn set_picker_dir(&mut self, dir: std::path::PathBuf) {
        match read_picker_entries(&dir) {
            Ok(entries) => {
                self.picker_dir = dir;
                self.picker_entries = entries;
                self.picker_state.select(Some(0));
                self.clear_error();
            }
            Err(err) => self.set_error(format!("无法读取目录 {}: {err}", show_path(&dir))),
        }
    }

    /// Number of picker rows: the virtual "select current" row plus each
    /// subdirectory.
    fn picker_row_count(&self) -> usize {
        1 + self.picker_entries.len()
    }

    /// Subdirectory highlighted in the picker, if any (row 0 is the virtual
    /// "select current directory" row and yields `None`).
    fn picker_selected_entry(&self) -> Option<std::path::PathBuf> {
        let row = self.picker_state.selected()?;
        if row == 0 {
            return None;
        }
        self.picker_entries.get(row - 1).cloned()
    }

    /// Enter the highlighted subdirectory (picker `l` / `→`).
    fn picker_descend(&mut self) {
        if let Some(target) = self.picker_selected_entry() {
            self.set_picker_dir(target);
        }
    }

    /// Move to the parent directory (picker `h` / `←` / Backspace).
    ///
    /// At the filesystem root there is no parent: report that on the status
    /// line instead of silently ignoring the key.
    fn picker_ascend(&mut self) {
        let parent = self
            .picker_dir
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(std::path::Path::to_path_buf);
        match parent {
            Some(parent) => self.set_picker_dir(parent),
            None => self.set_info("已在根目录，无法返回上级"),
        }
    }

    /// Submit the picker selection (Enter): row 0 picks the current
    /// directory, other rows pick the highlighted subdirectory. The
    /// [`PathBuf`] goes straight into [`App::add_scan_dir`], so names that
    /// are not valid UTF-8 never round-trip through `String`; a failure
    /// keeps the picker open with the error on the status line.
    fn picker_submit(&mut self) {
        let path = if let Some(0) = self.picker_state.selected() {
            // Row 0 submits the browsed directory; an empty one (picker
            // opened at an unreadable start dir) submits nothing, as before.
            if self.picker_dir.as_os_str().is_empty() {
                return;
            }
            self.picker_dir.clone()
        } else {
            let Some(entry) = self.picker_selected_entry() else {
                return;
            };
            entry
        };
        self.add_scan_dir(path);
    }

    /// Trim and validate the typed path, then hand it to
    /// [`App::add_scan_dir`] (convention 5).
    ///
    /// An empty / whitespace-only input reports the error here; everything
    /// else is checked on the [`PathBuf`], keeping this function to the
    /// `String → Path` boundary.
    pub fn submit_config_add(&mut self) {
        let path = self.config_input.trim().to_string();
        if path.is_empty() {
            self.set_error("目录不能为空");
            return;
        }
        self.add_scan_dir(std::path::PathBuf::from(path));
    }

    /// Validate, deduplicate, persist, and rescan a scan dir (convention 5).
    ///
    /// The path is taken as a [`PathBuf`] so picker selections with non-UTF-8
    /// names are checked against the real filesystem entry instead of a
    /// lossy `display().to_string()` rendering. Each validation failure keeps
    /// the input open so it can be corrected: missing / not a directory /
    /// duplicate (empty input is [`App::submit_config_add`]'s job). The
    /// canonical form is stored while the message echoes the submitted
    /// spelling. On success the config is saved, repos refreshed, and the new
    /// entry selected; the success message is written after the refresh,
    /// which would otherwise clear it.
    fn add_scan_dir(&mut self, dir: std::path::PathBuf) {
        let shown = show_path(&dir);
        if !dir.exists() {
            self.set_error(format!("目录不存在: {shown}"));
            return;
        }
        if !dir.is_dir() {
            self.set_error(format!("不是目录: {shown}"));
            return;
        }
        // Compare and store the canonical form so `..`/`.` traversal,
        // symlinks, and relative spellings of an entry already in the list
        // collapse into one path (and a relative input cannot drift with the
        // working directory).
        let normalized = git::canonical_or_self(&dir);
        let duplicate = self
            .config
            .scan_dirs
            .iter()
            .any(|existing| git::canonical_or_self(existing) == normalized);
        if duplicate {
            self.set_error("目录已在列表中");
            return;
        }

        self.config.scan_dirs.push(normalized);
        if let Err(err) = self.config.save() {
            // Keep memory and disk consistent, and keep the submitted path
            // visible so the save can be retried (convention 9).
            self.config.scan_dirs.pop();
            self.set_error(format!("保存配置失败: {err:#}"));
            return;
        }
        self.cancel_config_add();
        self.refresh_repos();
        self.config_state
            .select(Some(self.config.scan_dirs.len() - 1));
        self.set_info_unless_error(format!("已添加扫描目录: {shown}"));
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
        clamp_selection(&mut self.config_state, self.config.scan_dirs.len());
        self.show_config_input = false;
        self.show_config_picker = false;
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
        let Some(i) = valid_selection(&self.config_state, self.config.scan_dirs.len()) else {
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
        self.set_info_unless_error(format!("已删除扫描目录: {}", show_path(&removed)));
    }

    /// Whether a background batch operation is currently running.
    pub fn batch_running(&self) -> bool {
        self.batch_rx.is_some()
    }

    /// Name of the repository the branch panel edits (empty when invalid).
    pub fn branch_repo_name(&self) -> &str {
        self.repo_of(self.branch_repo).map_or("", |r| &r.name)
    }

    /// Path of the repository the branch panel edits, when still valid.
    fn branch_repo_path(&self) -> Option<std::path::PathBuf> {
        self.repo_of(self.branch_repo).map(|r| r.path.clone())
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
            self.set_error(invalid_repo_msg("分支面板"));
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
    /// Validation, the git call and the shared error wording live in
    /// [`App::submit_name_create`]; the panel-specific refresh stays here.
    pub fn submit_branch_create(&mut self) {
        let name = self.branch_input.trim().to_string();
        let path = self.branch_repo_path();
        // `false`: the branch panel never renders push feedback, so it keeps
        // its historical no-op and leaves a settled remote result alone.
        let created = self.submit_name_create(&name, "分支", path, false, |path| {
            git::create_branch(&path, &name)
        });
        if created {
            self.show_branch_input = false;
            self.branch_input.clear();
            self.after_branch_change();
            if let Some(i) = self.branches.iter().position(|b| b.name == name) {
                self.branch_state.select(Some(i));
            }
            self.finish_create("分支", &name);
        }
    }

    /// Shared create flow behind the tag and branch name inputs (G4):
    /// validate the trimmed `name` for `noun` (`标签` / `分支`), resolve the
    /// repository, and run `create_fn` against it.
    ///
    /// Every message is the wording both panels already used verbatim:
    /// `<noun>名不能为空`, [`invalid_repo_msg`] with `创建< noun >失败`, and
    /// `创建< noun >失败: {e:#}` — the `#` keeps a cause added by the git
    /// layer's context on the status line (H2). Returns `true` only when
    /// `create_fn` succeeded, so the caller refreshes its own list and
    /// selection on success alone.
    ///
    /// `clear_stale_feedback` drops settled push feedback before the git
    /// call runs (refresh-first ordering: the tag panel renders it above the
    /// status line, so a stale result would outrank the create error). The
    /// branch panel passes `false` because it never shows that feedback.
    fn submit_name_create(
        &mut self,
        name: &str,
        noun: &str,
        path: Option<std::path::PathBuf>,
        clear_stale_feedback: bool,
        create_fn: impl FnOnce(std::path::PathBuf) -> Result<()>,
    ) -> bool {
        if name.is_empty() {
            self.set_error(format!("{noun}名不能为空"));
            return false;
        }
        let Some(path) = path else {
            self.set_error(invalid_repo_msg(&format!("创建{noun}失败")));
            return false;
        };
        if clear_stale_feedback {
            self.clear_finished_push_feedback();
        }
        match create_fn(path) {
            Ok(()) => true,
            Err(e) => {
                self.set_error(format!("创建{noun}失败: {e:#}"));
                false
            }
        }
    }

    /// Shared success report of the create flows (G4): `noun` is the panel's
    /// word (`标签` / `分支`), so both read `已创建< noun > <name>`.
    fn finish_create(&mut self, noun: &str, name: &str) {
        self.set_info_unless_error(format!("已创建{noun} {name}"));
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

    /// Single entry point for every key press (DESIGN §0/§2.1). The press is
    /// resolved in a fixed order, so no view can redefine the global verbs:
    ///
    /// 1. key-release event → ignored
    /// 2. `Ctrl-C` → force quit (any state, modals included)
    /// 3. open modal → focus trap (§2.4), only the overlay's own keys act
    /// 4. `q` → quit the app from any non-modal screen
    /// 5. global verbs (`Esc`/`?`/`Ctrl-P`/`Tab`/`BackTab`/`<`/`>`/`r`/`/`/
    ///    `g`/`G`/`Home`/`End`)
    /// 6. the current view's own keys (its `Enter` "open" verbs included)
    ///
    /// Returns `false` when the app should exit.
    pub fn dispatch_key(&mut self, key: KeyEvent) -> bool {
        if key.kind == KeyEventKind::Release {
            return true;
        }
        // `Ctrl-C` force-quits in every state, modals included (§0: 任何时刻生效).
        if is_ctrl_c(key) {
            return false;
        }
        // Modal focus trap (§2.4): while an overlay owns the keyboard no
        // global verb reaches the view — `q` types, `r` types, `Esc` closes
        // the overlay instead of popping the screen behind it.
        if self.active_modal().is_some() {
            return self.dispatch_modal_key(key);
        }
        // Wave-2 float overlays (`?` help / Ctrl-P palette) own the keyboard
        // just like the v1 modals (§2.4 / §6.11): `q` and `Esc` dismiss them
        // instead of quitting or popping the screen behind, `?`/Ctrl-P still
        // toggle between the two, and the palette's j/k/Enter navigate+jump.
        if self.help_open || self.palette_open {
            return self.dispatch_overlay_key(key);
        }
        // `q` exits the app from every non-modal screen (§0 / §1.3); the
        // v0.4.0 "q returns to the repo view" readings are gone.
        if is_plain_char(key, 'q') {
            return false;
        }
        match key.code {
            KeyCode::Esc => self.pop_one_level(),
            KeyCode::Char('?') if is_plain(key) => self.toggle_help_overlay(),
            KeyCode::Char('p') if is_ctrl(key) => self.toggle_palette_overlay(),
            // Panel focus and context tags never enter the stack (§1.1);
            // their cycling state lands with the repo-workspace tabs, so the
            // keys are reserved here and stay no-ops for now.
            KeyCode::Tab | KeyCode::BackTab => {}
            KeyCode::Char('<') if is_plain(key) => {}
            KeyCode::Char('>') if is_plain(key) => {}
            KeyCode::Char('r') if is_plain(key) => self.refresh_current_view(),
            // Search/filter state lands with the view subtasks; owning the
            // key here keeps it from ever falling through to a view verb.
            KeyCode::Char('/') if is_plain(key) => {}
            KeyCode::Char('g' | 'G') | KeyCode::Home | KeyCode::End if is_plain(key) => {
                // Transitional exception: repo `g` still opens the receipts
                // hub (`g 进入` hint) until the repo workspace replaces that
                // entry with its 远端 context tab (DESIGN §6.2.5); on every
                // other screen `g` is the "jump to the first row" verb (§0).
                if self.view == View::Repo && key.code == KeyCode::Char('g') {
                    return self.handle_repo_key(key);
                }
                self.jump_focus_list(key.code);
            }
            _ => return self.handle_view_key(key),
        }
        true
    }

    /// Route a key the global layer does not own to the current view's own
    /// verbs (DESIGN §2.1): views get `Enter` (open), the letter verbs
    /// (`s`/`u`/`c`/`n`/`d`/…) and `j`/`k` list movement.
    fn handle_view_key(&mut self, key: KeyEvent) -> bool {
        match self.view {
            View::Dashboard => self.handle_dashboard_key(key),
            View::Repo => self.handle_repo_key(key),
            View::Diff => self.handle_diff_key(key),
            // Unreachable: an open input is caught by the focus trap above.
            View::Commit => true,
            View::Branches => self.handle_branches_key(key),
            View::Config => self.handle_config_key(key),
            View::History => self.handle_history_key(key),
            View::Tags => self.handle_tags_key(key),
            View::Pulls
            | View::PullDetail
            | View::Checks
            | View::Issues
            | View::Notifications
            | View::IssueDetail => self.handle_github_key(key),
        }
    }

    /// The overlay (if any) that owns the keyboard right now — the single
    /// source for the focus trap (§2.4: 同时只允许一个模态). Exactly one flag
    /// can be set at a time (`start_*` always clears the sibling overlays).
    fn active_modal(&self) -> Option<ModalKind> {
        if self.view == View::Commit {
            return Some(ModalKind::Input(InputSlot::Commit));
        }
        if self.show_branch_input {
            return Some(ModalKind::Input(InputSlot::BranchCreate));
        }
        if self.show_branch_delete_confirm {
            return Some(ModalKind::Confirm(ConfirmSlot::BranchDelete));
        }
        if self.show_config_picker {
            return Some(ModalKind::Picker);
        }
        if self.show_config_input {
            return Some(ModalKind::Input(InputSlot::ConfigAdd));
        }
        if self.show_config_delete_confirm {
            return Some(ModalKind::Confirm(ConfirmSlot::ConfigDelete));
        }
        if let Some(modal) = &self.tags_state.modal {
            return Some(match modal {
                TagsModal::Input { .. } => ModalKind::Input(InputSlot::TagCreate),
                TagsModal::Confirm { .. } => ModalKind::Confirm(ConfirmSlot::TagDelete),
            });
        }
        if self.show_discard_confirm {
            return Some(ModalKind::Confirm(ConfirmSlot::Discard));
        }
        None
    }

    /// Run one key press through the overlay returned by
    /// [`App::active_modal`] (§2.4): text input, y/n confirmation or the
    /// directory picker. Only the overlay's own keys act — `Enter`/`Esc`
    /// first, then its letters — and everything else (including `q`, `r`,
    /// `?`, `Ctrl-P`) is literal input or swallowed. Always returns `true`:
    /// an overlay never quits the app.
    fn dispatch_modal_key(&mut self, key: KeyEvent) -> bool {
        let Some(modal) = self.active_modal() else {
            return self.handle_view_key(key);
        };
        match modal {
            ModalKind::Input(slot) => {
                let outcome = match slot {
                    InputSlot::Commit => input_modal_key(key, &mut self.commit_message),
                    InputSlot::BranchCreate => input_modal_key(key, &mut self.branch_input),
                    InputSlot::ConfigAdd => input_modal_key(key, &mut self.config_input),
                    InputSlot::TagCreate => match self.tags_state.modal.as_mut() {
                        Some(TagsModal::Input { buffer }) => input_modal_key(key, buffer),
                        // Raced away between the two lookups: swallow.
                        _ => InputOutcome::Consumed,
                    },
                };
                match (slot, outcome) {
                    (InputSlot::Commit, InputOutcome::Cancel) => self.cancel_commit(),
                    (InputSlot::Commit, InputOutcome::Submit) => self.submit_commit(),
                    (InputSlot::BranchCreate, InputOutcome::Cancel) => self.cancel_branch_create(),
                    (InputSlot::BranchCreate, InputOutcome::Submit) => self.submit_branch_create(),
                    (InputSlot::ConfigAdd, InputOutcome::Cancel) => self.cancel_config_add(),
                    (InputSlot::ConfigAdd, InputOutcome::Submit) => self.submit_config_add(),
                    (InputSlot::TagCreate, InputOutcome::Cancel) => self.cancel_tag_create(),
                    (InputSlot::TagCreate, InputOutcome::Submit) => self.submit_tag_create(),
                    (_, InputOutcome::Consumed) => {}
                }
            }
            ModalKind::Confirm(slot) => match (slot, confirm_modal_key(key)) {
                (ConfirmSlot::Discard, ConfirmOutcome::Yes) => self.confirm_discard(),
                (ConfirmSlot::Discard, ConfirmOutcome::No) => self.cancel_discard(),
                (ConfirmSlot::BranchDelete, ConfirmOutcome::Yes) => self.confirm_branch_delete(),
                (ConfirmSlot::BranchDelete, ConfirmOutcome::No) => self.cancel_branch_delete(),
                (ConfirmSlot::ConfigDelete, ConfirmOutcome::Yes) => self.confirm_config_delete(),
                (ConfirmSlot::ConfigDelete, ConfirmOutcome::No) => self.cancel_config_delete(),
                (ConfirmSlot::TagDelete, ConfirmOutcome::Yes) => self.confirm_tag_delete(),
                (ConfirmSlot::TagDelete, ConfirmOutcome::No) => self.cancel_tag_delete(),
                (_, ConfirmOutcome::Ignored) => {}
            },
            ModalKind::Picker => self.handle_config_picker_key(key),
        }
        true
    }

    /// `Esc`: pop exactly one level of the screen stack (§1.1). The root
    /// screen never exits — it toasts 「已在顶层」 instead, because quitting is
    /// `q`'s job (§1.2). A prepared help/palette overlay closes first.
    fn pop_one_level(&mut self) {
        if self.help_open || self.palette_open {
            self.help_open = false;
            self.palette_open = false;
            return;
        }
        match self.view {
            // Root screen (stack depth 1): no exit on `Esc`.
            View::Dashboard => self.set_info("已在顶层"),
            View::Repo => self.view = View::Dashboard,
            View::Diff => self.view = View::Repo,
            // Defensive: the commit input is an overlay, closed by the trap.
            View::Commit => self.cancel_commit(),
            View::Branches => self.leave_branches(),
            View::Config => self.leave_config(),
            View::History => self.leave_history(),
            View::Tags => self.leave_tags(),
            // Both detail arms drop the in-flight fetch on the way back, the
            // same way `switch_github_tab` does when a tab key does it: a detail
            // fetch lands in `github_state.detail` and never refreshes the list
            // the user just returned to, so keeping it would leave stale rows
            // under a green 「成功」 that names an abandoned request. `Esc` and
            // the tab key are two doors onto one intent, so they answer alike.
            View::PullDetail => {
                self.github_rx = None;
                self.view = View::Pulls;
                self.github_state.tab = GithubTab::Pulls;
            }
            View::IssueDetail => {
                self.github_rx = None;
                self.view = View::Issues;
                self.github_state.tab = GithubTab::Issues;
            }
            View::Pulls | View::Checks | View::Issues | View::Notifications => {
                self.view = View::Repo;
                self.leave_receipt_domain();
            }
        }
    }

    /// `r`: reload the current view's data (§0/§1.3), the same reload each
    /// view's entry point performs — one rule for local git and GitHub alike.
    fn refresh_current_view(&mut self) {
        match self.view {
            View::Dashboard => self.refresh_repos(),
            View::Repo => {
                if let Some(idx) = self.current_repo.filter(|i| *i < self.repos.len()) {
                    self.refresh_repo_status(idx);
                }
            }
            // Re-read the diff of the highlighted file (resets the scroll).
            View::Diff => self.view_file_diff(),
            View::Commit => {}
            View::Branches => self.load_branches(),
            // The scan-dir list is in-memory; `r` re-runs the scan it shows.
            View::Config => self.refresh_repos(),
            View::History => self.load_history(),
            View::Tags => self.load_tags(),
            View::Pulls
            | View::PullDetail
            | View::Checks
            | View::Issues
            | View::Notifications
            | View::IssueDetail => self.refresh_github(),
        }
    }

    /// `g`/`Home` → first row, `G`/`End` → last row of the focused list
    /// (§0 辅动词): never enters the stack, so it is safe on every screen.
    /// Screens without a list (the diff scroll area jumps to its top) ignore
    /// the key they cannot honour.
    fn jump_focus_list(&mut self, code: KeyCode) {
        let head = matches!(code, KeyCode::Char('g') | KeyCode::Home);
        match self.view {
            View::Dashboard => jump_selection(&mut self.dashboard_state, self.repos.len(), head),
            View::Repo => jump_selection(&mut self.file_state, self.file_entries.len(), head),
            View::Diff => {
                if head {
                    self.diff_scroll = 0;
                } else {
                    // One logical line can wrap into several rows, so landing
                    // on the last line is the closest safe estimate: scrolling
                    // past the end would render a blank panel.
                    self.diff_scroll =
                        u16::try_from(self.diff_content.lines().count().saturating_sub(1))
                            .unwrap_or(u16::MAX);
                }
            }
            View::Commit => {}
            View::Branches => jump_selection(&mut self.branch_state, self.branches.len(), head),
            View::Config => {
                jump_selection(&mut self.config_state, self.config.scan_dirs.len(), head)
            }
            View::History => {
                let idx = if head {
                    0
                } else {
                    self.history.commits.len().saturating_sub(1)
                };
                let before = self.history.selected;
                self.history.select(idx);
                // The detail pane follows the cursor, like `j`/`k` do.
                if self.history.selected != before {
                    self.load_history_detail();
                }
            }
            View::Tags => {
                let idx = if head {
                    0
                } else {
                    self.tags_state.tags.len().saturating_sub(1)
                };
                self.tags_state.select(idx);
            }
            View::Pulls
            | View::PullDetail
            | View::Checks
            | View::Issues
            | View::Notifications
            | View::IssueDetail => {
                let len = self.github_list_len();
                self.github_state.selected = if head { 0 } else { len.saturating_sub(1) };
            }
        }
    }

    /// `?` (§6.11): toggle the help overlay state. The dispatch owns the key
    /// so it can never be read as a view verb; Wave 2 renders the overlay.
    fn toggle_help_overlay(&mut self) {
        self.help_open = !self.help_open;
        if self.help_open {
            self.palette_open = false;
        }
    }

    /// `Ctrl-P` (§6.11): toggle the command-palette state, same prepared-state
    /// contract as [`App::toggle_help_overlay`]. Never reached from an input
    /// modal, where the chord is swallowed as a literal chord would be.
    fn toggle_palette_overlay(&mut self) {
        self.palette_open = !self.palette_open;
        if self.palette_open {
            self.help_open = false;
        }
    }

    /// Jump targets the command palette lists (§6.11): every screen, plus the
    /// current repo / branch when a repository is open. The list is read-only
    /// here so the palette can render and the dispatch can navigate it while the
    /// view beneath it stays untouched.
    pub(crate) fn palette_targets(&self) -> Vec<PaletteTarget> {
        let mut targets = SCREENS
            .iter()
            .copied()
            .map(PaletteTarget::View)
            .collect::<Vec<_>>();
        if let Some(idx) = self.current_repo.filter(|i| *i < self.repos.len()) {
            let repo = &self.repos[idx];
            targets.push(PaletteTarget::Repo(repo.name.clone()));
            targets.push(PaletteTarget::Branch(repo.branch.clone()));
        }
        targets
    }
    /// Route a key through an open float overlay (§6.11): `Esc` and `q` dismiss
    /// the overlay (never quit, never pop the screen behind it), `?`/Ctrl-P
    /// toggle between help and palette, and the palette's j/k/Enter navigate
    /// and jump. Every other key is swallowed while an overlay is open so the
    /// view beneath it is never disturbed.
    fn dispatch_overlay_key(&mut self, key: KeyEvent) -> bool {
        // `Esc` and `q` both close whichever float is open; while an overlay
        // owns the keyboard they must never reach the quit / pop path.
        if key.code == KeyCode::Esc || is_plain_char(key, 'q') {
            self.help_open = false;
            self.palette_open = false;
            return true;
        }
        // `?` / Ctrl-P toggle between the two overlays (mutual exclusion), the
        // same verbs the global layer owns when no overlay is open.
        if is_plain(key) && matches!(key.code, KeyCode::Char('?')) {
            self.toggle_help_overlay();
            return true;
        }
        if is_ctrl(key) && matches!(key.code, KeyCode::Char('p')) {
            self.toggle_palette_overlay();
            return true;
        }
        if self.help_open {
            // Read-only modal: only its close/toggle verbs act above.
            return true;
        }
        // Command palette: `j`/`k` (and the arrows) move the cursor, `Enter`
        // jumps, and everything else stays inside the palette.
        if is_plain(key) {
            match key.code {
                KeyCode::Char('j') | KeyCode::Down | KeyCode::Char(' ') => {
                    self.move_palette_selection(true);
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.move_palette_selection(false);
                }
                KeyCode::Enter => {
                    self.jump_palette_selection();
                }
                _ => {}
            }
        }
        true
    }

    /// Move the command-palette cursor one row (`down` toward the last target),
    /// clamped to the list and anchored to the visible targets.
    fn move_palette_selection(&mut self, down: bool) {
        let len = self.palette_targets().len();
        if len == 0 {
            self.palette_state.select(None);
            return;
        }
        let idx = self.palette_state.selected().unwrap_or(0).min(len - 1);
        let next = if down {
            (idx + 1).min(len - 1)
        } else {
            idx.saturating_sub(1)
        };
        self.palette_state.select(Some(next));
    }

    /// Jump to the highlighted palette target: reset the view stack to it and
    /// dismiss the palette. Only the screens and the current repo/branch are
    /// wired here; full §6.11 fuzzy search and branch/context switching is the
    /// documented residual.
    fn jump_palette_selection(&mut self) {
        let targets = self.palette_targets();
        let Some(idx) = self.palette_state.selected() else {
            return;
        };
        let Some(target) = targets.get(idx) else {
            return;
        };
        let leaving_receipts = self.view.is_receipt();
        match target {
            PaletteTarget::View(view) => self.jump_to_view(*view),
            // These two write `view` directly instead of delegating to
            // `jump_to_view`, so they are the only ones that still have to run
            // the receipt exit — a `View` target has already run it inside
            // `jump_to_view`, and running it twice would clear the same state
            // on every palette jump. Outside the hub there is nothing to drop,
            // so the guard keeps a settled push on screen when the target is
            // simply another repository.
            PaletteTarget::Repo(_) => {
                self.view = View::Repo;
                if leaving_receipts {
                    self.leave_receipt_domain();
                }
            }
            PaletteTarget::Branch(_) => {
                self.view = View::Branches;
                if leaving_receipts {
                    self.leave_receipt_domain();
                }
            }
        }
        self.palette_open = false;
        self.help_open = false;
    }

    /// Reset the view stack to `view` and its row cursor to the first row
    /// (DESIGN §6.11 "directly reset the stack"). Every panel tolerates an
    /// empty load, so landing on a screen with no data shows its empty state.
    fn jump_to_view(&mut self, view: View) {
        // The palette is a second door out of the hub, so it drops the same
        // feedback the `Esc` exit does (B2).
        let leaving_receipts = self.view.is_receipt() && !view.is_receipt();
        self.view = view;
        self.clear_error();
        if leaving_receipts {
            self.leave_receipt_domain();
        }
        match view {
            View::Dashboard => {
                self.dashboard_state
                    .select(if self.repos.is_empty() { None } else { Some(0) });
            }
            View::Repo => {
                self.file_state.select(if self.file_entries.is_empty() {
                    None
                } else {
                    Some(0)
                });
            }
            View::Branches => {
                self.branch_state.select(if self.branches.is_empty() {
                    None
                } else {
                    Some(0)
                });
            }
            View::Config => self
                .config_state
                .select(if self.config.scan_dirs.is_empty() {
                    None
                } else {
                    Some(0)
                }),
            View::History => self.history.select(0),
            View::Tags => self.tags_state.select(0),
            View::Pulls | View::Checks | View::Issues | View::Notifications => {
                self.github_state.selected = 0;
            }
            // Drill-down states reached by Enter inside a screen, not a jump.
            View::Diff | View::Commit | View::PullDetail | View::IssueDetail => {}
        }
    }

    /// Key handler for the dashboard: list navigation and view entry points.
    fn handle_dashboard_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                move_selection(&mut self.dashboard_state, self.repos.len(), true);
                true
            }
            KeyCode::Char('k') | KeyCode::Up => {
                move_selection(&mut self.dashboard_state, self.repos.len(), false);
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

    /// Key handler for the single-repo view's own verbs. The global layer
    /// owns `Esc` (back to the dashboard), `q` (quit), `r` (refresh) and the
    /// `g`/`G`/`Home`/`End` jumps — except `g`, which still opens the
    /// receipts hub until the 远端 context tab takes over; the discard
    /// confirmation is caught by the focus trap before this runs.
    fn handle_repo_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                move_selection(&mut self.file_state, self.file_entries.len(), true);
                true
            }
            KeyCode::Char('k') | KeyCode::Up => {
                move_selection(&mut self.file_state, self.file_entries.len(), false);
                true
            }
            // `Enter` = 打开 (§0): open the highlighted file's diff sub-screen
            // (`d` is the pre-redesign alias kept until task 7 remaps it).
            KeyCode::Enter => {
                self.view_file_diff();
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
            // Enter the GitHub receipts hub from the repo view (M1.2: `g` is
            // the only receipts entry key in the repo view).
            KeyCode::Char('g') => {
                self.enter_github();
                true
            }
            // The five entry keys are the same shape — run one action,
            // keep looping — so they share a single arm instead of five
            // copies (G15): push / pull, then the branch, history and
            // tag panels.
            KeyCode::Char(c) => {
                match c {
                    'p' => self.start_remote("push"),
                    'l' => self.start_remote("pull"),
                    'b' => self.enter_branches(),
                    'h' => self.enter_history(),
                    't' => self.enter_tags(),
                    _ => {}
                }
                true
            }
            _ => true,
        }
    }

    /// Key handler for the diff viewer: vertical scroll only — `Esc` (back to
    /// the repo view), `q` (quit) and the `g`/`Home` jump to the top belong
    /// to [`App::dispatch_key`].
    fn handle_diff_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.diff_scroll = self.diff_scroll.saturating_add(1);
                true
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.diff_scroll = self.diff_scroll.saturating_sub(1);
                true
            }
            _ => true,
        }
    }

    /// Key handler for the branch panel: list navigation and its two overlay
    /// openers. The open overlays and the global verbs (`Esc` back, `q` quit,
    /// `r` reload) are handled before this runs.
    fn handle_branches_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
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
            _ => true,
        }
    }

    /// Key handler for the commit history view: `j/k` and the arrow keys move
    /// the selection and reload the highlighted commit's detail. `Esc` (back
    /// to the repo view) and `q` (quit) belong to [`App::dispatch_key`].
    fn handle_history_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.move_history_selection(true),
            KeyCode::Char('k') | KeyCode::Up => self.move_history_selection(false),
            _ => {}
        }
        true
    }

    /// Move the history selection one row and reload the highlighted
    /// commit's detail when the cursor actually moved (G13: both `j` and `k`
    /// used to repeat this pair of steps verbatim).
    ///
    /// A press that cannot move the cursor (empty list, already at the edge)
    /// skips the reload: it re-opens the repository and re-diffs the whole
    /// tree, and a clamped press would otherwise pay for that on every
    /// repeat. See [`App::load_history_detail`] for why that reload still
    /// runs synchronously (known limitation, G38).
    fn move_history_selection(&mut self, down: bool) {
        let before = self.history.selected;
        self.history.move_selection(down);
        if self.history.selected != before {
            self.load_history_detail();
        }
    }

    /// Key handler for the tag panel's list. Both overlays (create input,
    /// delete confirmation) are caught by the modal focus trap before this
    /// runs, as are `Esc` (back to the repo view) and `q` (quit).
    fn handle_tags_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.tags_state.move_selection(true),
            KeyCode::Char('k') | KeyCode::Up => self.tags_state.move_selection(false),
            KeyCode::Char('c') => self.start_tag_create(),
            KeyCode::Char('d') => self.start_tag_delete(),
            KeyCode::Char('p') => self.start_tag_push(false),
            KeyCode::Char('a') => self.start_tag_push(true),
            _ => {}
        }
        true
    }

    /// Key handler for the config panel's list. The picker, the path input
    /// and the delete confirmation are caught by the modal focus trap before
    /// this runs, as are `Esc` (back to the dashboard), `q` (quit) and `r`
    /// (rescan).
    fn handle_config_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('a') => {
                self.start_config_add();
                true
            }
            KeyCode::Char('i') => {
                self.start_config_manual();
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

    /// Key handler for the directory picker overlay: list navigation,
    /// descend/ascend, selection routed through `add_scan_dir`, and a
    /// switch to the manual input. `Esc` cancels; every other global verb
    /// (`q`, `r`, `?`, …) is swallowed by the focus trap before it gets here.
    fn handle_config_picker_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.cancel_config_add(),
            KeyCode::Char('i') => self.start_config_manual(),
            KeyCode::Char('j') | KeyCode::Down => {
                let len = self.picker_row_count();
                move_selection(&mut self.picker_state, len, true);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let len = self.picker_row_count();
                move_selection(&mut self.picker_state, len, false);
            }
            KeyCode::Char('l') | KeyCode::Right => self.picker_descend(),
            KeyCode::Char('h') | KeyCode::Left | KeyCode::Backspace => self.picker_ascend(),
            KeyCode::Enter => self.picker_submit(),
            _ => {}
        }
    }
}

/// List the subdirectories of `dir`, sorted by name.
///
/// Entries that vanish or cannot be inspected mid-read are skipped; only a
/// failed `read_dir` (missing path, no permission) is reported as an error.
fn read_picker_entries(dir: &std::path::Path) -> io::Result<Vec<std::path::PathBuf>> {
    let mut entries: Vec<std::path::PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    entries.sort();
    Ok(entries)
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

/// Highlight `idx` clamped to `len` rows, writing the index and the
/// [`ListState`] cursor together — the single place the panel's duplicated
/// `selected` / `list_state` pair stays in sync. An empty list clears both.
fn select_row(selected: &mut usize, list_state: &mut ListState, len: usize, idx: usize) {
    if len == 0 {
        *selected = 0;
        list_state.select(None);
        return;
    }
    *selected = idx.min(len - 1);
    list_state.select(Some(*selected));
}

/// Move the shared panel cursor one row via [`move_selection`], then mirror
/// the result into `selected` so the pair stays in sync.
fn move_row(selected: &mut usize, list_state: &mut ListState, len: usize, down: bool) {
    move_selection(list_state, len, down);
    *selected = list_state.selected().unwrap_or(0);
}

/// The selection of `state` when it still indexes a list of `len` items;
/// stale (out-of-range) and absent selections yield `None`.
fn valid_selection(state: &ListState, len: usize) -> Option<usize> {
    state.selected().filter(|i| *i < len)
}

/// Re-clamp `state`'s cursor to a list of `len` items: an empty list clears
/// it, a stale or absent selection falls back to the first entry.
fn clamp_selection(state: &mut ListState, len: usize) {
    if len == 0 {
        state.select(None);
        return;
    }
    let idx = valid_selection(state, len).unwrap_or(0);
    state.select(Some(idx));
}

/// One jump target of the `Ctrl-P` command palette (DESIGN §6.11): a screen,
/// or the current repository / branch when one is open. The palette lists these
/// and resets the view stack onto the highlighted one on `Enter`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteTarget {
    /// A top-level screen, selected by its [`View`].
    View(View),
    /// The currently opened repository, landed on its workspace.
    Repo(String),
    /// The current branch of the open repository, landed on its panel.
    Branch(String),
}

/// Which overlay owns the keyboard, resolved by [`App::active_modal`] — the
/// single source behind the focus trap (§2.4: 同时只允许一个模态).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModalKind {
    /// Single-line text input; the slot names the buffer it edits.
    Input(InputSlot),
    /// y/n (+`Enter`) confirmation; the slot names the action it guards.
    Confirm(ConfirmSlot),
    /// Interactive directory picker (list-type overlay).
    Picker,
}

/// The text input behind [`ModalKind::Input`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputSlot {
    /// The commit-message input (`View::Commit`).
    Commit,
    /// The branch-create input.
    BranchCreate,
    /// The add-scan-dir input.
    ConfigAdd,
    /// The tag-create input.
    TagCreate,
}

/// The confirmation dialog behind [`ModalKind::Confirm`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfirmSlot {
    /// Discarding the highlighted worktree file.
    Discard,
    /// Deleting the highlighted branch.
    BranchDelete,
    /// Deleting the highlighted scan dir.
    ConfigDelete,
    /// Deleting the highlighted tag.
    TagDelete,
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
    /// y/Y/Enter: the caller runs the confirmed action.
    Yes,
    /// n/N/Esc: the caller dismisses the dialog.
    No,
    /// Any other key, swallowed while the dialog is open.
    Ignored,
}

/// Route one key press through the shared y/n confirmation modal
/// (discard-style, convention 8): `Enter` confirms the dialog's action
/// (§0: Enter = 确认模态框中的按钮), y/Y confirms, n/N/Esc cancels, and every
/// other key — `q`, `r`, navigation — is swallowed while it is open.
fn confirm_modal_key(key: KeyEvent) -> ConfirmOutcome {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => ConfirmOutcome::Yes,
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => ConfirmOutcome::No,
        _ => ConfirmOutcome::Ignored,
    }
}

/// `true` for a key press carrying no `Ctrl`/`Alt` chord — the shape every
/// "plain character" global verb (`q`, `r`, `<`, `>`, `?`, `g`, `G`, `/`)
/// must match, so the chords stay free for their own meanings (`Ctrl-C`,
/// `Ctrl-P`).
fn is_plain(key: KeyEvent) -> bool {
    !key.modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
}

/// `true` for the plain (unchorded) character `c`.
fn is_plain_char(key: KeyEvent, c: char) -> bool {
    key.code == KeyCode::Char(c) && is_plain(key)
}

/// `true` for the `Ctrl-P` command-palette chord (DESIGN §6.11).
fn is_ctrl(key: KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
}

/// `true` for `Ctrl-C`, the force-quit chord that applies in every state
/// (DESIGN §0: 终端惯例，任何时刻生效) — modals included, which is why the
/// dispatch checks it before the focus trap.
fn is_ctrl_c(key: KeyEvent) -> bool {
    is_ctrl(key) && key.code == KeyCode::Char('c')
}

/// Move the `g`/`G`/`Home`/`End` target: `head` selects the first row, tail
/// the last one of a `len`-row list. An empty list clears the cursor.
fn jump_selection(state: &mut ListState, len: usize, head: bool) {
    if len == 0 {
        state.select(None);
        return;
    }
    state.select(Some(if head { 0 } else { len - 1 }));
}

/// Status-line text for a stale repository index (G20): one template for
/// every entry point, `<op>: 仓库索引失效`, so the wording cannot drift
/// between the commit flow, the panels and the create/delete paths.
fn invalid_repo_msg(op: &str) -> String {
    format!("{op}: 仓库索引失效")
}

/// Run the TUI until the user quits (`q` / `Ctrl-C`).
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
                if !app.dispatch_key(key) {
                    return Ok(());
                }
            }
        }
        app.poll_remote();
        app.poll_batch();
        app.poll_github();
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

        assert!(app.dispatch_key(key(KeyCode::Char('j'))));
        assert_eq!(app.dashboard_state.selected(), Some(0));

        assert!(app.dispatch_key(key(KeyCode::Char('j'))));
        assert_eq!(app.dashboard_state.selected(), Some(1));

        assert!(app.dispatch_key(key(KeyCode::Char('k'))));
        assert_eq!(app.dashboard_state.selected(), Some(0));
    }

    #[test]
    fn dashboard_j_stops_at_last_repo() {
        let mut app = app_with_repos(2);
        app.dashboard_state.select(Some(1));
        assert!(app.dispatch_key(key(KeyCode::Char('j'))));
        assert_eq!(app.dashboard_state.selected(), Some(1));
    }

    #[test]
    fn dashboard_enter_opens_selected_repo() {
        let mut app = app_with_repos(1);
        // Nothing selected: Enter is a no-op.
        assert!(app.dispatch_key(key(KeyCode::Enter)));
        assert_eq!(app.view, View::Dashboard);

        app.dashboard_state.select(Some(0));
        assert!(app.dispatch_key(key(KeyCode::Enter)));
        assert_eq!(app.view, View::Repo);
    }

    #[test]
    fn dashboard_q_quits() {
        let mut app = app_with_repos(1);
        assert!(!app.dispatch_key(key(KeyCode::Char('q'))));
    }

    /// DESIGN §0/§1.3: `q` quits the application from every non-modal screen.
    /// It never pops the stack, so the view is untouched when the app stops.
    #[test]
    fn q_exits_from_every_non_modal_screen() {
        for view in [
            View::Dashboard,
            View::Repo,
            View::Diff,
            View::Branches,
            View::Config,
            View::History,
            View::Tags,
            View::Pulls,
            View::PullDetail,
            View::Checks,
            View::Issues,
            View::IssueDetail,
            View::Notifications,
        ] {
            let mut app = App::new(Config::default());
            app.view = view;
            assert!(
                !app.dispatch_key(key(KeyCode::Char('q'))),
                "q must exit from {view:?}"
            );
            assert_eq!(app.view, view, "q must not move the stack in {view:?}");
        }
    }

    /// `Ctrl-C` force-quits in every state — root screen, open input and open
    /// confirmation alike (§0: 任何时刻生效).
    #[test]
    fn ctrl_c_quits_in_any_state_including_modals() {
        let ctrl_c = || KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);

        let mut app = App::new(Config::default());
        assert!(!app.dispatch_key(ctrl_c()));

        // Input modal open: the chord beats the focus trap.
        app.view = View::Commit;
        app.commit_message = "draft".to_string();
        assert!(!app.dispatch_key(ctrl_c()));

        // Confirmation modal open: same rule.
        let mut app = App::new(Config::default());
        app.start_discard();
        assert!(!app.dispatch_key(ctrl_c()));
        assert!(
            app.show_discard_confirm,
            "quit happens without unwinding UI"
        );
    }

    /// §2.4 focus trap: while an input owns the keyboard every global verb is
    /// literal text — `q` types instead of quitting, `Esc` cancels the overlay
    /// instead of popping the screen behind it.
    #[test]
    fn input_modal_traps_global_keys_as_literal_input() {
        let dir = temp_dir("dispatch-input-trap");
        init_repo_with_commit(&dir);

        let mut app = app_with_repo_at(&dir);
        app.dashboard_state.select(Some(0));
        app.enter_repo();
        assert!(app.dispatch_key(key(KeyCode::Char('c'))));
        assert_eq!(app.view, View::Commit);

        for c in ['q', 'r', 'j', '?', '/', 'g', 'G'] {
            assert!(app.dispatch_key(key(KeyCode::Char(c))));
            assert_eq!(app.view, View::Commit, "{c} must stay inside the input");
        }
        assert_eq!(app.commit_message, "qrj?/gG");

        // Chords and panel keys are swallowed, never acted on.
        let ctrl_p = KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL);
        assert!(app.dispatch_key(ctrl_p));
        assert!(!app.palette_open, "the palette never opens over an input");
        assert!(app.dispatch_key(key(KeyCode::Tab)));
        assert!(app.dispatch_key(key(KeyCode::Enter)));
        // Enter submits, returning to the view behind the overlay.
        assert_eq!(app.view, View::Repo);

        // Esc cancels instead: reopen and cancel out.
        assert!(app.dispatch_key(key(KeyCode::Char('c'))));
        assert_eq!(app.view, View::Commit);
        assert!(app.dispatch_key(key(KeyCode::Esc)));
        assert_eq!(app.view, View::Repo);
        assert!(app.commit_message.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// §2.4 focus trap for confirmations: only y/Enter/n/Esc act, `q` and the
    /// navigation/refresh verbs are swallowed, `Enter` confirms the button (§0).
    #[test]
    fn confirm_modal_swallows_global_keys_and_enter_confirms() {
        let mut app = App::new(Config::default());
        app.start_discard();

        for code in [
            KeyCode::Char('q'),
            KeyCode::Char('r'),
            KeyCode::Char('j'),
            KeyCode::Char('g'),
            KeyCode::Tab,
            KeyCode::Char('?'),
        ] {
            assert!(app.dispatch_key(key(code)));
            assert!(app.show_discard_confirm, "{code:?} must not dismiss it");
        }

        // Enter presses the highlighted button and closes the dialog.
        assert!(app.dispatch_key(key(KeyCode::Enter)));
        assert!(!app.show_discard_confirm);
        assert_eq!(app.view, View::Dashboard);
    }

    /// §6.11 prepared overlays: `?` and `Ctrl-P` toggle one at a time and Esc
    /// closes them before it pops a screen.
    #[test]
    fn help_and_palette_toggles_are_mutually_exclusive_and_esc_closes_them_first() {
        let mut app = App::new(Config::default());

        assert!(app.dispatch_key(key(KeyCode::Char('?'))));
        assert!(app.help_open);

        let ctrl_p = KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL);
        assert!(app.dispatch_key(ctrl_p));
        assert!(app.palette_open);
        assert!(
            !app.help_open,
            "opening the palette closes the help overlay"
        );

        // Esc closes the overlay only; the stack and status line stay put.
        assert!(app.dispatch_key(key(KeyCode::Esc)));
        assert!(!app.help_open);
        assert!(!app.palette_open);
        assert_eq!(app.view, View::Dashboard);
        assert!(app.status.is_none());

        // `?` toggles the help overlay back off.
        assert!(app.dispatch_key(key(KeyCode::Char('?'))));
        assert!(app.help_open);
        assert!(app.dispatch_key(key(KeyCode::Char('?'))));
        assert!(!app.help_open);

        // With no overlay open, Esc at the root still toasts 「已在顶层」.
        assert!(app.dispatch_key(key(KeyCode::Esc)));
        assert_eq!(app.status.as_ref().unwrap().message, "已在顶层");
    }

    /// §6.11 `q` inside the help overlay closes it instead of quitting the
    /// application (the bug this wave fixes; `q` still quits from a plain view).
    #[test]
    fn q_closes_help_overlay_instead_of_quitting() {
        let mut app = App::new(Config::default());
        app.toggle_help_overlay();
        assert!(app.help_open);

        // The `q` inside help is a close verb, not the app-exit verb.
        assert!(app.dispatch_key(key(KeyCode::Char('q'))), "must not quit");
        assert!(!app.help_open, "help must close");
        assert!(!app.palette_open, "palette must stay closed");
        assert_eq!(app.view, View::Dashboard);
    }

    /// §6.11 `q` inside the command palette closes it instead of quitting.
    #[test]
    fn q_closes_command_palette_instead_of_quitting() {
        let mut app = App::new(Config::default());
        app.toggle_palette_overlay();
        assert!(app.palette_open);

        assert!(app.dispatch_key(key(KeyCode::Char('q'))), "must not quit");
        assert!(!app.palette_open, "palette must close");
        assert!(!app.help_open);
    }

    /// §6.11 command palette: `j`/`k` move the cursor (clamped) and `Enter`
    /// resets the view stack onto the highlighted screen, then dismisses.
    #[test]
    fn palette_j_k_moves_and_enter_jumps_to_screen() {
        let mut app = App::new(Config::default());
        app.toggle_palette_overlay();

        // `k` clamps at the top; `j`/`Down` advance to the next target.
        app.dispatch_key(key(KeyCode::Char('k')));
        assert_eq!(app.palette_state.selected(), Some(0));
        app.dispatch_key(key(KeyCode::Char('j')));
        assert_eq!(app.palette_state.selected(), Some(1));

        // Jumping to the Dashboard target resets the stack to the dashboard and
        // dismisses the palette — not a quit.
        app.palette_state.select(Some(0));
        assert!(app.dispatch_key(key(KeyCode::Enter)), "jump must not quit");
        assert!(!app.palette_open, "palette must dismiss on Enter");
        assert_eq!(app.view, View::Dashboard);
    }

    /// §0 辅动词: `g`/`Home` jump to the first row, `G`/`End` to the last of
    /// the focused list — never into the stack.
    #[test]
    fn g_and_g_jump_the_focused_list() {
        let mut app = app_with_repos(3);
        app.dashboard_state.select(Some(1));

        assert!(app.dispatch_key(key(KeyCode::Char('G'))));
        assert_eq!(app.dashboard_state.selected(), Some(2));
        assert!(app.dispatch_key(key(KeyCode::Char('g'))));
        assert_eq!(app.dashboard_state.selected(), Some(0));
        assert!(app.dispatch_key(key(KeyCode::End)));
        assert_eq!(app.dashboard_state.selected(), Some(2));
        assert!(app.dispatch_key(key(KeyCode::Home)));
        assert_eq!(app.dashboard_state.selected(), Some(0));
        assert_eq!(app.view, View::Dashboard, "a jump never enters the stack");
    }

    /// Transitional exception to §2.1: the repo view keeps `g` as the receipts
    /// hub entry (`g 进入` hint) until the 远端 context tab replaces it, while
    /// `G` already behaves as the "jump to the last row" verb on that screen.
    #[test]
    fn repo_g_still_enters_the_receipts_hub_and_g_jumps_the_file_list() {
        let dir = temp_dir("dispatch-repo-g");
        init_repo_with_commit(&dir);

        let mut app = app_with_repo_at(&dir);
        app.dashboard_state.select(Some(0));
        app.enter_repo();

        assert!(app.dispatch_key(key(KeyCode::Char('G'))));
        assert_eq!(
            app.view,
            View::Repo,
            "G jumps the file list, no stack entry"
        );

        assert!(app.dispatch_key(key(KeyCode::Char('g'))));
        assert_eq!(app.view, View::Pulls, "g is still the receipts entry key");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// §1.1: panel focus (`Tab`/`BackTab`), context tags (`<`/`>`) and search
    /// (`/`) are reserved for the upcoming subtasks and act as no-ops today.
    #[test]
    fn tab_context_and_search_keys_are_reserved_no_ops() {
        let dir = temp_dir("dispatch-reserved");
        init_repo_with_commit(&dir);

        let mut app = app_with_repo_at(&dir);
        app.dashboard_state.select(Some(0));
        app.enter_repo();

        for code in [
            KeyCode::Tab,
            KeyCode::BackTab,
            KeyCode::Char('<'),
            KeyCode::Char('>'),
            KeyCode::Char('/'),
        ] {
            assert!(app.dispatch_key(key(code)));
            assert_eq!(app.view, View::Repo, "{code:?} must be a reserved no-op");
            assert!(app.status.is_none());
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// §0/§1.3: `r` reloads the open view wherever the dispatch lands it —
    /// local git views and the GitHub hub share one rule.
    #[test]
    fn global_r_refreshes_the_open_view() {
        let dir = temp_dir("dispatch-refresh");
        init_repo_with_commit(&dir);

        let mut app = app_with_repo_at(&dir);
        app.dashboard_state.select(Some(0));
        app.enter_repo();
        assert!(app.dispatch_key(key(KeyCode::Char('r'))));
        assert_eq!(app.view, View::Repo);

        enter_history(&mut app);
        assert!(app.dispatch_key(key(KeyCode::Char('r'))));
        assert_eq!(app.view, View::History);
        assert!(!app.history.commits.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// §0: `Enter` = 打开 — in the repo view it opens the highlighted file's
    /// diff sub-screen (the `d` alias stays until task 7 remaps it).
    #[test]
    fn enter_opens_the_selected_file_diff() {
        let dir = temp_dir("dispatch-enter-diff");
        init_repo_with_commit(&dir);
        std::fs::write(dir.join("a.txt"), "changed\n").unwrap();

        let mut app = app_with_repo_at(&dir);
        app.dashboard_state.select(Some(0));
        app.enter_repo();
        assert_eq!(app.file_entries.len(), 1);

        assert!(app.dispatch_key(key(KeyCode::Enter)));
        assert_eq!(app.view, View::Diff);
        assert!(!app.diff_content.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// §1.1 stack discipline: one Esc per level, and the root screen toasts
    /// 「已在顶层」 instead of exiting (quitting is `q`'s job, §1.2).
    #[test]
    fn esc_pops_one_level_per_press_and_toasts_at_the_root() {
        let dir = temp_dir("dispatch-esc-stack");
        init_repo_with_commit(&dir);
        std::fs::write(dir.join("a.txt"), "changed\n").unwrap();

        let mut app = app_with_repo_at(&dir);
        app.dashboard_state.select(Some(0));
        assert!(app.dispatch_key(key(KeyCode::Enter)));
        assert_eq!(app.view, View::Repo);
        assert!(app.dispatch_key(key(KeyCode::Enter)));
        assert_eq!(app.view, View::Diff);

        assert!(app.dispatch_key(key(KeyCode::Esc)));
        assert_eq!(app.view, View::Repo);
        assert!(app.dispatch_key(key(KeyCode::Esc)));
        assert_eq!(app.view, View::Dashboard);
        assert!(app.dispatch_key(key(KeyCode::Esc)));
        assert_eq!(app.view, View::Dashboard);
        assert_eq!(app.status.as_ref().unwrap().message, "已在顶层");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dashboard_r_refreshes_and_clears_error() {
        let mut app = app_with_repos(1);
        app.set_error("boom".to_string());
        assert!(app.dispatch_key(key(KeyCode::Char('r'))));
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

    /// Canonical form of `path`, matching what [`App`] stores for a scan
    /// dir: `add_scan_dir` persists the canonicalized path, so asserting
    /// against a literal `temp_dir()` spelling breaks when the temp
    /// directory sits behind a symlink (macOS `/var`, a redirected `TMPDIR`).
    fn canon(path: &std::path::Path) -> std::path::PathBuf {
        git::canonical_or_self(path)
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

    /// Repository with one commit whose `origin` points at `url` — the two
    /// shapes the receipts hub must tell apart (a platform URL, a local path).
    fn repo_with_remote_at(dir: &std::path::Path, url: &str) {
        init_repo_with_commit(dir);
        let repo = git2::Repository::open(dir).unwrap();
        repo.remote("origin", url).unwrap();
    }

    /// Arm `github_rx` with a sender that is already gone, so the next
    /// `poll_github` observes the disconnected arm.
    fn arm_dead_receiver(app: &mut App) {
        let (tx, rx) = mpsc::unbounded_channel();
        app.github_rx = Some(rx);
        drop(tx);
    }

    // ---------- receipts hub: remote as the single source of truth ----------

    /// U1: `remote` is the only place the receipt copy comes from, so entering
    /// the hub fills it and every failure path empties it.
    #[tokio::test]
    async fn enter_github_clears_the_previous_remote_when_the_next_one_cannot_be_parsed() {
        let root = temp_dir("github-remote-reset");
        // A credential is needed for A to cache a gate, which is what the `r`
        // case below relies on; the fetch itself never reaches the network.
        if std::env::var("GITHUB_TOKEN").is_err() {
            std::env::set_var("GITHUB_TOKEN", "test-token");
        }
        let good = root.join("good");
        repo_with_remote_at(&good, "https://github.com/octo/demo.git");
        // A local path has no scheme/host, so `parse_remote` rejects it.
        let bad = root.join("bad");
        repo_with_remote_at(&bad, root.join("origin.git").to_str().unwrap());
        std::fs::create_dir_all(root.join("origin.git")).unwrap();

        let mut app = App::new(Config::default());
        app.repos.push(git::query_status(&good).unwrap());
        app.repos.push(git::query_status(&bad).unwrap());

        // A resolves: the panels read owner/repo/platform off the remote.
        app.current_repo = Some(0);
        app.enter_github();
        assert_eq!(app.github_state.owner(), Some("octo"));
        assert_eq!(app.github_state.repo(), Some("demo"));
        assert_eq!(
            app.github_state.kind(),
            Some(crate::platform::PlatformKind::GitHub)
        );
        // The readiness gate cached while entering A is what makes the `r`
        // case below bite; the fetch A armed is dropped so the B `r` is the
        // only receiver in play.
        assert!(
            app.github_state.client.is_some(),
            "A must cache a gate for the stale-gate case to bite"
        );
        app.github_rx = None;

        // B does not: nothing of A may survive, or the panel would keep
        // showing A's repo and `r` would fetch A's data.
        app.current_repo = Some(1);
        app.enter_github();
        assert!(
            app.github_state.remote.is_none(),
            "stale remote survived: {:?}",
            app.github_state.remote
        );
        assert_eq!(app.github_state.owner(), None, "titles must fall back");
        assert_eq!(app.github_state.repo(), None, "titles must fall back");
        assert_eq!(app.github_state.kind(), None);
        let status = app.status.as_ref().expect("the parse failure is reported");
        assert!(status.is_error, "expected an error status: {status:?}");

        // The gate cached from A must not turn `r` into a fetch against it.
        assert!(app.dispatch_key(key(KeyCode::Char('r'))));
        assert!(app.github_state.remote.is_none(), "remote stays empty");
        assert!(
            app.github_rx.is_none(),
            "no fetch may be armed without a remote"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// U1: the accessors are views onto `remote`, including its two "not
    /// usable for a title" shapes — no remote, and a half-filled one.
    #[test]
    fn github_state_accessors_mirror_the_remote_or_report_nothing() {
        let state = GithubState::default();
        assert_eq!(state.owner(), None, "no remote resolved");
        assert_eq!(state.repo(), None);
        assert_eq!(state.kind(), None);

        let mut state = GithubState::default();
        let mut remote = crate::platform::parse_remote("https://github.com/octo/demo.git").unwrap();
        state.remote = Some(remote.clone());
        assert_eq!(state.owner(), Some("octo"));
        assert_eq!(state.repo(), Some("demo"));
        assert_eq!(state.kind(), Some(crate::platform::PlatformKind::GitHub));

        // An empty segment can never complete an `owner/repo` title.
        remote.owner.clear();
        state.remote = Some(remote.clone());
        assert_eq!(state.owner(), None, "an empty owner is no owner");
        assert_eq!(state.repo(), Some("demo"), "the other half still reads");
        remote.repo.clear();
        state.remote = Some(remote);
        assert_eq!(state.repo(), None, "an empty repo is no repo");
    }

    /// U5: a git-layer failure reaches the panel verbatim, without being
    /// wrapped in a platform wording it does not belong to.
    #[test]
    fn poll_github_shows_a_non_platform_failure_untouched() {
        let mut app = App::new(Config::default());
        app.view = View::Pulls;
        let (tx, rx) = mpsc::unbounded_channel();
        tx.send(GithubMsg::Other(
            "未打开仓库，无法获取 HEAD 提交".to_string(),
        ))
        .unwrap();
        drop(tx);
        app.github_rx = Some(rx);
        app.poll_github();
        assert_eq!(
            app.remote_status,
            Some(RemoteStatus::Error(
                "未打开仓库，无法获取 HEAD 提交".to_string()
            )),
            "a git-layer message is shown as written"
        );
    }

    /// U5: a platform failure is rendered once, in `poll_github`, so every
    /// sender agrees on the wording.
    #[test]
    fn poll_github_renders_a_platform_failure_through_the_shared_renderer() {
        let mut app = App::new(Config::default());
        app.view = View::Pulls;
        let (tx, rx) = mpsc::unbounded_channel();
        let error = crate::platform::PlatformError::Network("boom".to_string());
        let expected = crate::platform::render_platform_error(&error);
        assert_eq!(expected, "网络错误，请检查网络连接", "the shared wording");
        tx.send(GithubMsg::Error(error)).unwrap();
        drop(tx);
        app.github_rx = Some(rx);
        app.poll_github();
        assert_eq!(app.remote_status, Some(RemoteStatus::Error(expected)));
    }

    /// U3: with no repository open there is no remote to name, so the copy stays
    /// neutral — but a GitHub remote keeps its v0.5.0 wording.
    #[test]
    fn enter_github_without_a_repository_names_only_a_known_platform() {
        let mut app = App::new(Config::default());
        app.enter_github();
        let message = &app.status.as_ref().expect("an error is reported").message;
        assert!(message.contains("未打开仓库"), "unexpected copy: {message}");
        assert!(message.contains("平台"), "neutral fallback: {message}");

        app.github_state.remote = Some(crate::ui::github_remote());
        app.enter_github();
        assert_eq!(
            app.status.as_ref().map(|s| s.message.as_str()),
            Some("打开 GitHub 回执失败: 未打开仓库"),
            "GitHub copy must stay byte-identical to v0.5.0"
        );
    }

    /// U2: `r` without a resolved remote reports instead of panicking — both
    /// with no client at all and with a stale gate left by an earlier repo.
    #[test]
    fn refresh_without_a_remote_reports_instead_of_panicking() {
        let mut app = App::new(Config::default());
        app.view = View::Pulls;
        assert!(app.dispatch_key(key(KeyCode::Char('r'))));
        assert!(app.github_rx.is_none(), "no client, nothing spawned");

        app.github_state.client = Some(crate::ui::pulls::offline_client());
        assert!(app.dispatch_key(key(KeyCode::Char('r'))));
        assert!(app.github_rx.is_none(), "no remote, nothing spawned");
        match &app.remote_status {
            Some(RemoteStatus::Error(msg)) => {
                assert!(msg.contains("未解析到远端"), "unhelpful status: {msg}");
            }
            other => panic!("expected an explanatory error, got {other:?}"),
        }
    }

    /// U2: the detail views guard the same way — the panel still opens, so the
    /// user gets the explanation instead of a panic.
    #[test]
    fn opening_a_detail_without_a_remote_reports_instead_of_panicking() {
        let mut app = App::new(Config::default());
        app.view = View::Pulls;
        app.github_state.client = Some(crate::ui::pulls::offline_client());
        app.github_state.pulls = vec![crate::platform::PrSummary {
            number: 1,
            title: "t".to_string(),
            state: "open".to_string(),
            draft: false,
            author: "alice".to_string(),
            head_ref: "feat".to_string(),
            base_ref: "main".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        }];
        assert!(app.dispatch_key(key(KeyCode::Enter)));
        assert_eq!(app.view, View::PullDetail, "the panel still opens");
        assert!(app.github_rx.is_none(), "nothing spawned");
        assert!(
            app.status
                .as_ref()
                .is_some_and(|s| s.message.contains("未解析到远端")),
            "the user is told why nothing loads"
        );
    }

    /// U3: the op label follows the remote's platform; GitHub keeps its v0.5.0
    /// `github` slug so `github 成功` stays byte-identical.
    #[test]
    fn receipt_op_label_follows_the_remote_platform() {
        use crate::platform::PlatformKind;
        assert_eq!(platform_op_slug(PlatformKind::GitHub), "github");
        assert_eq!(platform_op_slug(PlatformKind::GitLab), "gitlab");
        assert_eq!(platform_op_slug(PlatformKind::Gitea), "gitea");
        assert_eq!(platform_op_slug(PlatformKind::SelfHosted), "self-hosted");
        for kind in [
            PlatformKind::GitLab,
            PlatformKind::Gitea,
            PlatformKind::SelfHosted,
        ] {
            let op = platform_op_slug(kind);
            assert!(
                op != "github" && op != "GitHub",
                "{kind:?} borrows the GitHub op label"
            );
        }
    }

    /// U3: the op label a fetch arms is the remote's platform, so the status
    /// row reads `gitlab 成功` / `gitea 成功` and never borrows the GitHub name.
    #[tokio::test]
    async fn refresh_github_labels_the_op_with_the_remote_platform() {
        for (url, env, expected) in [
            (
                "https://gitlab.com/group/proj.git",
                "GITLAB_TOKEN",
                "gitlab",
            ),
            ("https://gitea.com/octo/demo.git", "GITEA_TOKEN", "gitea"),
        ] {
            if std::env::var(env).is_err() {
                std::env::set_var(env, "test-token");
            }
            let mut app = App::new(Config::default());
            app.view = View::Pulls;
            let remote = crate::platform::parse_remote(url).unwrap();
            // The gate is what `refresh_github` checks before arming; the
            // spawned task is never polled, so this never reaches the network.
            app.github_state.client = Some(crate::platform::build_client(&remote).expect("gate"));
            app.github_state.remote = Some(remote);
            app.refresh_github();
            assert_eq!(
                app.receipt_op, expected,
                "{url}: wrong op label ({expected} 成功 is what the user reads)"
            );
            assert_ne!(app.receipt_op, "github", "{url}: GitHub op leaked");
            assert_eq!(
                app.receipt_op_label(),
                expected,
                "the receipt status row reads the platform slug"
            );
            // The push/pull field stays out of the receipt domain (B2).
            assert_ne!(app.remote_op, expected, "{url}: receipt wrote remote_op");
            app.github_rx = None;
        }
    }

    /// B2: leaving the hub drops the settled receipt feedback, so the repo
    /// status row never renders `gitlab 成功` for an operation it never ran.
    #[test]
    fn leaving_the_receipt_hub_clears_the_platform_feedback() {
        let mut app = App::new(Config::default());
        app.view = View::Pulls;
        app.receipt_op = "gitlab";
        app.remote_status = Some(RemoteStatus::Success);
        app.dispatch_key(key(KeyCode::Esc));
        assert_eq!(app.view, View::Repo, "the hub exits to the repo view");
        assert!(app.remote_status.is_none(), "receipt feedback dropped");
        assert_eq!(
            app.receipt_op_label(),
            app.remote_op,
            "falls back to push/pull"
        );
    }

    /// B2: the command palette is a second door out of the hub, so jumping
    /// away drops the receipt feedback the same way `Esc` does — otherwise the
    /// repo status row renders `操作完成` for a fetch that never ran there.
    #[test]
    fn palette_jump_out_of_the_hub_drops_the_receipt_feedback() {
        let mut app = App::new(Config::default());
        app.view = View::Pulls;
        app.receipt_op = "gitlab";
        app.remote_status = Some(RemoteStatus::Success);
        app.toggle_palette_overlay();
        let idx = app
            .palette_targets()
            .iter()
            .position(|t| matches!(t, PaletteTarget::View(View::Repo)))
            .expect("the repo screen is a palette target");
        app.palette_state.select(Some(idx));
        assert!(app.dispatch_key(key(KeyCode::Enter)), "jump must not quit");
        assert_eq!(app.view, View::Repo);
        assert!(
            app.remote_status.is_none(),
            "the settled receipt feedback followed the user out of the hub"
        );
        assert_eq!(app.receipt_op_label(), "", "the platform slug leaked too");
    }

    /// Round-3 🟡1: leaving the hub disarms a fetch still in flight —
    /// `poll_github` would otherwise drain its result every frame and repaint
    /// `操作完成` onto a repo status row that never asked for it.
    #[test]
    fn leaving_the_hub_disarms_an_in_flight_receipt_fetch() {
        let mut app = App::new(Config::default());
        app.view = View::Pulls;
        let (_tx, rx) = mpsc::unbounded_channel::<GithubMsg>();
        app.github_rx = Some(rx);
        app.dispatch_key(key(KeyCode::Esc));
        assert_eq!(app.view, View::Repo, "the hub exits to the repo view");
        assert!(
            app.github_rx.is_none(),
            "the in-flight fetch must not outlive the hub it was started for"
        );
    }

    /// Round-3 🟡1: a tab switch arms a fetch for the *new* tab, so the old
    /// tab's receiver must go first instead of refusing the new fetch or
    /// landing its rows under the new tab's title.
    #[test]
    fn switching_tabs_disarms_the_previous_tabs_fetch() {
        let mut app = App::new(Config::default());
        let (_tx, rx) = mpsc::unbounded_channel::<GithubMsg>();
        app.github_rx = Some(rx);
        assert!(app.switch_github_tab(GithubTab::Checks), "switch succeeds");
        assert_eq!(app.view, View::Checks);
        assert!(
            app.github_rx.is_none(),
            "the old tab's fetch has nowhere to land now"
        );
    }

    /// The guard's false side: re-pressing the tab we are already on keeps the
    /// in-flight fetch, so the same key cannot drop a loading result and spend
    /// another request per keypress (v0.5.0 idempotency).
    #[test]
    fn re_pressing_the_current_tab_keeps_the_in_flight_fetch() {
        let mut app = App::new(Config::default());
        app.view = View::Pulls;
        app.github_state.tab = GithubTab::Pulls;
        let (_tx, rx) = mpsc::unbounded_channel::<GithubMsg>();
        app.github_rx = Some(rx);
        assert!(
            app.switch_github_tab(GithubTab::Pulls),
            "re-pressing the tab succeeds"
        );
        assert_eq!(app.view, View::Pulls, "still on the same list");
        assert!(
            app.github_rx.is_some(),
            "a fetch for the screen we never left must survive"
        );
    }

    /// Leaving a detail for its own list: the tab does not change, but the
    /// in-flight fetch belongs to the detail and its rows the list will never
    /// render, so it is dropped and the list can load instead of staying on
    /// stale data behind the in-flight guard.
    #[test]
    fn leaving_a_detail_for_its_list_disarms_the_details_fetch() {
        let mut app = App::new(Config::default());
        app.view = View::PullDetail;
        app.github_state.tab = GithubTab::Pulls;
        let (_tx, rx) = mpsc::unbounded_channel::<GithubMsg>();
        app.github_rx = Some(rx);
        assert!(app.switch_github_tab(GithubTab::Pulls), "switch succeeds");
        assert_eq!(app.view, View::Pulls, "the list is now on screen");
        assert!(
            app.github_rx.is_none(),
            "the detail fetch has no list rows to land"
        );
    }

    /// `Esc` out of a detail is a second door onto the same intent as the tab
    /// key, so it drops the detail's in-flight fetch too: keeping it would land
    /// a `GithubMsg::Detail` result in a field the list never renders, leaving
    /// stale rows under a green 「成功」 that names an abandoned request.
    #[test]
    fn esc_out_of_a_detail_drops_the_details_fetch_too() {
        for (view, tab, list) in [
            (View::PullDetail, GithubTab::Pulls, View::Pulls),
            (View::IssueDetail, GithubTab::Issues, View::Issues),
        ] {
            let mut app = App::new(Config::default());
            app.view = view;
            app.github_state.tab = tab;
            let (_tx, rx) = mpsc::unbounded_channel::<GithubMsg>();
            app.github_rx = Some(rx);
            assert!(app.dispatch_key(key(KeyCode::Esc)));
            assert_eq!(app.view, list, "{view:?} returns to its own list");
            assert_eq!(
                app.github_state.tab, tab,
                "{view:?} returns to the tab that owns it"
            );
            assert!(
                app.github_rx.is_none(),
                "the detail fetch has no list rows to land"
            );
        }
    }

    /// B1: entering the hub drops every receipt the previous repository left
    /// behind — no list, no detail, no cached gate and no in-flight receiver —
    /// so nothing can be rendered under the new repository's title.
    #[test]
    fn entering_the_hub_drops_the_previous_receipts() {
        use crate::platform::{
            CheckSummary, IssueDetail, IssueSummary, NotificationSummary, PrDetail, PrSummary,
        };
        let pr = PrSummary {
            number: 7,
            title: "A 的 PR".to_string(),
            state: "open".to_string(),
            draft: false,
            author: "alice".to_string(),
            head_ref: "feat".to_string(),
            base_ref: "main".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let issue = IssueSummary {
            number: 3,
            title: "A 的 Issue".to_string(),
            state: "open".to_string(),
            author: "alice".to_string(),
            assignee: None,
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            labels: vec![],
        };
        let mut app = App::new(Config::default());
        app.view = View::Pulls;
        app.github_state.pulls = vec![pr.clone()];
        app.github_state.checks = vec![CheckSummary {
            name: "ci".to_string(),
            status: "completed".to_string(),
            conclusion: "success".to_string(),
            html_url: String::new(),
        }];
        app.github_state.issues = vec![issue.clone()];
        app.github_state.notifications = vec![NotificationSummary {
            id: "1".to_string(),
            repo: "octo/demo".to_string(),
            subject_title: "A 的通知".to_string(),
            subject_type: "Issue".to_string(),
            reason: "mention".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            unread: true,
        }];
        app.github_state.detail = Some(PrDetail {
            summary: pr,
            body: String::new(),
            labels: vec![],
            mergeable: Some(true),
            checks: vec![],
        });
        app.github_state.issue_detail = Some(IssueDetail {
            summary: issue,
            body: String::new(),
        });
        app.github_state.client = Some(crate::ui::pulls::offline_client());
        app.github_state.remote = Some(crate::ui::github_remote());
        app.receipt_op = "github";
        let (tx, rx) = mpsc::unbounded_channel();
        app.github_rx = Some(rx);
        drop(tx);
        assert!(app.github_state.connected(), "A is fully connected first");

        app.reset_github_state();

        assert!(
            !app.github_state.connected(),
            "the cached gate goes with the remote, so no panel reads a stale one"
        );
        assert!(
            app.github_state.pulls.is_empty(),
            "A's PRs would render under the new repository's title"
        );
        assert!(app.github_state.checks.is_empty());
        assert!(app.github_state.issues.is_empty());
        assert!(app.github_state.notifications.is_empty());
        assert!(app.github_state.detail.is_none());
        assert!(app.github_state.issue_detail.is_none());
        assert!(
            app.github_rx.is_none(),
            "an in-flight result has no receiver left to land in"
        );
        assert_eq!(
            app.receipt_op_label(),
            "",
            "the platform slug is dropped too"
        );
    }

    /// U3: a task that dies is reported by platform, not by a hardcoded name.
    #[test]
    fn poll_github_names_the_platform_of_a_dead_fetch_task() {
        let mut app = App::new(Config::default());
        app.view = View::Pulls;
        app.github_state.remote =
            Some(crate::platform::parse_remote("https://gitlab.com/group/proj.git").unwrap());
        arm_dead_receiver(&mut app);
        app.poll_github();
        assert!(app.github_rx.is_none(), "the dead receiver is dropped");
        assert_eq!(
            app.remote_status,
            Some(RemoteStatus::Error("GitLab 任务意外退出".to_string())),
            "a GitLab remote must not be reported as GitHub"
        );

        // GitHub keeps its v0.5.0 wording; an unknown remote stays neutral.
        app.github_state.remote = Some(crate::ui::github_remote());
        arm_dead_receiver(&mut app);
        app.poll_github();
        assert_eq!(
            app.remote_status,
            Some(RemoteStatus::Error("GitHub 任务意外退出".to_string()))
        );

        app.github_state.remote = None;
        arm_dead_receiver(&mut app);
        app.poll_github();
        assert_eq!(
            app.remote_status,
            Some(RemoteStatus::Error("平台 任务意外退出".to_string()))
        );
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

        // ...but is blocked while a remote operation is in flight, and the
        // key says so instead of dropping silently (G21, R7).
        let (_tx, rx) = mpsc::unbounded_channel();
        app.remote_rx = Some(rx);
        app.view = View::Repo;
        app.open_commit();
        assert_eq!(app.view, View::Repo);
        let st = app.status.as_ref().unwrap();
        assert!(!st.is_error, "{}", st.message);
        assert_eq!(st.message, "已有远端操作进行中");
        app.remote_rx = None;
        app.status = None;

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
        assert!(app.dispatch_key(key(KeyCode::Char('j'))));
        assert_eq!(app.dashboard_state.selected(), None);
        assert!(app.dispatch_key(key(KeyCode::Char('k'))));
        assert_eq!(app.dashboard_state.selected(), None);
        assert!(app.dispatch_key(key(KeyCode::Down)));
        assert!(app.dispatch_key(key(KeyCode::Up)));
        assert_eq!(app.dashboard_state.selected(), None);

        // Enter and b are no-ops without a selection.
        assert!(app.dispatch_key(key(KeyCode::Enter)));
        assert_eq!(app.view, View::Dashboard);
        assert!(app.dispatch_key(key(KeyCode::Char('b'))));
        assert_eq!(app.view, View::Dashboard);

        // l with no repos reports info and never starts a batch.
        assert!(app.dispatch_key(key(KeyCode::Char('l'))));
        let st = app.status.as_ref().unwrap();
        assert!(!st.is_error);
        assert!(st.message.contains("没有仓库"), "{}", st.message);
        assert!(!app.batch_running());

        // Esc at the root screen never quits: it toasts 「已在顶层」 (§1.2).
        assert!(app.dispatch_key(key(KeyCode::Esc)));
        assert_eq!(app.view, View::Dashboard);
        let st = app.status.as_ref().unwrap();
        assert!(!st.is_error);
        assert_eq!(st.message, "已在顶层");
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
        assert!(app.dispatch_key(key(KeyCode::Char('c'))));
        assert_eq!(app.view, View::Commit);
        app.commit_message = "draft".to_string();
        assert!(app.dispatch_key(key(KeyCode::Esc)));
        assert_eq!(app.view, View::Repo);
        assert!(app.commit_message.is_empty());

        // x opens the discard dialog, n cancels it.
        assert!(app.dispatch_key(key(KeyCode::Char('x'))));
        assert!(app.show_discard_confirm);
        assert!(app.dispatch_key(key(KeyCode::Char('n'))));
        assert!(!app.show_discard_confirm);

        // File-list keys on an empty list are safe no-ops.
        assert!(app.file_entries.is_empty());
        let sel = app.file_state.selected();
        assert!(app.dispatch_key(key(KeyCode::Char('j'))));
        assert_eq!(app.file_state.selected(), sel);
        assert!(app.dispatch_key(key(KeyCode::Char('k'))));
        assert_eq!(app.file_state.selected(), sel);

        // b opens the branch panel with the repo view as origin; Esc returns.
        assert!(app.dispatch_key(key(KeyCode::Char('b'))));
        assert_eq!(app.view, View::Branches);
        assert!(!app.branches.is_empty());
        assert!(app.dispatch_key(key(KeyCode::Esc)));
        assert_eq!(app.view, View::Repo);

        // p / l are no-ops without a current repo (nothing is spawned).
        app.current_repo = None;
        assert!(app.dispatch_key(key(KeyCode::Char('p'))));
        assert!(app.remote_rx.is_none());
        assert!(app.dispatch_key(key(KeyCode::Char('l'))));
        assert!(app.remote_rx.is_none());
        app.current_repo = Some(0);

        // Esc steps back to the dashboard; q exits from the repo view.
        assert!(app.dispatch_key(key(KeyCode::Esc)));
        assert_eq!(app.view, View::Dashboard);
        app.view = View::Repo;
        assert!(!app.dispatch_key(key(KeyCode::Char('q'))));

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
        assert!(app.dispatch_key(key(KeyCode::Char('d'))));
        assert_eq!(app.view, View::Diff);
        assert!(!app.diff_content.is_empty());
        assert_eq!(app.diff_scroll, 0);

        assert!(app.dispatch_key(key(KeyCode::Char('j'))));
        assert_eq!(app.diff_scroll, 1);
        assert!(app.dispatch_key(key(KeyCode::Down)));
        assert_eq!(app.diff_scroll, 2);
        assert!(app.dispatch_key(key(KeyCode::Char('k'))));
        assert_eq!(app.diff_scroll, 1);
        assert!(app.dispatch_key(key(KeyCode::Up)));
        assert_eq!(app.diff_scroll, 0);
        assert!(app.dispatch_key(key(KeyCode::Char('k')))); // saturates at 0
        assert_eq!(app.diff_scroll, 0);

        assert!(!app.dispatch_key(key(KeyCode::Char('q'))));
        assert!(app.dispatch_key(key(KeyCode::Esc)));
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
        assert!(app.dispatch_key(key(KeyCode::Char('d'))));
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

        assert!(app.dispatch_key(key(KeyCode::Char('h'))));
        assert!(app.dispatch_key(key(KeyCode::Char('i'))));
        assert_eq!(app.commit_message, "hi");

        // Ctrl+char does not type into the input.
        let ctrl = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL);
        assert!(app.dispatch_key(ctrl));
        assert_eq!(app.commit_message, "hi");

        assert!(app.dispatch_key(key(KeyCode::Backspace)));
        assert_eq!(app.commit_message, "h");

        // Esc cancels: back to the repo view with a cleared input.
        assert!(app.dispatch_key(key(KeyCode::Esc)));
        assert_eq!(app.view, View::Repo);
        assert!(app.commit_message.is_empty());

        // Enter submits (no repo here → error, returns to the repo view).
        app.view = View::Commit;
        app.commit_message = "msg".to_string();
        assert!(app.dispatch_key(key(KeyCode::Enter)));
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

        assert!(app.dispatch_key(key(KeyCode::Char('s'))));
        assert!(index_has_staged(&dir, "a.txt"));
        assert!(app.status.is_none());

        assert!(app.dispatch_key(key(KeyCode::Char('u'))));
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
    fn dispatch_key_ignores_release_events() {
        let mut app = app_with_repos(2);
        let mut k = key(KeyCode::Char('j'));
        k.kind = KeyEventKind::Release;
        assert!(app.dispatch_key(k));
        assert_eq!(app.dashboard_state.selected(), None);
    }

    // ---------- config panel skeleton ----------

    #[test]
    fn dashboard_c_enters_config_and_esc_returns() {
        let mut app = App::new(Config::default());
        app.set_error("stale".to_string());

        assert!(app.dispatch_key(key(KeyCode::Char('c'))));
        assert_eq!(app.view, View::Config);
        assert!(app.status.is_none()); // entering clears the stale status

        assert!(app.dispatch_key(key(KeyCode::Esc)));
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

        assert!(app.dispatch_key(key(KeyCode::Char('j'))));
        assert_eq!(app.config_state.selected(), Some(1));
        assert!(app.dispatch_key(key(KeyCode::Char('j')))); // clamps at the end
        assert_eq!(app.config_state.selected(), Some(1));
        assert!(app.dispatch_key(key(KeyCode::Down)));
        assert_eq!(app.config_state.selected(), Some(1));
        assert!(app.dispatch_key(key(KeyCode::Char('k'))));
        assert_eq!(app.config_state.selected(), Some(0));
        assert!(app.dispatch_key(key(KeyCode::Char('k')))); // clamps at the start
        assert_eq!(app.config_state.selected(), Some(0));

        // q exits the program from the config panel.
        assert!(!app.dispatch_key(key(KeyCode::Char('q'))));
    }

    #[test]
    fn config_enter_leave_handles_empty_list_and_stale_selection() {
        let mut app = App::new(Config::default());

        // Empty list: no selection, Esc returns to the dashboard.
        app.enter_config();
        assert_eq!(app.view, View::Config);
        assert_eq!(app.config_state.selected(), None);
        assert!(app.dispatch_key(key(KeyCode::Char('j'))));
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
        let _io = crate::config::config_io_guard();

        let mut app = App::new(Config::default());
        app.enter_config();

        // a opens a clean picker; i switches to the manual input, where
        // keys type and edit the path.
        assert!(app.dispatch_key(key(KeyCode::Char('a'))));
        assert!(app.show_config_picker);
        assert!(!app.show_config_input);
        assert!(app.dispatch_key(key(KeyCode::Char('i'))));
        assert!(!app.show_config_picker);
        assert!(app.show_config_input);
        assert!(app.config_input.is_empty());
        for c in "ab".chars() {
            assert!(app.dispatch_key(key(KeyCode::Char(c))));
        }
        assert_eq!(app.config_input, "ab");
        assert!(app.dispatch_key(key(KeyCode::Backspace)));
        assert_eq!(app.config_input, "a");
        // Ctrl+char does not type into the input.
        let ctrl = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL);
        assert!(app.dispatch_key(ctrl));
        assert_eq!(app.config_input, "a");

        // Empty path: error, input stays open.
        app.cancel_config_add();
        app.start_config_manual();
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
        assert!(app.dispatch_key(key(KeyCode::Esc)));
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
        app.dispatch_key(key(KeyCode::Char('a')));
        app.dispatch_key(key(KeyCode::Char('i')));
        for c in dir.display().to_string().chars() {
            app.dispatch_key(key(KeyCode::Char(c)));
        }
        assert!(app.dispatch_key(key(KeyCode::Enter)));

        // Input closed and cleared, list updated, new entry selected.
        assert!(!app.show_config_input);
        assert!(app.config_input.is_empty());
        assert_eq!(app.config.scan_dirs, vec![canon(&dir)]);
        assert_eq!(app.config_state.selected(), Some(0));

        // Persisted to the config file (restart would load the same dirs).
        let loaded = Config::load().expect("load saved config");
        assert_eq!(loaded.scan_dirs, vec![canon(&dir)]);

        // Auto-rescan discovered the repository under the new scan dir.
        assert_eq!(app.repos.len(), 1);
        assert_eq!(app.repos[0].path, canon(&dir));

        // Success message survives the refresh (which clears the status).
        let st = app.status.as_ref().unwrap();
        assert!(!st.is_error, "{}", st.message);
        assert_eq!(st.message, format!("已添加扫描目录: {}", dir.display()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_add_rejects_equivalent_spelling_of_existing_dir() {
        let _io = crate::config::config_io_guard();

        let base = temp_dir("config-canon");
        let dir = base.join("alpha");
        std::fs::create_dir_all(&dir).unwrap();

        let mut app = App::new(Config::default());
        app.enter_config();
        app.start_config_manual();
        app.config_input = dir.display().to_string();
        app.submit_config_add();
        let st = app.status.as_ref().unwrap();
        assert!(!st.is_error, "{}", st.message);
        assert_eq!(app.config.scan_dirs, vec![canon(&dir)]);

        // Reaching the same directory through `..` traversal canonicalizes
        // to the stored entry, so it is a duplicate: the list is untouched.
        app.start_config_manual();
        app.config_input = base.join("alpha/../alpha").display().to_string();
        app.submit_config_add();
        let st = app.status.as_ref().unwrap();
        assert!(st.is_error);
        assert_eq!(st.message, "目录已在列表中");
        assert_eq!(app.config.scan_dirs, vec![canon(&dir)]);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn config_add_rejects_symlink_alias_of_existing_dir() {
        let _io = crate::config::config_io_guard();

        let base = temp_dir("config-symlink");
        let dir = base.join("alpha");
        std::fs::create_dir_all(&dir).unwrap();
        let link = base.join("alias");
        std::os::unix::fs::symlink(&dir, &link).unwrap();

        let mut app = App::new(Config::default());
        app.enter_config();
        app.start_config_manual();
        app.config_input = dir.display().to_string();
        app.submit_config_add();
        let st = app.status.as_ref().unwrap();
        assert!(!st.is_error, "{}", st.message);
        assert_eq!(app.config.scan_dirs, vec![canon(&dir)]);

        // The symlink names the same directory after canonicalization.
        app.start_config_manual();
        app.config_input = link.display().to_string();
        app.submit_config_add();
        let st = app.status.as_ref().unwrap();
        assert!(st.is_error);
        assert_eq!(st.message, "目录已在列表中");
        assert_eq!(app.config.scan_dirs, vec![canon(&dir)]);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn config_add_canonicalizes_relative_path_and_dedupes_absolute_spelling() {
        let _io = crate::config::config_io_guard();

        // Sits under the crate root, which is the test process cwd, so the
        // relative spelling below resolves without depending on /tmp.
        let name = format!("target/gitpulse-rel-{}", std::process::id());
        let dir = std::env::current_dir().unwrap().join(&name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut app = App::new(Config::default());
        app.enter_config();
        app.start_config_manual();
        app.config_input = name.clone();
        app.submit_config_add();
        let st = app.status.as_ref().unwrap();
        assert!(!st.is_error, "{}", st.message);
        // Stored as the canonical absolute path: no cwd drift on reload.
        assert_eq!(app.config.scan_dirs, vec![canon(&dir)]);

        // The absolute spelling of the same directory now maps onto that
        // canonical entry.
        app.start_config_manual();
        app.config_input = dir.display().to_string();
        app.submit_config_add();
        let st = app.status.as_ref().unwrap();
        assert!(st.is_error);
        assert_eq!(st.message, "目录已在列表中");
        assert_eq!(app.config.scan_dirs, vec![canon(&dir)]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---------- config add: interactive directory picker ----------

    /// Temp base directory containing two empty subdirectories (`alpha`,
    /// `beta`) for the picker to browse.
    fn picker_base(tag: &str) -> std::path::PathBuf {
        let base = temp_dir(tag);
        std::fs::create_dir_all(base.join("alpha")).unwrap();
        std::fs::create_dir_all(base.join("beta")).unwrap();
        base
    }

    #[test]
    fn read_picker_entries_lists_only_directories_sorted() {
        let base = temp_dir("picker-list");
        std::fs::create_dir_all(base.join("b")).unwrap();
        std::fs::create_dir_all(base.join("a")).unwrap();
        std::fs::write(base.join("file.txt"), "x").unwrap();
        let entries = read_picker_entries(&base).unwrap();
        assert_eq!(entries, vec![base.join("a"), base.join("b")]);
        // A missing directory is an error the picker surfaces (not an
        // empty listing that looks like success).
        assert!(read_picker_entries(&base.join("nope")).is_err());
    }

    #[test]
    fn config_add_opens_picker_at_scan_parent_and_navigates() {
        let _io = crate::config::config_io_guard();

        let base = picker_base("picker-open");
        let scan = base.join("alpha");

        let mut app = App::new(Config::default());
        app.config.scan_dirs = vec![scan.clone()];
        app.enter_config();

        // a opens the picker (not the manual input) at the parent of the
        // first scan dir, listing its subdirectories.
        assert!(app.dispatch_key(key(KeyCode::Char('a'))));
        assert!(app.show_config_picker);
        assert!(!app.show_config_input);
        assert_eq!(app.picker_dir, base);
        assert_eq!(
            app.picker_entries,
            vec![base.join("alpha"), base.join("beta")]
        );
        assert_eq!(app.picker_state.selected(), Some(0));

        // j/k/方向键 move across the virtual row + entries, clamping at
        // both ends.
        assert!(app.dispatch_key(key(KeyCode::Char('k'))));
        assert_eq!(app.picker_state.selected(), Some(0));
        assert!(app.dispatch_key(key(KeyCode::Char('j'))));
        assert_eq!(app.picker_state.selected(), Some(1));
        assert!(app.dispatch_key(key(KeyCode::Down)));
        assert_eq!(app.picker_state.selected(), Some(2));
        assert!(app.dispatch_key(key(KeyCode::Down)));
        assert_eq!(app.picker_state.selected(), Some(2));
        assert!(app.dispatch_key(key(KeyCode::Char('k'))));
        assert_eq!(app.picker_state.selected(), Some(1));

        // l descends into the highlighted subdirectory; selection resets
        // to the virtual row of the new listing.
        assert!(app.dispatch_key(key(KeyCode::Char('l'))));
        assert_eq!(app.picker_dir, scan);
        assert!(app.picker_entries.is_empty());
        assert_eq!(app.picker_state.selected(), Some(0));

        // h / 方向左 return to the parent directory.
        assert!(app.dispatch_key(key(KeyCode::Char('h'))));
        assert_eq!(app.picker_dir, base);
        assert!(app.dispatch_key(key(KeyCode::Backspace)));
        assert_eq!(app.picker_dir, std::env::temp_dir());
        // (Up-at-root is covered by config_picker_at_root_reports_no_parent.)

        // Esc cancels: overlay closed, browse state cleared, still in panel.
        assert!(app.dispatch_key(key(KeyCode::Esc)));
        assert!(!app.show_config_picker);
        assert!(app.picker_entries.is_empty());
        assert!(app.config_input.is_empty());
        assert_eq!(app.view, View::Config);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn config_picker_falls_back_to_home_without_scan_dirs() {
        let _io = crate::config::config_io_guard();

        let mut app = App::new(Config::default());
        app.enter_config();
        assert!(app.dispatch_key(key(KeyCode::Char('a'))));
        let home = crate::config::home_dir();
        let expected = if std::fs::read_dir(&home).is_ok() {
            home
        } else {
            std::path::PathBuf::from("/")
        };
        assert_eq!(app.picker_dir, expected);
        assert_eq!(app.picker_state.selected(), Some(0));
    }

    #[test]
    fn config_picker_at_root_reports_no_parent() {
        let mut app = App::new(Config::default());
        app.enter_config();
        app.show_config_picker = true;
        app.picker_dir = std::path::PathBuf::from("/");
        app.picker_state.select(Some(0));
        app.set_error("stale".to_string());

        // Up keys at the root stay put and report why (info, not error).
        for code in [KeyCode::Char('h'), KeyCode::Left, KeyCode::Backspace] {
            assert!(app.dispatch_key(key(code)));
            assert_eq!(app.picker_dir, std::path::PathBuf::from("/"));
            let st = app.status.as_ref().unwrap();
            assert!(!st.is_error, "{code:?}: {}", st.message);
            assert!(st.message.contains("根目录"), "{code:?}: {}", st.message);
        }
    }

    #[test]
    fn config_picker_descend_failure_keeps_listing_and_reports_error() {
        let base = temp_dir("picker-denied");
        let child = base.join("child");
        std::fs::create_dir_all(&child).unwrap();

        let mut app = App::new(Config::default());
        app.enter_config();
        app.show_config_picker = true;
        app.picker_dir = base.clone();
        app.picker_entries = vec![child.clone()];
        app.picker_state.select(Some(1));
        // The child disappears between listing and descend: read fails.
        std::fs::remove_dir_all(&child).unwrap();

        assert!(app.dispatch_key(key(KeyCode::Char('l'))));
        assert_eq!(
            app.picker_dir, base,
            "failed descend must keep the previous listing"
        );
        assert_eq!(app.picker_entries, vec![child]);
        assert_eq!(app.picker_state.selected(), Some(1));
        let st = app.status.as_ref().unwrap();
        assert!(st.is_error);
        assert!(st.message.contains("无法读取目录"), "{}", st.message);
    }

    #[test]
    fn config_picker_duplicate_selection_reports_error_and_stays_open() {
        let _io = crate::config::config_io_guard();

        let base = picker_base("picker-dup");
        let taken = base.join("alpha");

        let mut app = App::new(Config::default());
        app.config.scan_dirs = vec![taken.clone()];
        app.enter_config();
        assert!(app.dispatch_key(key(KeyCode::Char('a'))));
        assert_eq!(app.picker_dir, base);

        // Highlight `alpha` (row 1) and submit: duplicate is rejected by
        // the existing validation, without writing to the list.
        assert!(app.dispatch_key(key(KeyCode::Char('j'))));
        assert_eq!(app.picker_state.selected(), Some(1));
        assert!(app.dispatch_key(key(KeyCode::Enter)));
        let st = app.status.as_ref().unwrap();
        assert!(st.is_error);
        assert_eq!(st.message, "目录已在列表中");
        assert!(app.show_config_picker, "picker must stay open: {st:?}");
        assert_eq!(app.config.scan_dirs, vec![taken]);

        // Esc cancels the picker and clears any residual typed path.
        assert!(app.dispatch_key(key(KeyCode::Esc)));
        assert!(!app.show_config_picker);
        assert!(app.config_input.is_empty());
        assert_eq!(app.view, View::Config);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn config_picker_selection_saves_validates_and_refreshes() {
        // Holds the I/O guard: the saves below write the redirected config.
        let _io = crate::config::config_io_guard();

        let base = picker_base("picker-save");
        let mut app = App::new(Config::default());
        app.enter_config();
        app.show_config_picker = true;
        app.picker_dir = base.clone();
        app.picker_entries = vec![base.join("alpha"), base.join("beta")];
        app.picker_state.select(Some(2));

        // Enter on a subdirectory row adds that directory through the
        // shared submit path (save + rescan + success message).
        assert!(app.dispatch_key(key(KeyCode::Enter)));
        assert!(!app.show_config_picker, "success closes the picker");
        assert!(app.config_input.is_empty());
        assert_eq!(app.config.scan_dirs, vec![canon(&base.join("beta"))]);
        assert_eq!(app.config_state.selected(), Some(0));
        let loaded = Config::load().expect("load saved config");
        assert_eq!(loaded.scan_dirs, vec![canon(&base.join("beta"))]);
        let st = app.status.as_ref().unwrap();
        assert!(!st.is_error, "{}", st.message);
        assert_eq!(
            st.message,
            format!("已添加扫描目录: {}", base.join("beta").display())
        );

        // Enter on row 0 adds the current directory itself.
        app.show_config_picker = true;
        app.picker_dir = base.clone();
        app.picker_entries.clear();
        app.picker_state.select(Some(0));
        assert!(app.dispatch_key(key(KeyCode::Enter)));
        assert!(!app.show_config_picker);
        assert_eq!(
            app.config.scan_dirs,
            vec![canon(&base.join("beta")), canon(&base)]
        );
        let st = app.status.as_ref().unwrap();
        assert!(!st.is_error, "{}", st.message);
        assert_eq!(st.message, format!("已添加扫描目录: {}", base.display()));

        let _ = std::fs::remove_dir_all(&base);
    }

    /// A directory name that is not valid UTF-8 is readable in the picker, so
    /// submitting it must reach the filesystem checks unchanged: the old
    /// `display().to_string()` round trip replaced the bytes with U+FFFD and
    /// reported `目录不存在` for a directory that exists.
    ///
    /// The duplicate check needs no write, so it locks the lossless picker →
    /// [`App::add_scan_dir`] hand-off on its own. A fresh submission is then
    /// checked for the same symptom; whether such a path can be persisted at
    /// all is up to the config layer (TOML strings are UTF-8, so serde rejects
    /// a non-UTF-8 `PathBuf` and the save rolls the entry back).
    #[cfg(unix)]
    #[test]
    fn config_picker_submits_non_utf8_dir_losslessly() {
        use std::os::unix::ffi::OsStrExt;

        // Holds the I/O guard: the submission below reaches `Config::save`.
        let _io = crate::config::config_io_guard();

        let base = temp_dir("picker-nonutf8");
        let dir = base.join(std::ffi::OsStr::from_bytes(b"caf\xe9"));
        std::fs::create_dir(&dir).unwrap();

        let mut app = App::new(Config::default());
        app.config.scan_dirs = vec![dir.clone()];
        app.enter_config();
        app.show_config_picker = true;
        app.picker_dir = base.clone();
        app.picker_entries = vec![dir.clone()];
        app.picker_state.select(Some(1));

        // A mangled spelling can never equal the real path: hitting the
        // duplicate branch proves the PathBuf arrived intact.
        assert!(app.dispatch_key(key(KeyCode::Enter)));
        let st = app.status.as_ref().unwrap();
        assert_eq!(st.message, "目录已在列表中");
        assert!(
            app.show_config_picker,
            "failed submit keeps the picker open"
        );
        assert_eq!(app.config.scan_dirs, vec![dir.clone()]);

        // The same directory submitted fresh is never reported missing or
        // as a non-directory: validation ran against the real entry.
        app.config.scan_dirs.clear();
        app.add_scan_dir(dir.clone());
        let st = app.status.as_ref().unwrap();
        assert!(!st.message.starts_with("目录不存在"), "{}", st.message);
        assert!(!st.message.starts_with("不是目录"), "{}", st.message);
        assert!(!st.message.contains('\u{FFFD}'), "{}", st.message);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn config_i_opens_manual_input_from_list_and_picker() {
        let _io = crate::config::config_io_guard();

        let base = picker_base("picker-i");
        let mut app = App::new(Config::default());
        app.config.scan_dirs = vec![base.join("alpha")];
        app.enter_config();

        // From the list, i opens the manual input directly.
        assert!(app.dispatch_key(key(KeyCode::Char('i'))));
        assert!(app.show_config_input);
        assert!(!app.show_config_picker);
        assert!(app.dispatch_key(key(KeyCode::Esc)));
        assert!(!app.show_config_input);

        // From the picker, i switches over to manual entry.
        assert!(app.dispatch_key(key(KeyCode::Char('a'))));
        assert!(app.show_config_picker);
        assert!(app.dispatch_key(key(KeyCode::Char('i'))));
        assert!(!app.show_config_picker);
        assert!(app.show_config_input);
        assert!(app.dispatch_key(key(KeyCode::Char('/'))));
        assert_eq!(app.config_input, "/");

        let _ = std::fs::remove_dir_all(&base);
    }

    /// A failed `Config::save` must leave memory and disk consistent: the
    /// pushed entry is popped, the removed entry re-inserted, and the error
    /// reported (convention 5 rollback branches).
    ///
    /// The save is forced to fail by replacing the shared config *file* with
    /// a directory of the same name, so `fs::write` hits `EISDIR`; the
    /// original bytes are restored afterwards to prove the failed operations
    /// never reached the disk. Root ignores chmod, hence the directory trick.
    #[test]
    fn config_save_failure_rolls_back_add_and_delete() {
        use std::fs;

        /// Restores the shared config file to `seeded` bytes on drop, so a
        /// failing assertion after the file was replaced by a directory still
        /// leaves the redirect writable for the lock-holding tests that run
        /// next. Every step ignores errors: `Drop` must not panic.
        struct ConfigFileGuard {
            path: std::path::PathBuf,
            seeded: Option<Vec<u8>>,
        }

        impl Drop for ConfigFileGuard {
            fn drop(&mut self) {
                if self.path.is_dir() {
                    let _ = std::fs::remove_dir_all(&self.path);
                }
                if let Some(bytes) = self.seeded.take() {
                    let _ = std::fs::write(&self.path, bytes);
                }
            }
        }

        // Holds the I/O guard: the config path below is the shared redirect.
        let _io = crate::config::config_io_guard();
        let cfg_path = crate::config::config_path();

        let dir_a = temp_dir("save-fail-a");
        let dir_b = temp_dir("save-fail-b");
        let mut app = App::new(Config::default());
        app.config.scan_dirs = vec![dir_a.clone()];

        // Seed the disk, then snapshot the bytes the rollback must preserve.
        app.config.save().expect("seed config");
        let seeded = fs::read(&cfg_path).expect("read seeded config");

        // Armed before the break so a panic anywhere below still restores the
        // seeded file (idempotent with the explicit restore at the end).
        let _restore = ConfigFileGuard {
            path: cfg_path.clone(),
            seeded: Some(seeded.clone()),
        };

        // Break the save: config.toml becomes a directory.
        fs::remove_file(&cfg_path).expect("drop seeded file");
        fs::create_dir_all(&cfg_path).expect("block config file");

        // Add: the push is rolled back, the error surfaces, and the input
        // stays open with the typed path for a retry (convention 9).
        app.enter_config();
        app.start_config_manual();
        app.config_input = dir_b.display().to_string();
        app.submit_config_add();
        assert_eq!(app.config.scan_dirs, vec![dir_a.clone()], "add rolled back");
        assert!(app.show_config_input, "input kept open for retry");
        assert_eq!(app.config_input, dir_b.display().to_string());
        let st = app.status.as_ref().expect("save error reported");
        assert!(st.is_error, "{}", st.message);
        assert!(
            st.message.starts_with("保存配置失败"),
            "unexpected message: {}",
            st.message
        );

        // Delete: the removal is rolled back and the entry survives.
        app.config_state.select(Some(0));
        app.start_config_delete();
        app.confirm_config_delete();
        assert_eq!(
            app.config.scan_dirs,
            vec![dir_a.clone()],
            "delete rolled back"
        );
        let st = app.status.as_ref().expect("save error reported");
        assert!(st.is_error, "{}", st.message);
        assert!(
            st.message.starts_with("保存配置失败"),
            "unexpected message: {}",
            st.message
        );

        // Restore the seeded file: neither failed operation touched the disk.
        fs::remove_dir_all(&cfg_path).expect("unblock config file");
        fs::write(&cfg_path, &seeded).expect("restore seeded config");
        let loaded = Config::load().expect("load restored config");
        assert_eq!(loaded, app.config, "disk matches memory after rollbacks");

        let _ = fs::remove_dir_all(&dir_a);
        let _ = fs::remove_dir_all(&dir_b);
    }

    // ---------- B: branch panel ----------

    #[test]
    fn enter_branches_loads_list_and_head_selection() {
        let dir = temp_dir("enter-branches");
        init_repo_with_commit(&dir);
        git::create_branch(&dir, "feature").unwrap();

        let mut app = app_with_repo_at(&dir);
        app.dashboard_state.select(Some(0));
        assert!(app.dispatch_key(key(KeyCode::Char('b'))));
        assert_eq!(app.view, View::Branches);
        assert_eq!(app.branches.len(), 2);
        let sel = app.branch_state.selected().unwrap();
        assert!(app.branches[sel].is_head);
        assert_eq!(app.branch_repo, Some(0));
        assert_eq!(app.branches_from, View::Dashboard);

        assert!(app.dispatch_key(key(KeyCode::Esc)));
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
        assert!(app.dispatch_key(key(KeyCode::Char('j'))));
        assert_eq!(app.branch_state.selected(), Some(head_idx + 1));
        assert!(app.dispatch_key(key(KeyCode::Char('k'))));
        assert_eq!(app.branch_state.selected(), Some(head_idx));

        // n opens the create input; Esc closes it but stays in the panel.
        assert!(app.dispatch_key(key(KeyCode::Char('n'))));
        assert!(app.show_branch_input);
        assert!(app.dispatch_key(key(KeyCode::Esc)));
        assert!(!app.show_branch_input);
        assert_eq!(app.view, View::Branches);

        // q exits from the list; Esc leaves the panel back to the dashboard.
        assert!(!app.dispatch_key(key(KeyCode::Char('q'))));
        assert!(app.dispatch_key(key(KeyCode::Esc)));
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
        assert!(app.dispatch_key(key(KeyCode::Char('n'))));
        for c in "feature".chars() {
            assert!(app.dispatch_key(key(KeyCode::Char(c))));
        }
        assert_eq!(app.branch_input, "feature");
        assert!(app.dispatch_key(key(KeyCode::Backspace)));
        assert_eq!(app.branch_input, "featur");
        assert!(app.dispatch_key(key(KeyCode::Char('e'))));
        assert!(app.dispatch_key(key(KeyCode::Enter)));

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
        assert!(app.dispatch_key(key(KeyCode::Char('d'))));
        assert!(app.show_branch_delete_confirm);
        assert!(app.dispatch_key(key(KeyCode::Esc)));
        assert!(!app.show_branch_delete_confirm);
        assert_eq!(app.view, View::Branches);

        // ...d + n cancels as well...
        assert!(app.dispatch_key(key(KeyCode::Char('d'))));
        assert!(app.dispatch_key(key(KeyCode::Char('n'))));
        assert!(!app.show_branch_delete_confirm);

        // ...and d + Y on the head branch is refused: branch kept, error shown.
        assert!(app.dispatch_key(key(KeyCode::Char('d'))));
        assert!(app.dispatch_key(key(KeyCode::Char('Y'))));
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
        let (rtx, rrx) = mpsc::unbounded_channel::<Result<(), String>>();
        app.remote_rx = Some(rrx);
        app.remote_op = "push";
        app.start_remote("pull");
        assert_eq!(app.remote_op, "push");
        app.remote_rx = None;
        drop(rtx);

        // Valid start: arms the receiver, initial running status, clears the
        // status.
        app.status = Some(StatusLine {
            message: "old".to_string(),
            is_error: true,
        });
        app.start_remote("pull");
        assert!(app.remote_rx.is_some());
        assert_eq!(app.remote_op, "pull");
        assert!(matches!(app.remote_status, Some(RemoteStatus::Running(_))));
        assert!(app.status.is_none());
    }

    #[test]
    fn poll_remote_drains_done_and_disconnect() {
        let mut app = App::new(Config::default());

        // No receiver: no-op.
        app.poll_remote();

        // Done(Err) surfaces the reason and closes the receiver.
        let (tx, rx) = mpsc::unbounded_channel();
        app.remote_rx = Some(rx);
        tx.send(Err("boom".to_string())).unwrap();
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
        tx.send(Ok(())).unwrap();
        app.poll_remote();
        assert_eq!(app.remote_status, Some(RemoteStatus::Success));
        assert!(app.remote_rx.is_none());
        let st = app.status.as_ref().expect("refresh failure must surface");
        assert!(st.is_error, "{}", st.message);
        let _ = std::fs::remove_dir_all(&broken);

        // Sender dropped without a result: explicit disconnect error.
        let mut app = App::new(Config::default());
        let (tx, rx) = mpsc::unbounded_channel::<Result<(), String>>();
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
        assert!(app.dispatch_key(key(KeyCode::Char('d'))));
        assert!(app.show_config_delete_confirm);
        assert!(!app.show_config_input);
        assert_eq!(app.config_state.selected(), Some(1));

        // Other keys are swallowed while the dialog is open.
        assert!(app.dispatch_key(key(KeyCode::Char('j'))));
        assert!(app.show_config_delete_confirm);
        assert_eq!(app.config_state.selected(), Some(1));

        // Esc (before y) cancels: nothing removed, still in the panel.
        assert!(app.dispatch_key(key(KeyCode::Esc)));
        assert!(!app.show_config_delete_confirm);
        assert_eq!(app.config.scan_dirs.len(), 2);
        assert_eq!(app.view, View::Config);

        // n also cancels.
        assert!(app.dispatch_key(key(KeyCode::Char('d'))));
        assert!(app.show_config_delete_confirm);
        assert!(app.dispatch_key(key(KeyCode::Char('n'))));
        assert!(!app.show_config_delete_confirm);
        assert_eq!(app.config.scan_dirs.len(), 2);
    }

    #[test]
    fn config_delete_on_empty_list_reports_error() {
        let mut app = App::new(Config::default());
        app.enter_config();

        assert!(app.dispatch_key(key(KeyCode::Char('d'))));
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
        assert!(app.dispatch_key(key(KeyCode::Char('d'))));
        assert!(app.dispatch_key(key(KeyCode::Char('y'))));
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

    // ---------- history view ----------

    /// Append a commit on top of HEAD at an explicit `seconds` timestamp, so
    /// the newest-first history order never depends on wall-clock ties.
    fn commit_at(dir: &std::path::Path, message: &str, seconds: i64) {
        let repo = git2::Repository::open(dir).unwrap();
        std::fs::write(dir.join("a.txt"), format!("{message}\n")).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("a.txt")).unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        let sig = git2::Signature::new(
            "GitPulse Test",
            "test@gitpulse.local",
            &git2::Time::new(seconds, 0),
        )
        .unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])
            .unwrap();
    }

    /// Repo fixture with two commits whose order is fixed by time.
    fn app_with_two_commit_repo(tag: &str) -> (std::path::PathBuf, App) {
        let dir = temp_dir(tag);
        init_repo_with_commit(&dir);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        commit_at(&dir, "second commit", now + 600);
        let app = app_with_repo_at(&dir);
        (dir, app)
    }

    /// Render one frame through the view dispatcher and return the
    /// whitespace-stripped buffer text (wide CJK glyphs leave padding cells).
    fn render_text(app: &mut App) -> String {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| crate::ui::render(f, app)).unwrap();
        let raw: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        raw.chars().filter(|c| !c.is_whitespace()).collect()
    }

    /// Enter the repo view and then the commit-history view.
    fn enter_history(app: &mut App) {
        app.dashboard_state.select(Some(0));
        app.enter_repo();
        assert!(app.dispatch_key(key(KeyCode::Char('h'))));
    }

    #[test]
    fn repo_h_enters_history_and_loads_commits() {
        let dir = temp_dir("history-enter");
        init_repo_with_commit(&dir);

        let mut app = app_with_repo_at(&dir);
        enter_history(&mut app);

        assert_eq!(app.view, View::History);
        assert_eq!(app.history.commits.len(), 1);
        assert_eq!(app.history.commits[0].summary, "v1");
        assert_eq!(app.history.graph.len(), 1);
        assert_eq!(app.history.graph[0][0], '●');
        // First commit selected, cursor and detail preloaded, no error.
        assert_eq!(app.history.selected, 0);
        assert_eq!(app.history.list_state.selected(), Some(0));
        assert_eq!(app.history.detail.as_ref().unwrap().message, "v1");
        assert!(app.status.is_none());

        // h without an open repository never leaves the repo view.
        app.current_repo = None;
        app.view = View::Repo;
        assert!(app.dispatch_key(key(KeyCode::Char('h'))));
        assert_eq!(app.view, View::Repo);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn history_j_k_move_selection_reload_detail_and_stop_at_edges() {
        let (dir, mut app) = app_with_two_commit_repo("history-move");
        enter_history(&mut app);
        assert_eq!(app.history.commits.len(), 2);

        let newest = app.history.commits[0].summary.clone();
        let older = app.history.commits[1].summary.clone();
        assert_eq!(newest, "second commit");
        assert_eq!(older, "v1");
        assert_eq!(app.history.detail.as_ref().unwrap().message, newest);

        // j / Down move one row and reload the detail for that row.
        assert!(app.dispatch_key(key(KeyCode::Char('j'))));
        assert_eq!(app.history.selected, 1);
        assert_eq!(app.history.list_state.selected(), Some(1));
        assert_eq!(app.history.detail.as_ref().unwrap().message, older);
        assert!(app.dispatch_key(key(KeyCode::Down)));
        assert_eq!(app.history.selected, 1, "j stops at the last row");
        assert_eq!(app.history.detail.as_ref().unwrap().message, older);

        // k / Up move back and stop at the first row.
        assert!(app.dispatch_key(key(KeyCode::Char('k'))));
        assert_eq!(app.history.selected, 0);
        assert_eq!(app.history.list_state.selected(), Some(0));
        assert_eq!(app.history.detail.as_ref().unwrap().message, newest);
        assert!(app.dispatch_key(key(KeyCode::Up)));
        assert_eq!(app.history.selected, 0, "k stops at the first row");
        assert_eq!(app.history.detail.as_ref().unwrap().message, newest);
        assert!(app.status.is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A press that cannot move the cursor must not pay for the detail
    /// reload: it re-opens the repository and re-diffs the whole tree, and
    /// a clamped `j` / `k` would otherwise clear the status line on every
    /// repeat.
    #[test]
    fn history_clamped_move_skips_detail_reload() {
        let (dir, mut app) = app_with_two_commit_repo("history-clamp");
        enter_history(&mut app);
        assert_eq!(app.history.commits.len(), 2);

        // Move to the last row, then plant a stale error on the status line.
        assert!(app.dispatch_key(key(KeyCode::Char('j'))));
        assert_eq!(app.history.selected, 1);
        app.set_error("stale".to_string());

        // `j` clamps there: no reload runs, so the error survives.
        assert!(app.dispatch_key(key(KeyCode::Char('j'))));
        assert_eq!(app.history.selected, 1, "j stops at the last row");
        assert_eq!(
            app.status.as_ref().map(|s| s.message.as_str()),
            Some("stale"),
            "a clamped press must not reload the detail"
        );

        // `k` really moves, so the reload runs and clears the status line.
        assert!(app.dispatch_key(key(KeyCode::Char('k'))));
        assert_eq!(app.history.selected, 0);
        assert!(app.status.is_none());

        // Same at the top edge: `k` clamps and keeps the planted error.
        app.set_error("stale".to_string());
        assert!(app.dispatch_key(key(KeyCode::Char('k'))));
        assert_eq!(app.history.selected, 0, "k stops at the first row");
        assert_eq!(
            app.status.as_ref().map(|s| s.message.as_str()),
            Some("stale"),
            "a clamped press must not reload the detail"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn history_esc_returns_to_repo_and_q_exits_app() {
        let dir = temp_dir("history-exit");
        init_repo_with_commit(&dir);

        let mut app = app_with_repo_at(&dir);
        enter_history(&mut app);
        assert_eq!(app.view, View::History);

        // `q` is the app-wide exit key and never pops the view stack.
        assert!(!app.dispatch_key(key(KeyCode::Char('q'))));

        assert!(app.dispatch_key(key(KeyCode::Char('h'))));
        assert_eq!(app.view, View::History);
        assert!(app.dispatch_key(key(KeyCode::Esc)));
        assert_eq!(app.view, View::Repo);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn history_empty_repo_enters_without_panic_and_renders_empty_state() {
        let dir = temp_dir("history-empty");
        git2::Repository::init(&dir).unwrap();

        let mut app = app_with_repo_at(&dir);
        enter_history(&mut app);
        assert_eq!(app.view, View::History);
        assert!(app.history.commits.is_empty());
        assert!(app.history.graph.is_empty());
        assert!(app.history.detail.is_none());
        assert_eq!(app.history.selected, 0);
        assert_eq!(app.history.list_state.selected(), None);

        // Navigation on an empty list is a no-op and never errors.
        assert!(app.dispatch_key(key(KeyCode::Char('j'))));
        assert!(app.dispatch_key(key(KeyCode::Char('k'))));
        assert!(app.status.is_none());

        let text = render_text(&mut app);
        assert!(text.contains("暂无提交记录"), "missing empty state: {text}");
        assert!(
            text.contains("选择一个提交查看详情"),
            "missing placeholder: {text}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A detail reload against a repository that vanished reports the reason
    /// on the status line instead of panicking, and the view stays put.
    #[test]
    fn history_detail_load_error_reports_on_status_line() {
        // Two commits so `j` really moves the selection: a clamped press
        // (single-row history) deliberately skips the reload and would never
        // reach the failing lookup this test exercises.
        let (dir, mut app) = app_with_two_commit_repo("history-detail-error");
        enter_history(&mut app);
        assert!(app.history.detail.is_some());

        std::fs::remove_dir_all(&dir).unwrap();
        assert!(app.dispatch_key(key(KeyCode::Char('j'))));
        assert_eq!(app.view, View::History);
        assert!(app.history.detail.is_none());
        let st = app.status.as_ref().unwrap();
        assert!(st.is_error);
        assert!(
            st.message.contains("加载提交详情失败"),
            "unexpected message: {}",
            st.message
        );
    }

    // ---------- tags panel ----------

    /// Repo fixture with the given tag names created at HEAD.
    fn app_with_tagged_repo(tag: &str, names: &[&str]) -> (std::path::PathBuf, App) {
        let dir = temp_dir(tag);
        init_repo_with_commit(&dir);
        for name in names {
            git::create_tag(&dir, name).unwrap();
        }
        let app = app_with_repo_at(&dir);
        (dir, app)
    }

    /// Enter the repo view and then the tag panel.
    fn enter_tags(app: &mut App) {
        app.dashboard_state.select(Some(0));
        app.enter_repo();
        assert!(app.dispatch_key(key(KeyCode::Char('t'))));
        assert_eq!(app.view, View::Tags);
    }

    #[test]
    fn repo_t_enters_tags_and_loads_tag_list() {
        let (dir, mut app) = app_with_tagged_repo("tags-enter", &["v1.0"]);
        app.dashboard_state.select(Some(0));
        app.enter_repo();
        assert_eq!(app.view, View::Repo);
        app.set_error("stale".to_string());
        // Stale push feedback from the repo view must not outrank the panel.
        app.remote_status = Some(RemoteStatus::Success);

        assert!(app.dispatch_key(key(KeyCode::Char('t'))));
        assert_eq!(app.view, View::Tags);
        assert_eq!(app.tags_state.tags.len(), 1);
        assert_eq!(app.tags_state.tags[0].name, "v1.0");
        assert_eq!(app.tags_state.selected, 0);
        assert_eq!(app.tags_state.list_state.selected(), Some(0));
        assert!(app.tags_state.modal.is_none());
        assert!(app.status.is_none()); // entering clears the stale status
        assert!(app.remote_status.is_none()); // …and the stale push result

        // t without an open repository never leaves the repo view.
        app.current_repo = None;
        app.view = View::Repo;
        assert!(app.dispatch_key(key(KeyCode::Char('t'))));
        assert_eq!(app.view, View::Repo);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tags_j_k_move_selection_and_stop_at_edges() {
        let (dir, mut app) = app_with_tagged_repo("tags-move", &["v1", "v2"]);
        enter_tags(&mut app);
        assert_eq!(app.tags_state.tags.len(), 2);

        // j / Down move one row and clamp at the last tag.
        assert!(app.dispatch_key(key(KeyCode::Char('j'))));
        assert_eq!(app.tags_state.selected, 1);
        assert_eq!(app.tags_state.list_state.selected(), Some(1));
        assert!(app.dispatch_key(key(KeyCode::Down)));
        assert_eq!(app.tags_state.selected, 1, "j stops at the last tag");

        // k / Up move back and clamp at the first tag.
        assert!(app.dispatch_key(key(KeyCode::Char('k'))));
        assert_eq!(app.tags_state.selected, 0);
        assert!(app.dispatch_key(key(KeyCode::Up)));
        assert_eq!(app.tags_state.selected, 0, "k stops at the first tag");
        assert!(app.status.is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tags_create_flow_submits_refreshes_and_reports_errors() {
        let (dir, mut app) = app_with_tagged_repo("tags-create", &[]);
        enter_tags(&mut app);
        assert!(app.tags_state.tags.is_empty());

        // c opens a clean input overlay.
        assert!(app.dispatch_key(key(KeyCode::Char('c'))));
        assert!(
            matches!(app.tags_state.modal, Some(TagsModal::Input { .. })),
            "create input must open"
        );

        // Empty name: red status, overlay kept open (convention 9).
        assert!(app.dispatch_key(key(KeyCode::Enter)));
        let st = app.status.as_ref().unwrap();
        assert!(st.is_error);
        assert!(st.message.contains("标签名不能为空"), "{}", st.message);
        assert!(app.tags_state.modal.is_some());

        // Type the name and submit: list refreshed, overlay closed, the new
        // tag selected, success on the status line.
        for c in "v1.0".chars() {
            assert!(app.dispatch_key(key(KeyCode::Char(c))));
        }
        assert!(app.dispatch_key(key(KeyCode::Enter)));
        assert!(app.tags_state.modal.is_none());
        assert!(app.tags_state.tags.iter().any(|t| t.name == "v1.0"));
        let sel = app.tags_state.selected;
        assert_eq!(app.tags_state.tags[sel].name, "v1.0");
        let st = app.status.as_ref().unwrap();
        assert!(!st.is_error, "{}", st.message);
        assert!(st.message.contains("已创建标签 v1.0"), "{}", st.message);

        // Duplicate name: red status with the git-side reason, the overlay
        // stays open carrying the typed text for a correction.
        assert!(app.dispatch_key(key(KeyCode::Char('c'))));
        for c in "v1.0".chars() {
            assert!(app.dispatch_key(key(KeyCode::Char(c))));
        }
        assert!(app.dispatch_key(key(KeyCode::Enter)));
        let st = app.status.as_ref().unwrap();
        assert!(st.is_error);
        assert!(st.message.contains("创建标签失败"), "{}", st.message);
        assert!(st.message.contains("已存在"), "{}", st.message);
        match &app.tags_state.modal {
            Some(TagsModal::Input { buffer }) => assert_eq!(buffer, "v1.0"),
            other => panic!("input overlay must stay open, got {other:?}"),
        }
        assert_eq!(app.tags_state.tags.len(), 1, "no tag was created twice");

        // Esc closes the stale overlay, a fresh `c` opens a clean one.
        assert!(app.dispatch_key(key(KeyCode::Esc)));
        assert!(app.tags_state.modal.is_none());

        // Invalid name: the status line carries the full git error chain,
        // including the underlying `invalid reference name` (H2).
        assert!(app.dispatch_key(key(KeyCode::Char('c'))));
        for c in "v1.0 beta".chars() {
            assert!(app.dispatch_key(key(KeyCode::Char(c))));
        }
        assert!(app.dispatch_key(key(KeyCode::Enter)));
        let st = app.status.as_ref().unwrap();
        assert!(st.is_error);
        assert!(st.message.contains("创建标签失败"), "{}", st.message);
        // The underlying libgit2 cause ("… reference name … is not valid")
        // must surface past the anyhow context (H2).
        assert!(st.message.contains("is not valid"), "{}", st.message);
        match &app.tags_state.modal {
            Some(TagsModal::Input { buffer }) => assert_eq!(buffer, "v1.0 beta"),
            other => panic!("input overlay must stay open, got {other:?}"),
        }
        assert_eq!(app.tags_state.tags.len(), 1, "no tag was created");

        // Esc cancels the overlay but stays in the panel.
        assert!(app.dispatch_key(key(KeyCode::Esc)));
        assert!(app.tags_state.modal.is_none());
        assert_eq!(app.view, View::Tags);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tags_delete_confirm_y_removes_and_n_cancels() {
        let (dir, mut app) = app_with_tagged_repo("tags-delete", &["drop", "keep"]);
        enter_tags(&mut app);
        assert_eq!(app.tags_state.tags.len(), 2);

        // `drop` sorts first, so it is the highlighted tag.
        assert_eq!(app.tags_state.selected_tag().unwrap().name, "drop");

        // d opens the confirmation naming the tag; n cancels it.
        assert!(app.dispatch_key(key(KeyCode::Char('d'))));
        assert_eq!(
            app.tags_state.modal,
            Some(TagsModal::Confirm {
                tag_to_delete: "drop".to_string()
            })
        );
        assert!(app.dispatch_key(key(KeyCode::Char('n'))));
        assert!(app.tags_state.modal.is_none());
        assert_eq!(app.tags_state.tags.len(), 2, "n must not delete");

        // d + Esc cancels the same way.
        assert!(app.dispatch_key(key(KeyCode::Char('d'))));
        assert!(app.dispatch_key(key(KeyCode::Esc)));
        assert!(app.tags_state.modal.is_none());
        assert_eq!(app.tags_state.tags.len(), 2);

        // d + y deletes and refreshes the list, reporting success.
        assert!(app.dispatch_key(key(KeyCode::Char('d'))));
        assert!(app.dispatch_key(key(KeyCode::Char('y'))));
        assert!(app.tags_state.modal.is_none());
        assert_eq!(app.tags_state.tags.len(), 1);
        assert_eq!(app.tags_state.tags[0].name, "keep");
        assert_eq!(app.tags_state.list_state.selected(), Some(0));
        let st = app.status.as_ref().unwrap();
        assert!(!st.is_error, "{}", st.message);
        assert!(st.message.contains("已删除标签 drop"), "{}", st.message);

        // Deleting the last tag empties the panel; `d` then reports why.
        assert!(app.dispatch_key(key(KeyCode::Char('d'))));
        assert!(app.dispatch_key(key(KeyCode::Char('y'))));
        assert!(app.tags_state.tags.is_empty());
        assert_eq!(app.tags_state.list_state.selected(), None);
        assert!(app.dispatch_key(key(KeyCode::Char('d'))));
        assert!(app.tags_state.modal.is_none());
        let st = app.status.as_ref().unwrap();
        assert!(st.is_error);
        assert!(st.message.contains("没有可删除的标签"), "{}", st.message);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// H1/V2: a tag whose name is not valid UTF-8 stays visible in the panel
    /// as an escaped display form, but that form is not the ref name — `d`
    /// and `p` must refuse with an explicit reason instead of opening a
    /// confirmation that would delete nothing or spawning a push whose
    /// refspec can never match.
    #[cfg(unix)]
    #[test]
    fn tags_non_utf8_name_refuses_delete_and_push_with_explicit_reason() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let dir = temp_dir("tags-nonutf8");
        init_repo_with_commit(&dir);
        {
            let repo = git2::Repository::open(&dir).unwrap();
            let head = repo.head().unwrap();
            let object = head.peel(git2::ObjectType::Commit).unwrap();
            git::create_tag(&dir, "utf8-tag").unwrap();
            // `git2` only accepts `&str` reference names, so the loose ref
            // file is written directly with a name that is not valid UTF-8.
            let ref_dir = repo.path().join("refs/tags");
            std::fs::create_dir_all(&ref_dir).unwrap();
            std::fs::write(
                ref_dir.join(OsStr::from_bytes(b"bad-\xFF-tag")),
                format!("{}\n", object.id()),
            )
            .unwrap();
        }

        let mut app = app_with_repo_at(&dir);
        enter_tags(&mut app);
        assert_eq!(app.tags_state.tags.len(), 2);
        // The escaped name sorts first (`"` precedes every letter), so the
        // highlighted tag is the non-UTF-8 one.
        assert_eq!(
            app.tags_state.selected_tag().unwrap().name,
            "\"bad-\\xFF-tag\""
        );

        // d refuses before the confirmation: explicit error, no overlay,
        // and nothing deleted.
        assert!(app.dispatch_key(key(KeyCode::Char('d'))));
        assert!(app.tags_state.modal.is_none(), "confirmation must not open");
        assert_eq!(app.tags_state.tags.len(), 2, "no tag was deleted");
        let st = app.status.as_ref().unwrap();
        assert!(st.is_error);
        assert!(
            st.message.contains("非 UTF-8 标签名暂不支持删除"),
            "{}",
            st.message
        );

        // p refuses the same way and never spawns a push task (a
        // `tokio::spawn` outside a runtime would panic).
        assert!(app.dispatch_key(key(KeyCode::Char('p'))));
        assert!(app.remote_rx.is_none());
        let st = app.status.as_ref().unwrap();
        assert!(st.is_error);
        assert!(
            st.message.contains("非 UTF-8 标签名暂不支持推送"),
            "{}",
            st.message
        );

        // The UTF-8 tag beside it still opens the confirmation normally.
        assert!(app.dispatch_key(key(KeyCode::Char('j'))));
        assert_eq!(app.tags_state.selected_tag().unwrap().name, "utf8-tag");
        assert!(app.dispatch_key(key(KeyCode::Char('d'))));
        assert_eq!(
            app.tags_state.modal,
            Some(TagsModal::Confirm {
                tag_to_delete: "utf8-tag".to_string()
            })
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tags_esc_closes_overlay_first_then_returns_to_repo_while_q_exits() {
        let (dir, mut app) = app_with_tagged_repo("tags-exit", &["v1"]);
        enter_tags(&mut app);

        // The create input is a focus trap: `q` is typed as text and Esc
        // cancels; neither leaves the panel from under the overlay.
        assert!(app.dispatch_key(key(KeyCode::Char('c'))));
        assert!(app.dispatch_key(key(KeyCode::Char('q'))));
        assert!(app.tags_state.modal.is_some(), "q must not leave the panel");
        assert_eq!(app.view, View::Tags);
        assert!(app.dispatch_key(key(KeyCode::Esc)));
        assert!(app.tags_state.modal.is_none());
        assert_eq!(app.view, View::Tags);

        // The delete confirmation swallows `q` (focus trap: only Enter/Esc/
        // Tab act), so Esc is the key that dismisses it.
        assert!(app.dispatch_key(key(KeyCode::Char('d'))));
        assert!(app.tags_state.modal.is_some());
        assert!(app.dispatch_key(key(KeyCode::Char('q'))));
        assert!(
            app.tags_state.modal.is_some(),
            "q is literal/no-op inside a modal, it must not dismiss it"
        );
        assert_eq!(app.view, View::Tags);
        assert!(app.dispatch_key(key(KeyCode::Esc)));
        assert!(app.tags_state.modal.is_none());
        assert_eq!(app.view, View::Tags);
        assert_eq!(app.tags_state.tags.len(), 1, "no tag was deleted");

        // Without an overlay, `q` exits the app and Esc pops one level.
        assert!(!app.dispatch_key(key(KeyCode::Char('q'))));
        assert_eq!(app.view, View::Tags);
        assert!(app.dispatch_key(key(KeyCode::Esc)));
        assert_eq!(app.view, View::Repo);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tags_empty_repo_enters_without_panic_and_renders_empty_state() {
        let dir = temp_dir("tags-empty");
        git2::Repository::init(&dir).unwrap();

        let mut app = app_with_repo_at(&dir);
        enter_tags(&mut app);
        assert!(app.tags_state.tags.is_empty());
        assert_eq!(app.tags_state.selected, 0);
        assert_eq!(app.tags_state.list_state.selected(), None);
        assert!(app.status.is_none());

        // Navigation on an empty list is a no-op and never errors.
        for code in [
            KeyCode::Char('j'),
            KeyCode::Char('k'),
            KeyCode::Down,
            KeyCode::Up,
        ] {
            assert!(app.dispatch_key(key(code)));
        }
        assert_eq!(app.tags_state.list_state.selected(), None);
        assert!(app.status.is_none());

        // p reports why there is nothing to push and never spawns a task
        // (a `tokio::spawn` outside a runtime would panic).
        assert!(app.dispatch_key(key(KeyCode::Char('p'))));
        assert!(app.remote_rx.is_none());
        let st = app.status.as_ref().unwrap();
        assert!(st.is_error);
        assert!(st.message.contains("没有可推送的标签"), "{}", st.message);

        // a (push all) is guarded the same way: no tag list means no task
        // and no fake success report.
        assert!(app.dispatch_key(key(KeyCode::Char('a'))));
        assert!(app.remote_rx.is_none());
        let st = app.status.as_ref().unwrap();
        assert!(st.is_error);
        assert!(st.message.contains("没有可推送的标签"), "{}", st.message);

        let text = render_text(&mut app);
        assert!(text.contains("暂无标签"), "missing empty state: {text}");
        assert!(text.contains("(0)"), "missing zero count: {text}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tags_render_shows_name_short_hash_kind_and_confirm_text() {
        let dir = temp_dir("tags-render");
        init_repo_with_commit(&dir);
        git::create_tag(&dir, "v-light").unwrap();
        {
            let repo = git2::Repository::open(&dir).unwrap();
            let object = repo.head().unwrap().peel(git2::ObjectType::Commit).unwrap();
            let sig = git2::Signature::now("GitPulse Test", "test@gitpulse.local").unwrap();
            repo.tag("v-annotated", &object, &sig, "note", false)
                .unwrap();
        }

        let mut app = app_with_repo_at(&dir);
        enter_tags(&mut app);
        let target = app.tags_state.selected_tag().unwrap().target.clone();

        let text = render_text(&mut app);
        assert!(text.contains("v-light"), "missing tag name: {text}");
        assert!(text.contains("v-annotated"), "missing tag name: {text}");
        assert!(text.contains(&target), "missing short hash: {text}");
        assert!(text.contains("轻量"), "missing lightweight marker: {text}");
        assert!(text.contains("注释"), "missing annotated marker: {text}");
        assert!(text.contains("c新建"), "missing create hint: {text}");
        assert!(text.contains("a全部推送"), "missing push-all hint: {text}");

        // Create input overlay: title plus the typed name after the prompt.
        assert!(app.dispatch_key(key(KeyCode::Char('c'))));
        for c in "v3".chars() {
            assert!(app.dispatch_key(key(KeyCode::Char(c))));
        }
        let text = render_text(&mut app);
        assert!(text.contains("新建标签"), "missing input title: {text}");
        assert!(text.contains(">v3"), "missing typed name: {text}");
        assert!(app.dispatch_key(key(KeyCode::Esc)));

        // Delete confirmation copy.
        assert!(app.dispatch_key(key(KeyCode::Char('d'))));
        let text = render_text(&mut app);
        assert!(
            text.contains("确认删除标签"),
            "missing dialog title: {text}"
        );
        assert!(text.contains("确定删除标签"), "missing dialog body: {text}");
        assert!(text.contains("y确认"), "missing confirm key: {text}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Wait for the background push spawned by the tag panel to settle.
    async fn settle_remote(app: &mut App) {
        for _ in 0..600 {
            app.poll_remote();
            if app.remote_rx.is_none() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("remote operation did not finish in time");
    }

    #[tokio::test]
    async fn tags_push_without_remote_reports_reason() {
        let (dir, mut app) = app_with_tagged_repo("tags-push-noremote", &["v1.0"]);
        enter_tags(&mut app);

        // p arms the task with an initial running status line…
        assert!(app.dispatch_key(key(KeyCode::Char('p'))));
        assert!(app.remote_rx.is_some());
        assert!(matches!(app.remote_status, Some(RemoteStatus::Running(_))));

        // …and finishes with the readable "no remote" reason in red.
        settle_remote(&mut app).await;
        match &app.remote_status {
            Some(RemoteStatus::Error(reason)) => {
                assert!(reason.contains("未配置远端"), "unexpected reason: {reason}");
            }
            other => panic!("expected an error status, got {other:?}"),
        }
        assert!(app.remote_rx.is_none());

        // a (push all) takes the same path with the same reason.
        assert!(app.dispatch_key(key(KeyCode::Char('a'))));
        settle_remote(&mut app).await;
        assert!(matches!(app.remote_status, Some(RemoteStatus::Error(_))));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn tags_push_single_and_all_publish_to_bare_remote() {
        let root = temp_dir("tags-push-origin");
        let origin_path = root.join("origin.git");
        git2::Repository::init_bare(&origin_path).unwrap();
        let work = root.join("work");
        std::fs::create_dir_all(&work).unwrap();
        init_repo_with_commit(&work);
        {
            let repo = git2::Repository::open(&work).unwrap();
            repo.remote("origin", origin_path.to_str().unwrap())
                .unwrap();
        }
        git::create_tag(&work, "v1.0").unwrap();

        let mut app = app_with_repo_at(&work);
        enter_tags(&mut app);
        assert_eq!(app.tags_state.tags.len(), 1);

        // p pushes the selected tag and reports success.
        assert!(app.dispatch_key(key(KeyCode::Char('p'))));
        settle_remote(&mut app).await;
        assert_eq!(app.remote_status, Some(RemoteStatus::Success));
        let origin = git2::Repository::open(&origin_path).unwrap();
        assert!(
            origin.find_reference("refs/tags/v1.0").is_ok(),
            "v1.0 must reach the remote"
        );

        // Create a second tag through the panel, then a pushes them all.
        assert!(app.dispatch_key(key(KeyCode::Char('c'))));
        for c in "v2.0".chars() {
            assert!(app.dispatch_key(key(KeyCode::Char(c))));
        }
        assert!(app.dispatch_key(key(KeyCode::Enter)));
        assert_eq!(app.tags_state.tags.len(), 2);

        assert!(app.dispatch_key(key(KeyCode::Char('a'))));
        settle_remote(&mut app).await;
        assert_eq!(app.remote_status, Some(RemoteStatus::Success));
        let origin = git2::Repository::open(&origin_path).unwrap();
        for name in ["refs/tags/v1.0", "refs/tags/v2.0"] {
            assert!(
                origin.find_reference(name).is_ok(),
                "{name} must reach the remote"
            );
        }

        let _ = std::fs::remove_dir_all(&root);
    }

    /// F1/G1: pressing `p` while a push is already in flight records
    /// `已有远端操作进行中` — and that notice must be on screen beside the
    /// running feedback that used to swallow it, in the very same frame
    /// (the feedback is the running text now, not a frozen gauge).
    #[test]
    fn tags_push_rejected_while_running_shows_notice_beside_running_text() {
        let (dir, mut app) = app_with_tagged_repo("tags-push-busy", &["v1.0"]);
        enter_tags(&mut app);

        // A push already in flight, with its running text on the status row.
        let (_tx, rx) = mpsc::unbounded_channel::<Result<(), String>>();
        app.remote_rx = Some(rx);
        app.remote_op = "push";
        app.remote_status = Some(RemoteStatus::Running("正在 push 标签 v1.0…".to_string()));

        assert!(app.dispatch_key(key(KeyCode::Char('p'))));
        // The running operation is untouched: no second task, same status.
        assert!(app.remote_rx.is_some());
        assert!(matches!(app.remote_status, Some(RemoteStatus::Running(_))));
        let st = app.status.as_ref().expect("notice must be recorded");
        assert!(!st.is_error, "{}", st.message);
        assert_eq!(st.message, "已有远端操作进行中");

        // Both share one row: the running text keeps its cell and the
        // notice renders beside it — and no frozen percentage is drawn.
        let text = render_text(&mut app);
        assert!(
            text.contains("已有远端操作进行中"),
            "notice hidden by the running text: {text}"
        );
        assert!(
            text.contains("正在push标签v1.0…"),
            "running text must stay on screen: {text}"
        );
        assert!(
            !text.contains('%'),
            "frozen progress percent must be gone: {text}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Arm the receiver and running status of a push that is still in
    /// flight — the state `spawn_remote_task` publishes — and hand back the
    /// sender so the channel stays connected for the duration of the test.
    fn arm_remote_running(app: &mut App) -> mpsc::UnboundedSender<Result<(), String>> {
        let (tx, rx) = mpsc::unbounded_channel();
        app.remote_rx = Some(rx);
        app.remote_op = "push";
        app.remote_status = Some(RemoteStatus::Running("正在 push 标签 v1.0…".to_string()));
        tx
    }

    /// R6: while a push is running, opening the tag-create input must not
    /// wipe its live running text — the receiver keeps reporting the state,
    /// so clearing `remote_status` here would hide it.
    #[test]
    fn tags_create_input_keeps_live_progress_while_remote_running() {
        let (dir, mut app) = app_with_tagged_repo("tags-busy-create", &["v1.0"]);
        enter_tags(&mut app);
        let _tx = arm_remote_running(&mut app);
        let running = app.remote_status.clone();

        assert!(app.dispatch_key(key(KeyCode::Char('c'))));
        assert!(
            matches!(app.tags_state.modal, Some(TagsModal::Input { .. })),
            "the input overlay must still open"
        );
        assert!(
            app.remote_rx.is_some(),
            "the running push keeps its receiver"
        );
        assert_eq!(
            app.remote_status, running,
            "live running status must not be cleared by `start_tag_create`"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// R6: submitting the tag create while a push is running still creates
    /// the tag, but the running text survives the entry point.
    #[test]
    fn tags_create_submit_keeps_live_progress_while_remote_running() {
        let (dir, mut app) = app_with_tagged_repo("tags-busy-create-submit", &["v1.0"]);
        enter_tags(&mut app);
        let _tx = arm_remote_running(&mut app);
        let running = app.remote_status.clone();

        assert!(app.dispatch_key(key(KeyCode::Char('c'))));
        for c in "v2.0".chars() {
            assert!(app.dispatch_key(key(KeyCode::Char(c))));
        }
        assert!(app.dispatch_key(key(KeyCode::Enter)));

        // Submit still creates the tag — the running overlay closes and the
        // new tag lands on the list, selected.
        assert!(
            app.tags_state.modal.is_none(),
            "submit must close the input overlay"
        );
        assert!(
            app.tags_state.tags.iter().any(|t| t.name == "v2.0"),
            "the tag must still be created while a push is running"
        );
        assert_eq!(app.tags_state.tags[app.tags_state.selected].name, "v2.0");
        let st = app.status.as_ref().expect("success status expected");
        assert!(!st.is_error, "{}", st.message);
        assert!(st.message.contains("已创建标签 v2.0"), "{}", st.message);

        // ...but the live running text survives the create entry point (R6):
        // a finished-pending wipe never clears a still-running operation.
        assert_eq!(
            app.remote_status, running,
            "live running text must survive the tag create submit"
        );
        assert!(
            app.remote_rx.is_some(),
            "the running push keeps its receiver"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
