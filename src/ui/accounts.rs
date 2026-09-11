use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::accounts::Account;

/// 账户面板状态
pub struct AccountState {
    /// 账户列表
    pub accounts: Vec<Account>,
    /// 选中的账户索引
    pub selected: Option<usize>,
    /// 面板是否可见
    pub is_visible: bool,
    /// 当前活跃账户名称
    pub active_account: Option<String>,
}

impl AccountState {
    /// 创建新的账户面板状态
    pub fn new() -> Self {
        Self {
            accounts: Vec::new(),
            selected: Some(0),
            is_visible: false,
            active_account: None,
        }
    }

    /// 切换面板显示
    pub fn toggle_visibility(&mut self) {
        self.is_visible = !self.is_visible;
    }

    /// 更新账户数据
    pub fn update_accounts(&mut self, accounts: Vec<Account>) {
        self.accounts = accounts;
        self.selected = Some(0);
    }

    /// 设置活跃账户
    pub fn set_active_account(&mut self, name: Option<String>) {
        self.active_account = name;
    }

    /// 获取当前选中的账户
    pub fn selected_account(&self) -> Option<&Account> {
        self.selected.and_then(|i| self.accounts.get(i))
    }
}

/// 渲染账户面板
pub fn render_accounts(f: &mut Frame, state: &AccountState, area: Rect) {
    // 主布局：左侧账户列表 + 右侧账户详情

    // 主布局：左侧账户列表 + 右侧账户详情
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    // 账户列表
    let account_items: Vec<ListItem> = state
        .accounts
        .iter()
        .map(|account| {
            let is_active = state.active_account.as_deref() == Some(&account.name);
            let status_icon = if is_active { "● " } else { "  " };
            let status_style = if is_active {
                Style::default().fg(Color::Green)
            } else {
                Style::default()
            };

            ListItem::new(Line::from(vec![
                Span::styled(status_icon, status_style),
                Span::raw(&account.name),
                Span::raw(" "),
                Span::styled(
                    format!("({})", account.platform),
                    Style::default().fg(Color::DarkGray),
                ),
            ]))
        })
        .collect();

    let mut list_state = ListState::default();
    list_state.select(state.selected);

    let account_list = List::new(account_items)
        .block(Block::default().title("账户列表").borders(Borders::ALL))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol(">> ");

    f.render_stateful_widget(account_list, chunks[0], &mut list_state);

    // 账户详情
    if let Some(account) = state.selected_account() {
        let detail = format!(
            "名称: {}\n平台: {}\n用户名: {}\n邮箱: {}",
            account.name,
            account.platform,
            account.default_username.as_deref().unwrap_or("未设置"),
            account.default_email.as_deref().unwrap_or("未设置")
        );
        let paragraph = Paragraph::new(detail)
            .block(Block::default().title("账户详情").borders(Borders::ALL))
            .wrap(Wrap { trim: true });
        f.render_widget(paragraph, chunks[1]);
    }
}

/// 处理账户面板的键盘事件
pub fn handle_account_input(key: crossterm::event::KeyCode, state: &mut AccountState) -> bool {
    if !state.is_visible {
        return false;
    }

    match key {
        crossterm::event::KeyCode::Esc | crossterm::event::KeyCode::Char('A') => {
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
                if i >= state.accounts.len().saturating_sub(1) {
                    0
                } else {
                    i + 1
                }
            });
            state.selected = Some(i);
            true
        }
        crossterm::event::KeyCode::Enter => {
            if let Some(account) = state.selected_account() {
                state.set_active_account(Some(account.name.clone()));
            }
            true
        }
        _ => false,
    }
}
