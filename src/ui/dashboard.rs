//! Repository dashboard: renders `App.repos` with selection and error state.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, ListItem, ListState, Paragraph},
    Frame,
};

use crate::app::BatchLogLine;
use crate::git::RepoInfo;

use super::{hint, pulls};
use crate::ui::components::cells;
use crate::ui::components::footer::{hints as join_hints, HintRow};
use crate::ui::components::state_panel::{self, PanelState};
use crate::ui::components::theme::{color, role, spacing, width as theme_width};

/// Borrowed render state for the dashboard, referencing `App.repos` directly
/// so no mock data source exists (checkpoint 2).
pub struct DashboardState<'a> {
    /// Live repository snapshots from `App.repos`.
    pub repos: &'a [RepoInfo],
    /// Scan failure from `App.load_error`; the list is replaced by the `error`
    /// state while it is set (§4.3).
    pub load_error: Option<&'a str>,
    /// Selection cursor shared across frames.
    pub list_state: &'a mut ListState,
    /// Rolling per-repository batch progress log (convention 6).
    pub batch_log: &'a [BatchLogLine],
    /// Label of the running/last batch operation (`pull` / `fetch`).
    pub batch_op: &'static str,
    /// Whether the batch operation is still running.
    pub batch_running: bool,
    /// Whether the repository list holds the screen's focus (§3.1): it picks
    /// the border color and whether the list draws the `▌` / `surface_alt`
    /// cursor.
    pub focused: bool,
}

/// Bordered-block title shared by every dashboard state: the product label
/// plus the visible repository count, so the empty state (`0`) and the list
/// state stay word-identical.
fn dashboard_title(count: usize) -> String {
    format!("GitPulse 仓库状态 ({count})")
}

/// Minimum width of the preview panel in cells (§5.4) — read from the shared
/// floor in [`theme::width`] so the dashboard no longer keeps its own copy of
/// the canonical §5.4 thresholds (6a).
///
/// Ceiling on the preview panel: it never grows fat enough to starve the list.
const PREVIEW_MAX: u16 = 44;
/// Frame width at which the preview opens beside the list (≥96 列左右分栏);
/// below it the preview hides and the list owns the whole width (§6.1:274
/// 可选预览面板（窄屏隐藏）, §5.4 80 列仅当列表 ≥28 且详情 ≥32 才分栏 — hiding
/// is the allowed alternative there).
const SPLIT_MIN: u16 = 96;

/// Render the repository dashboard overview.
///
/// Layout (§6.1:274): the repository list is the focus panel pinned to the left
/// (min `LIST_MIN` cells); a preview of the selected repository opens beside it
/// once the frame can fund both minimum budgets — otherwise the list owns the
/// whole width and the preview hides (§5.4: 80 列下仅当列表 ≥28 且详情 ≥32 才
/// 允许左右分栏).
///
/// A failed scan replaces the repository list with the panel's `error` state so
/// it never masquerades as an empty list (§4.3); the batch progress log still
/// reaches the frame through the footer's status row (§4.1), and an operation
/// failure leaves the list untouched.
pub fn render_dashboard(f: &mut Frame, content: Rect, state: &mut DashboardState<'_>) {
    // The preview only mirrors a real selection, so it shares the list pane's
    // column budget only when there is something to show — on `empty` / `error`
    // the single panel spans the width (§6.1:266 预览面板 / 空态 / 错误态 are
    // alternatives, not siblings).
    let has_repos = !state.repos.is_empty();
    let show_preview = has_repos && state.load_error.is_none();
    let (list_pane, preview_area) = if show_preview {
        split_list_and_preview(content)
    } else {
        (content, None)
    };

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
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(list_pane);
    let list_area = chunks[0];
    let batch_area = (batch_rows > 0).then(|| chunks[1]);

    if let Some(message) = state.load_error {
        state_panel::render_state_panel_with_focus(
            f,
            list_area,
            &dashboard_title(0),
            &PanelState::Error { message },
            state.focused,
        );
        // The content behind the error never arrived (§4.3), so the preview
        // has nothing honest to mirror; only the list pane shows the failure.
        return;
    }

    // Empty state: scan dirs may be unconfigured or simply contain no repos,
    // so guide the user into the config panel instead of an empty list.
    if has_repos {
        render_repo_list(f, list_area, state);
    } else {
        render_empty_state(f, list_area, state.focused);
    }

    // Shared tail (empty state and list alike; the error branch above keeps
    // its early return without the batch log, matching prior behavior).
    if let Some(batch_area) = batch_area {
        render_batch_log(f, batch_area, state);
    }

    if let Some(preview) = preview_area {
        render_preview(f, preview, state);
    }
}

/// Horizontal budget: split the dashboard into the left repository list and the
/// (optionally hidden) right preview when the frame can fund both minimum
/// widths (§5.4), keeping the focus list at or above `theme_width::LIST_MIN`.
fn split_list_and_preview(content: Rect) -> (Rect, Option<Rect>) {
    if content.width < SPLIT_MIN {
        return (content, None);
    }
    let preview_w = (content.width - theme_width::LIST_MIN - spacing::GUTTER)
        .clamp(theme_width::PREVIEW_MIN, PREVIEW_MAX);
    let list_w = content.width - spacing::GUTTER - preview_w;
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(list_w),
            Constraint::Length(spacing::GUTTER),
            Constraint::Length(preview_w),
        ])
        .split(content);
    (chunks[0], Some(chunks[2]))
}

/// The selected repository's factual detail (§6.1 preview panel): branch,
/// ahead/behind, dirty counts and the most recent commit. It mirrors the list
/// cursor rather than owning focus, so it never draws a cursor (§3.1) and takes
/// the unfocused border.
fn render_preview(f: &mut Frame, area: Rect, state: &DashboardState<'_>) {
    let block = Block::default()
        .title("预览")
        .borders(Borders::ALL)
        .border_style(role::border(false));
    let lines = match state.list_state.selected().and_then(|i| state.repos.get(i)) {
        Some(repo) => {
            let dirty = &repo.dirty;
            let total = dirty.untracked + dirty.modified + dirty.added + dirty.deleted;
            vec![
                Line::from(format!("分支 {}", repo.branch)),
                Line::from(format!("领先 ↑{} 落后 ↓{}", repo.ahead, repo.behind)),
                Line::from(format!(
                    "未提交 {total} ｜ 增{} 修{} 加{} 删{}",
                    dirty.untracked, dirty.modified, dirty.added, dirty.deleted
                )),
                Line::from(format!("最近提交 {}", repo.last_commit.summary)),
                Line::from(format!(
                    "{} · {}",
                    repo.last_commit.author,
                    format_time(repo.last_commit.time)
                )),
            ]
        }
        None => vec![Line::from("暂无选中仓库 — 用 j/k 选择")],
    };
    f.render_widget(Paragraph::new(lines).block(block), area);
}

/// Cells the dirty count column holds (`{:>4}` — fixed width, right-aligned so
/// it never shifts the columns beside it).
const DIRTY_CELLS: usize = 4;
/// Cells the ahead/behind marker holds (`4↑2`) when the row is drawn whole.
const SYNC_CELLS: usize = 4;
/// Cells the last-commit timestamp holds (`YYYY-MM-DD HH:MM`); the `-`
/// placeholder for a repository without history is shorter.
const TIME_CELLS: usize = 16;
/// Cells the two high-priority columns reserve when the pane has room to spare
/// (§5.4 表格列宽): name and branch identify the row, so they flex inside a floor
/// instead of ever being dropped.
const NAME_CELLS: usize = 20;
const BRANCH_CELLS: usize = 15;
/// Floors the two high-priority columns shrink to before a Low column would be
/// sacrificed — a row is never allowed to chew into identification before it has
/// dropped the columns §5.4 calls "low priority".
const NAME_MIN: usize = 12;
const BRANCH_MIN: usize = 8;

/// Priority of a repository row column (§5.4 表格列宽): a Low column vanishes
/// whole when the row cannot fund its cells, whereas a High column only shrinks
/// to its floor first — identification is never erased to save a timestamp.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Priority {
    /// Identifies the row: the name and the branch. Never dropped, only
    /// squeezed within its floor, and grown back to its preferred width
    /// whenever the pane has the cells.
    High,
    /// Context the preview panel repeats (dirty count, ahead/behind, time):
    /// dropped from the right edge of the row before the High columns shrink.
    Low,
}

/// The data a repository row column renders — §6.1:262 fixes this exact set.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Column {
    /// Repository name.
    Name,
    /// Current branch.
    Branch,
    /// Uncommitted change count.
    Dirty,
    /// Ahead / behind marker.
    Sync,
    /// Last-commit timestamp — the lowest-priority column, dropped first.
    Time,
}

/// §5.4 column priority table for a repository row: an ordered slice of
/// `(column, min_width, priority)` — every column's `min_width` (the cells it
/// keeps while drawn) and its priority (whether it may be dropped when the row
/// runs out of cells). Ordered high → low, so the renderer keeps a prefix of
/// this slice while the row fits and cuts the tail: `Time` goes before `Sync`,
/// `Sync` before `Dirty`, and the two High columns never go at all.
const REPO_COLUMNS: [(Column, usize, Priority); 5] = [
    (Column::Name, NAME_MIN, Priority::High),
    (Column::Branch, BRANCH_MIN, Priority::High),
    (Column::Dirty, DIRTY_CELLS, Priority::Low),
    (Column::Sync, SYNC_CELLS, Priority::Low),
    (Column::Time, TIME_CELLS, Priority::Low),
];

/// The width a `High` column grows back to whenever the pane has cells to
/// spare (`[min_width, preferred]` — the §5.4 `min_width` itself lives in
/// [`REPO_COLUMNS`]).
fn preferred_width(column: &Column) -> Option<usize> {
    match column {
        Column::Name => Some(NAME_CELLS),
        Column::Branch => Some(BRANCH_CELLS),
        Column::Dirty | Column::Sync | Column::Time => None,
    }
}

/// Draw the bordered repository list: name, branch, dirty count, ahead /
/// behind markers, and last-commit time, with the shared highlight style.
///
/// The row is budgeted by §5.4 through [`REPO_COLUMNS`]: columns are included
/// in priority order while they fit the row's available width and dropped from
/// the right when they don't. Name and branch are High and flex inside their
/// floors; the status columns disappear whole before they shrink (§5.4
/// 低优先列先消失, 不是缩小).
fn render_repo_list(f: &mut Frame, area: Rect, state: &mut DashboardState<'_>) {
    let budget = pulls::row_width(area);
    let items: Vec<ListItem> = state
        .repos
        .iter()
        .map(|repo| ListItem::new(repo_row(repo, budget)))
        .collect();

    super::render_list_panel(
        f,
        area,
        dashboard_title(state.repos.len()),
        items,
        state.focused,
        state.list_state,
    );
}

/// One repository row fitted to `budget` cells (§5.4). The High columns (name,
/// branch) are always present and absorb leftover cells up to their preferred
/// widths; the Low columns (dirty count, ahead/behind, timestamp) are shown
/// whole or dropped whole from the right — never squeezed alongside a shrunken
/// identifier (§5.4 低优先列先消失, 不是缩小).
fn repo_row(repo: &RepoInfo, budget: usize) -> Line<'static> {
    let dirty_count =
        repo.dirty.untracked + repo.dirty.modified + repo.dirty.added + repo.dirty.deleted;
    let dirty_color = if dirty_count == 0 {
        color::SUCCESS
    } else {
        color::DANGER
    };
    let sync_span = Span::styled(
        format!("{}↑{}↓", repo.ahead, repo.behind),
        if repo.ahead > 0 {
            Style::default().fg(color::WARNING)
        } else {
            Style::default()
        },
    );
    let time_span = Span::raw(format_time(repo.last_commit.time));
    let gap = cells::width("  ");

    // One payload per column of [`REPO_COLUMNS`], in table order.
    let payload = |column: &Column| -> Span<'static> {
        match column {
            Column::Name => Span::raw(repo.name.clone()),
            Column::Branch => Span::styled(repo.branch.clone(), Style::default().fg(color::ACCENT)),
            Column::Dirty => Span::styled(
                format!("{:>4}", dirty_count),
                Style::default().fg(dirty_color),
            ),
            Column::Sync => sync_span.clone(),
            Column::Time => time_span.clone(),
        }
    };
    // Cells a column costs while it is drawn whole: a High column at its
    // preferred width, a Low column at its natural width plus the gap §6.1
    // puts before a status column — so one drop takes column and gap together.
    let cost = |column: &Column| -> usize {
        let natural = cells::width(&payload(column).content);
        preferred_width(column).unwrap_or(natural + gap)
    };

    // Drop Low columns from the right until the row fits. The table is ordered
    // high → low, so the tail is always the lowest-priority column still drawn.
    let mut kept = REPO_COLUMNS.len();
    while kept > HEAD_COLUMNS
        && REPO_COLUMNS[..kept]
            .iter()
            .map(|(column, _, _)| cost(column))
            .sum::<usize>()
            > budget
    {
        kept -= 1;
    }

    // The surviving Low columns keep their natural width; the High columns
    // split what is left. The name wins contested cells first (it is the row's
    // primary identifier) and the branch keeps only its floor until the name
    // has reached its preferred width.
    let low_total: usize = REPO_COLUMNS[HEAD_COLUMNS..kept]
        .iter()
        .map(|(column, _, _)| cost(column))
        .sum();
    let head_budget = budget.saturating_sub(low_total);
    let name_cells = if head_budget >= NAME_CELLS + BRANCH_MIN {
        NAME_CELLS
    } else {
        head_budget
            .saturating_sub(BRANCH_MIN)
            .clamp(NAME_MIN, NAME_CELLS)
    };
    let branch_cells = head_budget
        .saturating_sub(name_cells)
        .clamp(BRANCH_MIN, BRANCH_CELLS);

    let mut spans: Vec<Span<'static>> = Vec::new();
    for (column, _, _) in REPO_COLUMNS.iter().take(kept) {
        match column {
            Column::Name => spans.push(Span::raw(cells::pad_cells(
                &cells::ellipsize(&repo.name, name_cells),
                name_cells,
            ))),
            Column::Branch => spans.push(Span::styled(
                cells::pad_cells(&cells::ellipsize(&repo.branch, branch_cells), branch_cells),
                Style::default().fg(color::ACCENT),
            )),
            Column::Dirty => spans.push(Span::styled(
                format!("{:>4}", dirty_count),
                Style::default().fg(dirty_color),
            )),
            Column::Sync | Column::Time => {
                spans.push(Span::raw("  "));
                spans.push(payload(column));
            }
        }
    }

    // Belt-and-braces: even when a long name plus the floors cannot all fit,
    // the row never exceeds the pane (§5.4 单行永不换行).
    cells::fit_line(Line::from(spans), budget)
}

/// How many [`REPO_COLUMNS`] entries are the High-priority heads — the count
/// that is never dropped however narrow the row gets.
const HEAD_COLUMNS: usize = 2;

/// Cells the batch log's repository-name column reserves when the pane has
/// room for it (§5.4): the name identifies *which* repository a line is
/// about, so it never vanishes — it shrinks toward the pane's width instead,
/// and the message takes whatever cells remain (ellipsized to them).
const BATCH_NAME_CELLS: usize = 20;

/// Draw the scrolling per-repository batch progress log (convention 6):
/// the newest lines stay visible; failures render red, successes green.
fn render_batch_log(f: &mut Frame, area: Rect, state: &DashboardState<'_>) {
    let title = if state.batch_running {
        format!("批量 {} 进度", state.batch_op)
    } else {
        format!("批量 {} 结果", state.batch_op)
    };
    let budget = pulls::block_width(area);
    let name_cells = BATCH_NAME_CELLS.min(budget);
    let message_cells = budget.saturating_sub(name_cells);
    let visible = state
        .batch_log
        .len()
        .saturating_sub(area.height.saturating_sub(2) as usize);
    let lines: Vec<Line> = state.batch_log[visible.min(state.batch_log.len())..]
        .iter()
        .map(|entry| {
            let tone = if entry.is_error {
                color::DANGER
            } else if entry.is_success {
                color::SUCCESS
            } else {
                color::FG_MUTED
            };
            Line::from(vec![
                Span::styled(
                    cells::pad_cells(&cells::ellipsize(&entry.name, name_cells), name_cells),
                    Style::default().fg(tone),
                ),
                Span::styled(
                    cells::ellipsize(&entry.message, message_cells),
                    Style::default().fg(tone),
                ),
            ])
        })
        .collect();
    // The batch log is not a focusable panel (§1.1): it has no cursor and no
    // verbs of its own, so it always wears the unfocused border.
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(role::border(false));
    f.render_widget(Paragraph::new(lines).block(block), area);
}

/// The dashboard's `empty` state (§3.2): the title, one line of why it is
/// empty, and the key that fills it.
///
/// The copy is §6.1:288 verbatim — `还没有仓库` + `按 n 添加扫描目录，或按
/// ? 查看帮助` — and the action hint names `n`, the same key the hint row's
/// view segment advertises (`⏎ 进入仓库  n 添加目录`). It used to advertise
/// `c`, which the dashboard has no binding for: `l/f/b/c` were dropped as
/// stale dispatch surfaces (§6.1 KEYS), so the empty state pointed at a key
/// that did nothing — a §2.3 lie of the same shape as the `P` pull key (B4-09).
fn render_empty_state(f: &mut Frame, area: Rect, focused: bool) {
    state_panel::render_state_panel_with_focus(
        f,
        area,
        &dashboard_title(0),
        &state_panel::empty_state(
            "还没有仓库",
            "按 n 添加扫描目录，或按 ? 查看帮助",
            Some(super::hint("n", "添加扫描目录")),
        ),
        focused,
    );
}

/// Footer hint row for the dashboard (§2.3): the verbs this panel adds on
/// top of the global segment. Key caps take [`super::hint`]'s `key` style and
/// the actions the muted `caption` style, and the pairs are joined by
/// [`join_hints`] so the gap between two key caps is the two spaces §2.3
/// prescribes — the dashboard draws no hint row of its own any more.
///
/// §6.1:285 fixes this view segment to `⏎ 进入仓库  n 添加目录`: `r` and `q`
/// already ride in the global segment, `j/k` are the obvious cursor verbs that
/// the spec leaves unhinted, and the former `l/f/b/c` entries were dashboard
/// roots that §6.1 folds into the context-tab strip (5b) — keeping them here
/// would advertise a second, stale dispatch surface (§6.1 KEYS).
pub(crate) fn dashboard_hints() -> HintRow {
    HintRow::new(join_hints([hint("⏎", "进入仓库"), hint("n", "添加目录")]))
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
            remote: None,
        }
    }

    /// The panel draws content only (§2.3), so the frame goes through
    /// [`crate::ui::components::footer::draw_frame`] — the shape
    /// `ui::render` uses — and the hint row is the bottom row.
    fn render_to_text(state: &mut DashboardState<'_>) -> String {
        compact_text(&render_to_terminal(state, None))
    }

    /// Same frame with a message on the footer's status row, so a test can
    /// place an *operation* result next to the panel (§4.3).
    fn render_with_status(state: &mut DashboardState<'_>, message: &str) -> Terminal<TestBackend> {
        let status = crate::ui::status::StatusLine {
            message: message.to_string(),
            is_error: true,
        };
        render_to_terminal(state, Some(&status))
    }

    /// Frame height the panel tests draw: the content area plus the two footer
    /// rows, so a test can address `FRAME_ROWS - 2` (§4.1).
    const FRAME_ROWS: u16 = 12;

    /// The panel draws content only (§4.1), so the frame goes through
    /// [`crate::ui::components::footer::draw_frame`] — the shape
    /// `ui::render` uses — and the footer takes the bottom two rows.
    fn render_to_terminal(
        state: &mut DashboardState<'_>,
        status: Option<&crate::ui::status::StatusLine>,
    ) -> Terminal<TestBackend> {
        let hints = dashboard_hints();
        let backend = TestBackend::new(80, FRAME_ROWS);
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
    }

    fn compact_text(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    /// Rows whose whitespace-stripped text contains `needle` — how a test
    /// counts how often one async result reached the screen (§4.1).
    fn rows_with(terminal: &Terminal<TestBackend>, needle: &str) -> Vec<u16> {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .filter(|y| {
                let raw: String = (0..buffer.area.width)
                    .map(|x| buffer[(x, *y)].symbol().to_string())
                    .collect();
                let row: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
                row.contains(needle)
            })
            .collect()
    }

    #[test]
    fn render_with_repos_shows_names_and_branch() {
        let repos = vec![make_repo("alpha", "main"), make_repo("beta", "dev")];
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        let text = render_to_text(&mut DashboardState {
            repos: &repos,
            load_error: None,
            list_state: &mut list_state,
            batch_log: &[],
            batch_op: "",
            batch_running: false,
            focused: true,
        });
        assert!(text.contains("alpha"), "missing name: {text}");
        assert!(text.contains("beta"), "missing name: {text}");
        assert!(text.contains("main"), "missing branch: {text}");
        assert!(text.contains("dev"), "missing branch: {text}");
    }

    /// §4.3: a failed *scan* is a load failure, so the panel paints its
    /// `error` state (`加载失败` + reason + `按 r 重试`) instead of the list,
    /// and nothing reaches the status row.
    #[test]
    fn render_load_error_shows_the_error_state_not_the_list() {
        let repos = vec![make_repo("alpha", "main")];
        let mut list_state = ListState::default();
        let text = compact(&render_to_text(&mut DashboardState {
            repos: &repos,
            load_error: Some("扫描仓库失败: 权限不足"),
            list_state: &mut list_state,
            batch_log: &[],
            batch_op: "",
            batch_running: false,
            focused: true,
        }));
        assert!(text.contains("加载失败"), "missing error title: {text}");
        assert!(
            text.contains("扫描仓库失败:权限不足"),
            "missing reason: {text}"
        );
        assert!(text.contains("按r重试"), "missing retry hint: {text}");
        assert!(!text.contains("alpha"), "repo list leaked: {text}");
    }

    /// §4.1: a non-error status keeps the list and reaches the footer status
    /// row — once, and nowhere in the content area.
    #[test]
    fn render_status_reaches_the_footer_status_row_only() {
        let repos = vec![make_repo("alpha", "main")];
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        let terminal = render_to_terminal(
            &mut DashboardState {
                repos: &repos,
                load_error: None,
                list_state: &mut list_state,
                batch_log: &[],
                batch_op: "",
                batch_running: false,
                focused: true,
            },
            Some(&crate::ui::status::StatusLine {
                message: "refreshed: 3 repos".to_string(),
                is_error: false,
            }),
        );
        let text = compact(&compact_text(&terminal));
        // Non-error status keeps the list and reaches the status row only.
        assert!(text.contains("alpha"), "list hidden: {text}");
        assert_eq!(
            rows_with(&terminal, "refreshed:3repos"),
            vec![FRAME_ROWS - 2],
            "the status must land on the footer status row only: {text}"
        );
    }

    /// §4.3: an *operation* failure (a batch run that lost a repository) keeps
    /// the loaded list and reports on the status row — the panel does not
    /// blank itself for something it already loaded.
    #[test]
    fn render_operation_error_keeps_the_list_and_reaches_the_status_row_only() {
        let repos = vec![make_repo("alpha", "main")];
        let mut list_state = ListState::default();
        let terminal = render_with_status(
            &mut DashboardState {
                repos: &repos,
                load_error: None,
                list_state: &mut list_state,
                batch_log: &[],
                batch_op: "",
                batch_running: false,
                focused: true,
            },
            "批量 pull 完成: 1 成功 / 1 失败",
        );
        let text = compact(&compact_text(&terminal));
        assert!(text.contains("alpha"), "the list must survive: {text}");
        assert_eq!(
            rows_with(&terminal, "批量pull完成:1成功/1失败"),
            vec![FRAME_ROWS - 2],
            "the result belongs on the footer status row only: {text}"
        );
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
            load_error: None,
            list_state: &mut list_state,
            batch_log: &batch_log,
            batch_op: "pull",
            batch_running: true,
            focused: true,
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

    /// §3.2: the dashboard's `empty` state is the three-part shape — the
    /// title, one line of description and the action that fills it, all three
    /// verbatim from DESIGN §6.1:288. The negative assertion is the point of
    /// this test: `c` was advertised here while the dashboard binds no `c`, so
    /// an empty dashboard pointed at a key that did nothing (B4-09).
    #[test]
    fn render_empty_repos_shows_the_design_copy_and_no_dead_key() {
        let mut list_state = ListState::default();
        let text = render_to_text(&mut DashboardState {
            repos: &[],
            load_error: None,
            list_state: &mut list_state,
            batch_log: &[],
            batch_op: "",
            batch_running: false,
            focused: true,
        });
        let compact = compact(&text);
        assert!(
            compact.contains("还没有仓库"),
            "missing empty-state title: {compact}"
        );
        assert!(
            compact.contains("按n添加扫描目录，或按?查看帮助"),
            "missing empty-state description: {compact}"
        );
        assert!(
            compact.contains("n添加扫描目录"),
            "missing empty-state action: {compact}"
        );
        assert!(
            !compact.contains("按c管理扫描目录"),
            "empty state advertises `c`, which the dashboard does not bind \
             (§2.3): {compact}"
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
            load_error: None,
            list_state: &mut list_state,
            batch_log: &[],
            batch_op: "",
            batch_running: false,
            focused: true,
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
        let mut list_state = ListState::default();
        let text = render_to_text(&mut DashboardState {
            repos: &[],
            load_error: Some("扫描仓库失败: 权限不足"),
            list_state: &mut list_state,
            batch_log: &[],
            batch_op: "",
            batch_running: false,
            focused: true,
        });
        let compact = compact(&text);
        assert!(compact.contains("扫描仓库失败"), "missing error: {compact}");
        assert!(
            !compact.contains("暂无仓库"),
            "hint must not mask the error: {compact}"
        );
    }

    /// §2.3 + §6.1:285: the dashboard view segment is the spec-fixed verbs only —
    /// `⏎ 进入仓库  n 添加目录` — and never the reverse-frozen `c/l/f/b/j/k` set
    /// that §6.1 KEYS folds into the context-tab strip (5b) or the global
    /// segment. This pins the new segment so the stale dispatch surface cannot
    /// drift back in: DESIGN.md §6.1 wins over the old test.
    #[test]
    fn hint_row_view_segment_follows_design_6_1() {
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
        assert!(
            compact.contains("进入仓库"),
            "missing dashboard verb: {compact}"
        );
        assert!(
            compact.contains("n添加目录"),
            "missing add-dir verb: {compact}"
        );
        assert!(
            !compact.contains("c配置"),
            "stale config entry regressed: {compact}"
        );
        assert!(
            !compact.contains("j/k选择"),
            "stale nav entry regressed: {compact}"
        );
    }
}
