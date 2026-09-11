use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::github::pulls::PullRequest;

/// PR 面板状态
pub struct PullRequestState {
    pub pulls: Vec<PullRequest>,
    pub selected: Option<usize>,
    pub is_visible: bool,
    pub filter: Option<String>,
}

impl PullRequestState {
    pub fn new() -> Self {
        Self {
            pulls: Vec::new(),
            selected: Some(0),
            is_visible: false,
            filter: None,
        }
    }

    pub fn toggle_visibility(&mut self) {
        self.is_visible = !self.is_visible;
    }

    pub fn update_pulls(&mut self, pulls: Vec<PullRequest>) {
        self.pulls = pulls;
        self.selected = Some(0);
    }

    pub fn selected_pull(&self) -> Option<&PullRequest> {
        self.selected.and_then(|i| self.pulls.get(i))
    }
}

pub fn render_pulls(f: &mut Frame, state: &PullRequestState, area: Rect) {
    if !state.is_visible {
        let help = Paragraph::new(vec![
            Line::from("按 'P' 键打开 PR 列表"),
            Line::from(""),
            Line::from(format!("PR 数量: {}", state.pulls.len())),
        ])
        .block(Block::default().title("Pull Requests").borders(Borders::ALL))
        .wrap(Wrap { trim: true });
        f.render_widget(help, area);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    let pull_items: Vec<ListItem> = state
        .pulls
        .iter()
        .map(|pr| {
            let state_style = match pr.state.as_str() {
                "open" => Style::default().fg(Color::Green),
                "merged" => Style::default().fg(Color::Magenta),
                _ => Style::default().fg(Color::Red),
            };
            let ci_icon = match pr.ci_status.as_deref() {
                Some("success") => "✅",
                Some("failure") => "❌",
                _ => "❓",
            };

            ListItem::new(Line::from(vec![
                Span::styled(format!("[{}]", pr.state), state_style),
                Span::raw(format!(" #{}", pr.number)),
                Span::raw(" "),
                Span::raw(&pr.title),
                Span::raw(" "),
                Span::raw(ci_icon),
            ]))
        })
        .collect();

    let mut list_state = ListState::default();
    list_state.select(state.selected);

    let pull_list = List::new(pull_items)
        .block(Block::default().title("PR 列表").borders(Borders::ALL))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol(">> ");

    f.render_stateful_widget(pull_list, chunks[0], &mut list_state);

    if let Some(pr) = state.selected_pull() {
        let detail = format!(
            "#{}: {}\n状态: {}\n作者: {}\nCI: {}\n创建时间: {}",
            pr.number,
            pr.title,
            pr.state,
            pr.author,
            pr.ci_status.as_deref().unwrap_or("未知"),
            pr.created_at
        );
        let paragraph = Paragraph::new(detail)
            .block(Block::default().title("PR 详情").borders(Borders::ALL))
            .wrap(Wrap { trim: true });
        f.render_widget(paragraph, chunks[1]);
    }
}

pub fn handle_pull_input(key: crossterm::event::KeyCode, state: &mut PullRequestState) -> bool {
    if !state.is_visible {
        return false;
    }

    match key {
        crossterm::event::KeyCode::Esc | crossterm::event::KeyCode::Char('P') => {
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
                if i >= state.pulls.len().saturating_sub(1) { 0 } else { i + 1 }
            });
            state.selected = Some(i);
            true
        }
        _ => false,
    }
}