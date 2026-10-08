//! Commit-detail sub-screen (DESIGN §6.5).
//!
//! Reached from the history view by `Enter`, this is a drill-in on the payload
//! the history pane already holds — the same sub-screen shape the diff viewer
//! (`View::Diff`) uses: a full-screen body with a single panel, no focus trap,
//! `Esc` to return to the history view and `q` to quit the application.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};

use super::components::state_panel::{self, PanelState};
use super::components::theme::role;
use super::{hint, with_cursor, PanelFocus};
use crate::git::{CommitDetail, CommitRow};
use crate::ui::components::footer::{hints as join_hints, HintRow};

/// Title of the sub-screen, used by the breadcrumb (§1.2) and the title bar
/// when no commit is highlighted.
const PANEL_TITLE: &str = "提交详情";

/// Share of the panel body the message region keeps (§6.5). The message is
/// the region the reader scrolls, so it gets the larger share; the file list
/// needs only as many rows as it has files.
const BODY_PERCENT: u16 = 60;
const FILE_LIST_PERCENT: u16 = 40;

/// Render the commit-detail sub-screen (§6.5).
///
/// The body is the highlighted commit's message and changed-paths list, read
/// straight from `App.history` — one stack level pushed by `Enter` on the
/// history view, and popped by `Esc`. A load failure of the detail payload
/// lands on this panel's `error` state (§4.3), the same slot the history pane
/// surfaced it in.
pub fn render_commit_detail(f: &mut Frame, content: Rect, app: &mut crate::app::App) {
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
/// Two regions per §6.5: 正文 above, carrying the commit message the reader
/// scrolls, and the 变更文件 list below, one row per changed path. `Tab` moves
/// the focus between them (§3.1) and the border style says which one has it.
///
/// The placeholder case is the same for both regions: a selected commit whose
/// detail has not arrived — or never will, for a commit with nothing to diff —
/// shows the body placeholder and an empty file list; no commit at all (the
/// screen was opened on an empty history) is the same state. That is a "nothing
/// to show" case, never an `error` panel (§4.3).
fn render_commit_detail_content(f: &mut Frame, area: Rect, app: &mut crate::app::App, title: &str) {
    let body = commit_message_lines(app.history.selected_commit(), app.history.detail.as_ref());
    let body_focused = app.panel_is_focused(PanelFocus::Primary);

    // §6.5 draws 正文 and 变更文件 as the two regions of one full-screen
    // panel, and `Tab` moves the focus between them. Two bordered blocks are
    // what makes that focus legible: §3.1 gives the focused one
    // `border_focus` and the other `border`, so the reader can see which
    // region `j`/`k` will move.
    let regions = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(BODY_PERCENT),
            Constraint::Percentage(FILE_LIST_PERCENT),
        ])
        .split(area);

    f.render_widget(
        Paragraph::new(body)
            .wrap(Wrap { trim: false })
            .scroll((app.commit_detail_scroll, 0))
            .block(
                Block::default()
                    .title(title.to_string())
                    .borders(Borders::ALL)
                    .border_style(role::border(body_focused)),
            ),
        regions[0],
    );

    // The file rows carry the `▌` cursor over `surface_alt` only while the file
    // list holds focus; while the body does, the list shows no cursor at all.
    let list = with_cursor(
        List::new(changed_file_rows(app.history.detail.as_ref())),
        !body_focused,
    );
    f.render_stateful_widget(
        list.block(
            Block::default()
                .title(changed_file_title(app.history.detail.as_ref()))
                .borders(Borders::ALL)
                .border_style(role::border(!body_focused)),
        ),
        regions[1],
        &mut app.commit_file_state,
    );
}

/// Title of the changed-file region: the §6.5 `变更文件 (N)` header.
fn changed_file_title(detail: Option<&CommitDetail>) -> String {
    let count = detail.map_or(0, |d| d.files.len());
    format!("变更文件 ({count})")
}

/// One row per changed path.
///
/// The rows are read-only: `Enter` on a file would open that file's diff
/// inside the commit, and the git layer exposes no such API (`git::diff_file`
/// diffs the working tree only, and `src/git/` is frozen this release). So the
/// list advertises no `⏎ 文件diff` hint — a hint the screen cannot honour is
/// worse than a missing one — and `Tab` plus the scroll keys are the verbs the
/// region answers.
fn changed_file_rows(detail: Option<&CommitDetail>) -> Vec<ListItem<'static>> {
    match detail {
        Some(detail) => detail
            .files
            .iter()
            .map(|file| ListItem::new(file.clone()))
            .collect(),
        // No payload: the placeholder row is a caption, not a file, and the
        // cursor must never land on it — `commit_file_len` is what the key
        // handlers move against, and it counts files only.
        None => vec![ListItem::new(Span::styled(
            "（暂无变更文件）",
            role::caption(),
        ))],
    }
}

/// Number of rows the file region's cursor moves over: the changed paths, and
/// nothing else. The placeholder row of [`changed_file_rows`] is deliberately
/// outside this count so `j`/`k`/`g`/`G` cannot select a caption that has no
/// action behind it — the same "a list cursor never points at a non-actionable
/// row" rule the other panels follow.
pub(crate) fn commit_file_len(detail: Option<&CommitDetail>) -> usize {
    detail.map_or(0, |detail| detail.files.len())
}

/// The lines that the 正文 region scrolls: the commit message only. The
/// changed-path list is a separate region (see [`changed_file_rows`]), so the
/// body must not also carry the `变更文件 (N)` header and paths — it never did
/// in the two-region render §6.5 specifies.
///
/// `pub(crate)` because `App`'s `g`/`G` handler derives the 正文 row count
/// from *this* builder rather than re-deriving it arithmetically: the blank
/// separator is only inserted for a non-empty message, so a second formula
/// could not stay in step with the rendered rows.
pub(crate) fn commit_message_lines(
    selected: Option<&CommitRow>,
    detail: Option<&CommitDetail>,
) -> Vec<Line<'static>> {
    match (selected, detail) {
        (Some(_), Some(detail)) => detail
            .message
            .lines()
            .map(|line| Line::from(Span::raw(line.to_string())))
            .collect(),
        // No commit selected, or a commit with no detail payload to show:
        // the placeholder, never an error panel.
        _ => vec![Line::from(Span::styled(
            "选择一个提交查看详情",
            role::caption(),
        ))],
    }
}

/// Footer hint row for the commit-detail sub-screen (§6.5): the two region
/// verbs and the two exits.
///
/// `Tab 面板` is here because §6.5 makes `Tab` switch between 正文 and the
/// file list. `⏎ 文件diff` is deliberately absent: the changed-file rows are
/// read-only this release (see [`changed_file_rows`]), so advertising the key
/// would promise a screen the app cannot open. `Esc 返回` / `q 退出` are the
/// global segment (§2.3 item 1) and must not be restated here.
pub(crate) fn commit_detail_hints() -> HintRow {
    HintRow::new(join_hints([hint("j/k", "滚动"), hint("Tab", "面板")]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, View};
    use crate::config::Config;
    use crate::git::history::RefMark;
    use crate::git::{CommitRow, RefKind};
    use crate::ui::components::cells::{compact, rows_with};
    use crate::ui::components::footer::StatusRow;
    use crate::ui::components::theme::color;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyEvent};
    use ratatui::Terminal;

    /// Build a one-commit history with a detail payload, ready to push.
    fn app_with_commit() -> App {
        let mut app = App::new(Config::default());
        populate_commit(&mut app);
        // §6.5 is a sub-screen, so the tests reach it the way a user does:
        // through the history view's `Enter` (§6.2.3). That also gives the file
        // region the row-0 cursor `open_commit_detail` installs.
        app.view = View::History;
        app.open_commit_detail();
        app
    }

    /// Build a commit-detail screen without any commit: the empty-history case.
    fn app_without_commit() -> App {
        let mut app = App::new(Config::default());
        app.view = View::CommitDetail;
        app
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
    ///
    /// `&mut` because the file region is a stateful list: the screen borrows
    /// the cursor it paints.
    fn render_app(app: &mut App) -> Terminal<TestBackend> {
        // Read the borrowed status before handing `app` to the renderer, which
        // needs it `&mut` for the file region's cursor.
        let status = app.status.clone();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                crate::ui::components::footer::draw_frame(
                    f,
                    StatusRow::plain(status.as_ref()),
                    &commit_detail_hints(),
                    |f, body| render_commit_detail(f, body, app),
                );
            })
            .unwrap();
        terminal
    }

    fn buffer(terminal: &Terminal<TestBackend>) -> &ratatui::buffer::Buffer {
        terminal.backend().buffer()
    }

    /// Dispatch one key press through the real [`App::dispatch_key`] chain, so
    /// a test on this screen exercises the same routing production does (§2.1)
    /// instead of reaching into the private handler.
    fn press(app: &mut App, code: KeyCode) {
        app.dispatch_key(KeyEvent::new(
            code,
            ratatui::crossterm::event::KeyModifiers::NONE,
        ));
    }

    /// Row index of the `变更文件` region's top border, i.e. where the second
    /// region starts (§6.5). Read from the buffer instead of hard-coded so the
    /// test survives a change to the body/file split ratio.
    fn file_region_row(terminal: &Terminal<TestBackend>) -> u16 {
        rows_with(terminal.backend().buffer(), "变更文件")
            .into_iter()
            .next()
            .expect("file region missing")
    }

    /// Top border row of the 正文 region (the title row).
    fn body_region_row(terminal: &Terminal<TestBackend>) -> u16 {
        rows_with(terminal.backend().buffer(), "提交abc1234")
            .into_iter()
            .next()
            .expect("body region missing")
    }

    #[test]
    fn render_shows_commit_header_author_and_time() {
        let mut app = app_with_commit();
        let text = compact(render_app(&mut app).backend().buffer());
        assert!(text.contains("提交abc1234"), "missing short id: {text}");
        assert!(text.contains("作者:GitPulseTest"), "missing author: {text}");
        assert!(text.contains("2023-11-1422:18"), "missing time: {text}");
    }

    #[test]
    fn render_shows_message_body_and_changed_files() {
        let mut app = app_with_commit();
        let text = compact(render_app(&mut app).backend().buffer());
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
        let mut app = app_without_commit();
        let text = compact(render_app(&mut app).backend().buffer());
        assert!(
            text.contains("选择一个提交查看详情"),
            "missing placeholder: {text}"
        );
    }

    #[test]
    fn render_shows_placeholder_when_detail_is_empty() {
        let mut app = app_with_commit();
        app.history.detail = None;
        let text = compact(render_app(&mut app).backend().buffer());
        assert!(
            text.contains("选择一个提交查看详情"),
            "missing placeholder: {text}"
        );
        // The region keeps its header (§6.5 draws it unconditionally) but the
        // stale paths of the previous commit must be gone.
        assert!(!text.contains("src/main.rs"), "stale detail: {text}");
        assert!(!text.contains("src/lib.rs"), "stale detail: {text}");
    }

    /// §6.5 + §3.1: `Tab` moves the focus between 正文 and 变更文件, and the
    /// focused region is the one wearing `border_focus`.
    #[test]
    fn tab_moves_the_focus_border_to_the_file_list() {
        let mut app = app_with_commit();

        let on_body = render_app(&mut app);
        let body = body_region_row(&on_body);
        let file = file_region_row(&on_body);
        assert_ne!(body, file, "the two regions must be drawn apart");
        assert_eq!(
            buffer(&on_body)[(1, body)].fg,
            color::BORDER_FOCUS,
            "正文 opens focused"
        );
        assert_eq!(
            buffer(&on_body)[(1, file)].fg,
            color::BORDER,
            "变更文件 opens unfocused"
        );

        app.cycle_panel_focus(true);
        let on_file = render_app(&mut app);
        assert_eq!(
            buffer(&on_file)[(1, body)].fg,
            color::BORDER,
            "正文 drops the focus border"
        );
        assert_eq!(
            buffer(&on_file)[(1, file)].fg,
            color::BORDER_FOCUS,
            "变更文件 takes the focus border"
        );
    }

    /// §6.5: `Enter` on the changed-file row is not offered this release — the
    /// git layer exposes no per-file diff inside a commit, and the hint row
    /// deliberately withholds `⏎ 文件diff` rather than advertising a screen the
    /// app cannot open.
    #[test]
    fn enter_on_a_file_row_is_inert() {
        let mut app = app_with_commit();
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.commit_file_state.selected(), Some(1));

        assert!(app.dispatch_key(KeyEvent::new(
            KeyCode::Enter,
            ratatui::crossterm::event::KeyModifiers::NONE
        )));
        assert_eq!(app.view, View::CommitDetail, "no drill-in");
        assert_eq!(app.commit_file_state.selected(), Some(1), "no cursor move");
    }

    /// §3.1: the file rows carry the `▌` cursor only while their region has
    /// the focus.
    #[test]
    fn the_cursor_follows_the_focused_region() {
        let mut app = app_with_commit();
        press(&mut app, KeyCode::Tab);
        assert!(
            rows_with(render_app(&mut app).backend().buffer(), "▌src/main.rs").len() == 1,
            "the focused file list must paint the §3.1 cursor"
        );

        press(&mut app, KeyCode::Tab);
        let text = compact(render_app(&mut app).backend().buffer());
        assert!(
            !text.contains('▌'),
            "an unfocused region draws no cursor: {text}"
        );
    }

    /// §6.5: `j` scrolls 正文 while the body holds the focus and walks the
    /// changed-file cursor once the file list has it.
    #[test]
    fn j_scrolls_the_body_then_the_file_list() {
        let mut app = app_with_commit();

        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.commit_detail_scroll, 1, "正文 scrolls first");
        assert_eq!(
            app.commit_file_state.selected(),
            Some(0),
            "the unfocused file list keeps its own cursor"
        );

        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(
            app.commit_detail_scroll, 1,
            "the file list does not scroll 正文"
        );
        assert_eq!(app.commit_file_state.selected(), Some(1));
    }

    #[test]
    fn render_load_error_lands_on_the_panel_not_the_status_row() {
        let mut app = app_with_commit();
        app.load_error = Some("加载提交详情失败: boom".to_string());
        let terminal = render_app(&mut app);
        let text = compact(terminal.backend().buffer());
        assert!(text.contains("加载失败"), "missing error title: {text}");
        assert!(
            text.contains("加载提交详情失败:boom"),
            "missing reason: {text}"
        );
        assert!(text.contains("按r重试"), "missing retry hint: {text}");
        // §4.3: the load failure lands on the panel's error state, not on the
        // footer status row (row 22 in an 80x24 frame).
        assert!(
            !rows_with(terminal.backend().buffer(), "加载提交详情失败:boom").contains(&22),
            "load failure must not reach the footer status row: {text}"
        );
    }

    /// §2.3: the view segment carries only the two verbs the screen owns. The
    /// `Esc 返回` / `q 退出` pair belongs to the global segment, and `⏎
    /// 文件diff` is withheld because the file rows are read-only — so the
    /// rendered row is pinned on all three counts at once.
    #[test]
    fn commit_detail_view_segment_holds_only_the_screen_s_own_verbs() {
        let row = commit_detail_hints();
        let text: String = row
            .view
            .spans
            .iter()
            .map(|s| s.content.to_string())
            .collect();
        let compacted: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        assert_eq!(compacted, "j/k滚动Tab面板", "§6.5 view segment");

        let terminal = render_app(&mut app_with_commit());
        let row = compact(terminal.backend().buffer());
        assert!(row.contains("Esc返回"), "global segment missing: {row}");
        assert!(row.contains("q退出"), "global segment missing: {row}");
        assert!(
            !row.contains("文件diff"),
            "⏎ 文件diff must not be advertised: {row}"
        );
    }

    /// §5.4: scroll offset drops the first visible body line — the title stays
    /// in the border, the first message line scrolls up and out of view while
    /// later body lines remain.
    #[test]
    fn render_with_scroll_offset_loses_the_first_body_line() {
        let mut app = app_with_commit();
        app.commit_detail_scroll = 1;
        let text = compact(render_app(&mut app).backend().buffer());
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
