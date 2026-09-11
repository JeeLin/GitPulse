use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::search::SearchResult;

/// 搜索面板状态
pub struct SearchState {
    pub results: Vec<SearchResult>,
    pub selected: Option<usize>,
    pub is_visible: bool,
    pub query: String,
}

impl SearchState {
    pub fn new() -> Self {
        Self {
            results: Vec::new(),
            selected: Some(0),
            is_visible: false,
            query: String::new(),
        }
    }

    pub fn toggle_visibility(&mut self) {
        self.is_visible = !self.is_visible;
    }

    pub fn update_results(&mut self, results: Vec<SearchResult>) {
        self.results = results;
        self.selected = Some(0);
    }

    pub fn selected_result(&self) -> Option<&SearchResult> {
        self.selected.and_then(|i| self.results.get(i))
    }
}

pub fn render_search(f: &mut Frame, state: &SearchState, area: Rect) {
    if !state.is_visible {
        let help = Paragraph::new(vec![
            Line::from("按 '/' 键打开搜索"),
            Line::from(""),
            Line::from(format!("搜索结果: {}", state.results.len())),
        ])
        .block(Block::default().title("搜索").borders(Borders::ALL))
        .wrap(Wrap { trim: true });
        f.render_widget(help, area);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let result_items: Vec<ListItem> = state
        .results
        .iter()
        .map(|result| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("[{}]", result.repo_name),
                    Style::default().fg(Color::Cyan),
                ),
                Span::raw(" "),
                Span::raw(&result.file_path),
                Span::raw(":"),
                Span::styled(
                    format!("{}", result.line_number),
                    Style::default().fg(Color::Yellow),
                ),
            ]))
        })
        .collect();

    let mut list_state = ListState::default();
    list_state.select(state.selected);

    let result_list = List::new(result_items)
        .block(Block::default().title("搜索结果").borders(Borders::ALL))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol(">> ");

    f.render_stateful_widget(result_list, chunks[0], &mut list_state);

    if let Some(result) = state.selected_result() {
        let detail = format!(
            "仓库: {}\n文件: {}\n行号: {}\n内容: {}",
            result.repo_name, result.file_path, result.line_number, result.line_content
        );
        let paragraph = Paragraph::new(detail)
            .block(Block::default().title("结果详情").borders(Borders::ALL))
            .wrap(Wrap { trim: true });
        f.render_widget(paragraph, chunks[1]);
    }
}

pub fn handle_search_input(key: crossterm::event::KeyCode, state: &mut SearchState) -> bool {
    if !state.is_visible {
        return false;
    }

    match key {
        crossterm::event::KeyCode::Esc | crossterm::event::KeyCode::Char('/') => {
            state.toggle_visibility();
            true
        }
        crossterm::event::KeyCode::Up | crossterm::event::KeyCode::Char('k') => {
            let i = state.selected.map_or(0, |i| if i == 0 { 0 } else { i - 1 });
            state.selected = Some(i);
            true
        }
        crossterm::event::KeyCode::Down | crossterm::event::KeyCode::Char('j') => {
            let i = state.selected.map_or(0, |i| {
                if i >= state.results.len().saturating_sub(1) {
                    0
                } else {
                    i + 1
                }
            });
            state.selected = Some(i);
            true
        }
        _ => false,
    }
}
