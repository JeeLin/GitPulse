//! Pull-request hub: the open-PR list and the detail panel, plus the check
//! palette the detail's sub-list needs.
//!
//! The receipts spine's shared layer — panel titles, the five-state dispatch,
//! the row budget, the platform gate and the status/hint footer — lives in
//! [`crate::ui::components::receipts`], which this view and the three sibling
//! receipt panels (checks / issues / notifications) all draw through.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, ListItem, Paragraph, Wrap},
    Frame,
};

use crate::app::App;
use crate::platform::{CheckSummary, PrDetail, PrSummary};
use crate::ui::components::cells;
use crate::ui::components::receipts::{
    render_disconnected, render_load_error, render_receipts_list, ReceiptPanel,
};
use crate::ui::components::theme::color;

// The receipts spine's shared layer lives in `components::receipts`; these
// re-exports keep the sibling views, the footer dispatch and the local-git
// panels addressing the hint rows, the panel title and the row budget the way
// they always have.
pub(crate) use crate::ui::components::receipts::{
    detail_hints,
    panel_title,
    receipt_hints,
    row_width,
    // kept for any external readers; see module re-exports below.
};

#[cfg(test)]
pub(crate) use crate::ui::components::cells::rows_with;
#[cfg(test)]
pub(crate) use crate::ui::components::receipts::{draw_receipt_frame, offline_client};

/// Cells a paragraph row may use inside `area`'s block: minus the borders
/// only (no cursor on paragraph rows).
pub(crate) fn block_width(area: Rect) -> usize {
    area.width.saturating_sub(2) as usize
}

/// Empty-state copy of the pull list: zero open pulls is a valid state, not
/// a failure — the status row below covers failed loads.
const EMPTY_PULLS: &str = "—（暂无 PR）";

/// Placeholder of the detail view while its payload has not arrived yet.
const DETAIL_PLACEHOLDER: &str = "按 Enter 查看详情 · Esc 返回";

/// Blank columns between a row's head and its trailing meta part.
pub(crate) const META_GAP: &str = "  ";

/// Cells a row keeps for its head (`#12 title (open)`) before the trailing
/// meta part is dropped entirely: identification wins over metadata.
const MIN_HEAD_WIDTH: usize = 16;

/// Check rows drawn in the detail panel before the sub-list is capped, so a
/// PR with thirty runs cannot eat the description pane.
const MAX_DETAIL_CHECKS: usize = 10;

/// Cells of a `width`-wide row the trailing meta part (`author  labels  time`)
/// may occupy: everything it needs when there is room, else at least a third
/// of the row (never a sliver), and nothing at all on very narrow rows — the
/// head then uses the full width.
pub(crate) fn meta_budget(meta: &str, width: usize) -> usize {
    let needed = cells::width(meta);
    if needed == 0 {
        return 0;
    }
    if needed + cells::width(META_GAP) <= width {
        return needed;
    }
    let target = (width / 3).max(12).min(needed);
    let target = target.min(width.saturating_sub(MIN_HEAD_WIDTH + cells::width(META_GAP)));
    if target >= 8 {
        target
    } else {
        0
    }
}

/// Palette of a pull/issue state: open reads success, closed danger, any
/// other value (GitHub keeps inventing them) falls back to muted.
pub(crate) fn state_style(state: &str) -> Style {
    match state {
        "open" => Style::default().fg(color::SUCCESS),
        "closed" => Style::default().fg(color::DANGER),
        _ => Style::default().fg(color::FG_MUTED),
    }
}

/// Palette of a check conclusion (§5.1 `ci_*`): failures danger, success
/// `ci_pass`, the muted outcomes `ci_skip`, and anything unclassified plain.
pub(crate) fn conclusion_style(conclusion: &str) -> Style {
    match conclusion {
        "success" => Style::default().fg(color::CI_PASS),
        "failure" | "timed_out" | "action_required" => Style::default().fg(color::CI_FAIL),
        "neutral" | "skipped" | "cancelled" => Style::default().fg(color::CI_SKIP),
        _ => Style::default(),
    }
}

/// Palette of a check status: work in flight (queued / in_progress) reads
/// `ci_pending`, a settled run muted — the conclusion column then carries
/// verdict.
pub(crate) fn check_status_style(status: &str) -> Style {
    match status {
        "in_progress" | "queued" => Style::default().fg(color::CI_PENDING),
        "completed" => Style::default().fg(color::CI_SKIP),
        _ => Style::default().fg(color::FG_MUTED),
    }
}

/// Mergeability of the detail view: `None` means the platform has not computed
/// it yet, so the label says so instead of staying silent.
///
/// The blocking wording is per platform. GitHub's `mergeable` only goes false
/// on a real conflict, so it keeps its v0.5.0 `有冲突`; GitLab folds its
/// official blocking states — draft, approvals, path locks, another MR ahead
/// in the queue, a merged MR … — into the same `false` (the full list lives in
/// `RESEARCH-gitea-gitlab-apis.md` §2.4, not restated here so the count cannot
/// drift), so naming just one of them would be false — those read `不可合并`.
pub(crate) fn mergeable_label(
    kind: Option<crate::platform::PlatformKind>,
    mergeable: Option<bool>,
) -> (&'static str, Style) {
    use crate::platform::PlatformKind;
    match mergeable {
        Some(true) => ("可合并", Style::default().fg(color::SUCCESS)),
        Some(false) => (
            if kind == Some(PlatformKind::GitHub) {
                "有冲突"
            } else {
                "不可合并"
            },
            Style::default().fg(color::DANGER),
        ),
        None => ("未知", Style::default().fg(color::WARNING)),
    }
}

/// One `name  status  conclusion` check row fitted to `width` display cells;
/// an empty column reads `—` instead of vanishing.
pub(crate) fn check_line(check: &CheckSummary, width: usize) -> Line<'static> {
    let status = column(&check.status);
    let conclusion = column(&check.conclusion);
    let used = cells::width(&status) + cells::width(&conclusion) + 2 * cells::width(META_GAP);
    Line::from(vec![
        Span::raw(cells::ellipsize(&check.name, width.saturating_sub(used))),
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

/// Render the open-pull list (the receipts hub entry point).
///
/// The five-state dispatch, the shell and the cursor live in
/// [`crate::ui::components::receipts`]; this view only supplies its own rows.
pub fn render_pulls(f: &mut Frame, content: Rect, app: &App) {
    let meta = ReceiptPanel {
        label: "PR 列表",
        load_sub: "正在拉取 PR…",
        empty_copy: EMPTY_PULLS,
        empty_description: "这个仓库没有开放的 PR",
    };
    let pulls = &app.github_state.pulls;
    render_receipts_list(f, content, meta, app, app.github_state.selected, |width| {
        pulls.iter().map(|pr| pull_item(pr, width)).collect()
    });
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
    let fixed = cells::width(&number) + cells::width(&state_tag) + cells::width(&draft_tag);
    let head = Line::from(vec![
        Span::styled(number, Style::default().fg(color::INFO)),
        Span::raw(cells::ellipsize(&pr.title, width.saturating_sub(fixed))),
        Span::styled(state_tag, state_style(&pr.state)),
        Span::styled(draft_tag, Style::default().fg(color::WARNING)),
    ]);

    let meta = format!(
        "{}  {}→{}  {}",
        pr.author, pr.head_ref, pr.base_ref, pr.updated_at
    );
    let tail = Line::from(Span::styled(
        cells::ellipsize(&meta, width),
        Style::default().fg(color::FG_MUTED),
    ));
    ListItem::new(vec![head, tail])
}

/// Render the detail of the highlighted pull request: the bordered shell,
/// the placeholder while its payload has not arrived, and the three panes.
pub fn render_pull_detail(f: &mut Frame, content: Rect, app: &App) {
    receipt_detail_gate(
        f,
        content,
        app,
        "PR 详情",
        DETAIL_PLACEHOLDER,
        |f, inner| {
            let detail = app
                .github_state
                .detail
                .as_ref()
                .expect("detail panel gate only proceeds when the payload landed");
            render_detail_panes(f, detail, inner, app.github_state.kind());
        },
    );
}

/// The four gates every receipt-detail panel shares (B4-06): a failed load
/// paints the panel's `error` state, a disconnected client its own state, a
/// payload that has not landed the placeholder — and only then the bordered
/// shell, drawn around `body`.
///
/// Returns whether `body` ran. The order is the §4.3 rule: a load failure is a
/// panel state, not a status-row message, and it outranks the "not connected"
/// and "no payload yet" states.
///
/// `placeholder` is the copy this panel shows while loading, and the returned
/// `bool` lets each caller keep its own pane layout — a pull detail splits
/// into three panes, an issue detail into two.
///
/// The payload check is "neither kind of detail landed" because the two panels
/// read different fields of the same [`App`]; each closure re-reads its own
/// field and the one it needs is guaranteed present by that point.
pub(crate) fn receipt_detail_gate(
    f: &mut Frame,
    content: Rect,
    app: &App,
    label: &str,
    placeholder: &str,
    body: impl FnOnce(&mut Frame, Rect),
) -> bool {
    if let Some(message) = app.load_error.as_deref() {
        render_load_error(
            f,
            content,
            &panel_title(label, app.github_state.owner(), app.github_state.repo()),
            message,
        );
        return false;
    }

    if !app.github_state.connected() {
        render_disconnected(f, content, label, app.github_state.kind(), app);
        return false;
    }

    let block = Block::default()
        .title(panel_title(
            label,
            app.github_state.owner(),
            app.github_state.repo(),
        ))
        .borders(Borders::ALL);

    if app.github_state.detail.is_none() && app.github_state.issue_detail.is_none() {
        f.render_widget(
            Paragraph::new(Span::styled(
                placeholder,
                Style::default().fg(color::FG_MUTED),
            ))
            .block(block),
            content,
        );
        return false;
    }

    let inner = block.inner(content);
    f.render_widget(block, content);
    body(f, inner);
    true
}

/// Split the detail shell into its three panes — summary head, wrapped
/// description, check sub-list — and draw each one.
fn render_detail_panes(
    f: &mut Frame,
    detail: &PrDetail,
    inner: Rect,
    kind: Option<crate::platform::PlatformKind>,
) {
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
        Paragraph::new(summary_lines(detail, panes[0].width as usize, kind)),
        panes[0],
    );
    f.render_widget(body_pane(&detail.body, "描述", "（无描述）"), panes[1]);
    render_checks_pane(f, detail, panes[2]);
}

/// A detail pane's markdown body inside its titled block, soft-wrapped
/// without trimming the leading whitespace markdown relies on; an empty body
/// renders the empty-state copy instead of a blank pane.
///
/// The two detail panels word the block differently (`描述` / `正文`, and the
/// matching empty copy), so the label and its empty copy are the caller's —
/// everything else, the wrap and the muted empty copy, is shared (B4-06).
pub(crate) fn body_pane(body: &str, label: &str, empty_copy: &str) -> Paragraph<'static> {
    let lines: Vec<Line> = if body.trim().is_empty() {
        vec![Line::from(Span::styled(
            empty_copy.to_string(),
            Style::default().fg(color::FG_MUTED),
        ))]
    } else {
        body.lines().map(|l| Line::from(l.to_string())).collect()
    };
    Paragraph::new(lines)
        .block(
            Block::default()
                .title(label.to_string())
                .borders(Borders::ALL),
        )
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
                Style::default().fg(color::FG_MUTED),
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
fn summary_lines(
    detail: &PrDetail,
    width: usize,
    kind: Option<crate::platform::PlatformKind>,
) -> Vec<Line<'static>> {
    let summary = &detail.summary;
    let number = format!("#{} ", summary.number);
    let state_tag = format!(" ({})", summary.state);
    let draft_tag = if summary.draft {
        " [草稿]".to_string()
    } else {
        String::new()
    };
    let fixed = cells::width(&number) + cells::width(&state_tag) + cells::width(&draft_tag);
    let head = Line::from(vec![
        Span::styled(number, Style::default().fg(color::INFO)),
        Span::raw(cells::ellipsize(
            &summary.title,
            width.saturating_sub(fixed),
        )),
        Span::styled(state_tag, state_style(&summary.state)),
        Span::styled(draft_tag, Style::default().fg(color::WARNING)),
    ]);

    let (merge, merge_style) = mergeable_label(kind, detail.mergeable);
    let meta = format!(
        " · {} · {}→{} · {}",
        summary.author, summary.head_ref, summary.base_ref, summary.updated_at
    );
    let tail = Line::from(vec![
        Span::styled(merge.to_string(), merge_style),
        Span::styled(
            cells::ellipsize(&meta, width.saturating_sub(cells::width(merge))),
            Style::default().fg(color::FG_MUTED),
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

    /// Connected app: the `octo/demo` remote resolved and the readiness gate
    /// built, so the panels render the real titles instead of the `—` fallback.
    fn connected_app() -> App {
        let mut app = App::new(Config::default());
        app.github_state.remote = Some(crate::ui::github_remote());
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
    /// The panel plus its footer, so the hint row the panel no longer draws
    /// is on the buffer (§2.3). `show_enter` mirrors what the panel passes to
    /// [`receipt_hints`] in production.
    fn render_text(app: &App, draw: fn(&mut Frame, Rect, &App), show_enter: bool) -> String {
        render_text_sized(app, draw, show_enter, 80, 24)
    }

    fn render_text_sized(
        app: &App,
        draw: fn(&mut Frame, Rect, &App),
        show_enter: bool,
        width: u16,
        height: u16,
    ) -> String {
        let terminal = render_terminal(app, draw, show_enter, width, height);
        compact_frame(terminal.backend().buffer())
    }

    /// The frame the panel plus its footer produced, so a test can address a
    /// single row of it (§4.1).
    fn render_terminal(
        app: &App,
        draw: fn(&mut Frame, Rect, &App),
        show_enter: bool,
        width: u16,
        height: u16,
    ) -> Terminal<TestBackend> {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| draw_receipt_frame(f, app, show_enter, |f, body| draw(f, body, app)))
            .unwrap();
        terminal
    }

    /// Whitespace-stripped text of the whole frame (wide CJK glyphs leave
    /// padding cells).
    fn compact_frame(buffer: &ratatui::buffer::Buffer) -> String {
        let mut raw = String::new();
        for cell in buffer.content() {
            raw.push_str(cell.symbol());
        }
        raw.chars().filter(|c| !c.is_whitespace()).collect()
    }

    #[test]
    fn pulls_list_shows_title_rows_and_hints() {
        let mut app = connected_app();
        app.github_state.pulls = vec![pull(42, "Fix login bug", false), pull(7, "WIP thing", true)];
        app.github_state.selected = 0;
        let text = render_text_sized(&app, render_pulls, true, 120, 24);
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
        let text = render_text(&app, render_pulls, true);
        assert!(text.contains("#2"), "last row must stay selected: {text}");
    }

    #[test]
    fn pulls_list_truncates_a_long_title_with_ellipsis() {
        let mut app = connected_app();
        app.github_state.pulls = vec![pull(9, &"长".repeat(60), false)];
        let text = render_text(&app, render_pulls, true);
        assert!(text.contains('…'), "long title must be cut: {text}");
        // The meta line keeps its head on the second row, truncation or not.
        assert!(text.contains("alice"), "meta line missing: {text}");
    }

    #[test]
    fn pulls_list_shows_empty_copy_for_zero_rows() {
        let app = connected_app();
        let text = render_text(&app, render_pulls, true);
        assert!(text.contains("—（暂无PR）"), "missing empty copy: {text}");
    }

    /// §4.1/§4.2: while the platform client is missing the panel shows the gate
    /// copy and nothing else — the reason stays on the footer status row, the
    /// one landing spot for an async result.
    #[test]
    fn pulls_disconnected_keeps_the_reason_on_the_footer_status_row() {
        let mut app = App::new(Config::default());
        app.set_error("打开 GitHub 回执失败: 未打开仓库");
        app.github_state.pulls = vec![pull(1, "ghost", false)];
        let terminal = render_terminal(&app, render_pulls, true, 80, 24);
        let buffer = terminal.backend().buffer().clone();
        let text = compact_frame(&buffer);
        // No remote has been resolved, so the gate copy falls back to GitHub —
        // the platform v0.5.0 knew, and the one this path has always shown.
        assert!(text.contains("尚未连接GitHub"), "missing gate copy: {text}");
        assert!(
            !text.contains("#1ghost"),
            "content must not render without a client: {text}"
        );
        assert_eq!(
            rows_with(&buffer, "打开GitHub回执失败:未打开仓库"),
            vec![buffer.area.height - 2],
            "the reason belongs on the footer status row only: {text}"
        );
    }

    #[test]
    fn pull_detail_placeholder_waits_for_the_payload() {
        let app = connected_app();
        let text = render_text(&app, render_pull_detail, false);
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
        let text = render_text(&app, render_pull_detail, false);
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
        let text = render_text(&app, render_pull_detail, false);
        assert!(text.contains("（无描述）"), "missing body copy: {text}");
        assert!(
            text.contains("—（暂无检查记录）"),
            "missing checks copy: {text}"
        );
        assert!(text.contains("未知"), "uncomputed mergeability: {text}");
        assert!(text.contains("[草稿]"), "missing draft badge: {text}");
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

    /// U4: a GitLab/Gitea remote must reach the body, not just the helper.
    #[test]
    fn pulls_disconnected_renders_the_gate_copy_of_the_remote_platform() {
        let cases = [
            (
                "https://github.com/octo/demo.git",
                "尚未连接GitHub",
                "GITHUB_TOKEN",
                "GitLab",
            ),
            (
                "https://gitlab.com/group/proj.git",
                "尚未连接GitLab",
                "GITLAB_TOKEN",
                "GitHub",
            ),
            (
                "https://gitea.com/octo/demo.git",
                "尚未连接Gitea",
                "GITEA_TOKEN",
                "GitHub",
            ),
        ];
        for (url, gate, hint, foreign) in cases {
            let mut app = App::new(Config::default());
            app.github_state.remote = Some(crate::platform::parse_remote(url).unwrap());
            let text = render_text(&app, render_pulls, true);
            assert!(text.contains(gate), "{url}: wrong gate copy: {text}");
            assert!(text.contains(hint), "{url}: wrong hint: {text}");
            assert!(
                !text.contains(&format!("尚未连接{foreign}")),
                "{url}: foreign copy leaked: {text}"
            );
        }
    }

    #[test]
    fn state_and_conclusion_palettes_follow_the_repo_conventions() {
        assert_eq!(state_style("open").fg, Some(color::SUCCESS));
        assert_eq!(state_style("closed").fg, Some(color::DANGER));
        assert_eq!(state_style("draft").fg, Some(color::FG_MUTED));
        assert_eq!(conclusion_style("success").fg, Some(color::CI_PASS));
        assert_eq!(conclusion_style("failure").fg, Some(color::CI_FAIL));
        assert_eq!(conclusion_style("skipped").fg, Some(color::CI_SKIP));
        assert_eq!(
            check_status_style("in_progress").fg,
            Some(color::CI_PENDING)
        );
        assert_eq!(check_status_style("queued").fg, Some(color::CI_PENDING));
        use crate::platform::PlatformKind;
        assert_eq!(
            mergeable_label(Some(PlatformKind::GitHub), Some(true)).0,
            "可合并"
        );
        // GitHub only goes false on a real conflict, so its v0.5.0 wording
        // stays; GitLab folds its official blocking states into `false`
        // (RESEARCH §2.4 — the count is deliberately not restated here).
        assert_eq!(
            mergeable_label(Some(PlatformKind::GitHub), Some(false)).0,
            "有冲突"
        );
        assert_eq!(
            mergeable_label(Some(PlatformKind::GitLab), Some(false)).0,
            "不可合并"
        );
        assert_eq!(
            mergeable_label(Some(PlatformKind::Gitea), Some(false)).0,
            "不可合并"
        );
        assert_eq!(mergeable_label(None, Some(false)).0, "不可合并");
        assert_eq!(mergeable_label(Some(PlatformKind::GitHub), None).0, "未知");
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
