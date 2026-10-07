//! Notifications receipts panel: repository-scoped notifications, whichever
//! platform serves them (GitHub threads, GitLab To-Dos).

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::ListItem,
    Frame,
};

use crate::app::App;
use crate::platform::{notifications_label, NotificationSummary};
use crate::ui::components::theme::color;

use super::pulls::{
    cells, ellipsize, meta_budget, render_disconnected, render_load_error, render_receipts_empty,
    render_receipts_loading, render_receipts_rows, row_width, META_GAP,
};

/// Empty-state copy: nothing to review. Zero rows is a valid state — a failed
/// load paints the panel's `error` state.
const EMPTY_NOTIFICATIONS: &str = "—（暂无通知）";

/// Marker of an unread row: the dot is what catches the eye first, so it is
/// yellow and bold; read rows keep a blank marker to stay aligned.
const UNREAD_MARKER: &str = "● ";

/// Render the repository-scoped notifications list.
///
/// The panel draws its content only (§4.1): the status row and the hint row
/// are both painted once by the footer.
pub fn render_notifications(f: &mut Frame, content: Rect, app: &App) {
    // GitHub serves notification threads while GitLab serves To-Dos —
    // different objects sharing one receipt slot, so the title names which one
    // the remote actually has. The word itself comes from `platform` so the
    // footer hint and the command palette row cannot drift from this title.
    let label = notifications_label(app.github_state.kind());
    if let Some(message) = app.load_error.as_deref() {
        render_load_error(
            f,
            content,
            label,
            app.github_state.owner(),
            app.github_state.repo(),
            message,
        );
        return;
    }
    if !app.github_state.connected() {
        render_disconnected(f, content, label, app.github_state.kind(), app);
        return;
    }

    if app.github_loading() {
        render_receipts_loading(f, content, label, app, "正在拉取通知…");
        return;
    }

    let notifications = &app.github_state.notifications;
    if notifications.is_empty() {
        render_receipts_empty(
            f,
            content,
            label,
            app,
            EMPTY_NOTIFICATIONS,
            "这个仓库没有新通知",
        );
    } else {
        let width = row_width(content);
        let items = notifications
            .iter()
            .map(|note| notification_item(note, width))
            .collect();
        render_receipts_rows(f, content, label, app, items, app.github_state.selected);
    }
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
            .fg(color::WARNING)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(color::FG_MUTED)
    };

    let mut spans = vec![
        Span::styled(marker, marker_style),
        Span::styled(
            ellipsize(&note.subject_title, head_width.saturating_sub(fixed)),
            title_style,
        ),
        Span::styled(type_tag, Style::default().fg(color::INFO)),
    ];
    if budget > 0 {
        spans.push(Span::styled(
            format!("{META_GAP}{}", ellipsize(&meta, budget)),
            Style::default().fg(color::FG_MUTED),
        ));
    }
    ListItem::new(Line::from(spans))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::config::Config;
    use crate::ui::pulls::{draw_receipt_frame, offline_client};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// Connected app: the `octo/demo` remote resolved and the readiness gate
    /// built, so the panel renders the real title instead of the `—` fallback.
    fn connected_app() -> App {
        let mut app = App::new(Config::default());
        app.github_state.remote = Some(crate::ui::github_remote());
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

    /// The panel plus its footer: the hint row the panel no longer draws
    /// reaches the buffer through [`crate::ui::components::footer`]
    /// (§2.3), so a test that skipped it would miss the row entirely.
    fn render(app: &App) -> Terminal<TestBackend> {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw_receipt_frame(f, app, false, |f, body| render_notifications(f, body, app))
            })
            .unwrap();
        terminal
    }

    fn render_text(app: &App) -> String {
        compact_text(render(app).backend().buffer())
    }

    /// Whitespace-stripped text of the whole frame (wide CJK glyphs leave
    /// padding cells).
    fn compact_text(buffer: &ratatui::buffer::Buffer) -> String {
        let mut raw = String::new();
        for cell in buffer.content() {
            raw.push_str(cell.symbol());
        }
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

    /// §4.1/§4.2: the readiness gate replaces the list, and the reason stays on
    /// the footer status row — the one landing spot for an async result.
    #[test]
    fn notifications_disconnected_keeps_the_reason_on_the_footer_status_row() {
        let mut app = App::new(Config::default());
        app.set_error("gh 不可用，无法自动获取 token");
        app.github_state.notifications = vec![note("ghost", "Issue", true)];
        let buffer = render(&app).backend().buffer().clone();
        let text = compact_text(&buffer);
        // No resolved remote means `kind == None`, which falls back to the
        // GitHub gate copy, verbatim as in v0.5.0.
        assert!(text.contains("尚未连接GitHub"), "missing gate copy: {text}");
        assert!(!text.contains("ghost"), "no rows without a client: {text}");
        assert!(!text.contains('●'), "no marker without a client: {text}");
        assert_eq!(
            crate::ui::pulls::rows_with(&buffer, "gh不可用，无法自动获取token"),
            vec![buffer.area.height - 2],
            "the reason belongs on the footer status row only: {text}"
        );
    }

    /// B6: GitLab's `/todos` are a different object from GitHub's
    /// notification threads, so the panel title names which one is on screen.
    /// GitHub keeps its v0.5.0 wording.
    #[test]
    fn gitlab_receipts_are_labelled_to_dos() {
        use crate::platform::PlatformKind;
        assert_eq!(notifications_label(Some(PlatformKind::GitLab)), "To-Dos");
        assert_eq!(notifications_label(Some(PlatformKind::GitHub)), "通知");
        assert_eq!(notifications_label(Some(PlatformKind::Gitea)), "通知");
        assert_eq!(
            notifications_label(Some(PlatformKind::SelfHosted)),
            "通知",
            "unsupported instance keeps the neutral name"
        );
        assert_eq!(notifications_label(None), "通知", "no remote stays neutral");
    }

    /// B6: the label reaches the rendered title of a connected GitLab repo.
    #[test]
    fn gitlab_panel_title_says_to_dos() {
        let mut app = connected_app();
        let mut remote = crate::ui::github_remote();
        remote.kind = crate::platform::PlatformKind::GitLab;
        app.github_state.remote = Some(remote);
        app.github_state.notifications = vec![note("todo-1", "Issue", true)];
        let text = render_text(&app);
        assert!(text.contains("To-Dos"), "missing To-Dos title: {text}");
        assert!(
            !text.contains("通知列表"),
            "GitHub wording must not leak: {text}"
        );
    }

    /// B6: the footer key hint and the panel title name the same thing — a
    /// GitLab repo must not show `To-Dos` above an `n 通知` hint, which would
    /// read as two different slots on one screen.
    #[test]
    fn gitlab_panel_hint_agrees_with_its_title() {
        let mut app = connected_app();
        let mut remote = crate::ui::github_remote();
        remote.kind = crate::platform::PlatformKind::GitLab;
        app.github_state.remote = Some(remote);
        app.github_state.notifications = vec![note("todo-1", "Issue", true)];
        let text = render_text(&app);
        assert!(text.contains("nTo-Dos"), "missing To-Dos hint: {text}");
        assert!(
            !text.contains("n通知"),
            "the hint must follow the panel label: {text}"
        );
    }
}
