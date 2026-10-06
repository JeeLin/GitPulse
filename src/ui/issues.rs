//! Issues receipts panel: the open issues of the current repository.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, ListItem, Paragraph, Wrap},
    Frame,
};

use crate::app::App;
use crate::platform::{IssueDetail, IssueSummary};

use super::pulls::{
    cells, ellipsize, meta_budget, panel_title, render_disconnected, render_load_error,
    render_receipts_empty, render_receipts_loading, render_receipts_rows, row_width, state_style,
    META_GAP,
};
/// Empty-state copy: no open issues. Zero rows is a valid state — a failed load
/// paints the panel's `error` state.
const EMPTY_ISSUES: &str = "—（暂无 Issues）";

/// Render the open-issues list (excluding pull requests).
///
/// The panel draws its content only (§4.1): the status row and the hint row
/// are both painted once by the footer.
pub fn render_issues(f: &mut Frame, content: Rect, app: &App) {
    if let Some(message) = app.load_error.as_deref() {
        render_load_error(
            f,
            content,
            "Issues",
            app.github_state.owner(),
            app.github_state.repo(),
            message,
        );
        return;
    }

    if !app.github_state.connected() {
        render_disconnected(f, content, "Issues", app.github_state.kind(), app);
        return;
    }

    if app.github_loading() {
        render_receipts_loading(f, content, "Issues", app, "正在拉取 Issues…");
        return;
    }

    let issues = &app.github_state.issues;
    if issues.is_empty() {
        render_receipts_empty(
            f,
            content,
            "Issues",
            app,
            EMPTY_ISSUES,
            "这个仓库没有开放的 Issue",
        );
    } else {
        let width = row_width(content);
        let items = issues.iter().map(|i| issue_item(i, width)).collect();
        render_receipts_rows(f, content, "Issues", app, items, app.github_state.selected);
    }
}

/// Placeholder of the issue detail while its payload has not arrived yet.
const ISSUE_DETAIL_PLACEHOLDER: &str = "—（暂无 Issue 详情）";

/// Render the selected issue's detail (summary head + wrapped body).
///
/// Mirrors the pull detail panel (`render_pull_detail`): a disconnected gate
/// while the GitHub client is not ready, the placeholder while the detail
/// payload has not landed, then a bordered shell split into a summary head
/// (identification + metadata) and a soft-wrapped body. Issues carry no
/// check sub-list, so there is no third pane.
pub fn render_issue_detail(f: &mut Frame, content: Rect, app: &App) {
    if let Some(message) = app.load_error.as_deref() {
        render_load_error(
            f,
            content,
            "Issue 详情",
            app.github_state.owner(),
            app.github_state.repo(),
            message,
        );
        return;
    }

    if !app.github_state.connected() {
        render_disconnected(f, content, "Issue 详情", app.github_state.kind(), app);
        return;
    }

    let block = Block::default()
        .title(panel_title(
            "Issue 详情",
            app.github_state.owner(),
            app.github_state.repo(),
        ))
        .borders(Borders::ALL);

    let Some(detail) = app.github_state.issue_detail.as_ref() else {
        f.render_widget(
            Paragraph::new(Span::styled(
                ISSUE_DETAIL_PLACEHOLDER,
                Style::default().fg(Color::DarkGray),
            ))
            .block(block),
            content,
        );
        return;
    };

    let inner = block.inner(content);
    f.render_widget(block, content);
    render_detail_panes(f, detail, inner);
}

/// Split the issue detail shell into its two panes — the summary head and the
/// wrapped body — and draw each one.
fn render_detail_panes(f: &mut Frame, detail: &IssueDetail, inner: Rect) {
    let panes = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // summary head
            Constraint::Min(3),    // wrapped body
        ])
        .split(inner);

    // The summary head sits directly on the pane (no block of its own), so it
    // uses the full inner width.
    f.render_widget(
        Paragraph::new(summary_lines(&detail.summary, panes[0].width as usize)),
        panes[0],
    );
    f.render_widget(body_paragraph(detail), panes[1]);
}

/// The issue's markdown body inside its `正文` block, soft-wrapped without
/// trimming the leading whitespace markdown relies on; an empty body renders
/// the empty-state copy instead of a blank pane.
fn body_paragraph(detail: &IssueDetail) -> Paragraph<'static> {
    let body: Vec<Line> = if detail.body.trim().is_empty() {
        vec![Line::from(Span::styled(
            "（无正文）",
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
        .block(Block::default().title("正文").borders(Borders::ALL))
        .wrap(Wrap { trim: false })
}

/// Summary head of the issue detail: `#N title (state)` over a dark meta line
/// of `author  assignee  labels  updated_at`. Each line is fitted to the pane
/// the way the list row is — `meta_budget` bounds the meta, `ellipsize` cuts
/// what does not fit, `cells` measures the fixed head — so a long label list
/// never pushes the identifying `#N title` off the row: identification wins
/// over metadata, the same trade-off `issue_item` makes.
fn summary_lines(issue: &IssueSummary, width: usize) -> Vec<Line<'static>> {
    let number = format!("#{} ", issue.number);
    let state_tag = format!(" ({})", issue.state);

    let mut meta = issue.author.clone();
    if let Some(assignee) = &issue.assignee {
        meta.push_str(META_GAP);
        meta.push_str(assignee);
    }
    if !issue.labels.is_empty() {
        meta.push_str(META_GAP);
        meta.push_str(&issue.labels.join("、"));
    }
    meta.push_str(META_GAP);
    meta.push_str(&issue.updated_at);

    // Same head/meta budget as `issue_item`: the meta keeps the budget it may
    // occupy, the head keeps what remains, and a meta that can't be squeezed
    // in (budget == 0) is dropped — the head then takes the full width.
    let budget = meta_budget(&meta, width);
    let (head_width, meta_width) = if budget > 0 {
        (width - budget - cells(META_GAP), budget)
    } else {
        (width, 0)
    };
    let fixed = cells(&number) + cells(&state_tag);

    let head = Line::from(vec![
        Span::styled(number, Style::default().fg(Color::Cyan)),
        Span::raw(ellipsize(&issue.title, head_width.saturating_sub(fixed))),
        Span::styled(state_tag, state_style(&issue.state)),
    ]);
    let mut lines = vec![head];
    if budget > 0 {
        lines.push(Line::from(Span::styled(
            ellipsize(&meta, meta_width),
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines
}

/// One issue row: `#N title (state)` followed by author, labels and the
/// update time, ellipsized as a pair so the identifying head never loses its
/// tail to a long label list.
fn issue_item(issue: &IssueSummary, width: usize) -> ListItem<'static> {
    let number = format!("#{} ", issue.number);
    let state_tag = format!(" ({})", issue.state);
    let meta = if issue.labels.is_empty() {
        format!("{}  {}", issue.author, issue.updated_at)
    } else {
        format!(
            "{}  {}  {}",
            issue.author,
            issue.labels.join("、"),
            issue.updated_at
        )
    };

    let budget = meta_budget(&meta, width);
    let head_width = if budget > 0 {
        width - budget - cells(META_GAP)
    } else {
        width
    };
    let fixed = cells(&number) + cells(&state_tag);

    let mut spans = vec![
        Span::styled(number, Style::default().fg(Color::Cyan)),
        Span::raw(ellipsize(&issue.title, head_width.saturating_sub(fixed))),
        Span::styled(state_tag, state_style(&issue.state)),
    ];
    if budget > 0 {
        spans.push(Span::styled(
            format!("{META_GAP}{}", ellipsize(&meta, budget)),
            Style::default().fg(Color::DarkGray),
        ));
    }
    ListItem::new(Line::from(spans))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::config::Config;
    use crate::ui::pulls::{draw_receipt_frame, offline_client};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// Connected app: the `octo/demo` remote resolved and the readiness gate
    /// built, so the panels render the real title instead of the `—` fallback.
    fn connected_app() -> App {
        let mut app = App::new(Config::default());
        app.github_state.remote = Some(crate::ui::github_remote());
        app.github_state.client = Some(offline_client());
        app
    }

    fn issue(number: u64, title: &str, state: &str, labels: &[&str]) -> IssueSummary {
        IssueSummary {
            number,
            title: title.to_string(),
            state: state.to_string(),
            author: "bob".to_string(),
            assignee: None,
            updated_at: "2026-02-03T04:05:06Z".to_string(),
            labels: labels.iter().map(|l| l.to_string()).collect(),
        }
    }

    /// The panel plus its footer: the hint row the panel no longer draws
    /// reaches the buffer through [`crate::ui::components::footer`]
    /// (§2.3), so a test that skipped it would miss the row entirely.
    fn render_text(app: &App) -> String {
        render_text_sized(app, 80)
    }

    fn render_text_sized(app: &App, width: u16) -> String {
        compact_text(&render_buffer_sized(app, width))
    }

    /// Whitespace-stripped text of the whole frame (wide CJK glyphs leave
    /// padding cells).
    fn compact_text(buffer: &ratatui::buffer::Buffer) -> String {
        let mut raw = String::new();
        for cell in buffer.content() {
            raw.push_str(cell.symbol());
        }
        raw.chars().filter(|c| !c.is_whitespace()).collect()
    }

    /// The rendered frame, so a test can address the single row an async
    /// result landed on (§4.1).
    fn render_buffer_sized(app: &App, width: u16) -> ratatui::buffer::Buffer {
        let backend = TestBackend::new(width, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| draw_receipt_frame(f, app, true, |f, body| render_issues(f, body, app)))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    #[test]
    fn issues_list_shows_head_meta_labels_and_hints() {
        let mut app = connected_app();
        app.github_state.issues = vec![
            issue(12, "Login fails on Safari", "open", &["bug", "urgent"]),
            issue(5, "Closed one", "closed", &[]),
        ];
        app.github_state.selected = 0;
        let text = render_text(&app);
        assert!(text.contains("Issues·octo/demo"), "missing title: {text}");
        assert!(
            text.contains("#12LoginfailsonSafari(open)"),
            "missing head: {text}"
        );
        assert!(
            text.contains("bobbug、urgent2026-02-03T04:05:06Z"),
            "missing meta: {text}"
        );
        // A closed issue keeps the red state of the shared palette.
        assert!(text.contains("#5Closedone(closed)"), "missing row: {text}");
        assert!(text.contains("iissues"), "missing tab hint: {text}");
        assert!(text.contains("r刷新"), "missing refresh hint: {text}");
    }

    #[test]
    fn issues_list_ellipsizes_the_meta_tail_before_the_head() {
        let mut app = connected_app();
        let long_labels: Vec<String> = (0..40).map(|i| format!("label-{i}")).collect();
        app.github_state.issues = vec![IssueSummary {
            number: 1,
            title: "Short".to_string(),
            state: "open".to_string(),
            author: "bob".to_string(),
            assignee: None,
            updated_at: "2026-02-03T04:05:06Z".to_string(),
            labels: long_labels,
        }];
        let text = render_text(&app);
        assert!(text.contains('…'), "long meta must be cut: {text}");
        assert!(
            !text.contains("label-39"),
            "the meta tail must not survive verbatim: {text}"
        );
        assert!(text.contains("#1Short"), "head must stay: {text}");
    }

    #[test]
    fn issues_list_shows_empty_copy_for_zero_rows() {
        let app = connected_app();
        let text = render_text(&app);
        assert!(text.contains("—（暂无Issues）"), "missing copy: {text}");
    }

    /// §4.1/§4.2: the readiness gate replaces the list, and the reason stays on
    /// the footer status row — the one landing spot for an async result.
    #[test]
    fn issues_disconnected_keeps_the_reason_on_the_footer_status_row() {
        let mut app = App::new(Config::default());
        app.set_error("GitHub 认证失败");
        app.github_state.issues = vec![issue(1, "ghost", "open", &[])];
        let buffer = render_buffer_sized(&app, 80);
        let text = compact_text(&buffer);
        // No resolved remote means `kind == None`, which falls back to the
        // GitHub gate copy, verbatim as in v0.5.0.
        assert!(text.contains("尚未连接GitHub"), "missing gate copy: {text}");
        assert!(
            !text.contains("#1ghost"),
            "no rows without a client: {text}"
        );
        assert_eq!(
            crate::ui::pulls::rows_with(&buffer, "GitHub认证失败"),
            vec![buffer.area.height - 2],
            "the reason belongs on the footer status row only: {text}"
        );
    }

    /// Render one frame of the issue detail view and return the
    /// whitespace-stripped buffer text (wide CJK glyphs leave padding cells).
    fn render_detail_text(app: &App) -> String {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_issue_detail(f, f.area(), app))
            .unwrap();
        let raw: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        raw.chars().filter(|c| !c.is_whitespace()).collect()
    }

    /// Build an [`IssueDetail`] reusing the list-row builder so the summary
    /// and the detail stay in lockstep.
    fn detail(
        number: u64,
        title: &str,
        state: &str,
        labels: &[&str],
        assignee: Option<&str>,
        body: &str,
    ) -> IssueDetail {
        let mut summary = issue(number, title, state, labels);
        summary.assignee = assignee.map(str::to_string);
        IssueDetail {
            summary,
            body: body.to_string(),
        }
    }

    #[test]
    fn issue_detail_disconnected_gates_before_the_head() {
        let mut app = App::new(Config::default());
        app.set_error("GitHub 认证失败");
        app.github_state.issue_detail = Some(detail(7, "Ghost", "open", &[], None, "body"));
        let text = render_detail_text(&app);
        assert!(text.contains("尚未连接GitHub"), "missing gate copy: {text}");
        assert!(
            !text.contains("#7"),
            "no head renders without a client: {text}"
        );
    }

    #[test]
    fn issue_detail_placeholder_waits_for_the_payload() {
        let app = connected_app();
        let text = render_detail_text(&app);
        assert!(
            text.contains("—（暂无Issue详情）"),
            "missing placeholder: {text}"
        );
        assert!(
            !text.contains('#'),
            "no head renders before the payload: {text}"
        );
    }

    #[test]
    fn issue_detail_renders_summary_meta_and_body() {
        let mut app = connected_app();
        app.github_state.issue_detail = Some(detail(
            12,
            "Bad login",
            "open",
            &["bug", "urgent"],
            Some("alice"),
            "Root cause: Safari rejects the cookie.",
        ));
        let text = render_detail_text(&app);
        assert!(
            text.contains("Issue详情·octo/demo"),
            "missing title: {text}"
        );
        assert!(text.contains("#12Badlogin(open)"), "missing head: {text}");
        assert!(
            text.contains("bobalicebug、urgent2026-02-03T04:05:06Z"),
            "missing meta line: {text}"
        );
        assert!(
            text.contains("Rootcause:Safarirejectsthecookie."),
            "missing body text: {text}"
        );
    }

    #[test]
    fn issue_detail_marks_an_empty_body() {
        let mut app = connected_app();
        app.github_state.issue_detail = Some(detail(3, "Bare", "closed", &[], None, ""));
        let text = render_detail_text(&app);
        assert!(text.contains("（无正文）"), "missing body copy: {text}");
        assert!(text.contains("#3Bare(closed)"), "missing head: {text}");
    }
}
