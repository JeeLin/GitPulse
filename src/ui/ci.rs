use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::github::ci::WorkflowRun;

/// CI 面板状态
pub struct CIState {
    pub runs: Vec<WorkflowRun>,
    pub selected: Option<usize>,
    pub is_visible: bool,
}

impl CIState {
    pub fn new() -> Self {
        Self {
            runs: Vec::new(),
            selected: Some(0),
            is_visible: false,
        }
    }

    pub fn toggle_visibility(&mut self) {
        self.is_visible = !self.is_visible;
    }

    pub fn update_runs(&mut self, runs: Vec<WorkflowRun>) {
        self.runs = runs;
        self.selected = Some(0);
    }

    pub fn selected_run(&self) -> Option<&WorkflowRun> {
        self.selected.and_then(|i| self.runs.get(i))
    }
}

pub fn render_ci(f: &mut Frame, state: &CIState, area: Rect) {
    if !state.is_visible {
        let help = Paragraph::new(vec![
            Line::from("按 'C' 键打开 CI 状态"),
            Line::from(""),
            Line::from(format!("Workflow 数量: {}", state.runs.len())),
        ])
        .block(Block::default().title("CI/CD").borders(Borders::ALL))
        .wrap(Wrap { trim: true });
        f.render_widget(help, area);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    let run_items: Vec<ListItem> = state
        .runs
        .iter()
        .map(|run| {
            let style = match run.conclusion.as_deref() {
                Some("success") => Style::default().fg(Color::Green),
                Some("failure") => Style::default().fg(Color::Red),
                _ => Style::default(),
            };

            ListItem::new(Line::from(vec![
                Span::styled(run.status_icon(), style),
                Span::raw(" "),
                Span::raw(&run.name),
                Span::styled(
                    format!(" ({})", run.status),
                    Style::default().fg(Color::DarkGray),
                ),
            ]))
        })
        .collect();

    let mut list_state = ListState::default();
    list_state.select(state.selected);

    let run_list = List::new(run_items)
        .block(Block::default().title("Workflow Runs").borders(Borders::ALL))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol(">> ");

    f.render_stateful_widget(run_list, chunks[0], &mut list_state);

    if let Some(run) = state.selected_run() {
        let detail = format!(
            "ID: {}\n名称: {}\n状态: {}\n结果: {}\n创建时间: {}\n更新时间: {}",
            run.id,
            run.name,
            run.status,
            run.conclusion.as_deref().unwrap_or("进行中"),
            run.created_at,
            run.updated_at
        );
        let paragraph = Paragraph::new(detail)
            .block(Block::default().title("Run 详情").borders(Borders::ALL))
            .wrap(Wrap { trim: true });
        f.render_widget(paragraph, chunks[1]);
    }
}

pub fn handle_ci_input(key: crossterm::event::KeyCode, state: &mut CIState) -> bool {
    if !state.is_visible {
        return false;
    }

    match key {
        crossterm::event::KeyCode::Esc | crossterm::event::KeyCode::Char('C') => {
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
                if i >= state.runs.len().saturating_sub(1) { 0 } else { i + 1 }
            });
            state.selected = Some(i);
            true
        }
        _ => false,
    }
}