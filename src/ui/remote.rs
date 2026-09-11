use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::github::repos::RemoteRepo;

/// 远端仓库面板状态
pub struct RemoteRepoState {
    /// 仓库列表
    pub repos: Vec<RemoteRepo>,
    /// 选中的仓库索引
    pub selected: Option<usize>,
    /// 面板是否可见
    pub is_visible: bool,
    /// 过滤条件（组织/个人）
    pub filter: Option<String>,
}

impl RemoteRepoState {
    /// 创建新的远端仓库面板状态
    pub fn new() -> Self {
        Self {
            repos: Vec::new(),
            selected: Some(0),
            is_visible: false,
            filter: None,
        }
    }

    /// 切换面板显示
    pub fn toggle_visibility(&mut self) {
        self.is_visible = !self.is_visible;
    }

    /// 更新仓库数据
    pub fn update_repos(&mut self, repos: Vec<RemoteRepo>) {
        self.repos = repos;
        self.selected = Some(0);
    }

    /// 获取当前选中的仓库
    pub fn selected_repo(&self) -> Option<&RemoteRepo> {
        self.selected.and_then(|i| self.repos.get(i))
    }
}

/// 渲染远端仓库面板
pub fn render_remote_repos(f: &mut Frame, state: &RemoteRepoState, area: Rect) {
    if !state.is_visible {
        let help = Paragraph::new(vec![
            Line::from("按 'R' 键打开远端仓库"),
            Line::from(""),
            Line::from(format!("仓库数量: {}", state.repos.len())),
        ])
        .block(Block::default().title("远端仓库").borders(Borders::ALL))
        .wrap(Wrap { trim: true });
        f.render_widget(help, area);
        return;
    }

    // 主布局：左侧仓库列表 + 右侧仓库详情
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    // 仓库列表
    let repo_items: Vec<ListItem> = state
        .repos
        .iter()
        .map(|repo| {
            let fork_icon = if repo.is_fork { "⑂ " } else { "  " };
            let star_style = if repo.stars > 0 {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            };

            ListItem::new(Line::from(vec![
                Span::raw(fork_icon),
                Span::raw(&repo.name),
                Span::raw(" "),
                Span::styled(format!("★{}", repo.stars), star_style),
            ]))
        })
        .collect();

    let mut list_state = ListState::default();
    list_state.select(state.selected);

    let repo_list = List::new(repo_items)
        .block(Block::default().title("仓库列表").borders(Borders::ALL))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol(">> ");

    f.render_stateful_widget(repo_list, chunks[0], &mut list_state);

    // 仓库详情
    if let Some(repo) = state.selected_repo() {
        let detail = format!(
            "名称: {}\n描述: {}\n语言: {}\nStars: {}\n更新时间: {}\nFork: {}\nURL: {}",
            repo.full_name,
            repo.description.as_deref().unwrap_or("无"),
            repo.language.as_deref().unwrap_or("未知"),
            repo.stars,
            repo.updated_at,
            if repo.is_fork { "是" } else { "否" },
            repo.clone_url
        );
        let paragraph = Paragraph::new(detail)
            .block(Block::default().title("仓库详情").borders(Borders::ALL))
            .wrap(Wrap { trim: true });
        f.render_widget(paragraph, chunks[1]);
    }
}

/// 处理远端仓库面板的键盘事件
pub fn handle_remote_repo_input(
    key: crossterm::event::KeyCode,
    state: &mut RemoteRepoState,
) -> bool {
    if !state.is_visible {
        return false;
    }

    match key {
        crossterm::event::KeyCode::Esc | crossterm::event::KeyCode::Char('R') => {
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
