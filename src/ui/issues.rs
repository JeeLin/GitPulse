use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::github::issues::Issue;

/// Issue 面板状态
pub struct IssueState {
    /// Issue 列表
    pub issues: Vec<Issue>,
    /// 选中的 Issue 索引
    pub selected: Option<usize>,
    /// 面板是否可见
    pub is_visible: bool,
    /// 过滤条件（状态过滤）
    pub filter: Option<String>,
}

impl IssueState {
    /// 创建新的 Issue 面板状态
    pub fn new() -> Self {
        Self {
            issues: Vec::new(),
            selected: Some(0),
            is_visible: false,
            filter: None,
        }
    }

    /// 切换面板显示
    pub fn toggle_visibility(&mut self) {
        self.is_visible = !self.is_visible;
    }

    /// 更新 Issue 数据
    pub fn update_issues(&mut self, issues: Vec<Issue>) {
        self.issues = issues;
        self.selected = Some(0);
    }

    /// 获取当前选中的 Issue
    pub fn selected_issue(&self) -> Option<&Issue> {
        self.selected.and_then(|i| self.issues.get(i))
    }
}

/// 渲染 Issue 面板
pub fn render_issues(f: &mut Frame, state: &IssueState, area: Rect) {
    if !state.is_visible {
        let help = Paragraph::new(vec![
            Line::from("按 'I' 键打开 Issue 列表"),
            Line::from(""),
            Line::from(format!("Issue 数量: {}", state.issues.len())),
        ])
        .block(Block::default().title("Issues").borders(Borders::ALL))
        .wrap(Wrap { trim: true });
        f.render_widget(help, area);
        return;
    }

    // 主布局：左侧 Issue 列表 + 右侧 Issue 详情
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    // Issue 列表
    let issue_items: Vec<ListItem> = state
        .issues
        .iter()
        .map(|issue| {
            let state_icon = if issue.state == "open" {
                "🟢"
            } else {
                "🔴"
            };
            let state_style = if issue.state == "open" {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::Red)
            };

            ListItem::new(Line::from(vec![
                Span::styled(state_icon, state_style),
                Span::raw(format!("#{}", issue.number)),
                Span::raw(" "),
                Span::raw(&issue.title),
            ]))
        })
        .collect();

    let mut list_state = ListState::default();
    list_state.select(state.selected);

    let issue_list = List::new(issue_items)
        .block(Block::default().title("Issue 列表").borders(Borders::ALL))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol(">> ");

    f.render_stateful_widget(issue_list, chunks[0], &mut list_state);

    // Issue 详情
    if let Some(issue) = state.selected_issue() {
        let detail = format!(
            "#{}: {}\n状态: {}\n作者: {}\n标签: {}\n创建时间: {}",
            issue.number,
            issue.title,
            issue.state,
            issue.author,
            issue.labels.join(", "),
            issue.created_at
        );
        let paragraph = Paragraph::new(detail)
            .block(Block::default().title("Issue 详情").borders(Borders::ALL))
            .wrap(Wrap { trim: true });
        f.render_widget(paragraph, chunks[1]);
    }
}

/// 处理 Issue 面板的键盘事件
pub fn handle_issue_input(key: crossterm::event::KeyCode, state: &mut IssueState) -> bool {
    if !state.is_visible {
        return false;
    }

    match key {
        crossterm::event::KeyCode::Esc | crossterm::event::KeyCode::Char('I') => {
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
                if i >= state.issues.len().saturating_sub(1) {
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
