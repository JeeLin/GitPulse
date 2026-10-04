//! CI checks receipts panel: the head-commit check runs of the selected pull.

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::Span,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::app::App;

use super::pulls::{
    check_line, list_cursor, panel_chunks, panel_title, render_disconnected, render_footer,
    row_width,
};

/// Empty-state copy: no selected pull, or a pull without check runs. Zero
/// runs is a valid state — the status row below covers failed loads.
const EMPTY_CHECKS: &str = "—（暂无检查记录）";

/// Render the head-commit check runs for the selected pull request.
pub fn render_checks(f: &mut Frame, content: Rect, app: &App) {
    let area = content;
    let chunks = panel_chunks(area);
    if app.github_state.client.is_none() {
        render_disconnected(f, chunks[0], "CI 检查", app);
        render_footer(f, chunks[1], chunks[2], app, false);
        return;
    }

    let checks = &app.github_state.checks;
    let block = Block::default()
        .title(panel_title(
            "CI 检查",
            &app.github_state.owner,
            &app.github_state.repo,
        ))
        .borders(Borders::ALL);

    if checks.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(
                EMPTY_CHECKS,
                Style::default().fg(Color::DarkGray),
            ))
            .block(block),
            chunks[0],
        );
    } else {
        let width = row_width(chunks[0]);
        let items: Vec<ListItem> = checks
            .iter()
            .map(|check| ListItem::new(check_line(check, width)))
            .collect();
        let mut state = ListState::default();
        state.select(list_cursor(app.github_state.selected, checks.len()));
        let list = List::new(items)
            .block(block)
            .highlight_style(super::list_highlight())
            .highlight_symbol(super::HIGHLIGHT_SYMBOL);
        f.render_stateful_widget(list, chunks[0], &mut state);
    }
    render_footer(f, chunks[1], chunks[2], app, false);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::config::Config;
    use crate::github::CheckSummary;
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

    fn check(name: &str, status: &str, conclusion: &str) -> CheckSummary {
        CheckSummary {
            name: name.to_string(),
            status: status.to_string(),
            conclusion: conclusion.to_string(),
            html_url: "https://example.invalid".to_string(),
        }
    }

    fn render_text(app: &App) -> String {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render_checks(f, f.area(), app)).unwrap();
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
    fn checks_list_shows_name_status_conclusion_and_title() {
        let mut app = connected_app();
        app.github_state.checks = vec![
            check("unit", "completed", "success"),
            check("e2e", "in_progress", ""),
            check("lint", "completed", "failure"),
        ];
        app.github_state.selected = 0;
        let text = render_text(&app);
        assert!(text.contains("CI检查·octo/demo"), "missing title: {text}");
        assert!(text.contains("unitcompletedsuccess"), "missing row: {text}");
        assert!(
            text.contains("e2ein_progress—"),
            "missing dash fill: {text}"
        );
        assert!(text.contains("lintcompletedfailure"), "missing row: {text}");
        assert!(text.contains("Esc返回"), "missing back hint: {text}");
    }

    #[test]
    fn checks_list_colors_verdicts_by_the_shared_palette() {
        let mut app = connected_app();
        app.github_state.checks = vec![check("unit", "completed", "failure")];
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render_checks(f, f.area(), &app)).unwrap();
        let buffer = terminal.backend().buffer();
        // Row 1 is the first item row (row 0 is the block's top border).
        let mut status_fg = None;
        let mut conclusion_fg = None;
        for x in 3..buffer.area.width {
            let cell = &buffer[(x, 1)];
            match cell.symbol() {
                "c" if status_fg.is_none() => status_fg = Some(cell.fg),
                "f" if conclusion_fg.is_none() => conclusion_fg = Some(cell.fg),
                _ => {}
            }
        }
        assert_eq!(status_fg, Some(Color::Gray), "settled status reads gray");
        assert_eq!(conclusion_fg, Some(Color::Red), "failure reads red");
    }

    #[test]
    fn checks_list_shows_empty_copy_for_zero_rows() {
        let app = connected_app();
        let text = render_text(&app);
        assert!(text.contains("—（暂无检查记录）"), "missing copy: {text}");
    }

    #[test]
    fn checks_disconnected_shows_the_status_line_instead() {
        let mut app = App::new(Config::default());
        app.set_error("解析远端失败: 未配置远端");
        app.github_state.checks = vec![check("unit", "completed", "success")];
        let text = render_text(&app);
        assert!(text.contains("尚未连接GitHub"), "missing gate copy: {text}");
        assert!(text.contains("未配置远端"), "status line must show: {text}");
        assert!(!text.contains("unit"), "no rows without a client: {text}");
    }
}
