use crate::notifications::{NotificationDb, NotificationState, NotificationType};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

/// 渲染通知面板
pub fn render_notifications(f: &mut Frame, state: &NotificationState, db: &NotificationDb, area: Rect) {
    if !state.is_visible {
        let unread_count = db.get_unread_notifications().map(|n| n.len()).unwrap_or(0);
        let help = Paragraph::new(vec![
            Line::from("按 'n' 键打开通知中心"),
            Line::from(""),
            Line::from(format!("共 {} 个未读通知", unread_count)),
        ])
        .block(Block::default().title("通知").borders(Borders::ALL))
        .wrap(Wrap { trim: true });
        f.render_widget(help, area);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
        .split(area);

    let items: Vec<ListItem> = state.notifications.iter().map(|n| {
        let type_str = match n.notification_type {
            NotificationType::Issue => "Issue",
            NotificationType::PullRequest => "PR",
            NotificationType::Ci => "CI",
            NotificationType::Release => "Release",
            NotificationType::Mention => "@",
        };
        let style = if n.unread { Style::default().fg(Color::Yellow) } else { Style::default().fg(Color::Gray) };
        ListItem::new(Line::from(vec![
            Span::styled(format!("[{}] ", type_str), style),
            Span::raw(&n.repo),
            Span::raw(" - "),
            Span::raw(&n.title),
        ]))
    }).collect();

    let list = List::new(items)
        .block(Block::default().title("通知列表").borders(Borders::ALL))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol(">> ");
    let mut ls = ListState::default();
    ls.select(state.list_selected);
    f.render_stateful_widget(list, chunks[0], &mut ls);

    if let Some(n) = state.selected_notification() {
        let time_str = n.updated_at.format("%Y-%m-%d %H:%M:%S").to_string();
        let detail = vec![
            Line::from(vec![Span::styled("标题: ", Style::default().fg(Color::Cyan)), Span::raw(&n.title)]),
            Line::from(vec![Span::styled("仓库: ", Style::default().fg(Color::Cyan)), Span::raw(&n.repo)]),
            Line::from(vec![Span::styled("原因: ", Style::default().fg(Color::Cyan)), Span::raw(&n.reason)]),
            Line::from(vec![Span::styled("时间: ", Style::default().fg(Color::Cyan)), Span::raw(&time_str)]),
            Line::from(""),
            Line::from("按 Enter 在浏览器中打开"),
        ];
        let paragraph = Paragraph::new(detail)
            .block(Block::default().title("详情").borders(Borders::ALL))
            .wrap(Wrap { trim: true });
        f.render_widget(paragraph, chunks[1]);
    }
}

/// 处理通知面板的键盘事件，返回 true 表示事件已处理
pub fn handle_notification_input(key: crossterm::event::KeyCode, state: &mut NotificationState, db: &NotificationDb) -> bool {
    if !state.is_visible { return false; }
    match key {
        crossterm::event::KeyCode::Esc | crossterm::event::KeyCode::Char('n') => {
            state.toggle_visibility(); true
        }
        crossterm::event::KeyCode::Up | crossterm::event::KeyCode::Char('k') => {
            let i = state.list_selected.map_or(0, |i| if i == 0 { state.notifications.len().saturating_sub(1) } else { i - 1 });
            state.list_selected = Some(i); true
        }
        crossterm::event::KeyCode::Down | crossterm::event::KeyCode::Char('j') => {
            let i = state.list_selected.map_or(0, |i| if i >= state.notifications.len().saturating_sub(1) { 0 } else { i + 1 });
            state.list_selected = Some(i); true
        }
        crossterm::event::KeyCode::Char('m') => {
            if let Some(n) = state.selected_notification() {
                let id = n.id.clone();
                if db.mark_as_read(&id).is_ok() {
                    if let Some(n) = state.notifications.iter_mut().find(|n| n.id == id) { n.unread = false; }
                }
            }
            true
        }
        crossterm::event::KeyCode::Char('a') => {
            if db.mark_all_as_read().is_ok() {
                for n in &mut state.notifications { n.unread = false; }
            }
            true
        }
        _ => false,
    }
}
