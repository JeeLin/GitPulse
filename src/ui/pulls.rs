//! Pull-request hub: the open-PR list and the detail panel, plus the small
//! helpers the sibling receipt panels (checks / issues / notifications)
//! share — panel titles, row fitting, the status/hint footer and the check
//! palette.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use super::{hint, list_highlight, HIGHLIGHT_SYMBOL};
use crate::app::App;
use crate::github::{CheckSummary, PrDetail, PrSummary};

/// Empty-state copy of the pull list: zero open pulls is a valid state, not
/// a failure — the status row below covers failed loads.
const EMPTY_PULLS: &str = "—（暂无 PR）";

/// Placeholder of the detail view while its payload has not arrived yet.
const DETAIL_PLACEHOLDER: &str = "按 Enter 查看详情 · Esc 返回";

/// Columns a list row gives up to the block border (2) and the highlight
/// symbol (2): [`List`] indents every line of the selected item by
/// [`HIGHLIGHT_SYMBOL`], so rows are fitted to what remains.
const ROW_GUTTER: u16 = 4;

/// Blank columns between a row's head and its trailing meta part.
pub(crate) const META_GAP: &str = "  ";

/// Cells a row keeps for its head (`#12 title (open)`) before the trailing
/// meta part is dropped entirely: identification wins over metadata.
const MIN_HEAD_WIDTH: usize = 16;

/// Check rows drawn in the detail panel before the sub-list is capped, so a
/// PR with thirty runs cannot eat the description pane.
const MAX_DETAIL_CHECKS: usize = 10;

/// Display width of `text` in terminal cells (a CJK glyph costs 2), the unit
/// ratatui lays out and clips rows with.
pub(crate) fn cells(text: &str) -> usize {
    Line::from(text).width()
}

/// Cut `text` to `max` display cells, marking the cut with a trailing `…` so
/// a truncated row reads as truncated instead of silently ending.
pub(crate) fn ellipsize(text: &str, max: usize) -> String {
    if cells(text) <= max {
        return text.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let budget = max - 1; // one cell for the ellipsis itself
    let mut out = String::new();
    let mut used = 0usize;
    for c in text.chars() {
        let mut buf = [0u8; 4];
        let cw = cells(c.encode_utf8(&mut buf));
        if used + cw > budget {
            break;
        }
        used += cw;
        out.push(c);
    }
    out.push('…');
    out
}

/// Cells of a `width`-wide row the trailing meta part (`author  labels  time`)
/// may occupy: everything it needs when there is room, else at least a third
/// of the row (never a sliver), and nothing at all on very narrow rows — the
/// head then uses the full width.
pub(crate) fn meta_budget(meta: &str, width: usize) -> usize {
    let needed = cells(meta);
    if needed == 0 {
        return 0;
    }
    if needed + cells(META_GAP) <= width {
        return needed;
    }
    let target = (width / 3).max(12).min(needed);
    let target = target.min(width.saturating_sub(MIN_HEAD_WIDTH + cells(META_GAP)));
    if target >= 8 {
        target
    } else {
        0
    }
}

/// Panel title: `label · owner/repo`, falling back to `—` while the remote
/// pair is unknown (a failed hub entry never fills it in).
pub(crate) fn panel_title(label: &str, owner: &str, repo: &str) -> String {
    if owner.is_empty() || repo.is_empty() {
        format!("{label} · —")
    } else {
        format!("{label} · {owner}/{repo}")
    }
}

/// Cursor for a `len`-row list: `None` for an empty list, otherwise the
/// requested index clamped to the last row. `App::github_list_len` is private
/// to the app module, so the panels clamp against the vec they display — the
/// same `saturating_sub(1)` the app-side clamp applies.
pub(crate) fn list_cursor(selected: usize, len: usize) -> Option<usize> {
    if len == 0 {
        None
    } else {
        Some(selected.min(len.saturating_sub(1)))
    }
}

/// Cells a list row may use inside `area`: minus the block borders and the
/// highlight symbol.
pub(crate) fn row_width(area: Rect) -> usize {
    area.width.saturating_sub(ROW_GUTTER) as usize
}

/// Cells a paragraph row may use inside `area`'s block: minus the borders
/// only (no highlight symbol on paragraph rows).
pub(crate) fn block_width(area: Rect) -> usize {
    area.width.saturating_sub(2) as usize
}

/// Palette of a pull/issue state: open reads green, closed red, any other
/// value (GitHub keeps inventing them) falls back to dark gray.
pub(crate) fn state_style(state: &str) -> Style {
    match state {
        "open" => Style::default().fg(Color::Green),
        "closed" => Style::default().fg(Color::Red),
        _ => Style::default().fg(Color::DarkGray),
    }
}

/// Palette of a check conclusion: failures red, success green, the muted
/// outcomes gray, and anything unclassified plain.
pub(crate) fn conclusion_style(conclusion: &str) -> Style {
    match conclusion {
        "success" => Style::default().fg(Color::Green),
        "failure" | "timed_out" | "action_required" => Style::default().fg(Color::Red),
        "neutral" | "skipped" | "cancelled" => Style::default().fg(Color::Gray),
        _ => Style::default(),
    }
}

/// Palette of a check status: work in flight (queued / in_progress) reads
/// yellow, a settled run gray — the conclusion column then carries verdict.
pub(crate) fn check_status_style(status: &str) -> Style {
    match status {
        "in_progress" | "queued" => Style::default().fg(Color::Yellow),
        "completed" => Style::default().fg(Color::Gray),
        _ => Style::default().fg(Color::DarkGray),
    }
}

/// Mergeability of the detail view: `None` means GitHub has not computed it
/// yet, so the label says so instead of staying silent.
pub(crate) fn mergeable_label(mergeable: Option<bool>) -> (&'static str, Style) {
    match mergeable {
        Some(true) => ("可合并", Style::default().fg(Color::Green)),
        Some(false) => ("有冲突", Style::default().fg(Color::Red)),
        None => ("未知", Style::default().fg(Color::Yellow)),
    }
}

/// One `name  status  conclusion` check row fitted to `width` display cells;
/// an empty column reads `—` instead of vanishing.
pub(crate) fn check_line(check: &CheckSummary, width: usize) -> Line<'static> {
    let status = column(&check.status);
    let conclusion = column(&check.conclusion);
    let used = cells(&status) + cells(&conclusion) + 2 * cells(META_GAP);
    Line::from(vec![
        Span::raw(ellipsize(&check.name, width.saturating_sub(used))),
        Span::styled(format!("{META_GAP}{status}"), check_status_style(&status)),
        Span::styled(
            format!("{META_GAP}{conclusion}"),
            conclusion_style(&conclusion),
        ),
    ])
}

/// One check column, with an empty value rendered as an em dash.
fn column(value: &str) -> String {
    if value.trim().is_empty() {
        "—".to_string()
    } else {
        value.to_string()
    }
}

/// Bottom hint row of every receipt panel: hub entry, the four tabs, the
/// refresh and the two ways out.
///
/// `Enter 详情` is only useful on the Pulls and Issues *lists* (where it opens
/// the selected item's detail); pass `show_enter = true` there and `false` for
/// the Checks / Notifications / detail views, where Enter is a no-op.
pub(crate) fn receipt_hints(show_enter: bool) -> Line<'static> {
    let mut hints: Vec<Line<'static>> = vec![
        hint("g", "进入"),
        hint("p", "PR"),
        hint("c", "checks"),
        hint("i", "issues"),
        hint("n", "通知"),
        hint("r", "刷新"),
        hint("q/Esc", "返回"),
    ];
    if show_enter {
        hints.push(hint("Enter", "详情"));
    }
    hints.into_iter().flatten().collect()
}

/// Readiness-gate client for the panel tests.
///
/// `GithubClient::new` only resolves a token (the environment wins) and
/// builds an octocrab instance — but octocrab spawns its tower buffer task,
/// which needs a Tokio context, so the build runs inside a throwaway
/// runtime. That task is never driven: these tests only render a frame, they
/// never issue a request.
#[cfg(test)]
pub(crate) fn offline_client() -> crate::github::GithubClient {
    if std::env::var("GITHUB_TOKEN").is_err() {
        std::env::set_var("GITHUB_TOKEN", "test-token");
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let _guard = runtime.enter();
    crate::github::GithubClient::new("octo", "demo").expect("offline client")
}

/// Vertical split every receipt panel uses: body, status row, hint row.
pub(crate) fn panel_chunks(area: Rect) -> std::rc::Rc<[Rect]> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),    // panel body
            Constraint::Length(1), // status line
            Constraint::Length(1), // key hints
        ])
        .split(area)
}

/// Bottom status and key-hint rows of every receipt panel (convention 3).
///
/// While no client is ready the body already carries the status/error line
/// (see [`render_disconnected`]), so the row stays blank rather than printing
/// the same message twice.
pub(crate) fn render_footer(
    f: &mut Frame,
    status_area: Rect,
    hints_area: Rect,
    app: &App,
    show_enter: bool,
) {
    if app.github_state.client.is_some() {
        super::render_status_or_remote(
            f,
            status_area,
            app.remote_status.as_ref(),
            app.remote_op,
            app.status.as_ref(),
        );
    }
    f.render_widget(Paragraph::new(receipt_hints(show_enter)), hints_area);
}

/// Panel body shown while the GitHub client is not ready: the status or
/// error line promoted into the content area, above the retry guidance,
/// instead of a list that could never load.
pub(crate) fn render_disconnected(f: &mut Frame, area: Rect, label: &str, app: &App) {
    let block = Block::default()
        .title(panel_title(
            label,
            &app.github_state.owner,
            &app.github_state.repo,
        ))
        .borders(Borders::ALL);

    let mut lines = vec![
        Line::from(Span::styled(
            "尚未连接 GitHub",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "配置 GITHUB_TOKEN 或 gh 登录后按 r 重试",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
    ];
    if let Some(status) = &app.status {
        let style = if status.is_error {
            Style::default().fg(Color::Red)
        } else {
            Style::default().fg(Color::Gray)
        };
        lines.push(Line::from(Span::styled(status.message.clone(), style)));
    }
    f.render_widget(Paragraph::new(lines).block(block), area);
}

/// Render the open-pull list (the receipts hub entry point).
pub fn render_pulls(f: &mut Frame, app: &App) {
    let area = f.area();
    let chunks = panel_chunks(area);
    if app.github_state.client.is_none() {
        render_disconnected(f, chunks[0], "PR 列表", app);
        render_footer(f, chunks[1], chunks[2], app, true);
        return;
    }

    let pulls = &app.github_state.pulls;
    let block = Block::default()
        .title(panel_title(
            "PR 列表",
            &app.github_state.owner,
            &app.github_state.repo,
        ))
        .borders(Borders::ALL);

    if pulls.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(
                EMPTY_PULLS,
                Style::default().fg(Color::DarkGray),
            ))
            .block(block),
            chunks[0],
        );
    } else {
        let width = row_width(chunks[0]);
        let items: Vec<ListItem> = pulls.iter().map(|pr| pull_item(pr, width)).collect();
        let mut state = ListState::default();
        state.select(list_cursor(app.github_state.selected, pulls.len()));
        let list = List::new(items)
            .block(block)
            .highlight_style(list_highlight())
            .highlight_symbol(HIGHLIGHT_SYMBOL);
        f.render_stateful_widget(list, chunks[0], &mut state);
    }
    render_footer(f, chunks[1], chunks[2], app, true);
}

/// One pull row: `#N title (state)` plus a `草稿` badge when the pull is a
/// draft, over a dark second line with author, branch pair and update time.
fn pull_item(pr: &PrSummary, width: usize) -> ListItem<'static> {
    let number = format!("#{} ", pr.number);
    let state_tag = format!(" ({})", pr.state);
    let draft_tag = if pr.draft {
        " [草稿]".to_string()
    } else {
        String::new()
    };
    let fixed = cells(&number) + cells(&state_tag) + cells(&draft_tag);
    let head = Line::from(vec![
        Span::styled(number, Style::default().fg(Color::Cyan)),
        Span::raw(ellipsize(&pr.title, width.saturating_sub(fixed))),
        Span::styled(state_tag, state_style(&pr.state)),
        Span::styled(draft_tag, Style::default().fg(Color::Yellow)),
    ]);

    let meta = format!(
        "{}  {}→{}  {}",
        pr.author, pr.head_ref, pr.base_ref, pr.updated_at
    );
    let tail = Line::from(Span::styled(
        ellipsize(&meta, width),
        Style::default().fg(Color::DarkGray),
    ));
    ListItem::new(vec![head, tail])
}

/// Render the detail of the highlighted pull request: the bordered shell,
/// the placeholder while its payload has not arrived, and the three panes.
pub fn render_pull_detail(f: &mut Frame, app: &App) {
    let area = f.area();
    let chunks = panel_chunks(area);
    if app.github_state.client.is_none() {
        render_disconnected(f, chunks[0], "PR 详情", app);
        render_footer(f, chunks[1], chunks[2], app, false);
        return;
    }

    let block = Block::default()
        .title(panel_title(
            "PR 详情",
            &app.github_state.owner,
            &app.github_state.repo,
        ))
        .borders(Borders::ALL);

    let Some(detail) = app.github_state.detail.as_ref() else {
        f.render_widget(
            Paragraph::new(Span::styled(
                DETAIL_PLACEHOLDER,
                Style::default().fg(Color::DarkGray),
            ))
            .block(block),
            chunks[0],
        );
        render_footer(f, chunks[1], chunks[2], app, false);
        return;
    };

    let inner = block.inner(chunks[0]);
    f.render_widget(block, chunks[0]);
    render_detail_panes(f, detail, inner);
    render_footer(f, chunks[1], chunks[2], app, false);
}

/// Split the detail shell into its three panes — summary head, wrapped
/// description, check sub-list — and draw each one.
fn render_detail_panes(f: &mut Frame, detail: &PrDetail, inner: Rect) {
    // The check sub-list is capped at half the pane so a PR with many runs
    // cannot push the description out of view.
    let checks_h = (detail.checks.len().min(MAX_DETAIL_CHECKS) as u16 + 2)
        .min(inner.height.div_ceil(2))
        .max(3);
    let panes = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),        // summary head + spacer
            Constraint::Min(3),           // wrapped description
            Constraint::Length(checks_h), // check sub-list
        ])
        .split(inner);

    // The summary head sits directly on the pane (no block of its own), so
    // it uses the full inner width.
    f.render_widget(
        Paragraph::new(summary_lines(detail, panes[0].width as usize)),
        panes[0],
    );
    f.render_widget(body_paragraph(detail), panes[1]);
    render_checks_pane(f, detail, panes[2]);
}

/// The pull's markdown body inside its `描述` block, soft-wrapped without
/// trimming the leading whitespace markdown relies on; an empty body renders
/// the empty-state copy instead of a blank pane.
fn body_paragraph(detail: &PrDetail) -> Paragraph<'static> {
    let body: Vec<Line> = if detail.body.trim().is_empty() {
        vec![Line::from(Span::styled(
            "（无描述）",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        detail
            .body
            .lines()
            .map(|l| Line::from(l.to_string()))
            .collect()
    };
    Paragraph::new(body)
        .block(Block::default().title("描述").borders(Borders::ALL))
        .wrap(Wrap { trim: false })
}

/// The `检查 (n)` sub-list: one `name  status  conclusion` row per run, or
/// the empty-state copy while the pull carries no runs.
fn render_checks_pane(f: &mut Frame, detail: &PrDetail, area: Rect) {
    let block = Block::default()
        .title(format!("检查 ({})", detail.checks.len()))
        .borders(Borders::ALL);
    if detail.checks.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(
                "—（暂无检查记录）",
                Style::default().fg(Color::DarkGray),
            ))
            .block(block),
            area,
        );
    } else {
        let width = block_width(area);
        let lines: Vec<Line> = detail
            .checks
            .iter()
            .map(|check| check_line(check, width))
            .collect();
        f.render_widget(Paragraph::new(lines).block(block), area);
    }
}

/// Summary head of the detail view: `#N title (state)` over a line with the
/// mergeability label and the pull's metadata.
fn summary_lines(detail: &PrDetail, width: usize) -> Vec<Line<'static>> {
    let summary = &detail.summary;
    let number = format!("#{} ", summary.number);
    let state_tag = format!(" ({})", summary.state);
    let draft_tag = if summary.draft {
        " [草稿]".to_string()
    } else {
        String::new()
    };
    let fixed = cells(&number) + cells(&state_tag) + cells(&draft_tag);
    let head = Line::from(vec![
        Span::styled(number, Style::default().fg(Color::Cyan)),
        Span::raw(ellipsize(&summary.title, width.saturating_sub(fixed))),
        Span::styled(state_tag, state_style(&summary.state)),
        Span::styled(draft_tag, Style::default().fg(Color::Yellow)),
    ]);

    let (merge, merge_style) = mergeable_label(detail.mergeable);
    let meta = format!(
        " · {} · {}→{} · {}",
        summary.author, summary.head_ref, summary.base_ref, summary.updated_at
    );
    let tail = Line::from(vec![
        Span::styled(merge.to_string(), merge_style),
        Span::styled(
            ellipsize(&meta, width.saturating_sub(cells(merge))),
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    vec![head, tail, Line::from("")]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::config::Config;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// Connected app: `owner`/`repo` filled in and the readiness gate built.
    fn connected_app() -> App {
        let mut app = App::new(Config::default());
        app.github_state.owner = "octo".to_string();
        app.github_state.repo = "demo".to_string();
        app.github_state.client = Some(offline_client());
        app
    }

    fn pull(number: u64, title: &str, draft: bool) -> PrSummary {
        PrSummary {
            number,
            title: title.to_string(),
            state: "open".to_string(),
            draft,
            author: "alice".to_string(),
            head_ref: "feat/login".to_string(),
            base_ref: "main".to_string(),
            updated_at: "2026-01-02T03:04:05Z".to_string(),
        }
    }

    /// Whitespace-stripped frame text (wide CJK glyphs leave padding cells).
    fn render_text(app: &App, draw: fn(&mut Frame, &App)) -> String {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
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
    fn pulls_list_shows_title_rows_and_hints() {
        let mut app = connected_app();
        app.github_state.pulls = vec![pull(42, "Fix login bug", false), pull(7, "WIP thing", true)];
        app.github_state.selected = 0;
        let text = render_text(&app, render_pulls);
        assert!(text.contains("PR列表·octo/demo"), "missing title: {text}");
        assert!(text.contains("#42Fixloginbug(open)"), "missing row: {text}");
        assert!(text.contains("[草稿]"), "missing draft badge: {text}");
        assert!(
            text.contains("alicefeat/login→main2026-01-02T03:04:05Z"),
            "missing meta line: {text}"
        );
        assert!(text.contains("g进入"), "missing entry hint: {text}");
        assert!(text.contains("Esc返回"), "missing back hint: {text}");
        assert!(text.contains("Enter详情"), "missing detail hint: {text}");
    }

    #[test]
    fn pulls_list_clamps_a_stale_selection_to_the_last_row() {
        let mut app = connected_app();
        app.github_state.pulls = vec![pull(1, "one", false), pull(2, "two", false)];
        app.github_state.selected = 99;
        let text = render_text(&app, render_pulls);
        assert!(text.contains("#2"), "last row must stay selected: {text}");
        assert_eq!(list_cursor(99, 2), Some(1));
        assert_eq!(list_cursor(0, 0), None, "empty list has no cursor");
    }

    #[test]
    fn pulls_list_truncates_a_long_title_with_ellipsis() {
        let mut app = connected_app();
        app.github_state.pulls = vec![pull(9, &"长".repeat(60), false)];
        let text = render_text(&app, render_pulls);
        assert!(text.contains('…'), "long title must be cut: {text}");
        // The meta line keeps its head on the second row, truncation or not.
        assert!(text.contains("alice"), "meta line missing: {text}");
    }

    #[test]
    fn pulls_list_shows_empty_copy_for_zero_rows() {
        let app = connected_app();
        let text = render_text(&app, render_pulls);
        assert!(text.contains("—（暂无PR）"), "missing empty copy: {text}");
    }

    #[test]
    fn pulls_disconnected_promotes_the_status_line_into_the_body() {
        let mut app = App::new(Config::default());
        app.set_error("打开 GitHub 回执失败: 未打开仓库");
        app.github_state.pulls = vec![pull(1, "ghost", false)];
        let text = render_text(&app, render_pulls);
        assert!(text.contains("尚未连接GitHub"), "missing gate copy: {text}");
        assert!(
            text.contains("未打开仓库"),
            "status line must be prominent: {text}"
        );
        assert!(
            !text.contains("#1ghost"),
            "content must not render without a client: {text}"
        );
        assert!(text.contains("r刷新"), "hints must stay: {text}");
    }

    #[test]
    fn pull_detail_placeholder_waits_for_the_payload() {
        let app = connected_app();
        let text = render_text(&app, render_pull_detail);
        assert!(
            text.contains("按Enter查看详情·Esc返回"),
            "missing placeholder: {text}"
        );
    }

    #[test]
    fn pull_detail_renders_summary_body_and_checks() {
        let mut app = connected_app();
        app.github_state.detail = Some(PrDetail {
            summary: pull(42, "Fix login bug", false),
            body: "Root cause and fix go here.".to_string(),
            labels: Vec::new(),
            mergeable: Some(false),
            checks: vec![CheckSummary {
                name: "unit".to_string(),
                status: "completed".to_string(),
                conclusion: "failure".to_string(),
                html_url: "https://example.invalid".to_string(),
            }],
        });
        let text = render_text(&app, render_pull_detail);
        assert!(text.contains("PR详情·octo/demo"), "missing title: {text}");
        assert!(
            text.contains("#42Fixloginbug(open)"),
            "missing summary: {text}"
        );
        assert!(text.contains("有冲突"), "missing mergeability: {text}");
        assert!(
            text.contains("Rootcauseandfixgohere."),
            "missing body: {text}"
        );
        assert!(text.contains("检查(1)"), "missing checks block: {text}");
        assert!(
            text.contains("unitcompletedfailure"),
            "missing check row: {text}"
        );
    }

    #[test]
    fn pull_detail_marks_an_empty_body_and_empty_checks() {
        let mut app = connected_app();
        app.github_state.detail = Some(PrDetail {
            summary: pull(3, "Bare", true),
            body: String::new(),
            labels: Vec::new(),
            mergeable: None,
            checks: Vec::new(),
        });
        let text = render_text(&app, render_pull_detail);
        assert!(text.contains("（无描述）"), "missing body copy: {text}");
        assert!(
            text.contains("—（暂无检查记录）"),
            "missing checks copy: {text}"
        );
        assert!(text.contains("未知"), "uncomputed mergeability: {text}");
        assert!(text.contains("[草稿]"), "missing draft badge: {text}");
    }

    #[test]
    fn ellipsize_cuts_by_display_width_not_char_count() {
        assert_eq!(ellipsize("hello", 10), "hello", "fits, stays verbatim");
        assert_eq!(ellipsize("hello", 5), "hello", "exact fit stays verbatim");
        assert_eq!(ellipsize("hello", 4), "hel…", "tail is cut, not the head");
        assert_eq!(
            ellipsize("中文标题", 5),
            "中文…",
            "wide glyphs count two, so only two fit under the mark"
        );
        assert_eq!(ellipsize("中文标题", 4), "中…", "one wide glyph + the mark");
        assert_eq!(ellipsize("hello", 0), "", "no budget, no text");
        assert_eq!(ellipsize("hello", 1), "…", "one cell only holds the mark");
    }

    #[test]
    fn meta_budget_keeps_meta_readable_and_head_intact() {
        // Everything fits: the meta gets exactly what it needs.
        assert_eq!(meta_budget("alice  2026", 40), 11);
        // Too long: a third of the row, never below twelve cells.
        assert_eq!(meta_budget(&"m".repeat(200), 72), 24);
        // Narrow row: the head wins and the meta is dropped.
        assert_eq!(meta_budget(&"m".repeat(40), 20), 0);
        // Empty meta never claims cells.
        assert_eq!(meta_budget("", 40), 0);
    }

    #[test]
    fn panel_title_falls_back_while_the_remote_pair_is_unknown() {
        assert_eq!(
            panel_title("PR 列表", "octo", "demo"),
            "PR 列表 · octo/demo"
        );
        assert_eq!(panel_title("PR 列表", "", ""), "PR 列表 · —");
        assert_eq!(panel_title("通知", "octo", ""), "通知 · —");
    }

    #[test]
    fn state_and_conclusion_palettes_follow_the_repo_conventions() {
        assert_eq!(state_style("open").fg, Some(Color::Green));
        assert_eq!(state_style("closed").fg, Some(Color::Red));
        assert_eq!(state_style("draft").fg, Some(Color::DarkGray));
        assert_eq!(conclusion_style("success").fg, Some(Color::Green));
        assert_eq!(conclusion_style("failure").fg, Some(Color::Red));
        assert_eq!(conclusion_style("skipped").fg, Some(Color::Gray));
        assert_eq!(check_status_style("in_progress").fg, Some(Color::Yellow));
        assert_eq!(check_status_style("queued").fg, Some(Color::Yellow));
        assert_eq!(mergeable_label(Some(true)).0, "可合并");
        assert_eq!(mergeable_label(Some(false)).0, "有冲突");
        assert_eq!(mergeable_label(None).0, "未知");
    }

    #[test]
    fn check_line_fills_empty_columns_and_fits_the_name() {
        let check = CheckSummary {
            name: "build".to_string(),
            status: "queued".to_string(),
            conclusion: String::new(),
            html_url: String::new(),
        };
        let text: String = check_line(&check, 40)
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(text.contains("build  queued  —"), "missing row: {text}");
        let long = CheckSummary {
            name: "x".repeat(120),
            ..check
        };
        let wide: String = check_line(&long, 30)
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(wide.contains('…'), "long name must be cut: {wide}");
        assert!(wide.ends_with("—"), "columns must survive: {wide}");
    }
}
