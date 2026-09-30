use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use super::{hint, remote, render_confirm_overlay, render_status_line};
use crate::app::{RemoteStatus, StatusLine};
use crate::git::{FileEntry, FileState};

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
    /// Whether the commit message input is open (`View::Commit`), which
    /// renders this view as the popup's background.
    pub show_commit_input: bool,
}

/// Render the single-repo file status list with a bottom status line and key hints.
pub fn render_repo(f: &mut Frame, mut state: RepoState<'_>) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),    // file list
            Constraint::Length(1), // status line
            Constraint::Length(1), // key hints
        ])
        .split(area);

    let file_list_area = chunks[0];
    let status_area = chunks[1];
    let hints_area = chunks[2];

    // Render file list
    render_file_list(f, file_list_area, &mut state);

    // Render status line: remote push/pull feedback takes precedence.
    match state.remote_status {
        Some(status) => remote::render_remote_status(f, status_area, status, state.remote_op),
        None => render_status_line(f, status_area, state.status),
    }

    // Render key hints (single-space separators keep the line under 80 cols)
    f.render_widget(
        Paragraph::new(key_hints(
            state.show_discard_confirm,
            state.show_commit_input,
        ))
        .style(Style::default()),
        hints_area,
    );

    // Overlay discard confirmation dialog
    if state.show_discard_confirm {
        if let Some(file) = state.confirm_file {
            render_discard_dialog(f, area, file);
        }
    }
}

/// Bottom key-hint row: y/n/Esc while the discard confirmation is open (every
/// other list key is swallowed then), Enter/Esc while the commit message input
/// is open (`input_modal_key` eats the buffer keys), the file-list actions
/// otherwise. Related keys share one hint (`s/u`, `p/l`, `b/h/t`) so the row
/// — including the panel entry — stays within the 80-column bottom line.
fn key_hints(discard_confirm_open: bool, commit_input_open: bool) -> Line<'static> {
    if discard_confirm_open {
        // Same y/n wording as the overlay itself.
        [
            hint("y", "确认"),
            hint("n", "取消"),
            hint("Esc", "取消对话框"),
        ]
        .into_iter()
        .flatten()
        .collect()
    } else if commit_input_open {
        // Matches the popup title `提交信息 (Enter 提交 / Esc 取消)`.
        [hint("Enter", "提交"), hint("Esc", "取消")]
            .into_iter()
            .flatten()
            .collect()
    } else {
        [
            hint("j/k", "选择"),
            hint("d", "diff"),
            hint("s/u", "暂存切换"),
            hint("x", "丢弃"),
            hint("c", "commit"),
            hint("p/l", "push/pull"),
            hint("b/h/t", "面板"),
            hint("Esc", "返回"),
        ]
        .into_iter()
        .flatten()
        .collect()
    }
}

fn render_file_list(f: &mut Frame, area: Rect, state: &mut RepoState<'_>) {
    let items: Vec<ListItem> = state
        .entries
        .iter()
        .map(|entry| {
            let (symbol, color) = match entry.state {
                FileState::Modified => ("M ", Color::Yellow),
                FileState::Added => ("A ", Color::Green),
                FileState::Deleted => ("D ", Color::Red),
                FileState::Untracked => ("? ", Color::Cyan),
                FileState::Conflicted => ("U ", Color::Magenta),
            };

            let line = Line::from(vec![
                Span::styled(symbol, Style::default().fg(color)),
                Span::raw(entry.path.display().to_string()),
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
    use crate::app::Progress;
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
            show_commit_input: false,
        }
    }

    fn render_to_terminal(state: RepoState<'_>) -> Terminal<TestBackend> {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render_repo(f, state)).unwrap();
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

    /// Foreground color of the status-line row (list 22 + status 1 + hints 1 = 24).
    fn status_fg(terminal: &Terminal<TestBackend>) -> Color {
        terminal.backend().buffer()[(0, 22)].fg
    }

    /// Whitespace-stripped text of a single row (the bottom hints row is 23).
    fn row_compact(terminal: &Terminal<TestBackend>, y: u16) -> String {
        let buffer = terminal.backend().buffer();
        let raw: String = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol().to_string())
            .collect();
        raw.chars().filter(|c| !c.is_whitespace()).collect()
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
        let hints = row_compact(&terminal, 23);
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
    /// list actions behind it may not be advertised either.
    #[test]
    fn render_commit_input_switches_bottom_key_hints() {
        let entries = vec![entry("src/lib.rs", FileState::Modified)];
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        let mut state = base_state(&entries, &mut list_state, None);
        state.show_commit_input = true;
        let terminal = render_to_terminal(state);
        let hints = row_compact(&terminal, 23);
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

    #[test]
    fn render_remote_progress_shows_gauge_instead_of_status_line() {
        let status = StatusLine {
            message: "底层状态行".to_string(),
            is_error: false,
        };
        let progress = RemoteStatus::Progress(Progress {
            percent: 42,
            message: "正在 push…".to_string(),
        });
        let mut list_state = ListState::default();
        let mut state = base_state(&[], &mut list_state, Some(&status));
        state.remote_status = Some(&progress);
        state.remote_op = "push";
        let terminal = render_to_terminal(state);
        let text = compact(&terminal);
        assert!(text.contains("正在push"), "missing gauge message: {text}");
        assert!(text.contains("42%"), "missing gauge percent: {text}");
        assert!(
            !text.contains("底层状态行"),
            "status line leaked over gauge: {text}"
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

    #[test]
    fn render_key_hints_show_action_keys() {
        let entries = vec![entry("src/lib.rs", FileState::Modified)];
        let mut list_state = ListState::default();
        let terminal = render_to_terminal(base_state(&entries, &mut list_state, None));
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

    /// The bottom hint row must never overflow the 80-column terminal: the
    /// panel entry (`b/h/t`) is budgeted together with the action hints, so a
    /// regression here would silently clip the trailing `Esc 返回`.
    #[test]
    fn key_hints_fit_within_80_columns() {
        let width = key_hints(false, false).width();
        assert!(width <= 80, "hints row is {width} columns wide");
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
