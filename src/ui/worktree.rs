use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::git::worktree::WorktreeInfo;

/// Worktree 面板状态
pub struct WorktreeState {
    pub worktrees: Vec<WorktreeInfo>,
    pub selected: Option<usize>,
    pub is_visible: bool,
}

impl WorktreeState {
    pub fn new() -> Self {
        Self {
            worktrees: Vec::new(),
            selected: Some(0),
            is_visible: false,
        }
    }

    pub fn toggle_visibility(&mut self) {
        self.is_visible = !self.is_visible;
    }

    pub fn update_worktrees(&mut self, worktrees: Vec<WorktreeInfo>) {
        self.worktrees = worktrees;
        self.selected = Some(0);
    }

    pub fn selected_worktree(&self) -> Option<&WorktreeInfo> {
        self.selected.and_then(|i| self.worktrees.get(i))
    }
}

pub fn render_worktrees(f: &mut Frame, state: &WorktreeState, area: Rect) {
    if !state.is_visible {
        let help = Paragraph::new(vec![
            Line::from("按 'W' 键打开 Worktree 面板"),
            Line::from(""),
            Line::from(format!("Worktree 数量: {}", state.worktrees.len())),
        ])
        .block(Block::default().title("Worktrees").borders(Borders::ALL))
        .wrap(Wrap { trim: true });
        f.render_widget(help, area);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    let wt_items: Vec<ListItem> = state
        .worktrees
        .iter()
        .map(|wt| {
            let main_icon = if wt.is_main { "● " } else { "  " };
            let style = if wt.is_main {
                Style::default().fg(Color::Green)
            } else {
                Style::default()
            };

            ListItem::new(Line::from(vec![
                Span::styled(main_icon, style),
                Span::raw(wt.display_name()),
                Span::styled(
                    format!(" ({})", wt.branch),
                    Style::default().fg(Color::DarkGray),
                ),
            ]))
        })
        .collect();

    let mut list_state = ListState::default();
    list_state.select(state.selected);

    let wt_list = List::new(wt_items)
        .block(Block::default().title("Worktree 列表").borders(Borders::ALL))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol(">> ");

    f.render_stateful_widget(wt_list, chunks[0], &mut list_state);

    if let Some(wt) = state.selected_worktree() {
        let detail = format!(
            "路径: {}\n分支: {}\nHEAD: {}\n主 Worktree: {}",
            wt.path.display(),
            wt.branch,
            wt.head,
            if wt.is_main { "是" } else { "否" }
        );
        let paragraph = Paragraph::new(detail)
            .block(Block::default().title("Worktree 详情").borders(Borders::ALL))
            .wrap(Wrap { trim: true });
        f.render_widget(paragraph, chunks[1]);
    }
}

pub fn handle_worktree_input(key: crossterm::event::KeyCode, state: &mut WorktreeState) -> bool {
    if !state.is_visible {
        return false;
    }

    match key {
        crossterm::event::KeyCode::Esc | crossterm::event::KeyCode::Char('W') => {
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
                if i >= state.worktrees.len().saturating_sub(1) { 0 } else { i + 1 }
            });
            state.selected = Some(i);
            true
        }
        _ => false,
    }
}