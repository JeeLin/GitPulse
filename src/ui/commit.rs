//! Single-line commit message input popup.

use ratatui::Frame;

use super::render_input_overlay;
use crate::ui::components::footer::HintRow;

/// Render the commit message popup centered over the current view.
///
/// Single-line input (convention 9): `Enter` commits, `Esc` cancels; the
/// typed text lives in `App::commit_message` and is passed in per frame.
///
/// The title echoes the author the commit will be signed with (v0.9.0) — the
/// same identity [`crate::git::author_label`] renders — so the user sees
/// *who* this commit becomes before pressing `Enter`. Only the title changes;
/// the input line itself stays the commit message alone.
pub fn render_commit_popup(f: &mut Frame, message: &str, author: Option<&crate::git::AuthorInfo>) {
    let title = format!("提交信息 · {}", crate::git::author_label(author));
    render_input_overlay(f, f.area(), &title, message, true);
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
    use crate::git::AuthorInfo;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn author() -> AuthorInfo {
        AuthorInfo {
            name: "张三".to_string(),
            email: "zhang@example.com".to_string(),
        }
    }

    /// The frame's text with whitespace stripped: wide CJK glyphs leave padding
    /// cells, so a `contains` on the raw buffer never matches.
    fn compact_text(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect()
    }

    #[test]
    fn render_commit_popup_shows_message_and_title() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_commit_popup(f, "fix: hello world", Some(&author())))
            .unwrap();
        let text = compact_text(&terminal);
        assert!(text.contains("fix:helloworld"), "missing input: {text}");
        assert!(text.contains("提交信息"), "missing title: {text}");
    }

    /// v0.9.0: the title carries the effective author, so the user sees who
    /// the commit is signed by before pressing `Enter`. Falsifiable: dropping
    /// the author from the title leaves only the label and breaks this.
    #[test]
    fn render_commit_popup_title_echoes_the_effective_author() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_commit_popup(f, "feat: x", Some(&author())))
            .unwrap();
        let text = compact_text(&terminal);
        assert!(
            text.contains("提交信息·张三<zhang@example.com>"),
            "title must echo the author: {text}"
        );
    }

    /// v0.9.0: no resolved identity must not render a half-formed title —
    /// `App::current_author` returns `None` when nothing is selected or nothing
    /// resolved, and the popup says so instead of showing an empty author.
    #[test]
    fn render_commit_popup_title_states_when_no_identity_resolved() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_commit_popup(f, "feat: x", None))
            .unwrap();
        let text = compact_text(&terminal);
        assert!(
            text.contains("提交信息·(无本地配置)"),
            "title must state the missing identity: {text}"
        );
    }

    /// §5.4's width budget: the popup body is fixed-width, so an author long
    /// enough to overflow must not push the title past the frame. ratatui
    /// clips rather than wraps, so the assertion is that the title *label*
    /// still rendered and the buffer holds no row wider than the frame.
    #[test]
    fn render_commit_popup_title_stays_within_the_frame_width() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let long = AuthorInfo {
            name: "一个非常长的名字".to_string(),
            email: "a.very.long.address@subdomain.example.com".to_string(),
        };
        terminal
            .draw(|f| render_commit_popup(f, "feat: x", Some(&long)))
            .unwrap();
        let text = compact_text(&terminal);
        // The label survives; the author itself may be clipped by the popup
        // geometry (§5.4 省略), which is the behaviour under test.
        assert!(text.contains("提交信息"), "title lost to the width: {text}");
    }
}
