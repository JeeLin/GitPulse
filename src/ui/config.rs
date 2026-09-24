//! Scan-dir configuration panel: `scan_dirs` list with add/delete overlays.

use std::path::PathBuf;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::app::StatusLine;

/// Borrowed render state for the config panel, referencing `App.config.scan_dirs`
/// directly so no mock data source exists (checkpoint 2).
pub struct ConfigState<'a> {
    /// Live scan-dir list from `App.config.scan_dirs`.
    pub scan_dirs: &'a [PathBuf],
    /// Current status line, if any; errors render red and persist.
    pub status: Option<&'a StatusLine>,
    /// Selection cursor shared across frames.
    pub list_state: &'a mut ListState,
}

/// Render the scan-dir list with the bottom status line and key hints.
///
/// Modal input / delete-confirm overlays land with later subtasks.
pub fn render_config(f: &mut Frame, mut state: ConfigState<'_>) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),    // scan-dir list
            Constraint::Length(1), // status line
            Constraint::Length(1), // key hints
        ])
        .split(area);

    render_dir_list(f, chunks[0], &mut state);

    if let Some(status) = state.status {
        let style = if status.is_error {
            Style::default().fg(Color::Red)
        } else {
            Style::default()
        };
        f.render_widget(
            Paragraph::new(status.message.clone()).style(style),
            chunks[1],
        );
    }

    let hints = Line::from(vec![
        Span::styled("j/k", Style::default().fg(Color::Cyan)),
        Span::raw(" 选择  "),
        Span::styled("a", Style::default().fg(Color::Cyan)),
        Span::raw(" 添加  "),
        Span::styled("d", Style::default().fg(Color::Cyan)),
        Span::raw(" 删除  "),
        Span::styled("Esc", Style::default().fg(Color::Cyan)),
        Span::raw(" 返回"),
    ]);
    f.render_widget(Paragraph::new(hints), chunks[2]);
}

/// Draw the bordered scan-dir list; the title carries the entry count.
fn render_dir_list(f: &mut Frame, area: Rect, state: &mut ConfigState<'_>) {
    let items: Vec<ListItem> = state
        .scan_dirs
        .iter()
        .map(|dir| ListItem::new(Line::raw(dir.display().to_string())))
        .collect();

    let block = Block::default()
        .title(format!("扫描目录配置 ({})", state.scan_dirs.len()))
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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn render_to_text(state: ConfigState<'_>) -> String {
        let backend = TestBackend::new(80, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render_config(f, state)).unwrap();
        // Wide CJK glyphs leave padding cells; compare whitespace-stripped.
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
    fn render_list_shows_title_dirs_and_hints() {
        let dirs = vec![
            PathBuf::from("/home/user/repos"),
            PathBuf::from("/opt/projects"),
        ];
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        let text = render_to_text(ConfigState {
            scan_dirs: &dirs,
            status: None,
            list_state: &mut list_state,
        });
        assert!(text.contains("扫描目录配置(2)"), "missing title: {text}");
        assert!(
            text.contains("/home/user/repos"),
            "missing first dir: {text}"
        );
        assert!(text.contains("/opt/projects"), "missing second dir: {text}");
        assert!(text.contains("j/k选择"), "missing nav hint: {text}");
        assert!(text.contains("a添加"), "missing add hint: {text}");
        assert!(text.contains("d删除"), "missing delete hint: {text}");
        assert!(text.contains("Esc返回"), "missing back hint: {text}");
    }

    #[test]
    fn render_empty_list_keeps_title_and_hints() {
        let mut list_state = ListState::default();
        let text = render_to_text(ConfigState {
            scan_dirs: &[],
            status: None,
            list_state: &mut list_state,
        });
        assert!(text.contains("扫描目录配置(0)"), "missing title: {text}");
        assert!(text.contains("Esc返回"), "missing back hint: {text}");
    }

    #[test]
    fn render_error_status_shows_reason_on_status_line() {
        let status = StatusLine {
            message: "目录不能为空".to_string(),
            is_error: true,
        };
        let mut list_state = ListState::default();
        let text = render_to_text(ConfigState {
            scan_dirs: &[],
            status: Some(&status),
            list_state: &mut list_state,
        });
        assert!(text.contains("目录不能为空"), "missing error: {text}");
    }
}
