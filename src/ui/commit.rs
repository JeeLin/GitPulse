//! Single-line commit message input popup.

use ratatui::Frame;

use super::render_input_overlay;
use crate::ui::components::footer::HintRow;

/// Render the commit message popup centered over the current view.
///
/// Single-line input (convention 9): `Enter` commits, `Esc` cancels; the
/// typed text lives in `App::commit_message` and is passed in per frame.
pub fn render_commit_popup(f: &mut Frame, message: &str) {
    render_input_overlay(f, f.area(), "提交信息", message, true);
}

/// Footer hint row for the commit view (§2.3): the commit input owns focus,
/// so the row advertises only its submit/cancel keys — `Enter 提交 / Esc 取消`
/// (convention 9), re-using [`super::submit_hints`] so the wording matches the
/// popup title `提交信息 (Enter 提交 / Esc 取消)`.
pub(crate) fn commit_hints() -> HintRow {
    HintRow::new(super::submit_hints("提交"))
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
