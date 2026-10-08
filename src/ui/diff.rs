//! Diff viewer for single file.

use crate::ui::components::cells;
use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use super::components::state_panel::{self, PanelState};
use super::hint;
use crate::ui::components::footer::{hints as join_hints, HintRow};
use crate::ui::components::theme::color;

/// Render the diff content with syntax highlighting for +/- lines.
///
/// A failed diff read is a load failure, so the pane shows its `error` state
/// with `按 r 重试` instead of the patch (§4.3).
pub fn render_diff(f: &mut Frame, content: Rect, app: &crate::app::App) {
    if let Some(message) = app.load_error.as_deref() {
        state_panel::render_state_panel(f, content, "文件差异", &PanelState::Error { message });
        return;
    }
    render_diff_content(f, content, &app.diff_content, app.diff_scroll);
}

/// Footer hint row for the diff view (§2.3): the one verb the screen owns.
/// `Esc 返回` and `q 退出` are global segments (§2.3 item 1), so this row
/// must not restate them.
pub(crate) fn diff_hints() -> HintRow {
    HintRow::new(join_hints([hint("j/k", "滚动")]))
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
                Style::default().fg(color::SUCCESS),
            )])
        } else if line.starts_with('-') {
            Line::from(vec![Span::styled(
                line.to_string(),
                Style::default().fg(color::DANGER),
            )])
        } else if line.starts_with("@@") {
            Line::from(vec![Span::styled(
                line.to_string(),
                Style::default().fg(color::INFO),
            )])
        } else {
            Line::from(vec![Span::raw(line.to_string())])
        };
        lines.push(styled_line);
    }

    let paragraph = Paragraph::new(
        lines
            .into_iter()
            .map(|line| cells::fit_line(line, area.width.saturating_sub(2) as usize))
            .collect::<Vec<_>>(),
    )
    .block(Block::default().title("文件差异").borders(Borders::ALL))
    .scroll((scroll, 0));

    f.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, View};
    use crate::config::Config;
    use crate::ui::components::footer::StatusRow;
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

    /// The panel draws content only (§2.3), so the frame goes through
    /// [`crate::ui::components::footer::draw_frame`] — the shape
    /// `ui::render` uses — and the hint row is the bottom row.
    fn render_app(app: &App) -> Terminal<TestBackend> {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                crate::ui::components::footer::draw_frame(
                    f,
                    StatusRow::plain(app.status.as_ref()),
                    &diff_hints(),
                    |f, body| render_diff(f, body, app),
                );
            })
            .unwrap();
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
        assert_eq!(buf[(1, 6)].fg, color::DANGER, "minus line must be danger");
        assert_eq!(buf[(1, 7)].symbol(), "+", "plus line row");
        assert_eq!(buf[(1, 7)].fg, color::SUCCESS, "plus line must be success");
        assert_eq!(buf[(1, 4)].symbol(), "@", "hunk header row");
        assert_eq!(buf[(1, 4)].fg, color::INFO, "hunk header must be info");

        // `---`/`+++` are file headers, not change lines: no +/- foreground.
        assert_eq!(buf[(1, 2)].symbol(), "-", "--- header row");
        assert_ne!(
            buf[(1, 2)].fg,
            color::DANGER,
            "--- header must not be danger"
        );
        assert_eq!(buf[(1, 3)].symbol(), "+", "+++ header row");
        assert_ne!(
            buf[(1, 3)].fg,
            color::SUCCESS,
            "+++ header must not be success"
        );
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

    /// §5.4: a diff line that is wider than the pane is truncated with `…` —
    /// it must never wrap onto two terminal rows (§5.4 单行永不换行).
    #[test]
    fn design_diff_truncates_a_long_line_with_an_ellipsis_instead_of_wrapping() {
        let long = format!("+{}-{}", "x".repeat(100), "trailer");
        let app = app_with_diff(&long);
        let terminal = render_app(&app);
        let text = compact(&terminal);
        assert!(
            text.contains('…'),
            "a too-wide diff line must end in an ellipsis, not wrap or clip: {text}"
        );
        assert!(
            text.contains("+xxxx"),
            "the head of a wide line must survive truncation (§5.4 cuts the right): {text}"
        );
        // §5.4 single-line-never-wraps: the long line occupies exactly one
        // content row. A wrap would spill `x` cells onto a second row.
        let buf = terminal.backend().buffer();
        let x_rows = (0..buf.area.height)
            .filter(|&y| (0..buf.area.width).any(|x| buf[(x, y)].symbol() == "x"))
            .count();
        assert_eq!(x_rows, 1, "the long line must not wrap: {text}");
    }
}
