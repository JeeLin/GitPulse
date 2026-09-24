//! Diff viewer for single file.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

/// Render the diff content with syntax highlighting for +/- lines.
pub fn render_diff(f: &mut Frame, app: &crate::app::App) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),    // diff content
            Constraint::Length(1), // key hints
        ])
        .split(area);

    let diff_area = chunks[0];
    let hints_area = chunks[1];

    // Render diff content
    render_diff_content(f, diff_area, &app.diff_content, app.diff_scroll);

    // Render key hints
    let hints = Line::from(vec![
        Span::styled("j/k", Style::default().fg(Color::Cyan)),
        Span::raw(" 滚动  "),
        Span::styled("Esc", Style::default().fg(Color::Cyan)),
        Span::raw(" 返回  "),
        Span::styled("q", Style::default().fg(Color::Cyan)),
        Span::raw(" 退出"),
    ]);
    f.render_widget(Paragraph::new(hints).style(Style::default()), hints_area);
}

fn render_diff_content(f: &mut Frame, area: Rect, content: &str, scroll: u16) {
    // Parse diff lines and style them
    let mut lines = Vec::new();
    for line in content.lines() {
        // `+++`/`---` are file headers in the patch, not +/- change lines.
        let styled_line = if line.starts_with("+++") || line.starts_with("---") {
            Line::from(Span::raw(line.to_string()))
        } else if line.starts_with('+') {
            Line::from(vec![Span::styled(
                line.to_string(),
                Style::default().fg(Color::Green),
            )])
        } else if line.starts_with('-') {
            Line::from(vec![Span::styled(
                line.to_string(),
                Style::default().fg(Color::Red),
            )])
        } else if line.starts_with("@@") {
            Line::from(vec![Span::styled(
                line.to_string(),
                Style::default().fg(Color::Cyan),
            )])
        } else {
            Line::from(vec![Span::raw(line)])
        };
        lines.push(styled_line);
    }

    let paragraph = Paragraph::new(lines)
        .block(Block::default().title("文件差异").borders(Borders::ALL))
        .wrap(Wrap { trim: true })
        .scroll((scroll, 0));

    f.render_widget(paragraph, area);
}
