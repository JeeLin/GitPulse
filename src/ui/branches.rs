use std::collections::HashMap;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::branches::{BranchInfo, BranchStatus};

/// 分支面板状态
pub struct BranchState {
    /// 按仓库分组的分支列表
    pub branches: HashMap<String, Vec<BranchInfo>>,
    /// 选中的分支索引
    pub selected: Option<usize>,
    /// 面板是否可见
    pub is_visible: bool,
    /// 当前过滤状态
    pub filter: Option<BranchStatus>,
    /// 是否触发删除已合并分支
    pub delete_merged: bool,
}

impl BranchState {
    /// 创建新的分支面板状态
    pub fn new() -> Self {
        Self {
            branches: HashMap::new(),
            selected: Some(0),
            is_visible: false,
            filter: None,
            delete_merged: false,
        }
    }

    /// 切换面板显示
    pub fn toggle_visibility(&mut self) {
        self.is_visible = !self.is_visible;
    }

    /// 更新分支数据
    pub fn update_branches(&mut self, branches: Vec<BranchInfo>) {
        self.branches.clear();
        for branch in branches {
            self.branches
                .entry(branch.repo_name.clone())
                .or_insert_with(Vec::new)
                .push(branch);
        }
        self.selected = Some(0);
    }

    /// 设置过滤状态
    pub fn set_filter(&mut self, status: Option<BranchStatus>) {
        self.filter = status;
        self.selected = Some(0);
    }

    /// 获取过滤后的分支列表
    pub fn filtered_branches(&self) -> Vec<&BranchInfo> {
        let mut result = Vec::new();
        for branches in self.branches.values() {
            for branch in branches {
                if self.filter.as_ref().map_or(true, |f| branch.status == *f) {
                    result.push(branch);
                }
            }
        }
        result
    }

    /// 获取各状态分支数量
    pub fn status_counts(&self) -> HashMap<BranchStatus, usize> {
        let mut counts = HashMap::new();
        for branches in self.branches.values() {
            for branch in branches {
                *counts.entry(branch.status.clone()).or_insert(0) += 1;
            }
        }
        counts
    }
}

/// 渲染分支面板
pub fn render_branches(f: &mut Frame, state: &BranchState, area: Rect) {
    if !state.is_visible {
        let help = Paragraph::new(vec![
            Line::from("按 'b' 键打开分支管理"),
            Line::from(""),
            Line::from(format!(
                "共 {} 个仓库",
                state.branches.len()
            )),
        ])
        .block(Block::default().title("分支管理").borders(Borders::ALL))
        .wrap(Wrap { trim: true });
        f.render_widget(help, area);
        return;
    }

    // 主布局：左侧仓库列表 + 右侧分支列表
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(area);

    // 仓库列表
    let repo_items: Vec<ListItem> = state
        .branches
        .keys()
        .map(|repo| ListItem::new(Line::from(Span::raw(repo))))
        .collect();

    let repo_list = List::new(repo_items)
        .block(Block::default().title("仓库").borders(Borders::ALL))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol(">> ");

    // 过滤后的分支列表
    let filtered = state.filtered_branches();
    let branch_items: Vec<ListItem> = filtered
        .iter()
        .map(|branch| {
            let status_str = match branch.status {
                BranchStatus::Clean => Span::styled("🟢 ", Style::default().fg(Color::Green)),
                BranchStatus::Stale => Span::styled("🟡 ", Style::default().fg(Color::Yellow)),
                BranchStatus::Orphaned => Span::styled("🟡 ", Style::default().fg(Color::Yellow)),
                BranchStatus::Merged => Span::styled("🔴 ", Style::default().fg(Color::Red)),
            };

            ListItem::new(Line::from(vec![
                status_str,
                Span::raw(&branch.name),
                Span::raw(" "),
                Span::styled(
                    format!("({})", if branch.is_remote { "remote" } else { "local" }),
                    Style::default().fg(Color::DarkGray),
                ),
            ]))
        })
        .collect();

    let mut list_state = ListState::default();
    list_state.select(state.selected);

    let branch_list = List::new(branch_items)
        .block(Block::default().title("分支列表").borders(Borders::ALL))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol(">> ");

    f.render_stateful_widget(repo_list, chunks[0], &mut list_state);
    f.render_stateful_widget(branch_list, chunks[1], &mut list_state);
}

/// 处理分支面板的键盘事件
pub fn handle_branch_input(key: crossterm::event::KeyCode, state: &mut BranchState) -> bool {
    if !state.is_visible {
        return false;
    }

    match key {
        crossterm::event::KeyCode::Esc | crossterm::event::KeyCode::Char('b') => {
            state.toggle_visibility();
            true
        }
        crossterm::event::KeyCode::Up | crossterm::event::KeyCode::Char('k') => {
            let i = state.selected.map_or(0, |i| if i == 0 { 0 } else { i - 1 });
            state.selected = Some(i);
            true
        }
        crossterm::event::KeyCode::Down | crossterm::event::KeyCode::Char('j') => {
            let filtered = state.filtered_branches();
            let i = state.selected.map_or(0, |i| {
                if i >= filtered.len().saturating_sub(1) {
                    0
                } else {
                    i + 1
                }
            });
            state.selected = Some(i);
            true
        }
        crossterm::event::KeyCode::Char('1') => {
            state.set_filter(Some(BranchStatus::Merged));
            true
        }
        crossterm::event::KeyCode::Char('2') => {
            state.set_filter(Some(BranchStatus::Stale));
            true
        }
        crossterm::event::KeyCode::Char('3') => {
            state.set_filter(Some(BranchStatus::Orphaned));
            true
        }
        crossterm::event::KeyCode::Char('4') => {
            state.set_filter(Some(BranchStatus::Clean));
            true
        }
        crossterm::event::KeyCode::Char('0') => {
            state.set_filter(None);
            true
        }
        crossterm::event::KeyCode::Char('D') => {
            // D 键：标记删除已合并分支
            state.delete_merged = true;
            true
        }
        _ => false,
    }
}