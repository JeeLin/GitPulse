use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::github::trending::TrendingRepo;

/// Trending 面板状态
pub struct TrendingState {
    pub repos: Vec<TrendingRepo>,
    pub selected: Option<usize>,
    pub is_visible: bool,
}

impl TrendingState {
    pub fn new() -> Self {
        Self {
            repos: Vec::new(),
            selected: Some(0),
            is_visible: false,
        }
    }

    pub fn toggle_visibility(&mut self) {
        self.is_visible = !self.is_visible;
    }

    pub fn update_repos(&mut self, repos: Vec<TrendingRepo>) {
        self.repos = repos;
        self.selected = Some(0);
    }

    pub fn selected_repo(&self) -> Option<&TrendingRepo> {
        self.selected.and_then(|i| self.repos.get(i))
    }
}

pub fn render_trending(f: &mut Frame, state: &TrendingState, area: Rect) {
    if !state.is_visible {
        let help = Paragraph::new(vec![
            Line::from("按 'T' 键打开 Trending"),
            Line::from(""),
            Line::from(format!("Trending 仓库: {}", state.repos.len())),
        ])
        .block(Block::default().title("Trending").borders(Borders::ALL))
        .wrap(Wrap { trim: true });
        f.render_widget(help, area);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(area);

    let repo_items: Vec<ListItem> = state
        .repos
        .iter()
        .map(|repo| {
            let lang = repo.language.as_deref().unwrap_or("N/A");
            ListItem::new(Line::from(vec![
                Span::styled(format!("[{}]", lang), Style::default().fg(Color::Cyan)),
                Span::raw(" "),
                Span::raw(&repo.full_name),
                Span::styled(
                    format!(" ⭐{}", repo.stars),
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled(
                    format!(" +{}/day", repo.stars_today),
                    Style::default().fg(Color::Green),
                ),
            ]))
        })
        .collect();

    let mut list_state = ListState::default();
    list_state.select(state.selected);

    let repo_list = List::new(repo_items)
        .block(
            Block::default()
                .title("Trending 仓库")
                .borders(Borders::ALL),
        )
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol(">> ");

    f.render_stateful_widget(repo_list, chunks[0], &mut list_state);

    if let Some(repo) = state.selected_repo() {
        let detail = format!(
            "{}\n描述: {}\n语言: {}\nStars: {} (今日 +{})\nForks: {}",
            repo.full_name,
            repo.description.as_deref().unwrap_or("无描述"),
            repo.language.as_deref().unwrap_or("N/A"),
            repo.stars,
            repo.stars_today,
            repo.forks
        );
        let paragraph = Paragraph::new(detail)
            .block(Block::default().title("仓库详情").borders(Borders::ALL))
            .wrap(Wrap { trim: true });
        f.render_widget(paragraph, chunks[1]);
    }
}

pub fn handle_trending_input(key: crossterm::event::KeyCode, state: &mut TrendingState) -> bool {
    if !state.is_visible {
        return false;
    }

    match key {
        crossterm::event::KeyCode::Esc | crossterm::event::KeyCode::Char('T') => {
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
                if i >= state.repos.len().saturating_sub(1) {
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
