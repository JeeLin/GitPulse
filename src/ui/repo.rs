use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::app::StatusLine;
use crate::git::{FileEntry, FileState};

/// State for rendering the single-repo view.
pub struct RepoState<'a> {
    pub entries: &'a [FileEntry],
    pub status: Option<&'a StatusLine>,
    pub list_state: &'a mut ListState,
    pub show_discard_confirm: bool,
    pub confirm_file: Option<&'a FileEntry>,
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

    // Render status line
    render_status_line(f, status_area, state.status);

    // Render key hints
    let hints = Line::from(vec![
        Span::styled("j/k", Style::default().fg(Color::Cyan)),
        Span::raw(" 选择  "),
        Span::styled("d", Style::default().fg(Color::Cyan)),
        Span::raw(" diff  "),
        Span::styled("s", Style::default().fg(Color::Cyan)),
        Span::raw(" stage  "),
        Span::styled("u", Style::default().fg(Color::Cyan)),
        Span::raw(" unstage  "),
        Span::styled("x", Style::default().fg(Color::Cyan)),
        Span::raw(" discard  "),
        Span::styled("Esc", Style::default().fg(Color::Cyan)),
        Span::raw(" 返回"),
    ]);
    f.render_widget(Paragraph::new(hints).style(Style::default()), hints_area);

    // Overlay discard confirmation dialog
    if state.show_discard_confirm {
        if let Some(file) = state.confirm_file {
            render_discard_dialog(f, area, file);
        }
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
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("► ");

    f.render_stateful_widget(list, area, state.list_state);
}

fn render_status_line(f: &mut Frame, area: Rect, status: Option<&StatusLine>) {
    if let Some(status) = status {
        let style = if status.is_error {
            Style::default().fg(Color::Red)
        } else {
            Style::default()
        };
        f.render_widget(Paragraph::new(status.message.clone()).style(style), area);
    }
}

fn render_discard_dialog(f: &mut Frame, area: Rect, file: &FileEntry) {
    let popup_area = centered_rect(50, 10, area);

    let block = Block::default()
        .title("确认丢弃更改")
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::Red));

    let text = vec![
        Line::from(vec![
            Span::raw("确定丢弃 "),
            Span::styled(
                file.path.display().to_string(),
                Style::default().fg(Color::Yellow),
            ),
            Span::raw(" 的更改?"),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("y", Style::default().fg(Color::Cyan)),
            Span::raw(" 确认  "),
            Span::styled("n", Style::default().fg(Color::Cyan)),
            Span::raw(" 取消"),
        ]),
    ];

    let paragraph = Paragraph::new(text).block(block).wrap(Wrap { trim: false });

    f.render_widget(Clear, popup_area);
    f.render_widget(paragraph, popup_area);
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
