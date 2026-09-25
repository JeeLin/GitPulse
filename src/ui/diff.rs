//! Diff viewer for single file.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use super::hint;

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
    let hints = [hint("j/k", "滚动"), hint("Esc", "返回"), hint("q", "退出")]
        .into_iter()
        .flatten()
        .collect::<Line>();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, View};
    use crate::config::Config;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    const SAMPLE_DIFF: &str = r#"diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,4 @@
 context line
-removed_keyword = 1;
+added_keyword = 2;
"#;

    fn app_with_diff(content: &str) -> App {
        let mut app = App::new(Config::default());
        app.view = View::Diff;
        app.diff_content = content.to_string();
        app
    }

    fn render_app(app: &App) -> Terminal<TestBackend> {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render_diff(f, app)).unwrap();
        terminal
    }

    /// Wide CJK glyphs leave padding cells; compare whitespace-stripped text.
    fn compact(terminal: &Terminal<TestBackend>) -> String {
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
    fn render_styles_change_lines_and_keeps_headers_plain() {
        let app = app_with_diff(SAMPLE_DIFF);
        let terminal = render_app(&app);
        let buf = terminal.backend().buffer();

        // Content starts at (1, 1): the block border occupies row 0 / column 0.
        assert_eq!(buf[(1, 6)].symbol(), "-", "minus line row");
        assert_eq!(buf[(1, 6)].fg, Color::Red, "minus line must be red");
        assert_eq!(buf[(1, 7)].symbol(), "+", "plus line row");
        assert_eq!(buf[(1, 7)].fg, Color::Green, "plus line must be green");
        assert_eq!(buf[(1, 4)].symbol(), "@", "hunk header row");
        assert_eq!(buf[(1, 4)].fg, Color::Cyan, "hunk header must be cyan");

        // `---`/`+++` are file headers, not change lines: no +/- foreground.
        assert_eq!(buf[(1, 2)].symbol(), "-", "--- header row");
        assert_ne!(buf[(1, 2)].fg, Color::Red, "--- header must not be red");
        assert_eq!(buf[(1, 3)].symbol(), "+", "+++ header row");
        assert_ne!(buf[(1, 3)].fg, Color::Green, "+++ header must not be green");
    }

    #[test]
    fn render_shows_added_removed_and_hunk_lines() {
        let app = app_with_diff(SAMPLE_DIFF);
        let terminal = render_app(&app);
        let text = compact(&terminal);
        assert!(text.contains("文件差异"), "missing title: {text}");
        assert!(text.contains("added_keyword"), "missing added line: {text}");
        assert!(
            text.contains("removed_keyword"),
            "missing removed line: {text}"
        );
        assert!(text.contains("@@"), "missing hunk header: {text}");
    }

    #[test]
    fn render_shows_scroll_and_back_hints() {
        let app = app_with_diff(SAMPLE_DIFF);
        let terminal = render_app(&app);
        let text = compact(&terminal);
        assert!(text.contains("j/k滚动"), "missing scroll hint: {text}");
        assert!(text.contains("Esc返回"), "missing back hint: {text}");
        assert!(text.contains("q退出"), "missing quit hint: {text}");
    }

    #[test]
    fn render_with_scroll_offset_stays_stable() {
        let mut lines = vec!["FIRST_MARKER".to_string()];
        lines.extend((1..40).map(|i| format!("body-{i}")));
        let mut app = app_with_diff(&lines.join("\n"));
        app.diff_scroll = 5;
        let terminal = render_app(&app);
        let text = compact(&terminal);
        assert!(text.contains("文件差异"), "title must stay visible: {text}");
        assert!(
            !text.contains("FIRST_MARKER"),
            "scrolled-away line still shown: {text}"
        );
        assert!(
            text.contains("body-5"),
            "first visible line missing: {text}"
        );
    }
}
