//! Issues receipts panel: the open issues of the current repository.

use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::app::App;
use crate::github::IssueSummary;

use super::pulls::{
    cells, ellipsize, list_cursor, meta_budget, panel_chunks, panel_title, render_disconnected,
    render_footer, row_width, state_style, META_GAP,
};

/// Empty-state copy: no open issues. Zero rows is a valid state — the status
/// row below covers failed loads.
const EMPTY_ISSUES: &str = "—（暂无 Issues）";

/// Render the open-issues list (excluding pull requests).
pub fn render_issues(f: &mut Frame, app: &App) {
    let area = f.area();
    let chunks = panel_chunks(area);
    if app.github_state.client.is_none() {
        render_disconnected(f, chunks[0], "Issues", app);
        render_footer(f, chunks[1], chunks[2], app);
        return;
    }

    let issues = &app.github_state.issues;
    let block = Block::default()
        .title(panel_title(
            "Issues",
            &app.github_state.owner,
            &app.github_state.repo,
        ))
        .borders(Borders::ALL);

    if issues.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(
                EMPTY_ISSUES,
                Style::default().fg(Color::DarkGray),
            ))
            .block(block),
            chunks[0],
        );
    } else {
        let width = row_width(chunks[0]);
        let items: Vec<ListItem> = issues
            .iter()
            .map(|issue| issue_item(issue, width))
            .collect();
        let mut state = ListState::default();
        state.select(list_cursor(app.github_state.selected, issues.len()));
        let list = List::new(items)
            .block(block)
            .highlight_style(super::list_highlight())
            .highlight_symbol(super::HIGHLIGHT_SYMBOL);
        f.render_stateful_widget(list, chunks[0], &mut state);
    }
    render_footer(f, chunks[1], chunks[2], app);
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
    use crate::ui::pulls::offline_client;
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

    fn issue(number: u64, title: &str, state: &str, labels: &[&str]) -> IssueSummary {
        IssueSummary {
            number,
            title: title.to_string(),
            state: state.to_string(),
            author: "bob".to_string(),
            updated_at: "2026-02-03T04:05:06Z".to_string(),
            labels: labels.iter().map(|l| l.to_string()).collect(),
        }
    }

    fn render_text(app: &App) -> String {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render_issues(f, app)).unwrap();
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

    #[test]
    fn issues_disconnected_shows_the_status_line_instead() {
        let mut app = App::new(Config::default());
        app.set_error("GitHub 认证失败");
        app.github_state.issues = vec![issue(1, "ghost", "open", &[])];
        let text = render_text(&app);
        assert!(text.contains("尚未连接GitHub"), "missing gate copy: {text}");
        assert!(text.contains("GitHub认证失败"), "status must show: {text}");
        assert!(
            !text.contains("#1ghost"),
            "no rows without a client: {text}"
        );
    }
}
