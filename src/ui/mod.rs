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
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::app::{App, View};
use crate::git::{FileState, RepoInfo};
use crate::ui::components::footer::hints as join_hints;
use crate::ui::components::footer::{self, HintRow, StatusRow};
use crate::ui::dashboard::DashboardState;
use crate::ui::status::{RemoteStatus, StatusLine};

/// Key/desc fragment of every hint row, re-exported from the footer so the
/// views keep building their segments through `super::hint`.
pub(crate) use crate::ui::components::footer::hint;

/// Shared selection style for every list: dark background + bold keeps the
/// cursor row readable on any block style.
pub(crate) fn list_highlight() -> Style {
    Style::default()
        .bg(Color::DarkGray)
        .add_modifier(Modifier::BOLD)
}

/// Marker rendered before the selected list row.
pub(crate) const HIGHLIGHT_SYMBOL: &str = "► ";

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

/// Draw the status row of the footer (§4.2): errors render red and persist
/// until the next operation, anything else keeps the default style. `None`
/// leaves the row blank.
///
/// Called through [`footer::render_footer`] — views never draw the row
/// themselves.
pub fn render_status_line(f: &mut Frame, area: Rect, status: Option<&StatusLine>) {
    if let Some(status) = status {
        let style = if status.is_error {
            Style::default().fg(Color::Red)
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

/// Panel skeleton shared by the two named-item lists — the commit history
/// and the tag list (G7): a bordered block titled `label [repo] (count)`
/// around either the rows the caller built or, when there are none, the
/// panel's own empty-state copy.
///
/// Zero rows is a valid state of both panels (an unborn HEAD, a repository
/// without tags) rather than a failure, so the empty copy is rendered as
/// data and the error status — drawn by the caller — still covers failed
/// loads. The rows are handed in already built because each panel maps its
/// own item type to a [`ListItem`]; only the block, the highlight style and
/// the highlight symbol are shared.
pub(crate) fn render_named_list(
    f: &mut Frame,
    area: Rect,
    label: &str,
    repo_name: &str,
    empty: &'static str,
    items: Vec<ListItem<'static>>,
    list_state: &mut ListState,
) {
    let block = Block::default()
        .title(format!("{label} [{repo_name}] ({})", items.len()))
        .borders(Borders::ALL);

    if items.is_empty() {
        let paragraph =
            Paragraph::new(Span::styled(empty, Style::default().fg(Color::DarkGray))).block(block);
        f.render_widget(paragraph, area);
        return;
    }

    let list = List::new(items)
        .block(block)
        .highlight_style(list_highlight())
        .highlight_symbol(HIGHLIGHT_SYMBOL);
    f.render_stateful_widget(list, area, list_state);
}

/// Symbol and color of a working-tree [`FileState`] in the file list (G11).
///
/// The mapping is git semantics — which state a status flag means and which
/// color stands for it — so it lives with the shared render helpers rather
/// than in the closure that builds the rows; the repo panel only composes
/// the styled spans. The symbol is two cells (`M `) so the path columns of
/// every row line up.
pub(crate) fn file_state_symbol(state: FileState) -> (&'static str, Color) {
    match state {
        FileState::Modified => ("M ", Color::Yellow),
        FileState::Added => ("A ", Color::Green),
        FileState::Deleted => ("D ", Color::Red),
        FileState::Untracked => ("? ", Color::Cyan),
        FileState::Conflicted => ("U ", Color::Magenta),
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
        .style(Style::default().fg(Color::Red));

    let mut lines = body;
    lines.push(Line::from(""));
    lines.push(
        [hint("y", "确认"), hint("n", "取消")]
            .into_iter()
            .flatten()
            .collect::<Line>(),
    );

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
/// `title` showing `text` after a cyan `> ` prompt. `show_cursor` appends the
/// `▌` caret (config manual input); popups whose hints live in the title
/// (branch create, commit message) omit it. The line scrolls horizontally by
/// the display width that does not fit so the tail — and with it the caret —
/// always stays inside the popup instead of being cut off.
pub fn render_input_overlay(f: &mut Frame, area: Rect, title: &str, text: &str, show_cursor: bool) {
    let popup = repo::centered_rect(60, 5, area);
    f.render_widget(Clear, popup);

    let mut spans = vec![
        Span::styled("> ", Style::default().fg(Color::Cyan)),
        Span::raw(text),
    ];
    if show_cursor {
        spans.push(Span::styled("▌", Style::default().fg(Color::Cyan)));
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
    let (content, footer_area) = footer::split_area(f.area());

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

/// Footer status row for the current view (§4.2).
///
/// Receipt views without a GitHub client omit the local status from the row
/// because their body — [`pulls::render_disconnected`] — already promotes it
/// into the content area; every other receipt paints the full status plus any
/// in-flight remote feedback. `Repo` / `Tags` are local-git views that own
/// remote feedback, so they always carry the remote triple. Everything else
/// (dashboard / branches / history / config / diffs / commit) takes the plain
/// status line.
pub(crate) fn view_status(app: &App) -> StatusRow<'_> {
    match app.view {
        View::Pulls
        | View::PullDetail
        | View::Issues
        | View::IssueDetail
        | View::Checks
        | View::Notifications => {
            if app.github_state.client.is_some() {
                StatusRow::new(
                    app.status.as_ref(),
                    app.remote_status.as_ref(),
                    app.remote_op,
                )
            } else {
                StatusRow::plain(None)
            }
        }
        View::Repo | View::Tags | View::Commit => StatusRow::new(
            app.status.as_ref(),
            app.remote_status.as_ref(),
            app.remote_op,
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
        View::Repo => repo::repo_hints(app.show_discard_confirm, false),
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
        View::Pulls => pulls::receipt_hints(true),
        View::PullDetail => pulls::pull_detail_hints(),
        View::Checks => pulls::receipt_hints(false),
        View::Issues => pulls::receipt_hints(true),
        View::IssueDetail => pulls::issue_detail_hints(),
        View::Notifications => pulls::receipt_hints(false),
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
    let mut state = DashboardState {
        repos: &app.repos,
        status: app.status.as_ref(),
        list_state: &mut app.dashboard_state,
        batch_log: &app.batch_log,
        batch_op: app.batch_op,
        batch_running,
    };
    dashboard::render_dashboard(f, content, &mut state);
}

/// Repo arm of [`render`]: the file list plus the discard confirmation.
fn render_repo_view(f: &mut Frame, content: Rect, app: &mut App) {
    let sel = app.file_state.selected();
    let confirm_file = if app.show_discard_confirm {
        sel.and_then(|i| app.file_entries.get(i))
    } else {
        None
    };
    let state = repo::RepoState {
        entries: &app.file_entries,
        status: app.status.as_ref(),
        list_state: &mut app.file_state,
        show_discard_confirm: app.show_discard_confirm,
        confirm_file,
        remote_status: app.remote_status.as_ref(),
        remote_op: app.remote_op,
        show_commit_input: false,
    };
    repo::render_repo(f, content, state);
}

/// Commit arm of [`render`]: repo view as background, commit input popup on
/// top.
fn render_commit_view(f: &mut Frame, content: Rect, app: &mut App) {
    let state = repo::RepoState {
        entries: &app.file_entries,
        status: app.status.as_ref(),
        list_state: &mut app.file_state,
        show_discard_confirm: false,
        confirm_file: None,
        remote_status: app.remote_status.as_ref(),
        remote_op: app.remote_op,
        show_commit_input: true,
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
    let state = branches::BranchesState {
        branches: &app.branches,
        status: app.status.as_ref(),
        list_state: &mut app.branch_state,
        repo_name: &repo_name,
        show_input: app.show_branch_input,
        input: &app.branch_input,
        show_delete_confirm: app.show_branch_delete_confirm,
        confirm_branch: confirm,
    };
    branches::render_branches(f, content, state);
}

/// Config panel arm of [`render`].
fn render_config_view(f: &mut Frame, content: Rect, app: &mut App) {
    // Field-level borrow of `app.config` coexists with the
    // `&mut app.config_state` below (disjoint fields).
    let state = config::ConfigState {
        scan_dirs: &app.config.scan_dirs,
        status: app.status.as_ref(),
        list_state: &mut app.config_state,
        show_input: app.show_config_input,
        input: &app.config_input,
        show_delete_confirm: app.show_config_delete_confirm,
        show_picker: app.show_config_picker,
        picker_dir: &app.picker_dir,
        picker_entries: &app.picker_entries,
        picker_state: &mut app.picker_state,
    };
    config::render_config(f, content, state);
}

/// History arm of [`render`].
///
/// Field-level borrows of `app.history` (data vs. cursor), `app.repos` (the
/// title) and the status line coexist (disjoint fields of `App`).
fn render_history_view(f: &mut Frame, content: Rect, app: &mut App) {
    let repo_name = repo_name_of(&app.repos, app.current_repo);
    let state = history::HistoryRender {
        commits: &app.history.commits,
        detail: app.history.detail.as_ref(),
        graph: &app.history.graph,
        list_state: &mut app.history.list_state,
        selected: app.history.selected,
        repo_name,
        status: app.status.as_ref(),
    };
    history::render_history(f, content, state);
}

/// Tags arm of [`render`].
///
/// Field-level borrows of `app.tags_state` (cursor vs. list), `app.repos`
/// (the title) and the status fields coexist (disjoint fields of `App`).
fn render_tags_view(f: &mut Frame, content: Rect, app: &mut App) {
    let repo_name = repo_name_of(&app.repos, app.current_repo);
    let state = tags::TagsRender {
        tags: &app.tags_state.tags,
        list_state: &mut app.tags_state.list_state,
        modal: app.tags_state.modal.as_ref(),
        status: app.status.as_ref(),
        remote_status: app.remote_status.as_ref(),
        remote_op: app.remote_op,
        repo_name,
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

    fn base_app() -> App {
        App::new(Config::default())
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

    #[test]
    fn dashboard_view_renders_title_for_empty_repo_list() {
        let mut app = base_app();
        let text = render_compact(&mut app);
        assert!(
            text.contains("GitPulse仓库状态(0)"),
            "missing dashboard title: {text}"
        );
    }

    #[test]
    fn hint_row_keeps_description_in_default_color() {
        let mut app = base_app();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let hint_row = buffer.area.height - 1;
        let mut key_fg = None;
        let mut desc_fg = None;
        for x in 0..buffer.area.width {
            let cell = &buffer[(x, hint_row)];
            match cell.symbol() {
                "j" if key_fg.is_none() => key_fg = Some(cell.fg),
                "选" => desc_fg = Some(cell.fg),
                _ => {}
            }
        }
        assert_eq!(key_fg, Some(Color::Cyan), "hint key must stay cyan");
        assert_eq!(
            desc_fg,
            Some(Color::Reset),
            "hint description must keep the default color"
        );
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
    /// carrying the footer's global key hints plus the current view's segment.
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
        assert!(text.contains("Enter进入"), "missing view hint: {text}");
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
}
