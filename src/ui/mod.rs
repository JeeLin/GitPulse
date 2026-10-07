//! TUI rendering: view dispatch shared by all screens.

pub mod branches;
pub mod checks;
pub mod commit;
pub mod components;
pub mod config;
pub mod dashboard;
pub mod diff;
pub mod help;
pub mod history;
pub mod issues;
pub mod notifications;
pub mod pulls;
pub mod remote;
pub mod repo;
pub mod status;
pub mod tags;

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::app::{App, PanelFocus, View};
use crate::git::{FileState, RepoInfo};
use crate::ui::components::footer::hints as join_hints;
use crate::ui::components::footer::{self, HintRow, StatusRow};
use crate::ui::components::state_panel;
use crate::ui::components::theme::{self, color};
use crate::ui::dashboard::DashboardState;
use crate::ui::status::{RemoteStatus, StatusLine};

/// Key/desc fragment of every hint row, re-exported from the footer so the
/// views keep building their segments through `super::hint`.
pub(crate) use crate::ui::components::footer::hint;

/// Shared selection style of the focused list's cursor row (§3.1): the
/// canonical `surface_alt` background, and nothing else — the design picked
/// the background over an accent foreground for the cursor.
pub(crate) fn list_highlight() -> Style {
    Style::default().bg(color::SURFACE_ALT)
}

/// Marker rendered at the row start of the focused list's selected row
/// (§3.1: 行首 `▌` + `surface_alt` 底色). One cell wide, so a selected row
/// keeps every column the unselected rows use.
pub(crate) const HIGHLIGHT_SYMBOL: &str = "▌";

/// Wire the §3.1 cursor onto a list panel: when `focused`, the selected row
/// carries `▌` over the `surface_alt` background; otherwise the list shows
/// neither, because the cursor exists only in the focused list panel.
///
/// One place, so every list in the app draws the same cursor — a panel cannot
/// keep a private marker or a private highlight.
pub(crate) fn with_cursor(list: List<'static>, focused: bool) -> List<'static> {
    if focused {
        list.highlight_style(list_highlight())
            .highlight_symbol(HIGHLIGHT_SYMBOL)
    } else {
        list.highlight_symbol("")
    }
}

/// Bottom hint row of every y/n confirmation overlay (convention 8): the
/// `y` / `n` wording of the overlay itself plus the dismiss key. Every panel
/// — repo discard, branch delete, tag delete, config remove — advertises the
/// exact same three keys while the dialog swallows every other press, so the
/// row lives here instead of being re-typed per panel (G5). It feeds the
/// footer's view segment while the dialog is open, see [`footer::HintRow`].
pub(crate) fn confirm_hints() -> Line<'static> {
    join_hints([
        hint("y", "确认"),
        hint("n", "取消"),
        hint("Esc", "取消对话框"),
    ])
}

/// Bottom hint row of the single-line input overlays (convention 9): `Enter`
/// submits with the verb the overlay's own title advertises, `Esc` cancels.
/// The verb is a parameter because the panels word it differently (`创建`
/// for create/branch/tag inputs, `提交` for the commit message, `确认` for
/// the manual config input), while the row shape is shared (G5).
pub(crate) fn submit_hints(submit: &str) -> Line<'static> {
    join_hints([hint("Enter", submit), hint("Esc", "取消")])
}

/// Draw the status row of the footer (§4.2): errors render on `danger` and
/// persist until the next operation, anything else keeps the default style.
/// `None` leaves the row blank.
///
/// Called through [`footer::render_footer`] — views never draw the row
/// themselves.
pub fn render_status_line(f: &mut Frame, area: Rect, status: Option<&StatusLine>) {
    if let Some(status) = status {
        let style = if status.is_error {
            Style::default().fg(color::DANGER)
        } else {
            Style::default()
        };
        f.render_widget(Paragraph::new(status.message.clone()).style(style), area);
    }
}

/// Render the bottom status row as push/pull feedback when a remote result
/// is present, and as the plain status line otherwise (G6).
///
/// The two panels that own remote feedback (repo, tags) used to duplicate
/// this `remote_status` / `status_line` split verbatim; keeping it here also
/// keeps the notice written while an operation runs visible beside it, see
/// [`remote::render_remote_status`].
pub fn render_status_or_remote(
    f: &mut Frame,
    area: Rect,
    remote_status: Option<&RemoteStatus>,
    remote_op: &str,
    status: Option<&StatusLine>,
) {
    match remote_status {
        Some(remote) => remote::render_remote_status(f, area, remote, remote_op, status),
        None => render_status_line(f, area, status),
    }
}

/// Panel title of the two named-item lists: `label [repo] (count)`.
///
/// Shared by [`render_named_list`] and by the `error` state the same panels
/// render when their load failed (§4.3), so a failed panel still names itself
/// the same way its ready shape does.
pub(crate) fn named_title(label: &str, repo_name: &str, count: usize) -> String {
    format!("{label} [{repo_name}] ({count})")
}

/// Panel skeleton shared by the two named-item lists — the commit history
/// and the tag list (G7): a bordered block titled `label [repo] (count)`
/// around either the rows the caller built or, when there are none, the
/// panel's own `empty` state.
///
/// Zero rows is a valid state of both panels (an unborn HEAD, a repository
/// without tags) rather than a failure, so the empty state is rendered as
/// data; a failed load is not rendered here at all — it paints the panel's
/// `error` state instead (§4.3). The rows are handed in already built because
/// each panel maps its own item type to a [`ListItem`]; only the block, the
/// border role, the cursor and the empty copy are shared.
///
/// `focused` is the panel's focus per §3.1: it picks the border (`border_focus`
/// against `border`) and whether the list draws the `▌` / `surface_alt` cursor.
///
/// The four descriptor fields travel together as [`NamedPanel`] so the call
/// stays within the argument budget and one panel's identity — what it is
/// called, which repository it reads, what it says when empty, and whether it
/// holds focus — is visible at a glance.
pub(crate) struct NamedPanel<'a> {
    /// Panel title prefix, e.g. `提交历史` / `标签`.
    pub label: &'a str,
    /// Repository shown in the title; empty when the panel is global.
    pub repo_name: &'a str,
    /// The §3.2 `empty` state shown when `items` is empty — a valid state,
    /// not a failure. It carries the title, one line of description and the
    /// key that fills the panel, so a panel with an action to offer cannot
    /// render a bare sentence.
    pub empty: &'a state_panel::PanelState<'a>,
    /// Panel focus per §3.1.
    pub focused: bool,
}

pub(crate) fn render_named_list(
    f: &mut Frame,
    area: Rect,
    meta: NamedPanel<'_>,
    items: Vec<ListItem<'static>>,
    list_state: &mut ListState,
) {
    let title = named_title(meta.label, meta.repo_name, items.len());

    if items.is_empty() {
        state_panel::render_state_panel_with_focus(f, area, &title, meta.empty, meta.focused);
        return;
    }

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(theme::role::border(meta.focused));

    let list = with_cursor(List::new(items).block(block), meta.focused);
    f.render_stateful_widget(list, area, list_state);
}

/// Symbol and style of a working-tree [`FileState`] in the file list (G11).
///
/// The mapping is git semantics — which state a status flag means and which
/// §5.1 `git_*` token stands for it — so it lives with the shared render
/// helpers rather than in the closure that builds the rows; the repo panel
/// only composes the styled spans. `Conflicted` borrows `danger` and adds
/// bold: §5.1 gives file states no hue of their own, and a merge conflict has
/// to read as loudly as the deletion it usually resolves into. `Untracked`
/// has no `git_*` alias either — §5.1 stops at the four tracked states — so
/// it takes the bare `info` hue, which is neutral where a `git_*` alias would
/// have mislabelled it. The symbol is
/// two cells (`M `) so the path columns of every row line up.
pub(crate) fn file_state_symbol(state: FileState) -> (&'static str, Style) {
    match state {
        FileState::Modified => ("M ", Style::default().fg(color::GIT_MOD)),
        FileState::Added => ("A ", Style::default().fg(color::GIT_ADD)),
        FileState::Deleted => ("D ", Style::default().fg(color::GIT_DEL)),
        FileState::Untracked => ("? ", Style::default().fg(color::INFO)),
        FileState::Conflicted => (
            "U ",
            Style::default()
                .fg(color::DANGER)
                .add_modifier(Modifier::BOLD),
        ),
    }
}

/// Shared y/n confirmation overlay (convention 8): a centered red-bordered
/// popup titled `title`, showing the caller's `body` lines above the standard
/// `y/n` hint row. `percent_x`/`height` size the popup; `wrap` soft-wraps
/// long target names (discard dialog) instead of clipping them.
pub fn render_confirm_overlay(
    f: &mut Frame,
    area: Rect,
    title: &str,
    percent_x: u16,
    height: u16,
    body: Vec<Line<'static>>,
    wrap: bool,
) {
    let popup = repo::centered_rect(percent_x, height, area);
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .style(Style::default().fg(color::DANGER));

    let mut lines = body;
    lines.push(Line::from(""));
    lines.push(join_hints([hint("y", "确认"), hint("n", "取消")]));

    let paragraph = Paragraph::new(lines).block(block);
    let paragraph = if wrap {
        paragraph.wrap(Wrap { trim: false })
    } else {
        paragraph
    };
    f.render_widget(Clear, popup);
    f.render_widget(paragraph, popup);
}

/// Shared single-line input overlay (convention 9): a centered popup titled
/// `title` showing `text` after an accent `> ` prompt. `show_cursor` appends the
/// `▌` caret (config manual input); popups whose hints live in the title
/// (branch create, commit message) omit it. The line scrolls horizontally by
/// the display width that does not fit so the tail — and with it the caret —
/// always stays inside the popup instead of being cut off.
pub fn render_input_overlay(f: &mut Frame, area: Rect, title: &str, text: &str, show_cursor: bool) {
    let popup = repo::centered_rect(60, 5, area);
    f.render_widget(Clear, popup);

    let mut spans = vec![
        Span::styled("> ", Style::default().fg(color::ACCENT)),
        Span::raw(text),
    ];
    if show_cursor {
        spans.push(Span::styled("▌", Style::default().fg(color::ACCENT)));
    }
    let block = Block::default().title(title).borders(Borders::ALL);
    let line = Line::from(spans);
    let scroll_x = tail_scroll(&line, block.inner(popup).width as usize);
    let paragraph = Paragraph::new(line).block(block).scroll((0, scroll_x));
    f.render_widget(paragraph, popup);
}

/// Horizontal scroll that keeps the last `inner_width` display columns of
/// `line` — and with them the caret — inside the popup.
///
/// ratatui consumes the offset in display cells (a CJK glyph costs 2), so a
/// char-counted offset under-scrolls wide text; and because a wide glyph that
/// straddles the offset is drawn in full, the offset is rounded up to the
/// next grapheme boundary instead of landing mid-glyph.
fn tail_scroll(line: &Line, inner_width: usize) -> u16 {
    let needed = line.width().saturating_sub(inner_width);
    if needed == 0 {
        return 0;
    }
    let mut skipped = 0;
    for grapheme in line.styled_graphemes(Style::default()) {
        skipped += Span::raw(grapheme.symbol).width();
        if skipped >= needed {
            break;
        }
    }
    skipped as u16
}

/// Dispatch rendering to the active view.
///
/// The frame is split once here into the content area and the two-line footer
/// (§4.1): every view paints the content, then the single canonical footer —
/// the status row plus the hint row — is drawn over the bottom two rows, so no
/// view carries its own status/hint rows any more (des-9). The current [`App`]
/// fields are borrowed directly into the view state (checkpoint 2: no separate
/// mock data source). Every arm is a thin `render_*_view` wrapper so this
/// function only dispatches (G12).
pub fn render(f: &mut Frame, app: &mut App) {
    let has_context_tags = matches!(
        app.view,
        View::Repo | View::Branches | View::History | View::Tags
    );
    let (title_area, tags_area, content, footer_area) =
        components::header::split_area(f.area(), has_context_tags);
    components::header::render_title_bar(f, title_area, &crumb(app), &view_right(app));

    // Repo-workspace context tags (§6.2): one strip under the title for the four
    // sibling contexts (`<`/`>` cycles them in place). `<`/`>` never enters the
    // stack (§1.1), so the tabs are a sibling switch, not a pushed screen.
    if let (Some(tags_area), Some(index)) = (tags_area, active_context_tab(app.view)) {
        components::header::render_context_tags(
            f,
            tags_area,
            &["工作区", "分支", "历史", "标签"],
            index,
        );
    }

    match app.view {
        View::Dashboard => render_dashboard_view(f, content, app),
        View::Repo => render_repo_view(f, content, app),
        View::Diff => diff::render_diff(f, content, app),
        View::Commit => render_commit_view(f, content, app),
        View::Branches => render_branches_view(f, content, app),
        View::Config => render_config_view(f, content, app),
        View::History => render_history_view(f, content, app),
        View::Tags => render_tags_view(f, content, app),
        View::Pulls => pulls::render_pulls(f, content, app),
        View::PullDetail => pulls::render_pull_detail(f, content, app),
        View::Checks => checks::render_checks(f, content, app),
        View::Issues => issues::render_issues(f, content, app),
        View::IssueDetail => issues::render_issue_detail(f, content, app),
        View::Notifications => notifications::render_notifications(f, content, app),
    }

    let status = view_status(app);
    let hints = view_hints(app);
    footer::render_footer(f, footer_area, status, &hints);

    // Wave-2 float overlays (`?` help / Ctrl-P palette) draw last, on top of
    // the footer (§6.11 / §127). The dispatch in `app.rs` owns their keys and
    // only the open bool is read here — an overlay is never rendered over
    // another (§2.4: 同时只允许一个模态).
    if app.help_open {
        help::render_help(f, app);
    } else if app.palette_open {
        help::render_palette(f, app);
    }
}

/// The breadcrumb trail for the current screen (§1.2): `仪表盘` at the root and
/// `›`-joined crumbs as you drill in — the header always shows where you came
/// from, so a pushed stack is legible without counting `Esc` presses.
///
/// Only the *stack* grows the trail. The repo workspace's four contexts are
/// siblings switched by `<`/`>` without ever entering the stack (§1.1), so
/// `Branches` / `History` / `Tags` show the same crumb `Repo` does — a pushed
/// `… › 分支` tail would read as a screen of its own and contradict §6.2,
/// whose layout draws one breadcrumb (`仪表盘 › GitPulse`) for every context.
fn crumb(app: &App) -> String {
    let repo = repo_name_of(&app.repos, app.current_repo);
    let repo_crumb = if repo.is_empty() {
        "我的仓库".to_string()
    } else {
        format!("我的仓库 › {repo}")
    };
    let tail = match app.view {
        View::Dashboard => return "仪表盘".to_string(),
        // One screen, four contexts: the breadcrumb stops at the repository.
        View::Repo | View::Branches | View::History | View::Tags => {
            return format!("仪表盘 › {repo_crumb}")
        }
        View::Diff => "Diff",
        View::Commit => "提交",
        View::Config => return "仪表盘 › 配置".to_string(),
        // The receipts spine hangs off the repo's remote hub (§1.2), so its
        // crumb names the repo before the tab, not the dashboard.
        View::Pulls
        | View::PullDetail
        | View::Checks
        | View::Issues
        | View::IssueDetail
        | View::Notifications => return format!("仪表盘 › {repo_crumb} › 远端中心"),
    };
    format!("仪表盘 › {repo_crumb} › {tail}")
}

/// Index of the repo-workspace context tag (§6.2: `工作区 │ 分支 │ 历史 │ 标签`)
/// that `view` selects, or `None` on the screens that have no context tabs yet.
///
/// The index is the position in the strip [`render`] draws, so `Repo` (工作区)
/// is 0, `Branches` (分支) 1, `History` (历史) 2 and `Tags` (标签) 3.
fn active_context_tab(view: View) -> Option<usize> {
    match view {
        View::Repo => Some(0),
        View::Branches => Some(1),
        View::History => Some(2),
        View::Tags => Some(3),
        _ => None,
    }
}

/// The right-flush status cell of the title bar (§6.1): the dashboard shows the
/// repository count, the receipt tabs show the running/last fetch, and every
/// other screen has nothing to add. The per-row `远端:` badge belongs to the
/// dashboard list itself, not the title — it needs a remote field that
/// `RepoInfo` does not yet carry, so it is recorded as a 5a 沉淀 item.
fn view_right(app: &App) -> String {
    match app.view {
        View::Dashboard => format!("仓库 {}", app.repos.len()),
        View::Pulls
        | View::PullDetail
        | View::Checks
        | View::Issues
        | View::IssueDetail
        | View::Notifications => app.receipt_op_label().to_string(),
        _ => String::new(),
    }
}

/// Footer status row for the current view (§4.2) — the one place an async
/// result reaches the screen: local git messages and GitHub receipts alike
/// land on this row, whether or not a remote is connected.
///
/// `Repo` / `Tags` / `Commit` and every receipt tab own remote feedback, so
/// they carry the remote triple and label it with their own operation name;
/// the rest (dashboard / branches / history / config / diffs) take the plain
/// status line. No panel reads these fields any more (§4.1).
pub(crate) fn view_status(app: &App) -> StatusRow<'_> {
    match app.view {
        View::Repo | View::Tags | View::Commit => StatusRow::new(
            app.status.as_ref(),
            app.remote_status.as_ref(),
            app.remote_op,
        ),
        View::Pulls
        | View::PullDetail
        | View::Issues
        | View::IssueDetail
        | View::Checks
        | View::Notifications => StatusRow::new(
            app.status.as_ref(),
            app.remote_status.as_ref(),
            app.receipt_op_label(),
        ),
        _ => StatusRow::plain(app.status.as_ref()),
    }
}

/// Footer hint row for the current view (§2.3).
///
/// Views build one through their exported `{view}_hints(..)` exporter via this
/// helper; receipt views reuse the shared one from `pulls` (G3). The hint row
/// itself is a `pub(crate)` helper so receipt-view tests can reuse it.
pub(crate) fn view_hints(app: &App) -> HintRow {
    match app.view {
        View::Dashboard => dashboard::dashboard_hints(),
        View::Repo => repo::repo_hints(app.show_discard_confirm),
        View::Diff => diff::diff_hints(),
        View::Commit => commit::commit_hints(),
        View::Branches => {
            branches::branches_hints(app.show_branch_input, app.show_branch_delete_confirm)
        }
        View::Config => config::config_hints(
            app.show_config_picker,
            app.show_config_input,
            app.show_config_delete_confirm,
        ),
        View::History => history::history_hints(),
        View::Tags => tags::tags_hints(app.tags_state.modal.as_ref()),
        View::Pulls => pulls::receipt_hints(true, app.github_state.kind()),
        View::PullDetail => pulls::pull_detail_hints(app.github_state.kind()),
        View::Checks => pulls::receipt_hints(false, app.github_state.kind()),
        View::Issues => pulls::receipt_hints(true, app.github_state.kind()),
        View::IssueDetail => pulls::issue_detail_hints(app.github_state.kind()),
        View::Notifications => pulls::receipt_hints(false, app.github_state.kind()),
    }
}

/// Name of the repository at `index` in `repos`, or `""` when the index is
/// stale — the same "empty on a stale index" result the panel accessors
/// produce.
///
/// It takes the two fields instead of `&App` so the returned `&str` borrows
/// `repos` alone: the history and tag panels then hold it beside their own
/// `&mut` cursor borrow instead of cloning it into a `String` every frame
/// (G25).
fn repo_name_of(repos: &[RepoInfo], index: Option<usize>) -> &str {
    index
        .and_then(|i| repos.get(i))
        .map_or("", |repo| repo.name.as_str())
}

/// Dashboard arm of [`render`].
fn render_dashboard_view(f: &mut Frame, content: Rect, app: &mut App) {
    let batch_running = app.batch_running();
    // Read the focus flag before the cursor borrow below: the accessor takes
    // `&App`, which cannot coexist with `&mut app.dashboard_state` (§3.1).
    let focused = app.panel_is_focused(PanelFocus::Primary);
    let mut state = DashboardState {
        repos: &app.repos,
        load_error: app.load_error.as_deref(),
        list_state: &mut app.dashboard_state,
        batch_log: &app.batch_log,
        batch_op: app.batch_op,
        batch_running,
        focused,
    };
    dashboard::render_dashboard(f, content, &mut state);
}

/// Repo arm of [`render`]: the file list plus the discard confirmation.
fn render_repo_view(f: &mut Frame, content: Rect, app: &mut App) {
    let sel = app.file_state.selected();
    let focused = app.panel_is_focused(PanelFocus::Primary);
    let confirm_file = if app.show_discard_confirm {
        sel.and_then(|i| app.file_entries.get(i))
    } else {
        None
    };
    let state = repo::RepoState {
        entries: &app.file_entries,
        list_state: &mut app.file_state,
        show_discard_confirm: app.show_discard_confirm,
        confirm_file,
        load_error: app.load_error.as_deref(),
        focused,
    };
    repo::render_repo(f, content, state);
}

/// Commit arm of [`render`]: repo view as background, commit input popup on
/// top.
fn render_commit_view(f: &mut Frame, content: Rect, app: &mut App) {
    let focused = app.panel_is_focused(PanelFocus::Primary);
    let state = repo::RepoState {
        entries: &app.file_entries,
        list_state: &mut app.file_state,
        show_discard_confirm: false,
        confirm_file: None,
        load_error: app.load_error.as_deref(),
        focused,
    };
    repo::render_repo(f, content, state);
    commit::render_commit_popup(f, &app.commit_message);
}

/// Branch panel arm of [`render`].
fn render_branches_view(f: &mut Frame, content: Rect, app: &mut App) {
    let confirm = if app.show_branch_delete_confirm {
        app.branch_state
            .selected()
            .and_then(|i| app.branches.get(i))
    } else {
        None
    };
    // `branch_repo` stays private to `App`, so the panel keeps asking the
    // accessor — its `&str` borrows the whole `App` and cannot sit next to
    // the `&mut` cursor borrow below. Unlike the history/tags arms the copy
    // cannot be dropped without exposing the private index (G25).
    let repo_name = app.branch_repo_name().to_string();
    let focused = app.panel_is_focused(PanelFocus::Primary);
    let state = branches::BranchesState {
        branches: &app.branches,
        list_state: &mut app.branch_state,
        repo_name: &repo_name,
        show_input: app.show_branch_input,
        input: &app.branch_input,
        show_delete_confirm: app.show_branch_delete_confirm,
        confirm_branch: confirm,
        load_error: app.load_error.as_deref(),
        focused,
    };
    branches::render_branches(f, content, state);
}

/// Config panel arm of [`render`].
fn render_config_view(f: &mut Frame, content: Rect, app: &mut App) {
    // Field-level borrow of `app.config` coexists with the
    // `&mut app.config_state` below (disjoint fields).
    let state = config::ConfigState {
        scan_dirs: &app.config.scan_dirs,
        list_state: &mut app.config_state,
        show_input: app.show_config_input,
        input: &app.config_input,
        show_delete_confirm: app.show_config_delete_confirm,
        show_picker: app.show_config_picker,
        picker_dir: &app.picker_dir,
        picker_entries: &app.picker_entries,
        picker_state: &mut app.picker_state,
        load_error: app.load_error.as_deref(),
    };
    config::render_config(f, content, state);
}

/// History arm of [`render`].
///
/// Field-level borrows of `app.history` (data vs. cursor) and `app.repos`
/// (the title) coexist as disjoint fields of `App`.
fn render_history_view(f: &mut Frame, content: Rect, app: &mut App) {
    let repo_name = repo_name_of(&app.repos, app.current_repo);
    let focused = app.panel_is_focused(PanelFocus::Primary);
    let state = history::HistoryRender {
        commits: &app.history.commits,
        detail: app.history.detail.as_ref(),
        graph: &app.history.graph,
        list_state: &mut app.history.list_state,
        selected: app.history.selected,
        repo_name,
        load_error: app.load_error.as_deref(),
        focused,
    };
    history::render_history(f, content, state);
}

/// Tags arm of [`render`].
///
/// Field-level borrows of `app.tags_state` (cursor vs. list) and `app.repos`
/// (the title) coexist as disjoint fields of `App`.
fn render_tags_view(f: &mut Frame, content: Rect, app: &mut App) {
    let repo_name = repo_name_of(&app.repos, app.current_repo);
    let focused = app.panel_is_focused(PanelFocus::Primary);
    let state = tags::TagsRender {
        tags: &app.tags_state.tags,
        list_state: &mut app.tags_state.list_state,
        modal: app.tags_state.modal.as_ref(),
        repo_name,
        load_error: app.load_error.as_deref(),
        focused,
    };
    tags::render_tags(f, content, state);
}

#[cfg(test)]
/// `octo/demo` on github.com: the remote behind the panel fixtures, kept in
/// one place so the `octo/demo` titles and the gate copy the receipt tests
/// assert on stay derived from the same source of truth.
pub(crate) fn github_remote() -> crate::platform::PlatformRemote {
    crate::platform::parse_remote("https://github.com/octo/demo.git").expect("github remote")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::git::{BranchInfo, FileEntry, FileState};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::fs;
    use std::path::Path;

    fn base_app() -> App {
        App::new(Config::default())
    }

    /// Receipt hub connected to `octo/demo`, so the panel under test renders
    /// its list rather than the readiness gate.
    fn connected_app() -> App {
        let mut app = App::new(Config::default());
        app.github_state.remote = Some(github_remote());
        app.github_state.client = Some(pulls::offline_client());
        app
    }

    /// Render one frame and hand back the buffer, so a test can address
    /// single rows and their cell styles.
    fn render_buffer(app: &mut App) -> ratatui::buffer::Buffer {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, app)).unwrap();
        terminal.backend().buffer().clone()
    }

    /// Display text of one row: ratatui stores a wide (CJK) glyph in two
    /// cells — the glyph plus a *padding* cell whose symbol is a space — so
    /// the raw row reads `选 择`. The padding cell is dropped here, which
    /// keeps the real gap widths (one space inside a pair, two between
    /// pairs) and lets a test measure the row with `cells::width`.
    fn row_text(buffer: &ratatui::buffer::Buffer, y: u16) -> String {
        let mut text = String::new();
        let mut prev_wide = false;
        for x in 0..buffer.area.width {
            let symbol = buffer[(x, y)].symbol().to_string();
            if symbol == " " && prev_wide {
                prev_wide = false;
                continue;
            }
            prev_wide = crate::ui::components::cells::width(&symbol) == 2;
            text.push_str(&symbol);
        }
        text.trim_end().to_string()
    }

    /// Whitespace-stripped text of one row.
    fn compact_row(buffer: &ratatui::buffer::Buffer, y: u16) -> String {
        row_text(buffer, y)
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect()
    }

    /// Rows whose text carries the global segment — the marker of a hint
    /// row, wherever a view might be drawing one (§2.3).
    fn hint_rows(buffer: &ratatui::buffer::Buffer) -> Vec<u16> {
        (0..buffer.area.height)
            .filter(|y| {
                let row = compact_row(buffer, *y);
                row.contains("q退出") && row.contains("r刷新")
            })
            .collect()
    }

    /// Global segment of the hint row, whitespace-stripped (§2.3).
    const GLOBAL_SEGMENT: &str = "⏎打开Esc返回r刷新q退出";

    /// The screens the milestone names: the two local-git views and the four
    /// receipt tabs. Every one of them draws its panel body only, so the
    /// bottom row is the one and only hint row.
    fn hint_row_screens() -> Vec<App> {
        let mut dashboard = base_app();
        dashboard.file_entries = vec![FileEntry {
            path: std::path::PathBuf::from("src/main.rs"),
            state: FileState::Modified,
        }];

        let mut repo = base_app();
        repo.view = View::Repo;
        repo.file_entries = dashboard.file_entries.clone();

        let mut pulls = connected_app();
        pulls.view = View::Pulls;

        let mut checks = connected_app();
        checks.view = View::Checks;

        let mut issues = connected_app();
        issues.view = View::Issues;

        let mut notifications = connected_app();
        notifications.view = View::Notifications;

        vec![dashboard, repo, pulls, checks, issues, notifications]
    }

    /// Render one frame through the view dispatcher and return the
    /// whitespace-stripped buffer text (wide CJK glyphs leave padding cells).
    fn render_compact(app: &mut App) -> String {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, app)).unwrap();
        let raw: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        raw.chars().filter(|c| !c.is_whitespace()).collect()
    }

    /// §2.3 assertion 1: the hint row appears exactly once per frame, on the
    /// bottom row, on every screen — no panel keeps a second one in its
    /// content area.
    #[test]
    fn hint_row_appears_once_on_the_bottom_row_of_every_screen() {
        for mut app in hint_row_screens() {
            let view = app.view;
            let buffer = render_buffer(&mut app);
            let bottom = buffer.area.height - 1;
            let rows = hint_rows(&buffer);
            assert_eq!(
                rows,
                vec![bottom],
                "view {view:?} must show the hint row only at the bottom"
            );
            assert!(
                compact_row(&buffer, bottom).starts_with(GLOBAL_SEGMENT),
                "view {view:?} must lead with the global segment: {}",
                compact_row(&buffer, bottom)
            );
        }
    }

    /// §2.3 assertion 2: two cells between adjacent key caps, one between a
    /// key and its action. The glued `j/k 选择g` form must not exist.
    #[test]
    fn hint_row_separates_pairs_with_the_canonical_gap_widths() {
        for mut app in hint_row_screens() {
            let view = app.view;
            let buffer = render_buffer(&mut app);
            let row = row_text(&buffer, buffer.area.height - 1);
            for (key, action) in [("⏎", "打开"), ("r", "刷新")] {
                assert!(
                    row.contains(&format!("{key} {action}")),
                    "view {view:?} must put one space between key and action: {row:?}"
                );
            }
            assert!(
                row.contains("打开  Esc"),
                "view {view:?} must put two spaces between pairs: {row:?}"
            );
            for glued in ["打开Esc", "刷新q"] {
                assert!(
                    !row.contains(glued),
                    "view {:?} shows the glued form {:?}: {:?}",
                    view,
                    glued,
                    row
                );
            }
        }
    }

    /// §2.3 assertion 3: at 80 columns the global segment keeps every pair
    /// and the row never exceeds the terminal width.
    #[test]
    fn hint_row_keeps_the_global_prefix_whole_at_80_columns() {
        for mut app in hint_row_screens() {
            let view = app.view;
            let buffer = render_buffer(&mut app);
            let row = compact_row(&buffer, buffer.area.height - 1);
            assert!(
                row.starts_with(GLOBAL_SEGMENT),
                "view {view:?} truncates the global segment: {row}"
            );
            let width =
                crate::ui::components::cells::width(&row_text(&buffer, buffer.area.height - 1));
            assert!(
                width <= 80,
                "view {view:?} hint row is {width} columns wide"
            );
        }
    }

    /// §2.3 assertion 4: when the view segment does not fit it is the one
    /// that goes, and the cut is marked with `…`.
    #[test]
    fn hint_row_truncates_the_view_segment_before_the_global_one() {
        let mut app = base_app();
        app.view = View::Repo;
        app.file_entries = (0..12)
            .map(|i| FileEntry {
                path: std::path::PathBuf::from(format!("src/module_{i}.rs")),
                state: FileState::Modified,
            })
            .collect();
        let buffer = render_buffer(&mut app);
        let row = compact_row(&buffer, buffer.area.height - 1);
        assert!(
            row.starts_with(GLOBAL_SEGMENT),
            "the global segment must survive: {row}"
        );
        assert!(
            row.contains('…'),
            "the view segment cut must be marked: {row}"
        );
        assert!(
            row.contains("j/k选择"),
            "the head of the view segment stays: {row}"
        );
    }

    /// Whitespace-stripped text of the whole frame, row after row — how a
    /// test counts how often one message reached the screen (§4.1).
    fn compact_frame(buffer: &ratatui::buffer::Buffer) -> String {
        let mut text = String::new();
        for y in 0..buffer.area.height {
            text.push_str(&compact_row(buffer, y));
        }
        text
    }

    /// Rows whose whitespace-stripped text contains `needle` — the row a
    /// message landed on (§4.1: the status row at `height - 2` and nothing
    /// else).
    fn rows_with(buffer: &ratatui::buffer::Buffer, needle: &str) -> Vec<u16> {
        (0..buffer.area.height)
            .filter(|y| compact_row(buffer, *y).contains(needle))
            .collect()
    }

    /// Message used to trace where one async result lands; a CJK run with no
    /// spaces so the whitespace-stripped comparison is exact.
    const STATUS_SENTINEL: &str = "异步回执落点";

    /// Every screen the footer serves: the six of the hint-row sweep plus the
    /// panel views that carry their own list state. The commit view is left
    /// out on purpose — its input popup is a centered overlay (§2.4), not a
    /// panel body.
    fn footer_row_screens() -> Vec<App> {
        let mut screens = hint_row_screens();
        for view in [
            View::Diff,
            View::Branches,
            View::Config,
            View::History,
            View::Tags,
        ] {
            let mut app = base_app();
            app.view = view;
            screens.push(app);
        }
        screens
    }

    /// §4.1 assertion 2: one async result, one rendering. A message reaches
    /// the frame exactly once — on the footer status row — never beside a
    /// second copy inside the panel body.
    #[test]
    fn async_result_reaches_the_frame_exactly_once() {
        for mut app in footer_row_screens() {
            let view = app.view;
            app.status = Some(StatusLine {
                message: STATUS_SENTINEL.to_string(),
                is_error: false,
            });
            let buffer = render_buffer(&mut app);
            assert_eq!(
                compact_frame(&buffer).matches(STATUS_SENTINEL).count(),
                1,
                "view {view:?} renders its async result more than once"
            );
        }
    }

    /// §4.1 assertion 3: the footer is always the last two rows — the status
    /// row at `height - 2`, the hint row at `height - 1` — whatever the view
    /// draws above them. The split itself is pinned by
    /// `footer::split_area_reserves_the_bottom_two_rows_for_the_footer`.
    #[test]
    fn footer_holds_the_status_row_and_the_hint_row_on_the_last_two_rows() {
        for mut app in footer_row_screens() {
            let view = app.view;
            app.status = Some(StatusLine {
                message: STATUS_SENTINEL.to_string(),
                is_error: false,
            });
            let buffer = render_buffer(&mut app);
            let height = buffer.area.height;
            assert_eq!(
                rows_with(&buffer, STATUS_SENTINEL),
                vec![height - 2],
                "view {view:?} moved its status off `height - 2`"
            );
            assert_eq!(
                hint_rows(&buffer),
                vec![height - 1],
                "view {view:?} moved its hint row off `height - 1`"
            );
        }
    }

    /// §4.1 assertion 4: each `RemoteStatus` shape renders once, on the status
    /// row, and the focus panel keeps the data it already had (§4.4: a long
    /// async operation must not blank the panel).
    #[test]
    fn remote_status_shapes_land_on_the_status_row_only() {
        let shapes = [
            (RemoteStatus::Running("正在 pull…".to_string()), "正在pull…"),
            (RemoteStatus::Success, "pull成功"),
            (RemoteStatus::Error("未配置远端".to_string()), "未配置远端"),
        ];
        for (remote_status, needle) in shapes {
            let mut app = base_app();
            app.view = View::Repo;
            app.file_entries = vec![FileEntry {
                path: std::path::PathBuf::from("src/main.rs"),
                state: FileState::Modified,
            }];
            app.remote_op = "pull";
            app.remote_status = Some(remote_status);
            let buffer = render_buffer(&mut app);
            assert_eq!(
                rows_with(&buffer, needle),
                vec![buffer.area.height - 2],
                "{needle:?} must render once, on the status row"
            );
            assert!(
                compact_frame(&buffer).contains("src/main.rs"),
                "{needle:?} must not blank the file list (§4.4)"
            );
        }
    }

    /// §4.3 assertion 3 + 4: an *operation* failure — a local git action the
    /// user triggered and a GitHub receipt action alike — reports on the
    /// status row in `danger`, and the panel stays `ready` with the rows it
    /// already loaded. One rule, two paths: the paired cases are the same
    /// assertion written once per backend.
    #[test]
    fn operation_failure_keeps_the_panel_ready_on_the_local_git_and_github_paths() {
        // Local git: a rejected stage, with the file list it failed to touch.
        let mut git_app = base_app();
        git_app.view = View::Repo;
        git_app.file_entries = vec![FileEntry {
            path: std::path::PathBuf::from("src/main.rs"),
            state: FileState::Modified,
        }];
        git_app.set_error("stage 失败: 索引已锁定");
        assert_operation_failure(&mut git_app, "stage失败:索引已锁定", "src/main.rs");

        // GitHub: a rejected receipt action, with the PR list it did not
        // reload. Same rule, same landing spot.
        let mut github_app = connected_app();
        github_app.view = View::Pulls;
        github_app.github_state.pulls = vec![crate::platform::PrSummary {
            number: 42,
            title: "Fix login bug".to_string(),
            state: "open".to_string(),
            draft: false,
            author: "alice".to_string(),
            head_ref: "feat/login".to_string(),
            base_ref: "main".to_string(),
            updated_at: "2026-01-02T03:04:05Z".to_string(),
        }];
        github_app.set_error("GitHub 操作失败: 401");
        assert_operation_failure(&mut github_app, "GitHub操作失败:401", "Fixloginbug");
    }

    /// One half of the pair above: the reason lands once, on the status row,
    /// in `danger`, and the panel keeps rendering `ready` — no `error` state,
    /// no `按 r 重试` over data that did load.
    fn assert_operation_failure(app: &mut App, needle: &str, list_row: &str) {
        let view = app.view;
        let buffer = render_buffer(app);
        let height = buffer.area.height;
        assert_eq!(
            rows_with(&buffer, needle),
            vec![height - 2],
            "view {view:?}: an operation result belongs on the status row only"
        );
        assert_eq!(
            buffer[(0, height - 2)].fg,
            color::DANGER,
            "view {view:?}: the operation failure renders in danger"
        );
        let text = compact_frame(&buffer);
        assert!(
            text.contains(list_row),
            "view {view:?}: the panel must stay ready with its rows: {text}"
        );
        assert!(
            !text.contains("加载失败") && !text.contains("按r重试"),
            "view {view:?}: an operation result never paints the error state: {text}"
        );
    }

    #[test]
    fn dashboard_view_renders_title_for_empty_repo_list() {
        let mut app = base_app();
        let text = render_compact(&mut app);
        assert!(
            text.contains("GitPulse仓库状态(0)"),
            "missing dashboard title: {text}"
        );
    }

    /// §2.3: every hint row — the dashboard included — takes its key caps
    /// from `role::key` (bold accent) and its actions from `role::caption`
    /// (`fg_muted`). The dashboard used to keep a private Cyan/Reset
    /// dialect for its inline row; with that row gone it uses the canonical
    /// pair like every other view.
    #[test]
    fn hint_row_styles_key_caps_and_actions_per_theme() {
        use ratatui::style::Modifier;

        use crate::ui::components::theme::color;

        let mut app = base_app();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let hint_row = buffer.area.height - 1;
        let mut key_fg = None;
        let mut key_bold = false;
        let mut desc_fg = None;
        for x in 0..buffer.area.width {
            let cell = &buffer[(x, hint_row)];
            match cell.symbol() {
                "n" if key_fg.is_none() => {
                    key_fg = Some(cell.fg);
                    key_bold = cell.modifier.contains(Modifier::BOLD);
                }
                "添" => desc_fg = Some(cell.fg),
                _ => {}
            }
        }
        assert_eq!(key_fg, Some(color::ACCENT), "key caps take the accent");
        assert!(key_bold, "key caps are bold");
        assert_eq!(desc_fg, Some(color::FG_MUTED), "actions read muted");
    }

    #[test]
    fn repo_view_renders_file_list_and_hints() {
        let mut app = base_app();
        app.view = View::Repo;
        app.file_entries = vec![FileEntry {
            path: std::path::PathBuf::from("src/main.rs"),
            state: FileState::Modified,
        }];
        app.file_state.select(Some(0));
        let text = render_compact(&mut app);
        assert!(text.contains("文件状态(1文件)"), "missing title: {text}");
        assert!(text.contains("Msrc/main.rs"), "missing entry: {text}");
        assert!(text.contains("Esc返回"), "missing hints: {text}");
    }

    #[test]
    fn repo_view_shows_discard_confirm_overlay() {
        let mut app = base_app();
        app.view = View::Repo;
        app.file_entries = vec![FileEntry {
            path: std::path::PathBuf::from("src/main.rs"),
            state: FileState::Modified,
        }];
        app.file_state.select(Some(0));
        app.show_discard_confirm = true;
        let text = render_compact(&mut app);
        assert!(
            text.contains("确认丢弃更改"),
            "missing dialog title: {text}"
        );
        assert!(text.contains("y确认"), "missing confirm key: {text}");
    }

    #[test]
    fn diff_view_renders_content_and_title() {
        let mut app = base_app();
        app.view = View::Diff;
        app.diff_content = "+added_line\n-removed_line\n".to_string();
        let text = render_compact(&mut app);
        assert!(text.contains("文件差异"), "missing title: {text}");
        assert!(text.contains("+added_line"), "missing added line: {text}");
        assert!(
            text.contains("-removed_line"),
            "missing removed line: {text}"
        );
    }

    #[test]
    fn commit_view_renders_popup_over_repo_background() {
        let mut app = base_app();
        app.view = View::Commit;
        app.commit_message = "feat: hello".to_string();
        let text = render_compact(&mut app);
        assert!(text.contains("提交信息"), "missing popup title: {text}");
        assert!(text.contains("feat:hello"), "missing message: {text}");
    }

    #[test]
    fn branches_view_renders_empty_list_with_title_and_hints() {
        let mut app = base_app();
        app.view = View::Branches;
        let text = render_compact(&mut app);
        // No dedicated empty-state copy: the title carries the zero count.
        assert!(text.contains("分支[](0)"), "missing empty title: {text}");
        assert!(text.contains("n新建"), "missing create hint: {text}");
        assert!(text.contains("Esc返回"), "missing back hint: {text}");
    }

    #[test]
    fn branches_view_shows_delete_confirm_overlay() {
        let mut app = base_app();
        app.view = View::Branches;
        app.branches = vec![BranchInfo {
            name: "feature".to_string(),
            is_head: false,
            upstream: None,
        }];
        app.branch_state.select(Some(0));
        app.show_branch_delete_confirm = true;
        let text = render_compact(&mut app);
        assert!(
            text.contains("确认删除分支"),
            "missing dialog title: {text}"
        );
        assert!(text.contains("feature"), "missing branch name: {text}");
    }

    #[test]
    fn config_view_renders_title_and_hints_via_dispatch() {
        let mut app = base_app();
        app.view = View::Config;
        let text = render_compact(&mut app);
        // Default config has no scan dirs: the title carries the zero count.
        assert!(
            text.contains("扫描目录配置(0)"),
            "missing config title: {text}"
        );
        assert!(text.contains("j/k选择"), "missing nav hint: {text}");
        assert!(text.contains("a添加"), "missing add hint: {text}");
        assert!(text.contains("i手动"), "missing manual hint: {text}");
        assert!(text.contains("d删除"), "missing delete hint: {text}");
        assert!(text.contains("Esc返回"), "missing back hint: {text}");
    }

    #[test]
    fn config_view_input_overlay_renders_via_dispatch() {
        let mut app = base_app();
        app.view = View::Config;
        app.show_config_input = true;
        app.config_input = "/home/user/repos".to_string();
        let text = render_compact(&mut app);
        assert!(text.contains("添加扫描目录"), "missing input title: {text}");
        assert!(
            text.contains(">/home/user/repos"),
            "missing typed path: {text}"
        );
        assert!(text.contains("Enter确认"), "missing confirm hint: {text}");
        assert!(text.contains("Esc取消"), "missing cancel hint: {text}");
    }

    #[test]
    fn input_overlay_scrolls_long_text_to_keep_cursor_visible() {
        let mut app = base_app();
        app.view = View::Config;
        app.show_config_input = true;
        app.config_input = format!("/home/user/repos/{}", "deep".repeat(20));
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, &mut app)).unwrap();
        let raw: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(
            raw.contains('▌'),
            "cursor scrolled out of the visible window: {raw}"
        );
        assert!(raw.contains("deep"), "tail of the long path missing: {raw}");

        // Callers without a caret (commit message) scroll to the tail too.
        let mut app = base_app();
        app.view = View::Commit;
        app.commit_message = format!("feat: {}", "tail".repeat(30));
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, &mut app)).unwrap();
        let raw: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(
            raw.contains("tailtail"),
            "tail of the long commit message missing: {raw}"
        );
    }

    /// Wide CJK glyphs cost two display cells each while `chars().count()`
    /// charges one, so a long Chinese path under-scrolls and drops the caret
    /// unless the offset is measured by display width.
    #[test]
    fn input_overlay_scrolls_wide_cjk_path_to_keep_cursor_visible() {
        let mut app = base_app();
        app.view = View::Config;
        app.show_config_input = true;
        app.config_input = format!("/数据/项目/{}", "中文目录".repeat(20));
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, &mut app)).unwrap();
        let raw: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(
            raw.contains('▌'),
            "cursor scrolled out of the visible window: {raw}"
        );
        // Wide glyphs leave a padding cell, so match the tail whitespace-free.
        let compact: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            compact.contains("目录"),
            "tail of the CJK path missing: {compact}"
        );
    }

    #[test]
    fn config_view_delete_confirm_renders_via_dispatch() {
        let mut app = base_app();
        app.view = View::Config;
        app.config_state.select(None);
        app.show_config_delete_confirm = true;
        let text = render_compact(&mut app);
        assert!(text.contains("移除该目录"), "missing prompt: {text}");
        assert!(text.contains("y确认"), "missing confirm key: {text}");
        assert!(text.contains("n取消"), "missing cancel key: {text}");
    }

    /// §6.11 help overlay (Wave 2): renders the centered modal titled `帮助`
    /// carrying its own hint rows (§6.11).
    #[test]
    fn help_overlay_renders_title_and_hints_via_dispatch() {
        let mut app = base_app(); // Dashboard view
        app.help_open = true;
        let text = render_compact(&mut app);
        assert!(text.contains("帮助"), "missing help title: {text}");
        assert!(
            text.contains("q退出") || text.contains("Esc返回"),
            "missing global key hint: {text}"
        );
        assert!(text.contains("进入仓库"), "missing view hint: {text}");
    }

    /// §6.11 command palette (Wave 2): renders the centered modal titled
    /// `命令面板` with its jump-target list.
    #[test]
    fn palette_overlay_renders_jump_targets_via_dispatch() {
        let mut app = base_app();
        app.palette_open = true;
        app.palette_state.select(Some(0));
        let text = render_compact(&mut app);
        assert!(text.contains("命令面板"), "missing palette title: {text}");
        assert!(text.contains("仪表盘"), "missing dashboard target: {text}");
        assert!(text.contains("分支"), "missing branch target: {text}");
    }

    /// §3.1 assertion 2: the cursor is the canonical `▌` at the row start over
    /// the `surface_alt` background — one shape for every list, drawn only by
    /// the focused panel.
    #[test]
    fn the_focused_list_cursor_is_the_bar_over_surface_alt() {
        let mut app = base_app();
        app.repos = vec![RepoInfo {
            name: "alpha".to_string(),
            path: std::path::PathBuf::from("/tmp/alpha"),
            branch: "main".to_string(),
            dirty: Default::default(),
            ahead: 0,
            behind: 0,
            last_commit: Default::default(),
        }];
        app.dashboard_state.select(Some(0));

        let buffer = render_buffer(&mut app);
        let cursor = cursor_cells(&buffer);
        assert_eq!(
            cursor.len(),
            1,
            "exactly one focused panel draws the cursor: {cursor:?}"
        );

        let (x, y) = cursor[0];
        assert_eq!(
            buffer[(x, y)].symbol(),
            HIGHLIGHT_SYMBOL,
            "the cursor marker must be `▌` at the row start"
        );
        assert_eq!(
            buffer[(x, y)].bg,
            color::SURFACE_ALT,
            "the cursor row must wear the `surface_alt` background"
        );

        // The whole selected row shares that background, not just the marker
        // cell — §3.1 calls for a 底色 on the cursor row. Only the panel's own
        // border glyphs are exempt: they are chrome, not cursor-row content.
        let stray: Vec<_> = (0..buffer.area.width)
            .filter(|col| !matches!(buffer[(*col, y)].symbol(), "│" | "┌" | "┐" | "└" | "┘"))
            .map(|col| (col, buffer[(col, y)].bg))
            .filter(|(_, bg)| *bg != color::SURFACE_ALT)
            .collect();
        assert!(
            stray.is_empty(),
            "every content cell of the cursor row must be `surface_alt`: {stray:?}"
        );
    }

    /// §3.1 assertion 1: exactly one panel wears `border_focus` at a time, every
    /// other panel keeps the muted `border`, and `Tab` moves which panel that
    /// is. The history screen is the one that draws a preview pane beside its
    /// commit list, so it is where the two are both on screen.
    #[test]
    fn one_focused_panel_wears_border_focus_and_the_rest_keep_the_muted_border() {
        let mut app = base_app();
        app.view = View::History;
        app.history.list_state.select(Some(0));

        let on_list = render_buffer(&mut app);
        app.cycle_panel_focus(true);
        let on_preview = render_buffer(&mut app);

        let focused_cells = |buffer: &ratatui::buffer::Buffer| -> Vec<(u16, u16)> {
            (0..buffer.area.height)
                .flat_map(|y| {
                    (0..buffer.area.width)
                        .filter(move |x| buffer[(*x, y)].fg == color::BORDER_FOCUS)
                        .map(move |x| (x, y))
                })
                .collect()
        };
        let muted_cells = |buffer: &ratatui::buffer::Buffer| -> usize {
            (0..buffer.area.height)
                .flat_map(|y| (0..buffer.area.width).map(move |x| (x, y)))
                .filter(|(x, y)| buffer[(*x, *y)].fg == color::BORDER)
                .count()
        };

        let list_focus = focused_cells(&on_list);
        let preview_focus = focused_cells(&on_preview);
        assert!(
            !list_focus.is_empty() && !preview_focus.is_empty(),
            "the focused panel must wear `border_focus` on both panes"
        );
        assert_ne!(
            list_focus, preview_focus,
            "`Tab` must move the `border_focus` border to another panel (§3.1)"
        );
        assert!(
            muted_cells(&on_list) > 0 && muted_cells(&on_preview) > 0,
            "every unfocused panel must keep the muted `border`"
        );
    }

    /// §6.2: Branches / History / Tags are context tabs of the repo workspace,
    /// not pushed screens — so they share the repo crumb with `Repo` instead of
    /// appending `› 分支` / `› 历史` / `› 标签`, which §6.2 forbids.
    #[test]
    fn workspace_context_tabs_share_one_repo_crumb() {
        let mut app = base_app();
        app.repos = vec![RepoInfo {
            name: "GitPulse".to_string(),
            path: std::path::PathBuf::from("/tmp/GitPulse"),
            branch: "main".to_string(),
            dirty: Default::default(),
            ahead: 0,
            behind: 0,
            last_commit: Default::default(),
        }];
        app.current_repo = Some(0);

        app.view = View::Repo;
        let repo_crumb = crumb(&app);
        // The crumb stops at the repository (the repo marker + repo name).
        assert!(
            repo_crumb.contains("GitPulse"),
            "repo crumb must name the repo: {repo_crumb}"
        );
        assert!(
            repo_crumb.contains("我的仓库"),
            "repo crumb must reach the repo list: {repo_crumb}"
        );

        // Every context tab renders that exact same crumb.
        for view in [View::Branches, View::History, View::Tags] {
            app.view = view;
            assert_eq!(
                crumb(&app),
                repo_crumb,
                "context tab {view:?} must share the repo crumb, not push a tail"
            );
        }

        // And none of the four may push a `› <tab>` tail (§6.2 §6.2:298).
        for view in [View::Repo, View::Branches, View::History, View::Tags] {
            app.view = view;
            let c = crumb(&app);
            assert!(
                !c.contains("› 分支") && !c.contains("› 历史") && !c.contains("› 标签"),
                "context tab {view:?} must not push a crumb tail: {c}"
            );
        }
    }

    /// §6.2/§5.2 integration: the `render` dispatcher reserves the context-tag
    /// strip for the four repo-workspace views and leaves the dashboard's layout
    /// untouched (no strip — those tabs are Phase 3).
    #[test]
    fn render_draws_the_context_tag_strip_only_on_workspace_views() {
        for view in [View::Repo, View::Branches, View::History, View::Tags] {
            let mut app = base_app();
            app.view = view;
            app.file_entries = vec![FileEntry {
                path: std::path::PathBuf::from("src/main.rs"),
                state: FileState::Modified,
            }];
            let buffer = render_buffer(&mut app);
            let tags_row = row_text(&buffer, 1);
            assert!(
                tags_row.contains("工作区")
                    && tags_row.contains("分支")
                    && tags_row.contains("历史")
                    && tags_row.contains("标签"),
                "view {view:?} tag strip is missing a label: {tags_row}"
            );
            assert!(
                tags_row.contains('│'),
                "view {view:?} tags must be joined by `│`: {tags_row}"
            );
            assert!(
                crate::ui::components::cells::width(&tags_row) <= 80,
                "view {view:?} tag strip overflows 80 columns: {tags_row}"
            );
        }

        // The dashboard has no context tabs yet, so its row 1 is panel chrome,
        // never the tag strip.
        let mut app = base_app();
        app.view = View::Dashboard;
        let buffer = render_buffer(&mut app);
        assert!(
            !row_text(&buffer, 1).contains("工作区"),
            "dashboard must not draw the context-tag strip"
        );
    }

    /// Positions of every `▌` in the frame, so a test can assert the cursor
    /// exists exactly once and sits at a row start.
    fn cursor_cells(buffer: &ratatui::buffer::Buffer) -> Vec<(u16, u16)> {
        (0..buffer.area.height)
            .flat_map(|y| {
                (0..buffer.area.width)
                    .filter(move |x| buffer[(*x, y)].symbol() == HIGHLIGHT_SYMBOL)
                    .map(move |x| (x, y))
            })
            .collect()
    }

    /// §5 hard rule: **颜色一律走 token，不写死色值**. `components::theme` is the
    /// single place allowed to name a `ratatui::style::Color`; every other
    /// module under `src/ui` reads its hues from `theme::color`, so a theme
    /// change stays one edit and no view can drift into a private palette.
    ///
    /// This is the source-level gate for that rule: it walks `src/ui/**` and
    /// fails on any `Color::` outside `theme.rs`. Test modules are stripped
    /// first (a test may legitimately pin a token's value, and the palette test
    /// inside `theme.rs` must name the hexes), so only production code is
    /// checked.
    #[test]
    fn design_no_bare_color_literals_outside_theme() {
        const THEME: &str = "src/ui/components/theme.rs";
        let ui_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui");

        let mut offenders: Vec<String> = Vec::new();
        let mut checked = 0usize;
        walk_rs_files(&ui_dir, &mut |path| {
            let relative = path
                .strip_prefix(env!("CARGO_MANIFEST_DIR"))
                .expect("ui files live under the manifest dir")
                .to_string_lossy()
                .replace('\\', "/");
            if relative == THEME {
                return;
            }
            let source = fs::read_to_string(path).expect("ui sources are readable");
            checked += 1;
            for (idx, line) in strip_test_modules(&source).lines().enumerate() {
                if line.contains("Color::") {
                    offenders.push(format!("{relative}:{}", idx + 1));
                }
            }
        });

        assert!(
            checked > 0,
            "the gate found no sources to check — it would pass vacuously"
        );
        assert!(
            offenders.is_empty(),
            "§5 forbids naming a `Color::` outside {THEME}; these lines do:\n  {}",
            offenders.join("\n  ")
        );
    }

    /// §2.1 / §2.2: every global/canonical key from DESIGN.md's binding table is
    /// wired into the key dispatch — pinned as a unit so a dropped binding, or a
    /// view quietly shadowing a global verb, fails `cargo test design` instead
    /// of slipping past an individual behavioral test.
    ///
    /// Global verbs (Esc, ? / Ctrl-P, Tab, BackTab, `<`, `>`, r, `/`, g/G/Home/End)
    /// live in `dispatch_key`; the owned-but-delegated ones (Enter, q, j/k/↑↓,
    /// n, d) are dispatched per-view — both sets must exist and stay global.
    #[test]
    fn design_canonical_keybindings_match_design_2_1_and_2_2() {
        let app = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/app.rs");
        let source = strip_test_modules(&fs::read_to_string(app).expect("app.rs is readable"));

        // (canonical §2.2 row, exact production token that binds it). `g` and
        // `G` share one arm, so their token is the combined pattern.
        let canonical: [(&str, &str); 21] = [
            ("quit q", "is_plain_char(key, 'q')"),
            ("refresh r", "KeyCode::Char('r')"),
            ("back Esc", "KeyCode::Esc"),
            ("open Enter", "KeyCode::Enter"),
            ("move down j", "KeyCode::Char('j')"),
            ("move up k", "KeyCode::Char('k')"),
            ("move down ↓", "KeyCode::Down"),
            ("move up ↑", "KeyCode::Up"),
            ("search /", "KeyCode::Char('/')"),
            ("help ?", "KeyCode::Char('?')"),
            ("palette Ctrl-P", "KeyCode::Char('p') if is_ctrl(key)"),
            ("new n", "KeyCode::Char('n')"),
            ("delete d", "KeyCode::Char('d')"),
            ("context <", "KeyCode::Char('<')"),
            ("context >", "KeyCode::Char('>')"),
            ("focus Tab", "KeyCode::Tab"),
            ("focus BackTab", "KeyCode::BackTab"),
            ("goto g", "KeyCode::Char('g')"),
            ("goto G", "'g' | 'G'"),
            ("goto Home", "KeyCode::Home"),
            ("goto End", "KeyCode::End"),
        ];

        let missing: Vec<&str> = canonical
            .iter()
            .filter(|(_, token)| !source.contains(token))
            .map(|(label, _)| *label)
            .collect();

        assert!(
            missing.is_empty(),
            "§2.2 canonical keys missing from app.rs dispatch: {}",
            missing.join(", ")
        );

        // §2.1: a modal (or overlay) must own the keyboard before any global
        // verb is evaluated — the trap is the guard, the literal-input test
        // downstream pins its behavior.
        assert!(
            source.contains("if self.active_modal().is_some()")
                && source.contains("fn dispatch_modal_key"),
            "§2.1 requires the modal focus trap to gate global dispatch"
        );
    }

    /// Depth-first walk of every `.rs` file under `dir`, calling `visit` on
    /// each. `read_dir` order is unspecified, so nothing here may assume an
    /// order.
    fn walk_rs_files(dir: &Path, visit: &mut impl FnMut(&Path)) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk_rs_files(&path, visit);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                visit(&path);
            }
        }
    }

    /// `source` with every `#[cfg(test)] mod … { … }` block removed, so the
    /// color gate reads production code only.
    ///
    /// Deliberately a scanner rather than a parser, and deliberately pragmatic:
    /// an `#[cfg(test)]` is only treated as a test module when the line it
    /// decorates starts with `mod` (`help.rs` gates a single `use` on it), and
    /// the block ends at the brace that balances its opening one — a
    /// string/char/comment-aware scan, because a `{` inside `"{}"` in an
    /// assertion message would otherwise close the block early.
    fn strip_test_modules(source: &str) -> String {
        let mut kept = String::with_capacity(source.len());
        let lines: Vec<&str> = source.lines().collect();
        let mut idx = 0usize;
        while idx < lines.len() {
            let line = lines[idx];
            let decorated_mod = line.trim_start().starts_with("#[cfg(test)]")
                && lines[idx + 1..]
                    .iter()
                    .find(|l| !l.trim_start().starts_with('#'))
                    .is_some_and(|l| l.trim_start().starts_with("mod "));
            if !decorated_mod {
                kept.push_str(line);
                kept.push('\n');
                idx += 1;
                continue;
            }
            idx += 1;
            let mut depth = 0i32;
            let mut in_string = false;
            let mut escaped = false;
            while idx < lines.len() {
                let current = lines[idx];
                idx += 1;
                for ch in current.chars() {
                    if escaped {
                        escaped = false;
                        continue;
                    }
                    match ch {
                        '\\' if in_string => escaped = true,
                        '"' => in_string = !in_string,
                        '{' if !in_string => depth += 1,
                        '}' if !in_string => depth -= 1,
                        _ => {}
                    }
                }
                if depth <= 0 {
                    break;
                }
            }
        }
        kept
    }

    /// The gate reads production code, so its own fixtures must not leak into
    /// the checked sources: a synthetic `Color::` line has to be reported (and
    /// a `#[cfg(test)]` line must be stripped) or the gate proves nothing.
    #[test]
    fn design_color_gate_scanner_isolates_production_code() {
        let source = "\
const A: u8 = 1;
#[cfg(test)]
mod tests {
    const IN_TEST: u8 = 2;
    #[test]
    fn t() {
        assert!(IN_TEST == 2, \"braces {} in a message\");
    }
}
const B: u8 = 3;
";
        let stripped = strip_test_modules(source);
        assert!(
            !stripped.contains("IN_TEST"),
            "the test module must be stripped: {stripped}"
        );
        assert!(
            !stripped.contains("mod tests"),
            "the test module header must be stripped: {stripped}"
        );
        assert!(
            stripped.contains("const A") && stripped.contains("const B"),
            "production lines on both sides must survive: {stripped}"
        );
    }
}
