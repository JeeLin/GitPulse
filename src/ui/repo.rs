use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState},
    Frame,
};

use super::{hint, pulls, render_confirm_overlay};
use crate::git::FileEntry;
use crate::ui::components::footer::{hints as join_hints, HintRow};
use crate::ui::status::{RemoteStatus, StatusLine};

/// State for rendering the single-repo view.
pub struct RepoState<'a> {
    pub entries: &'a [FileEntry],
    pub status: Option<&'a StatusLine>,
    pub list_state: &'a mut ListState,
    pub show_discard_confirm: bool,
    pub confirm_file: Option<&'a FileEntry>,
    /// Push/pull feedback; takes over the status line while present.
    pub remote_status: Option<&'a RemoteStatus>,
    /// Label of the running/last remote operation (`push` / `pull`).
    pub remote_op: &'a str,
}

/// Render the single-repo file status list above the footer's status row.
///
/// The panel draws its content only (§2.3): the hint row is composed once by
/// the footer from [`repo_hints`], so nothing here reserves a row for it.
pub fn render_repo(f: &mut Frame, content: Rect, mut state: RepoState<'_>) {
    let area = content;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),    // file list
            Constraint::Length(1), // status row
        ])
        .split(area);

    let file_list_area = chunks[0];
    let status_area = chunks[1];

    // Render file list
    render_file_list(f, file_list_area, &mut state);

    // Render status line: remote push/pull feedback takes precedence, and a
    // notice written while it runs shares the row with it (F1). The
    // branch itself is shared with the tag panel (G6).
    super::render_status_or_remote(
        f,
        status_area,
        state.remote_status,
        state.remote_op,
        state.status,
    );

    // Overlay discard confirmation dialog
    if state.show_discard_confirm {
        if let Some(file) = state.confirm_file {
            render_discard_dialog(f, area, file);
        }
    }
}

/// Bottom key-hint row: y/n/Esc while the discard confirmation is open (every
/// other list key is swallowed then), the file-list actions otherwise. Related
/// keys share one hint (`s/u`, `p/l`, `b/h/t`) so the row — including the
/// panel entry — stays within the 80-column bottom line. The commit message
/// input owns its own row, see [`crate::ui::commit::commit_hints`].
fn key_hints(discard_confirm_open: bool) -> Line<'static> {
    if discard_confirm_open {
        // Shared row (G5): the y/n/Esc wording of the confirmation, matching
        // the popup title `确认丢弃更改`.
        super::confirm_hints()
    } else {
        join_hints([
            hint("j/k", "选择"),
            hint("g", "进入"),
            hint("s/u", "暂存切换"),
            hint("x", "丢弃"),
            hint("c", "commit"),
            hint("p/l", "push/pull"),
            hint("b/h/t", "面板"),
            hint("Esc", "返回"),
        ])
    }
}

/// Footer hint row for the single-repo view (§2.3): the list actions, or the
/// y/n/Esc row while the discard dialog owns focus. The row itself is composed
/// by [`crate::ui::components::footer::compose_hint_row`] through
/// [`join_hints`], so the key/desc pairs keep the two-cell gap §2.3 fixes.
pub(crate) fn repo_hints(show_discard_confirm: bool) -> HintRow {
    HintRow::new(key_hints(show_discard_confirm))
}

/// Render file list
fn render_file_list(f: &mut Frame, area: Rect, state: &mut RepoState<'_>) {
    let width = pulls::row_width(area);
    let items: Vec<ListItem> = state
        .entries
        .iter()
        .map(|entry| {
            let (symbol, color) = super::file_state_symbol(entry.state);

            let line = Line::from(vec![
                Span::styled(symbol, Style::default().fg(color)),
                Span::raw(pulls::ellipsize(&entry.path.display().to_string(), width)),
            ]);
            ListItem::new(line)
        })
        .collect();

    let block = Block::default()
        .title(format!("文件状态 ({} 文件)", state.entries.len()))
        .borders(Borders::ALL);

    let list = List::new(items)
        .block(block)
        .highlight_style(super::list_highlight())
        .highlight_symbol(super::HIGHLIGHT_SYMBOL);

    f.render_stateful_widget(list, area, state.list_state);
}

fn render_discard_dialog(f: &mut Frame, area: Rect, file: &FileEntry) {
    let body = vec![Line::from(vec![
        Span::raw("确定丢弃 "),
        Span::styled(
            file.path.display().to_string(),
            Style::default().fg(Color::Yellow),
        ),
        Span::raw(" 的更改?"),
    ])];
    render_confirm_overlay(f, area, "确认丢弃更改", 50, 10, body, true);
}

/// Create a centered rectangle within the given area.
pub fn centered_rect(percent_x: u16, height: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - height) / 2),
            Constraint::Length(height),
            Constraint::Percentage((100 - height) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::FileState;
    use crate::ui::components::footer::StatusRow;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn entry(path: &str, state: FileState) -> FileEntry {
        FileEntry {
            path: std::path::PathBuf::from(path),
            state,
        }
    }

    fn base_state<'a>(
        entries: &'a [FileEntry],
        list_state: &'a mut ListState,
        status: Option<&'a StatusLine>,
    ) -> RepoState<'a> {
        RepoState {
            entries,
            status,
            list_state,
            show_discard_confirm: false,
            confirm_file: None,
            remote_status: None,
            remote_op: "",
        }
    }

    /// The panel draws content only (§2.3), so the frame goes through
    /// [`crate::ui::components::footer::draw_frame`] — the shape
    /// `ui::render` uses — and the footer takes the bottom two rows.
    fn render_to_terminal(state: RepoState<'_>) -> Terminal<TestBackend> {
        render_to_terminal_sized(state, 80, 24)
    }

    fn render_to_terminal_sized(
        state: RepoState<'_>,
        width: u16,
        height: u16,
    ) -> Terminal<TestBackend> {
        let hints = repo_hints(state.show_discard_confirm);
        let status_row = StatusRow::new(state.status, state.remote_status, state.remote_op);
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                crate::ui::components::footer::draw_frame(f, status_row, &hints, |f, body| {
                    render_repo(f, body, state)
                });
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

    /// Foreground color of the footer's status row: §4.1 puts it at
    /// `height - 2`, directly above the hint row.
    fn status_fg(terminal: &Terminal<TestBackend>) -> Color {
        let buffer = terminal.backend().buffer();
        buffer[(0, buffer.area.height - 2)].fg
    }

    /// Whitespace-stripped text of a single row; the hint row is `height-1`.
    fn row_compact(terminal: &Terminal<TestBackend>, y: u16) -> String {
        let buffer = terminal.backend().buffer();
        let raw: String = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol().to_string())
            .collect();
        raw.chars().filter(|c| !c.is_whitespace()).collect()
    }

    /// Render a hint row alone into an 80x24 frame, so a test can read the
    /// view segment the footer would compose from it.
    fn render_hints_row(row: &HintRow) -> Terminal<TestBackend> {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                crate::ui::components::footer::draw_frame(
                    f,
                    StatusRow::plain(None),
                    row,
                    |_, _| {},
                );
            })
            .unwrap();
        terminal
    }

    /// View segment of the hint row: the bottom row without the global
    /// segment (§2.3), which is fixed on every screen and would otherwise
    /// answer for words like `返回` in the assertions below.
    fn view_segment(terminal: &Terminal<TestBackend>) -> String {
        let row = row_compact(terminal, terminal.backend().buffer().area.height - 1);
        let prefix: String = crate::ui::components::footer::global_hints()
            .spans
            .iter()
            .map(|s| s.content.to_string())
            .collect();
        let prefix: String = prefix.chars().filter(|c| !c.is_whitespace()).collect();
        row.strip_prefix(&prefix)
            .unwrap_or(row.as_str())
            .to_string()
    }

    #[test]
    fn render_file_list_shows_paths_states_and_status_line() {
        let entries = vec![
            entry("src/lib.rs", FileState::Modified),
            entry("notes.txt", FileState::Untracked),
        ];
        let status = StatusLine {
            message: "工作区有 2 处变更".to_string(),
            is_error: false,
        };
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        let terminal = render_to_terminal(base_state(&entries, &mut list_state, Some(&status)));
        let text = compact(&terminal);
        assert!(text.contains("文件状态(2文件)"), "missing title: {text}");
        assert!(
            text.contains("Msrc/lib.rs"),
            "missing modified path: {text}"
        );
        assert!(
            text.contains("?notes.txt"),
            "missing untracked path: {text}"
        );
        assert!(text.contains("工作区有2处变更"), "missing status: {text}");
        // Non-error status renders with the default (unstyled) foreground.
        assert_eq!(status_fg(&terminal), Color::Reset, "status must be plain");
    }

    #[test]
    fn render_file_list_covers_remaining_state_symbols() {
        let entries = vec![
            entry("added.txt", FileState::Added),
            entry("deleted.txt", FileState::Deleted),
            entry("conflict.txt", FileState::Conflicted),
        ];
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        let terminal = render_to_terminal(base_state(&entries, &mut list_state, None));
        let text = compact(&terminal);
        assert!(text.contains("Aadded.txt"), "missing added symbol: {text}");
        assert!(
            text.contains("Ddeleted.txt"),
            "missing deleted symbol: {text}"
        );
        assert!(
            text.contains("Uconflict.txt"),
            "missing conflicted symbol: {text}"
        );
    }

    #[test]
    fn render_error_status_line_shows_reason_in_red() {
        let status = StatusLine {
            message: "加载文件列表失败: boom".to_string(),
            is_error: true,
        };
        let mut list_state = ListState::default();
        let terminal = render_to_terminal(base_state(&[], &mut list_state, Some(&status)));
        let text = compact(&terminal);
        assert!(text.contains("boom"), "missing error: {text}");
        assert_eq!(status_fg(&terminal), Color::Red, "error must render red");
    }

    #[test]
    fn render_discard_confirm_shows_title_file_and_keys() {
        let entries = vec![entry("src/lib.rs", FileState::Modified)];
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        let mut state = base_state(&entries, &mut list_state, None);
        state.show_discard_confirm = true;
        state.confirm_file = entries.first();
        let terminal = render_to_terminal(state);
        let text = compact(&terminal);
        assert!(
            text.contains("确认丢弃更改"),
            "missing dialog title: {text}"
        );
        assert!(text.contains("src/lib.rs"), "missing file name: {text}");
        assert!(text.contains("y确认"), "missing confirm key: {text}");
        assert!(text.contains("n取消"), "missing cancel key: {text}");
    }

    #[test]
    fn render_discard_confirm_switches_bottom_key_hints() {
        let entries = vec![entry("src/lib.rs", FileState::Modified)];
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        let mut state = base_state(&entries, &mut list_state, None);
        state.show_discard_confirm = true;
        state.confirm_file = entries.first();
        let terminal = render_to_terminal(state);
        let hints = view_segment(&terminal);
        // The dialog swallows every list key: none of them may be advertised.
        for swallowed in [
            "j/k",
            "diff",
            "暂存切换",
            "丢弃",
            "commit",
            "push",
            "pull",
            "面板",
        ] {
            assert!(
                !hints.contains(swallowed),
                "swallowed key {swallowed} still hinted: {hints}"
            );
        }
        // Only the keys the confirmation actually handles are shown.
        assert!(hints.contains("y确认"), "missing confirm hint: {hints}");
        assert!(hints.contains("n取消"), "missing cancel hint: {hints}");
        assert!(hints.contains("Esc取消对话框"), "missing Esc hint: {hints}");
    }

    /// The commit popup routes every press through `input_modal_key`, so the
    /// list actions behind it may not be advertised either. The popup is
    /// drawn by [`crate::ui::commit`] over this panel, so the switched row is
    /// [`crate::ui::commit::commit_hints`]' own output.
    #[test]
    fn render_commit_input_switches_bottom_key_hints() {
        let terminal = render_hints_row(&crate::ui::commit::commit_hints());
        let hints = view_segment(&terminal);
        for swallowed in [
            "j/k",
            "diff",
            "暂存切换",
            "丢弃",
            "commit",
            "push",
            "pull",
            "面板",
        ] {
            assert!(
                !hints.contains(swallowed),
                "swallowed key {swallowed} still hinted: {hints}"
            );
        }
        assert!(hints.contains("Enter提交"), "missing submit hint: {hints}");
        assert!(hints.contains("Esc取消"), "missing cancel hint: {hints}");
    }

    /// The status row used to drop a pending status whenever the running
    /// feedback was on screen, which hid the `已有远端操作进行中` notice of a
    /// rejected second push (F1). Both now share the row: the running
    /// message keeps its own cell and the status renders beside it — and
    /// with no progress source left (G1), the row carries the message
    /// rather than a frozen percentage.
    #[test]
    fn render_remote_running_shows_message_and_pending_status_line() {
        let status = StatusLine {
            message: "底层状态行".to_string(),
            is_error: false,
        };
        let running = RemoteStatus::Running("正在 push…".to_string());
        let mut list_state = ListState::default();
        let mut state = base_state(&[], &mut list_state, Some(&status));
        state.remote_status = Some(&running);
        state.remote_op = "push";
        let terminal = render_to_terminal(state);
        let text = compact(&terminal);
        assert!(text.contains("正在push"), "missing running message: {text}");
        assert!(
            !text.contains('%'),
            "frozen progress percent must be gone: {text}"
        );
        assert!(
            text.contains("底层状态行"),
            "pending status hidden by the running line: {text}"
        );
    }

    #[test]
    fn render_remote_success_names_operation_in_green() {
        let mut list_state = ListState::default();
        let mut state = base_state(&[], &mut list_state, None);
        state.remote_status = Some(&RemoteStatus::Success);
        state.remote_op = "push";
        let terminal = render_to_terminal(state);
        let text = compact(&terminal);
        assert!(text.contains("push成功"), "missing success text: {text}");
        assert_eq!(status_fg(&terminal), Color::Green, "success must be green");
    }

    #[test]
    fn render_remote_error_shows_reason_in_red() {
        let error = RemoteStatus::Error("pull 失败: diverged".to_string());
        let mut list_state = ListState::default();
        let mut state = base_state(&[], &mut list_state, None);
        state.remote_status = Some(&error);
        state.remote_op = "pull";
        let terminal = render_to_terminal(state);
        let text = compact(&terminal);
        assert!(text.contains("diverged"), "missing reason: {text}");
        assert_eq!(status_fg(&terminal), Color::Red, "error must be red");
    }

    /// A 120-column frame keeps the whole view segment on screen: at 80 the
    /// footer truncates it (§2.3), which {@link key_hints_fit_within_80_columns}
    /// pins separately.
    #[test]
    fn render_key_hints_show_action_keys() {
        let entries = vec![entry("src/lib.rs", FileState::Modified)];
        let mut list_state = ListState::default();
        let terminal =
            render_to_terminal_sized(base_state(&entries, &mut list_state, None), 120, 24);
        let text = compact(&terminal);
        assert!(text.contains("j/k选择"), "missing select hint: {text}");
        assert!(text.contains("暂存切换"), "missing stage hint: {text}");
        assert!(text.contains("丢弃"), "missing discard hint: {text}");
        assert!(text.contains("commit"), "missing commit hint: {text}");
        assert!(text.contains("push"), "missing push hint: {text}");
        assert!(text.contains("pull"), "missing pull hint: {text}");
        assert!(text.contains("b/h/t面板"), "missing panel hint: {text}");
        assert!(text.contains("Esc返回"), "missing back hint: {text}");
    }

    /// The hint row must never overflow an 80-column terminal. The repo
    /// view segment alone is wider than the 24 cells the global segment
    /// keeps, so the row ends in `…` (§2.3) rather than wrapping — and the
    /// global segment survives whole.
    #[test]
    fn key_hints_fit_within_80_columns() {
        let terminal = render_hints_row(&repo_hints(false));
        let hint = view_segment(&terminal);
        let hint = format!("⏎打开Esc返回r刷新q退出{hint}");
        assert!(
            hint.starts_with("⏎打开Esc返回r刷新q退出"),
            "global segment must stay whole: {hint}"
        );
        assert!(hint.ends_with('…'), "the cut must be marked: {hint}");
        assert!(
            hint.contains("j/k选择"),
            "the head of the view segment stays: {hint}"
        );
    }

    #[test]
    fn centered_rect_is_horizontally_centered_within_parent() {
        let r = centered_rect(50, 10, Rect::new(0, 0, 80, 24));
        assert_eq!(r.x, 20, "must be horizontally centered: {r:?}");
        assert_eq!(r.width, 40, "must take 50% of the width: {r:?}");
        assert!(r.height > 0, "height must be positive: {r:?}");
        assert!(r.y + r.height <= 24, "must stay inside the parent: {r:?}");
    }
}
