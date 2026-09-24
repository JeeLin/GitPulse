//! Single-line commit message input popup.

use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use super::repo::centered_rect;

/// Render the commit message popup centered over the current view.
///
/// Single-line input (convention 9): `Enter` commits, `Esc` cancels; the
/// typed text lives in `App::commit_message` and is passed in per frame.
pub fn render_commit_popup(f: &mut Frame, message: &str) {
    let area = centered_rect(60, 5, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .title("提交信息 (Enter 提交 / Esc 取消)")
        .borders(Borders::ALL);

    let line = Line::from(vec![
        Span::styled("> ", Style::default().fg(Color::Cyan)),
        Span::raw(message),
    ]);
    f.render_widget(Paragraph::new(line).block(block), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn render_commit_popup_shows_message_and_title() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_commit_popup(f, "fix: hello world"))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        // Wide CJK glyphs leave padding cells; compare whitespace-stripped.
        let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(compact.contains("fix:helloworld"), "missing input: {text}");
        assert!(compact.contains("提交信息"), "missing title: {text}");
    }
}
