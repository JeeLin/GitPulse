//! Notifications receipts panel: repository-scoped GitHub notifications.

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::app::App;
use crate::github::NotificationSummary;

use super::pulls::{
    cells, ellipsize, list_cursor, meta_budget, panel_chunks, panel_title, render_disconnected,
    render_footer, row_width, META_GAP,
};

/// Empty-state copy: nothing to review. Zero rows is a valid state — the
/// status row below covers failed loads.
const EMPTY_NOTIFICATIONS: &str = "—（暂无通知）";

/// Marker of an unread row: the dot is what catches the eye first, so it is
/// yellow and bold; read rows keep a blank marker to stay aligned.
const UNREAD_MARKER: &str = "● ";

/// Render the repository-scoped notifications list.
pub fn render_notifications(f: &mut Frame, app: &App) {
    let area = f.area();
    let chunks = panel_chunks(area);
    if app.github_state.client.is_none() {
        render_disconnected(f, chunks[0], "通知", app);
        render_footer(f, chunks[1], chunks[2], app);
        return;
    }

    let notifications = &app.github_state.notifications;
    let block = Block::default()
        .title(panel_title(
            "通知",
            &app.github_state.owner,
            &app.github_state.repo,
        ))
        .borders(Borders::ALL);

    if notifications.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(
                EMPTY_NOTIFICATIONS,
                Style::default().fg(Color::DarkGray),
            ))
            .block(block),
            chunks[0],
        );
    } else {
        let width = row_width(chunks[0]);
        let items: Vec<ListItem> = notifications
            .iter()
            .map(|note| notification_item(note, width))
            .collect();
        let mut state = ListState::default();
        state.select(list_cursor(app.github_state.selected, notifications.len()));
        let list = List::new(items)
            .block(block)
            .highlight_style(super::list_highlight())
            .highlight_symbol(super::HIGHLIGHT_SYMBOL);
        f.render_stateful_widget(list, chunks[0], &mut state);
    }
    render_footer(f, chunks[1], chunks[2], app);
}

/// One notification row: `subject_title [subject_type]` after the unread dot,
/// then reason, repository and the update time. Unread rows carry the dot and
/// a bold title; read rows keep the blank marker so titles line up.
fn notification_item(note: &NotificationSummary, width: usize) -> ListItem<'static> {
    let marker = if note.unread { UNREAD_MARKER } else { "  " };
    let type_tag = format!(" [{}]", note.subject_type);
    let meta = format!("{}  {}  {}", note.reason, note.repo, note.updated_at);

    let budget = meta_budget(&meta, width);
    let head_width = if budget > 0 {
        width - budget - cells(META_GAP)
    } else {
        width
    };
    let fixed = cells(marker) + cells(&type_tag);
    let title_style = if note.unread {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let marker_style = if note.unread {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let mut spans = vec![
        Span::styled(marker, marker_style),
        Span::styled(
            ellipsize(&note.subject_title, head_width.saturating_sub(fixed)),
            title_style,
        ),
        Span::styled(type_tag, Style::default().fg(Color::Cyan)),
    ];
    if budget > 0 {
        spans.push(Span::styled(
            format!("{META_GAP}{}", ellipsize(&meta, budget)),
            Style::default().fg(Color::DarkGray),
        ));
    }
    ListItem::new(Line::from(spans))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::config::Config;
    use crate::ui::pulls::offline_client;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// Connected app: `owner`/`repo` filled in and the readiness gate built.
    fn connected_app() -> App {
        let mut app = App::new(Config::default());
        app.github_state.owner = "octo".to_string();
        app.github_state.repo = "demo".to_string();
        app.github_state.client = Some(offline_client());
        app
    }

    fn note(title: &str, subject_type: &str, unread: bool) -> NotificationSummary {
        NotificationSummary {
            id: "n-1".to_string(),
            repo: "octo/demo".to_string(),
            subject_title: title.to_string(),
            subject_type: subject_type.to_string(),
            reason: "subscribed".to_string(),
            updated_at: "2026-03-04T05:06:07Z".to_string(),
            unread,
        }
    }

    fn render(app: &App) -> Terminal<TestBackend> {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render_notifications(f, app)).unwrap();
        terminal
    }

    fn render_text(app: &App) -> String {
        let raw: String = render(app)
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        raw.chars().filter(|c| !c.is_whitespace()).collect()
    }

    #[test]
    fn notifications_list_shows_subject_type_reason_and_hints() {
        let mut app = connected_app();
        app.github_state.notifications = vec![note("Fix login", "Issue", false)];
        app.github_state.selected = 0;
        let text = render_text(&app);
        assert!(text.contains("通知·octo/demo"), "missing title: {text}");
        assert!(
            text.contains("Fixlogin[Issue]"),
            "missing subject row: {text}"
        );
        assert!(
            text.contains("subscribedocto/demo2026-03-04T05:06:07Z"),
            "missing meta: {text}"
        );
        assert!(
            !text.contains('●'),
            "read rows carry no unread marker: {text}"
        );
        assert!(text.contains("n通知"), "missing tab hint: {text}");
    }

    #[test]
    fn unread_rows_carry_a_bold_dot_while_read_rows_stay_plain() {
        let mut app = connected_app();
        app.github_state.notifications = vec![
            note("Read one", "PullRequest", false),
            note("Unread one", "Issue", true),
        ];
        app.github_state.selected = 0;
        let terminal = render(&app);
        let buffer = terminal.backend().buffer();
        let mut dot: Option<(u16, u16)> = None;
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                if buffer[(x, y)].symbol() == "●" {
                    dot = Some((x, y));
                }
            }
        }
        let (x, y) = dot.expect("unread row must show its dot");
        assert!(
            buffer[(x, y)].modifier.contains(Modifier::BOLD),
            "unread dot must be bold"
        );
        // The title of the unread row is bold too — checked on the cell right
        // after the two-column marker, on the same row.
        let title_cell = &buffer[(x + 2, y)];
        assert!(
            title_cell.modifier.contains(Modifier::BOLD),
            "unread title must be bold"
        );
        // The selected (read) row only carries the list highlight.
        let text: String = buffer.content().iter().map(|cell| cell.symbol()).collect();
        let text: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(text.contains("Unreadone"), "missing unread row: {text}");
    }

    #[test]
    fn notifications_list_shows_empty_copy_for_zero_rows() {
        let app = connected_app();
        let text = render_text(&app);
        assert!(text.contains("—（暂无通知）"), "missing copy: {text}");
    }

    #[test]
    fn notifications_disconnected_shows_the_status_line_instead() {
        let mut app = App::new(Config::default());
        app.set_error("gh 不可用，无法自动获取 token");
        app.github_state.notifications = vec![note("ghost", "Issue", true)];
        let text = render_text(&app);
        assert!(text.contains("尚未连接GitHub"), "missing gate copy: {text}");
        assert!(text.contains("gh不可用"), "status must show: {text}");
        assert!(!text.contains("ghost"), "no rows without a client: {text}");
        assert!(!text.contains('●'), "no marker without a client: {text}");
    }
}
