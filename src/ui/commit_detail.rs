//! Commit-detail sub-screen (DESIGN §6.5).
//!
//! Reached from the history view by `Enter`, this is a drill-in on the payload
//! the history pane already holds — the same sub-screen shape the diff viewer
//! (`View::Diff`) uses: a full-screen body with a single panel, no focus trap,
//! `Esc` to return to the history view and `q` to quit the application.

use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use super::components::state_panel::{self, PanelState};
use super::components::theme::{color, role};
use super::hint;
use crate::git::{CommitDetail, CommitRow};
use crate::ui::components::footer::{hints as join_hints, HintRow};

/// Title of the sub-screen, used by the breadcrumb (§1.2) and the title bar
/// when no commit is highlighted.
const PANEL_TITLE: &str = "提交详情";

/// Render the commit-detail sub-screen (§6.5).
///
/// The body is the highlighted commit's message and changed-paths list, read
/// straight from `App.history` — one stack level pushed by `Enter` on the
/// history view, and popped by `Esc`. A load failure of the detail payload
/// lands on this panel's `error` state (§4.3), the same slot the history pane
/// surfaced it in.
pub fn render_commit_detail(f: &mut Frame, content: Rect, app: &crate::app::App) {
    let title = pane_title(app);

    if let Some(message) = app.load_error.as_deref() {
        // §4.3: the failure replaces the body. The title keeps naming the
        // commit so the trail still reads as a drill-in on a real row.
        state_panel::render_state_panel(f, content, &title, &PanelState::Error { message });
        return;
    }

    render_commit_detail_content(f, content, app, &title);
}

/// Header line of the sub-screen: `提交 {short_id}   作者: {author}   {time}`.
///
/// The approved deviation for this release (DESIGN §6.5 lists email, relative
/// time and the branch, none of which is shipped here): only the abbreviated
/// id, the author name and the absolute `YYYY-MM-DD HH:MM` are shown, matching
/// the history row's own columns so the two never disagree on a commit.
fn pane_title(app: &crate::app::App) -> String {
    match app.history.selected_commit() {
        Some(commit) => format!(
            "提交 {}   作者: {}   {}",
            commit.id, commit.author, commit.time
        ),
        None => PANEL_TITLE.to_string(),
    }
}

/// Body of the sub-screen once the error state is ruled out.
///
/// * A selected commit with a detail payload draws the message (soft-wrapped),
///   a blank separator and the changed-paths list.
/// * A selected commit whose detail still has not arrived — or never will, for
///   a commit with nothing to diff — falls back to the placeholder; this is a
///   "nothing to show" case, never an `error` panel (§4.3).
/// * No commit at all (the user opened the screen on an empty history) is the
///   same placeholder.
fn render_commit_detail_content(f: &mut Frame, area: Rect, app: &crate::app::App, title: &str) {
    let body = commit_detail_body(app.history.selected_commit(), app.history.detail.as_ref());

    let block = Block::default().title(title).borders(Borders::ALL);

    let paragraph = Paragraph::new(body)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((app.commit_detail_scroll, 0));

    f.render_widget(paragraph, area);
}

/// The body lines: message, blank line, the `变更文件 (N)` header and one
/// indented path per changed file — the layout §6.5 fixes, mirroring the
/// history detail pane's own composition.
fn commit_detail_body(
    selected: Option<&CommitRow>,
    detail: Option<&CommitDetail>,
) -> Vec<Line<'static>> {
    match (selected, detail) {
        (Some(_), Some(detail)) => {
            let mut lines: Vec<Line> = detail
                .message
                .lines()
                .map(|line| Line::from(Span::raw(line.to_string())))
                .collect();
            if !lines.is_empty() {
                lines.push(Line::from(""));
            }
            lines.push(Line::from(Span::styled(
                format!("变更文件 ({})", detail.files.len()),
                Style::default().fg(color::INFO),
            )));
            for file in &detail.files {
                lines.push(Line::from(Span::raw(format!("  {file}"))));
            }
            lines
        }
        // No commit selected, or a commit with no detail payload to show:
        // the placeholder, never an error panel.
        _ => vec![Line::from(Span::styled(
            "选择一个提交查看详情",
            role::caption(),
        ))],
    }
}

/// Footer hint row for the commit-detail sub-screen (§2.3): scroll verbs and
/// the two exits — mirrors [`crate::ui::diff::diff_hints`] so the two
/// full-screen bodies read the same.
pub(crate) fn commit_detail_hints() -> HintRow {
    HintRow::new(join_hints([
        hint("j/k", "滚动"),
        hint("Esc", "返回"),
        hint("q", "退出"),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, View};
    use crate::config::Config;
    use crate::git::history::RefMark;
    use crate::git::{CommitRow, RefKind};
    use crate::ui::components::footer::StatusRow;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// Build a one-commit history with a detail payload, ready to push.
    fn app_with_commit() -> App {
        App::new(Config::default())
    }

    fn populate_commit(app: &mut App) {
        app.history.set_commits(
            vec![CommitRow {
                full_id: "0123456789abcdef0123456789abcdef01234567".to_string(),
                id: "abc1234".to_string(),
                summary: "merge things".to_string(),
                author: "GitPulse Test".to_string(),
                time: "2023-11-14 22:18".to_string(),
                refs: vec![RefMark {
                    kind: RefKind::Head,
                    name: "HEAD".to_string(),
                }],
                parents: vec![],
            }],
            vec![vec!['●'; crate::git::MAX_LANES]],
        );
        // `set_commits` lands on row 0 and clears the detail.
        app.history.detail = Some(CommitDetail {
            message: "merge things\n\nBody of the commit.".to_string(),
            files: vec!["src/main.rs".to_string(), "src/lib.rs".to_string()],
        });
    }

    /// Render one frame through the full footer (§4.1), so the hint row sits
    /// on the bottom row like [`crate::ui::diff`] tests do.
    fn render_app(app: &App) -> Terminal<TestBackend> {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                crate::ui::components::footer::draw_frame(
                    f,
                    StatusRow::plain(app.status.as_ref()),
                    &commit_detail_hints(),
                    |f, body| render_commit_detail(f, body, app),
                );
            })
            .unwrap();
        terminal
    }

    /// Whitespace-stripped buffer text (wide CJK glyphs leave padding cells).
    fn compact(terminal: &Terminal<TestBackend>) -> String {
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

    fn rows_with(terminal: &Terminal<TestBackend>, needle: &str) -> Vec<u16> {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .filter(|y| {
                let raw: String = (0..buffer.area.width)
                    .map(|x| buffer[(x, *y)].symbol().to_string())
                    .collect();
                raw.chars()
                    .filter(|c| !c.is_whitespace())
                    .collect::<String>()
                    .contains(needle)
            })
            .collect()
    }

    #[test]
    fn render_shows_commit_header_author_and_time() {
        let mut app = app_with_commit();
        app.view = View::CommitDetail;
        populate_commit(&mut app);
        let text = compact(&render_app(&app));
        assert!(text.contains("提交abc1234"), "missing short id: {text}");
        assert!(text.contains("作者:GitPulseTest"), "missing author: {text}");
        assert!(text.contains("2023-11-1422:18"), "missing time: {text}");
    }

    #[test]
    fn render_shows_message_body_and_changed_files() {
        let mut app = app_with_commit();
        app.view = View::CommitDetail;
        populate_commit(&mut app);
        let text = compact(&render_app(&app));
        assert!(
            text.contains("Bodyofthecommit."),
            "missing message body: {text}"
        );
        assert!(text.contains("变更文件(2)"), "missing file header: {text}");
        assert!(text.contains("src/main.rs"), "missing path: {text}");
        assert!(text.contains("src/lib.rs"), "missing path: {text}");
    }

    #[test]
    fn render_shows_placeholder_without_a_selected_commit() {
        let mut app = App::new(Config::default());
        app.view = View::CommitDetail;
        let text = compact(&render_app(&app));
        assert!(
            text.contains("选择一个提交查看详情"),
            "missing placeholder: {text}"
        );
    }

    #[test]
    fn render_shows_placeholder_when_detail_is_empty() {
        let mut app = app_with_commit();
        app.view = View::CommitDetail;
        populate_commit(&mut app);
        app.history.detail = None;
        let text = compact(&render_app(&app));
        assert!(
            text.contains("选择一个提交查看详情"),
            "missing placeholder: {text}"
        );
        assert!(!text.contains("变更文件"), "stale detail: {text}");
    }

    #[test]
    fn render_load_error_lands_on_the_panel_not_the_status_row() {
        let mut app = app_with_commit();
        app.view = View::CommitDetail;
        populate_commit(&mut app);
        app.load_error = Some("加载提交详情失败: boom".to_string());
        let terminal = render_app(&app);
        let text = compact(&terminal);
        assert!(text.contains("加载失败"), "missing error title: {text}");
        assert!(
            text.contains("加载提交详情失败:boom"),
            "missing reason: {text}"
        );
        assert!(text.contains("按r重试"), "missing retry hint: {text}");
        // §4.3: the load failure lands on the panel's error state, not on the
        // footer status row (row 22 in an 80x24 frame).
        assert!(
            !rows_with(&terminal, "加载提交详情失败:boom").contains(&22),
            "load failure must not reach the footer status row: {text}"
        );
    }

    #[test]
    fn commit_detail_hints_show_scroll_back_and_quit() {
        let row = commit_detail_hints();
        let text: String = row
            .view
            .spans
            .iter()
            .map(|s| s.content.to_string())
            .collect();
        let compacted: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            compacted.contains("j/k滚动"),
            "missing scroll hint: {compacted}"
        );
        assert!(
            compacted.contains("Esc返回"),
            "missing back hint: {compacted}"
        );
        assert!(
            compacted.contains("q退出"),
            "missing quit hint: {compacted}"
        );
    }

    /// §5.4: scroll offset drops the first visible body line — the title stays
    /// in the border, the first message line scrolls up and out of view while
    /// later body lines remain.
    #[test]
    fn render_with_scroll_offset_loses_the_first_body_line() {
        let mut app = app_with_commit();
        app.view = View::CommitDetail;
        populate_commit(&mut app);
        app.commit_detail_scroll = 1;
        let text = compact(&render_app(&app));
        assert!(
            text.contains("提交abc1234"),
            "title must stay visible: {text}"
        );
        assert!(
            !text.contains("mergethings"),
            "scrolled-away first body line still shown: {text}"
        );
        assert!(
            text.contains("Bodyofthecommit."),
            "a later body line must stay visible: {text}"
        );
    }
}
