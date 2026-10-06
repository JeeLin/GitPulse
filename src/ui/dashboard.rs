//! Repository dashboard: renders `App.repos` with selection and error state.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::app::BatchLogLine;
use crate::git::RepoInfo;
use crate::ui::status::StatusLine;

use super::{hint, pulls, render_status_line};
use crate::ui::components::footer::{hints as join_hints, HintRow};

/// Borrowed render state for the dashboard, referencing `App.repos` directly
/// so no mock data source exists (checkpoint 2).
pub struct DashboardState<'a> {
    /// Live repository snapshots from `App.repos`.
    pub repos: &'a [RepoInfo],
    /// Current status line, if any; an error hides the repository list.
    pub status: Option<&'a StatusLine>,
    /// Selection cursor shared across frames.
    pub list_state: &'a mut ListState,
    /// Rolling per-repository batch progress log (convention 6).
    pub batch_log: &'a [BatchLogLine],
    /// Label of the running/last batch operation (`pull` / `fetch`).
    pub batch_op: &'static str,
    /// Whether the batch operation is still running.
    pub batch_running: bool,
}

/// Bordered-block title shared by every dashboard state: the product label
/// plus the visible repository count, so the empty state (`0`) and the list
/// state stay word-identical.
fn dashboard_title(count: usize) -> String {
    format!("GitPulse 仓库状态 ({count})")
}

/// Render the repository dashboard overview.
///
/// An error status replaces the repository list so a failed load never
/// masquerades as an empty list (checkpoint 2); any status also prints on
/// the panel's status row (convention 3).
pub fn render_dashboard(f: &mut Frame, content: Rect, state: &mut DashboardState<'_>) {
    // Bottom scrolling batch log (convention 6) takes space only while it
    // has content; the list keeps every remaining row otherwise.
    let batch_rows = if state.batch_log.is_empty() {
        0
    } else {
        (state.batch_log.len() as u16).saturating_add(2).min(8)
    };
    let mut constraints = vec![Constraint::Min(1)];
    if batch_rows > 0 {
        constraints.push(Constraint::Length(batch_rows));
    }
    constraints.push(Constraint::Length(1));
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(content);
    let list_area = chunks[0];
    let last = chunks.len();
    let batch_area = (batch_rows > 0).then(|| chunks[1]);
    let status_area = chunks[last - 1];

    if let Some(status) = state.status {
        if status.is_error {
            let block = Block::default()
                .title("GitPulse 仓库状态")
                .borders(Borders::ALL);
            let paragraph =
                Paragraph::new(status.message.clone()).style(Style::default().fg(Color::Red));
            f.render_widget(paragraph.block(block), list_area);
            render_status_line(f, status_area, Some(status));
            return;
        }
    }

    // Empty state: scan dirs may be unconfigured or simply contain no repos,
    // so guide the user into the config panel instead of an empty list.
    if state.repos.is_empty() {
        render_empty_state(f, list_area);
    } else {
        render_repo_list(f, list_area, state);
    }

    // Shared tail (empty state and list alike; the error branch above keeps
    // its early return without the batch log, matching prior behavior).
    if let Some(batch_area) = batch_area {
        render_batch_log(f, batch_area, state);
    }

    render_status_line(f, status_area, state.status);
}

/// Draw the bordered repository list: name, branch, dirty count, ahead /
/// behind markers, and last-commit time, with the shared highlight style.
fn render_repo_list(f: &mut Frame, area: Rect, state: &mut DashboardState<'_>) {
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
                Span::raw(pad_cells(&pulls::ellipsize(&repo.name, 20), 20)),
                Span::styled(
                    pad_cells(&pulls::ellipsize(&repo.branch, 15), 15),
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
        .title(dashboard_title(state.repos.len()))
        .borders(Borders::ALL);

    let list = List::new(items)
        .block(block)
        .highlight_style(super::list_highlight())
        .highlight_symbol(super::HIGHLIGHT_SYMBOL);

    f.render_stateful_widget(list, area, state.list_state);
}

/// Draw the scrolling per-repository batch progress log (convention 6):
/// the newest lines stay visible; failures render red, successes green.
fn render_batch_log(f: &mut Frame, area: Rect, state: &DashboardState<'_>) {
    let title = if state.batch_running {
        format!("批量 {} 进度", state.batch_op)
    } else {
        format!("批量 {} 结果", state.batch_op)
    };
    let visible = state
        .batch_log
        .len()
        .saturating_sub(area.height.saturating_sub(2) as usize);
    let lines: Vec<Line> = state.batch_log[visible.min(state.batch_log.len())..]
        .iter()
        .map(|entry| {
            let color = if entry.is_error {
                Color::Red
            } else if entry.is_success {
                Color::Green
            } else {
                Color::DarkGray
            };
            Line::from(vec![
                Span::styled(
                    pad_cells(&pulls::ellipsize(&entry.name, 20), 20),
                    Style::default().fg(color),
                ),
                Span::styled(entry.message.clone(), Style::default().fg(color)),
            ])
        })
        .collect();
    let block = Block::default().title(title).borders(Borders::ALL);
    f.render_widget(Paragraph::new(lines).block(block), area);
}

/// Replace the list body with a centered hint that points at the config panel
/// (`c`), keeping the usual bordered title and row count. The wording stays
/// neutral because an empty list has more than one cause.
fn render_empty_state(f: &mut Frame, area: Rect) {
    let block = Block::default()
        .title(dashboard_title(0))
        .borders(Borders::ALL);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(50),
            Constraint::Length(1),
            Constraint::Percentage(50),
        ])
        .split(inner);
    let hint = Line::from(vec![
        Span::raw("暂无仓库 — 按 "),
        Span::styled("c", Style::default().fg(Color::Cyan)),
        Span::raw(" 管理扫描目录"),
    ]);
    f.render_widget(Paragraph::new(hint).alignment(Alignment::Center), rows[1]);
}

/// Footer hint row for the dashboard (§2.3): the verbs this panel adds on
/// top of the global segment. Key caps take [`super::hint`]'s `key` style and
/// the actions the muted `caption` style, and the pairs are joined by
/// [`join_hints`] so the gap between two key caps is the two spaces §2.3
/// prescribes — the dashboard draws no hint row of its own any more.
pub(crate) fn dashboard_hints() -> HintRow {
    HintRow::new(join_hints([
        hint("j/k", "选择"),
        hint("Enter", "进入"),
        hint("r", "刷新"),
        hint("l", "pull"),
        hint("f", "fetch"),
        hint("b", "分支"),
        hint("c", "配置"),
        hint("q", "退出"),
    ]))
}

/// Pad `text` to exactly `width` terminal cells by appending spaces, counting
/// CJK glyphs as two cells (ratatui's display-width model) so the fixed columns
/// of the dashboard row stay aligned when a repo/branch name mixes scripts —
/// the same width model [`pulls::ellipsize`] and [`pulls::cells`] use.
fn pad_cells(text: &str, width: usize) -> String {
    let used = pulls::cells(text);
    if used >= width {
        text.to_string()
    } else {
        format!("{text}{}", " ".repeat(width - used))
    }
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
    use crate::ui::components::footer::StatusRow;
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

    /// The panel draws content only (§2.3), so the frame goes through
    /// [`crate::ui::components::footer::draw_frame`] — the shape
    /// `ui::render` uses — and the hint row is the bottom row.
    fn render_to_text(state: &mut DashboardState<'_>) -> String {
        let hints = dashboard_hints();
        let status = state.status;
        let backend = TestBackend::new(80, 12);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|f| {
                crate::ui::components::footer::draw_frame(
                    f,
                    StatusRow::plain(status),
                    &hints,
                    |f, body| render_dashboard(f, body, state),
                );
            })
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
            batch_log: &[],
            batch_op: "",
            batch_running: false,
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
            batch_log: &[],
            batch_op: "",
            batch_running: false,
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
            batch_log: &[],
            batch_op: "",
            batch_running: false,
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
            batch_log: &[],
            batch_op: "",
            batch_running: false,
        });
        assert!(text.contains("error"), "missing error: {text}");
        assert!(!text.contains("alpha"), "repo list leaked: {text}");
    }

    #[test]
    fn render_batch_log_shows_progress_lines() {
        let repos = vec![make_repo("alpha", "main")];
        let mut list_state = ListState::default();
        let batch_log = vec![
            BatchLogLine {
                name: "alpha".to_string(),
                message: "进行中".to_string(),
                is_error: false,
                is_success: false,
            },
            BatchLogLine {
                name: "beta".to_string(),
                message: "失败: 未配置远端".to_string(),
                is_error: true,
                is_success: false,
            },
        ];
        let text = render_to_text(&mut DashboardState {
            repos: &repos,
            status: None,
            list_state: &mut list_state,
            batch_log: &batch_log,
            batch_op: "pull",
            batch_running: true,
        });
        // Wide CJK glyphs leave padding cells; compare whitespace-stripped.
        let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(compact.contains("alpha"), "missing repo line: {compact}");
        assert!(compact.contains("进行中"), "missing progress: {compact}");
        assert!(compact.contains("失败"), "missing failure: {compact}");
        assert!(compact.contains("批量pull进度"), "missing title: {compact}");
    }

    #[test]
    fn format_time_handles_invalid_and_known_dates() {
        assert_eq!(format_time(0), "-");
        assert_eq!(format_time(-1), "-");
        // 2024-01-01 00:00:00 UTC
        assert!(format_time(1_704_067_200).starts_with("2024-01-01"));
    }

    /// Wide CJK glyphs leave padding cells; compare whitespace-stripped.
    fn compact(text: &str) -> String {
        text.chars().filter(|c| !c.is_whitespace()).collect()
    }

    #[test]
    fn render_empty_repos_shows_config_hint_and_title() {
        let mut list_state = ListState::default();
        let text = render_to_text(&mut DashboardState {
            repos: &[],
            status: None,
            list_state: &mut list_state,
            batch_log: &[],
            batch_op: "",
            batch_running: false,
        });
        let compact = compact(&text);
        assert!(
            compact.contains("暂无仓库—按c管理扫描目录"),
            "missing empty-state hint: {compact}"
        );
        assert!(
            compact.contains("GitPulse仓库状态(0)"),
            "missing title: {compact}"
        );
    }

    #[test]
    fn render_with_repos_hides_config_hint() {
        let repos = vec![make_repo("alpha", "main")];
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        let text = render_to_text(&mut DashboardState {
            repos: &repos,
            status: None,
            list_state: &mut list_state,
            batch_log: &[],
            batch_op: "",
            batch_running: false,
        });
        let compact = compact(&text);
        assert!(compact.contains("alpha"), "missing repo: {compact}");
        assert!(
            !compact.contains("暂无仓库"),
            "empty-state hint leaked: {compact}"
        );
    }

    #[test]
    fn render_error_state_shows_error_not_config_hint() {
        let status = StatusLine {
            message: "error: scan failed".to_string(),
            is_error: true,
        };
        let mut list_state = ListState::default();
        let text = render_to_text(&mut DashboardState {
            repos: &[],
            status: Some(&status),
            list_state: &mut list_state,
            batch_log: &[],
            batch_op: "",
            batch_running: false,
        });
        let compact = compact(&text);
        assert!(compact.contains("error"), "missing error: {compact}");
        assert!(
            !compact.contains("暂无仓库"),
            "hint must not mask the error: {compact}"
        );
    }

    /// §2.3: the dashboard's verbs reach the view segment through
    /// [`dashboard_hints`], so a frame wide enough to show it whole carries
    /// the config and navigation entries.
    #[test]
    fn hint_row_view_segment_includes_config_entry() {
        let backend = TestBackend::new(120, 12);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|f| {
                crate::ui::components::footer::draw_frame(
                    f,
                    StatusRow::plain(None),
                    &dashboard_hints(),
                    |_, _| {},
                );
            })
            .expect("draw frame");
        let buffer = terminal.backend().buffer();
        let raw: String = (0..buffer.area.width)
            .map(|x| buffer[(x, buffer.area.height - 1)].symbol().to_string())
            .collect();
        let compact: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(compact.contains("c配置"), "missing config hint: {compact}");
        assert!(compact.contains("j/k选择"), "missing nav hint: {compact}");
    }
}
