//! Repository dashboard: renders `App.repos` with selection and error state.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::app::StatusLine;
use crate::git::RepoInfo;

/// Borrowed render state for the dashboard, referencing `App.repos` directly
/// so no mock data source exists (checkpoint 2).
pub struct DashboardState<'a> {
    /// Live repository snapshots from `App.repos`.
    pub repos: &'a [RepoInfo],
    /// Current status line, if any; an error hides the repository list.
    pub status: Option<&'a StatusLine>,
    /// Selection cursor shared across frames.
    pub list_state: &'a mut ListState,
}

/// Render the repository dashboard overview.
///
/// An error status replaces the repository list so a failed load never
/// masquerades as an empty list (checkpoint 2); any status also prints on
/// the bottom status line (convention 3).
pub fn render_dashboard(f: &mut Frame, state: &mut DashboardState<'_>) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(f.area());
    let (list_area, status_area) = (chunks[0], chunks[1]);

    if let Some(status) = state.status {
        if status.is_error {
            let block = Block::default()
                .title("GitPulse 仓库状态")
                .borders(Borders::ALL);
            let paragraph =
                Paragraph::new(status.message.clone()).style(Style::default().fg(Color::Red));
            f.render_widget(paragraph.block(block), list_area);
            render_status_line(f, status_area, status);
            return;
        }
    }

    let items: Vec<ListItem> = state
        .repos
        .iter()
        .map(|repo| {
            let dirty_count =
                repo.dirty.untracked + repo.dirty.modified + repo.dirty.added + repo.dirty.deleted;
            let dirty_color = if dirty_count == 0 {
                Color::Green
            } else {
                Color::Red
            };

            let line = Line::from(vec![
                Span::raw(format!("{:<20}", repo.name)),
                Span::styled(
                    format!("{:<15}", repo.branch),
                    Style::default().fg(Color::Cyan),
                ),
                Span::styled(
                    format!("{:>4}", dirty_count),
                    Style::default().fg(dirty_color),
                ),
                Span::raw("  "),
                Span::styled(
                    format!("{}↑{}↓", repo.ahead, repo.behind),
                    if repo.ahead > 0 {
                        Style::default().fg(Color::Yellow)
                    } else {
                        Style::default()
                    },
                ),
                Span::raw("  "),
                Span::raw(format_time(repo.last_commit.time)),
            ]);
            ListItem::new(line)
        })
        .collect();

    let block = Block::default()
        .title(format!("GitPulse 仓库状态 ({})", state.repos.len()))
        .borders(Borders::ALL);

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("► ");

    f.render_stateful_widget(list, list_area, state.list_state);

    if let Some(status) = state.status {
        render_status_line(f, status_area, status);
    }
}

/// Draw the one-line status bar; errors render red and persist until the
/// next operation (convention 3).
fn render_status_line(f: &mut Frame, area: Rect, status: &StatusLine) {
    let style = if status.is_error {
        Style::default().fg(Color::Red)
    } else {
        Style::default()
    };
    f.render_widget(Paragraph::new(status.message.clone()).style(style), area);
}

/// Format a Unix timestamp as `YYYY-MM-DD HH:MM` (UTC) without chrono.
fn format_time(timestamp: i64) -> String {
    if timestamp <= 0 {
        return "-".to_string();
    }
    let secs = timestamp as u64;
    let days = secs / 86_400;
    let tod = secs % 86_400;
    let h = tod / 3600;
    let m = (tod % 3600) / 60;
    let (y, mo, d) = civil_from_days(days as i64);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02}")
}

/// Howard Hinnant's `civil_from_days`: days since epoch to (year, month, day).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if mo <= 2 { y + 1 } else { y };
    (year, mo as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn make_repo(name: &str, branch: &str) -> RepoInfo {
        RepoInfo {
            name: name.to_string(),
            path: std::path::PathBuf::from(format!("/tmp/{name}")),
            branch: branch.to_string(),
            dirty: Default::default(),
            ahead: 0,
            behind: 0,
            last_commit: Default::default(),
        }
    }

    fn render_to_text(state: &mut DashboardState<'_>) -> String {
        let backend = TestBackend::new(80, 12);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|f| render_dashboard(f, state))
            .expect("draw frame");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn render_with_repos_shows_names_and_branch() {
        let repos = vec![make_repo("alpha", "main"), make_repo("beta", "dev")];
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        let text = render_to_text(&mut DashboardState {
            repos: &repos,
            status: None,
            list_state: &mut list_state,
        });
        assert!(text.contains("alpha"), "missing name: {text}");
        assert!(text.contains("beta"), "missing name: {text}");
        assert!(text.contains("main"), "missing branch: {text}");
        assert!(text.contains("dev"), "missing branch: {text}");
    }

    #[test]
    fn render_error_state_shows_error_not_list() {
        let status = StatusLine {
            message: "error: failed to scan repos".to_string(),
            is_error: true,
        };
        let repos = vec![make_repo("alpha", "main")];
        let mut list_state = ListState::default();
        let text = render_to_text(&mut DashboardState {
            repos: &repos,
            status: Some(&status),
            list_state: &mut list_state,
        });
        assert!(text.contains("error"), "missing error: {text}");
        assert!(!text.contains("alpha"), "repo list leaked: {text}");
    }

    #[test]
    fn render_status_line_shows_message_below_list() {
        let status = StatusLine {
            message: "refreshed: 3 repos".to_string(),
            is_error: false,
        };
        let repos = vec![make_repo("alpha", "main")];
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        let text = render_to_text(&mut DashboardState {
            repos: &repos,
            status: Some(&status),
            list_state: &mut list_state,
        });
        // Non-error status keeps the list and appends the status line.
        assert!(text.contains("alpha"), "list hidden: {text}");
        assert!(
            text.contains("refreshed: 3 repos"),
            "missing status: {text}"
        );
    }

    #[test]
    fn render_error_also_appears_on_status_line() {
        let status = StatusLine {
            message: "error: scan failed".to_string(),
            is_error: true,
        };
        let repos = vec![make_repo("alpha", "main")];
        let mut list_state = ListState::default();
        let text = render_to_text(&mut DashboardState {
            repos: &repos,
            status: Some(&status),
            list_state: &mut list_state,
        });
        assert!(text.contains("error"), "missing error: {text}");
        assert!(!text.contains("alpha"), "repo list leaked: {text}");
    }

    #[test]
    fn format_time_handles_invalid_and_known_dates() {
        assert_eq!(format_time(0), "-");
        assert_eq!(format_time(-1), "-");
        // 2024-01-01 00:00:00 UTC
        assert!(format_time(1_704_067_200).starts_with("2024-01-01"));
    }
}
