//! Repository dashboard: renders `App.repos` with selection and error state.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, ListItem, ListState, Paragraph},
    Frame,
};

use crate::app::{BatchLogLine, DashboardRow};
use crate::git;
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
    /// The list positions to draw: section headers interleaved with the
    /// repositories they head (sub-task 6, §6.1 组头行).
    ///
    /// Borrowed from [`crate::app::App::dashboard_rows`], so the view never re-derives the
    /// grouping and can never disagree with the mapping the key handlers use.
    /// Each row carries its own `repos` index, which is what makes a header
    /// row survivable: the renderer needs `repo_row(repo, budget)` for a `Repo`
    /// and nothing at all for a `Header`.
    pub rows: &'a [DashboardRow],
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
    /// Whether the preview panel holds the screen's focus (§3.1): `Tab 面板`
    /// on the dashboard steps focus between the list and this panel, so it must
    /// wear `border_focus` — otherwise the key would move focus to a panel that
    /// looks no different for holding it, a B7-02-class fake closure. The
    /// preview itself is a fixed read-out of the selection (§6.1:274) and draws
    /// no cursor of its own, so it only borrows the border role.
    pub focused_preview: bool,
}

/// Bordered-block title shared by every dashboard state: the product label
/// plus the visible repository count, so the empty state (`0`) and the list
/// state stay word-identical.
fn dashboard_title(count: usize) -> String {
    format!("GitPulse 仓库状态 ({count})")
}

/// Minimum width of the preview panel in cells (§5.4) — read from the shared
/// floor in [`crate::ui::components::theme::width`] so the dashboard no longer keeps
/// its own copy of
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
/// cursor rather than owning a cursor (§3.1), so the only thing focus changes
/// here is which panel wears the accent border.
fn render_preview(f: &mut Frame, area: Rect, state: &DashboardState<'_>) {
    let block = Block::default()
        .title("预览")
        .borders(Borders::ALL)
        .border_style(role::border(state.focused_preview));
    let rows = state.rows;
    let lines = match state
        .list_state
        .selected()
        .and_then(|i| rows.get(i))
        .and_then(|row| row.repo())
        .and_then(|i| state.repos.get(i))
    {
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
/// Cells the conflict-marker column holds (`⚠2`); §6.1:262 splits conflicts out
/// of the dirty count so an unmergeable row is visible without arithmetic.
const WARN_CELLS: usize = 4;
/// Cells the remote-platform column holds — the longest known platform label
/// (`Bitbucket`) plus the two-cell gap §6.1 puts before a status column.
const REMOTE_CELLS: usize = 11;
/// Cells the author column holds (v0.9.0): the `user.name` git resolves for
/// this repository, ellipsized — the full `名字 <邮箱>` lives in the commit
/// popup title, where there is room for it.
const AUTHOR_CELLS: usize = 12;
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

/// The data a repository row column renders — the seven columns of §6.1:262's
/// sample row (`名称/分支/●3/⚠1/↑2↓0/远端: GitHub/作者`): dirty and warning stay
/// split (no mental arithmetic), and the timestamp is not a row column at all —
/// it lives in the preview panel instead.
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
    /// Conflict count — split out of `Dirty` per §6.1:262.
    Warn,
    /// Fetch-remote platform (`远端: GitHub`), or `无` for a local-only repo.
    Remote,
    /// Effective commit author (`user.name`), or `无` when nothing resolved
    /// (v0.9.0) — the row-level echo of what a commit here would be signed as.
    Author,
}

/// §5.4 column priority table for a repository row: an ordered slice of
/// `(column, min_width, priority)` — every column's `min_width` (the cells it
/// keeps while drawn) and its priority (whether it may be dropped when the row
/// runs out of cells). Ordered high → low, so the renderer keeps a prefix of
/// this slice while the row fits and cuts the tail: `Author` goes before
/// `Remote`, `Remote` before `Warn`, `Warn` before `Sync`, `Sync` before
/// `Dirty`, and the two High columns never go at all.
const REPO_COLUMNS: [(Column, usize, Priority); 7] = [
    (Column::Name, NAME_MIN, Priority::High),
    (Column::Branch, BRANCH_MIN, Priority::High),
    (Column::Dirty, DIRTY_CELLS, Priority::Low),
    (Column::Sync, SYNC_CELLS, Priority::Low),
    (Column::Warn, WARN_CELLS, Priority::Low),
    (Column::Remote, REMOTE_CELLS, Priority::Low),
    (Column::Author, AUTHOR_CELLS, Priority::Low),
];

/// The width a `High` column grows back to whenever the pane has cells to
/// spare (`[min_width, preferred]` — the §5.4 `min_width` itself lives in
/// [`REPO_COLUMNS`]).
fn preferred_width(column: &Column) -> Option<usize> {
    match column {
        Column::Name => Some(NAME_CELLS),
        Column::Branch => Some(BRANCH_CELLS),
        Column::Dirty | Column::Sync | Column::Warn | Column::Remote | Column::Author => None,
    }
}

/// Draw the bordered repository list: name, branch, dirty count, ahead /
/// behind markers, the split-out conflict count, and the remote platform
/// (§6.1:262), with the shared highlight style.
///
/// The row is budgeted by §5.4 through [`REPO_COLUMNS`]: columns are included
/// in priority order while they fit the row's available width and dropped from
/// the right when they don't. Name and branch are High and flex inside their
/// floors; the status columns disappear whole before they shrink (§5.4
/// 低优先列先消失, 不是缩小).
fn render_repo_list(f: &mut Frame, area: Rect, state: &mut DashboardState<'_>) {
    let budget = pulls::row_width(area);
    let items: Vec<ListItem> = state
        .rows
        .iter()
        .map(|row| match row {
            DashboardRow::Header {
                name,
                count,
                collapsed,
                ..
            } => ListItem::new(group_header_row(name, *count, *collapsed, budget)),
            DashboardRow::Repo { repo } => match state.repos.get(*repo) {
                Some(repo) => ListItem::new(repo_row(repo, budget)),
                // A row index the loader has since dropped cannot name a
                // repository; skipping it keeps the list from panicking on a
                // refresh that shrank the list under a stale cursor.
                None => ListItem::new(Line::from("")),
            },
        })
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

/// One group section header (§6.1 组头行, checkpoint ⑨).
///
/// `▸ name (N)` folded / `▾ name (N)` expanded, N counting the group's
/// **visible** repositories — the count a folded header shows is the number
/// hiding under it, not the group's lifetime total, so a fold reads as
/// "3 more" rather than as a smaller section.
///
/// Headers are non-selectable, so they carry no cursor of their own and take
/// §5.4's whole width: the name is the identity and ellipsizes, while the
/// glyph and count are fixed furniture that must survive, or the header stops
/// saying whether it is open.
fn group_header_row(name: &str, count: usize, collapsed: bool, budget: usize) -> Line<'static> {
    let glyph = if collapsed { "▸" } else { "▾" };
    let count_str = format!(" ({count})");
    // The glyph is 1 cell; the count is variable but always drawn, so the name
    // is funded with what is left after both (§5.4 低优先先消失).
    let fixed = cells::width(glyph) + cells::width(&count_str);
    let name = cells::ellipsize(name, budget.saturating_sub(fixed));
    Line::from(vec![
        Span::styled(glyph, Style::default().fg(color::ACCENT)),
        Span::styled(name, Style::default().fg(color::FG_MUTED)),
        Span::styled(count_str, Style::default().fg(color::FG_MUTED)),
    ])
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
    // §6.1:262 keeps conflicts in their own `⚠` column: a row that cannot be
    // merged must be visible without adding up the dirty count by hand.
    let warn_span = Span::styled(
        format!("⚠{}", repo.dirty.conflicted),
        if repo.dirty.conflicted > 0 {
            Style::default().fg(color::WARNING)
        } else {
            Style::default()
        },
    );
    let remote_span = Span::styled(
        format!("远端: {}", git::remote_label(repo)),
        Style::default().fg(color::FG_MUTED),
    );
    // v0.9.0: the author column echoes the `user.name` a commit here would be
    // signed with. `无` when nothing resolved, matching `远端: 无` — an
    // unconfigured repository is a normal row, not a failure. The name is
    // ellipsized to the column so a long identity cannot eat the columns
    // beside it (§5.4 单行永不换行).
    let author_span = Span::styled(
        cells::ellipsize(&author_name(repo), AUTHOR_CELLS),
        Style::default().fg(color::FG_MUTED),
    );
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
            Column::Warn => warn_span.clone(),
            Column::Remote => remote_span.clone(),
            Column::Author => author_span.clone(),
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
            Column::Sync | Column::Warn | Column::Remote | Column::Author => {
                spans.push(Span::raw("  "));
                spans.push(payload(column));
            }
        }
    }

    // Belt-and-braces: even when a long name plus the floors cannot all fit,
    // the row never exceeds the pane (§5.4 单行永不换行).
    cells::fit_line(Line::from(spans), budget)
}

/// The name a repository row's `Author` column shows (v0.9.0): the `user.name`
/// git resolves for this repository, or `无` when nothing resolved.
fn author_name(repo: &RepoInfo) -> String {
    repo.author
        .as_ref()
        .map(|author| author.name.clone())
        .unwrap_or_else(|| "无".to_string())
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
/// §6.1 fixes this view segment to `⏎ 进入仓库  n 添加目录`: `r` and `q`
/// already ride in the global segment, `j/k` are the obvious cursor verbs that
/// the spec leaves unhinted, and the former `l/f/b/c` entries were dashboard
/// roots that §6.1 folds into the context-tab strip (5b) — keeping them here
/// would advertise a second, stale dispatch surface (§6.1 KEYS).
///
/// `groups` adds `z 折叠分组` (§6.1.1) **only when scan groups are actually
/// configured**. §2.3 makes advertising a key a promise that dispatch moves
/// something observable; with no `scan_groups`, `z` is a no-op by design
/// ([`crate::app::App::toggle_selected_group`] returns early), so advertising it
    /// anyway
/// would be exactly the dead-key defect B7-01 was written to catch.
pub(crate) fn dashboard_hints(groups: bool) -> HintRow {
    let base = [hint("⏎", "进入仓库"), hint("n", "添加目录")];
    let with_fold = join_hints([base[0].clone(), base[1].clone(), hint("z", "折叠分组")]);
    HintRow::new(if groups { with_fold } else { join_hints(base) })
}

/// Format a Unix timestamp as `YYYY-MM-DD HH:MM` (UTC) without chrono.
///
/// Single source of truth is [`crate::datetime::format_local_time`] (B4-12):
/// the dashboard always renders UTC, so it passes a zero offset.
fn format_time(timestamp: i64) -> String {
    crate::datetime::format_local_time(timestamp, 0)
}

/// Regression hook for sub-task 4: the repository row is §6.1:262's six
/// columns — conflicts split out of the dirty count (`⚠`, not folded into
/// `●`), the remote platform drawn as `远端: …`, and no timestamp column.
/// Falsifiable: folding conflicts back into the dirty count breaks the `⚠`
/// assertion.
#[cfg(test)]
mod row_set_tests {
    use super::*;

    /// A row builder like [`make_repo`], lifted to module scope so the row-set
    /// assertions live next to the production code they pin.
    fn row_repo() -> RepoInfo {
        RepoInfo {
            name: "GitPulse".to_string(),
            path: std::path::PathBuf::from("/tmp/GitPulse"),
            branch: "main".to_string(),
            dirty: Default::default(),
            ahead: 0,
            behind: 0,
            last_commit: Default::default(),
            remote: Some("https://github.com/JeeLin/GitPulse.git".to_string()),
            is_worktree: false,
            main_repo: None,
            author: None,
        }
    }

    #[test]
    fn repo_row_shows_split_conflicts_and_the_remote_platform() {
        let mut repo = row_repo();
        repo.dirty.modified = 3;
        repo.dirty.conflicted = 1;

        let text = repo_row(&repo, 120).to_string();

        // The dirty column counts working-tree dirt; conflicts get their own.
        assert!(text.contains('3'), "dirty count must render: {text}");
        assert!(
            text.contains("⚠1"),
            "conflicts get their own column: {text}"
        );
        assert!(
            text.contains("远端: GitHub"),
            "remote platform column: {text}"
        );
    }

    #[test]
    fn repo_row_renders_no_remote_for_a_local_repository() {
        let mut repo = row_repo();
        repo.remote = None;
        let text = repo_row(&repo, 120).to_string();
        assert!(text.contains("远端: 无"), "missing remote cell: {text}");
    }

    /// v0.9.0: the row echoes the identity a commit here is signed with.
    /// Falsifiable: emptying the `author` field drops the name and this goes
    /// red — the column is an echo, not decoration.
    #[test]
    fn repo_row_shows_author_when_local_config_set() {
        let mut repo = row_repo();
        repo.author = Some(crate::git::AuthorInfo {
            name: "张三".to_string(),
            email: "zhang@example.com".to_string(),
        });

        let text = repo_row(&repo, 120).to_string();

        assert!(text.contains("张三"), "author name must render: {text}");
    }

    /// v0.9.0: an unconfigured repository shows `无` rather than an empty cell
    /// — the same shape `远端: 无` uses, so a missing identity never reads as
    /// a rendering bug.
    #[test]
    fn repo_row_renders_no_author_when_nothing_resolved() {
        let repo = row_repo();
        assert_eq!(repo.author, None);
        let text = repo_row(&repo, 120).to_string();
        assert!(text.contains("无"), "missing identity marker: {text}");
    }

    /// §5.4: `Author` is the lowest-priority column, so a narrow row drops it
    /// *whole* before it shrinks the name (§5.4 低优先列先消失, 不是缩小).
    /// Falsifiable: promoting `Author` above the other Low columns keeps it in
    /// this budget and breaks the assertion.
    #[test]
    fn repo_row_drops_author_column_at_floor() {
        let mut repo = row_repo();
        repo.author = Some(crate::git::AuthorInfo {
            name: "张三".to_string(),
            email: "zhang@example.com".to_string(),
        });

        // A budget wide enough for the High columns and the status columns but
        // not for another Low column: the author must disappear entirely.
        let narrow = repo_row(&repo, 40).to_string();
        assert!(
            !narrow.contains("张三"),
            "author must drop whole at a narrow budget: {narrow}"
        );
        // The row never exceeds its budget whatever it drops (§5.4).
        assert!(
            cells::width(&narrow) <= 40,
            "row exceeded the budget: {narrow}"
        );

        // With room, it is back — the column is budgeted, not removed.
        let wide = repo_row(&repo, 120).to_string();
        assert!(
            wide.contains("张三"),
            "author must render when it fits: {wide}"
        );
    }
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
            is_worktree: false,
            main_repo: None,
            author: None,
        }
    }

    /// Rows for a test with no grouping configured: one `Repo` row per
    /// repository, in order — the same shape `App::dashboard_rows` produces
    /// when `scan_groups` is empty. Tests that exercise grouping pass
    /// explicit rows instead of using this.
    fn flat_rows(repos: &[RepoInfo]) -> Vec<DashboardRow> {
        (0..repos.len())
            .map(|repo| DashboardRow::Repo { repo })
            .collect()
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
        let hints = dashboard_hints(false);
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
        let rows = flat_rows(&repos);
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        let text = render_to_text(&mut DashboardState {
            repos: &repos,
            rows: &rows,
            load_error: None,
            list_state: &mut list_state,
            batch_log: &[],
            batch_op: "",
            batch_running: false,
            focused: true,
            focused_preview: false,
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
        let rows = flat_rows(&repos);
        let mut list_state = ListState::default();
        let text = compact(&render_to_text(&mut DashboardState {
            repos: &repos,
            rows: &rows,
            load_error: Some("扫描仓库失败: 权限不足"),
            list_state: &mut list_state,
            batch_log: &[],
            batch_op: "",
            batch_running: false,
            focused: true,
            focused_preview: false,
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
        let rows = flat_rows(&repos);
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        let terminal = render_to_terminal(
            &mut DashboardState {
                repos: &repos,
                rows: &rows,
                load_error: None,
                list_state: &mut list_state,
                batch_log: &[],
                batch_op: "",
                batch_running: false,
                focused: true,
                focused_preview: false,
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
        let rows = flat_rows(&repos);
        let mut list_state = ListState::default();
        let terminal = render_with_status(
            &mut DashboardState {
                repos: &repos,
                rows: &rows,
                load_error: None,
                list_state: &mut list_state,
                batch_log: &[],
                batch_op: "",
                batch_running: false,
                focused: true,
                focused_preview: false,
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
        let rows = flat_rows(&repos);
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
            rows: &rows,
            load_error: None,
            list_state: &mut list_state,
            batch_log: &batch_log,
            batch_op: "pull",
            batch_running: true,
            focused: true,
            focused_preview: false,
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
            rows: &[],
            load_error: None,
            list_state: &mut list_state,
            batch_log: &[],
            batch_op: "",
            batch_running: false,
            focused: true,
            focused_preview: false,
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
        let rows = flat_rows(&repos);
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        let text = render_to_text(&mut DashboardState {
            repos: &repos,
            rows: &rows,
            load_error: None,
            list_state: &mut list_state,
            batch_log: &[],
            batch_op: "",
            batch_running: false,
            focused: true,
            focused_preview: false,
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
            rows: &[],
            load_error: Some("扫描仓库失败: 权限不足"),
            list_state: &mut list_state,
            batch_log: &[],
            batch_op: "",
            batch_running: false,
            focused: true,
            focused_preview: false,
        });
        let compact = compact(&text);
        assert!(compact.contains("扫描仓库失败"), "missing error: {compact}");
        assert!(
            !compact.contains("暂无仓库"),
            "hint must not mask the error: {compact}"
        );
    }

    /// §6.1.1: a header renders as `▾ 组名 (N)` open and `▸ 组名 (N)` folded, so
    /// the glyph is what says whether the section is open.
    ///
    /// Falsifiable: swapping the two glyphs, or drawing the count from the
    /// collapsed total instead of the visible one, turns this red.
    #[test]
    fn a_group_header_draws_its_glyph_name_and_count() {
        let expanded = group_header_row("alpha", 3, false, 60).to_string();
        assert!(
            expanded.contains("▾") && expanded.contains("alpha") && expanded.contains("(3)"),
            "an expanded header is `▾ name (N)`: {expanded}"
        );

        let folded = group_header_row("alpha", 3, true, 60).to_string();
        assert!(
            folded.contains("▸") && folded.contains("alpha") && folded.contains("(3)"),
            "a folded header is `▸ name (N)`: {folded}"
        );
        assert!(
            !expanded.contains("▸") && !folded.contains("▾"),
            "the two states must not share a glyph"
        );
    }

    /// §5.4 + §6.1.1: the glyph and the count are fixed furniture that survive any
    /// budget; the name is what gives way, elided with `…` rather than wrapped or
    /// dropped. A header that lost its glyph would stop saying whether it is open.
    #[test]
    fn a_group_header_keeps_its_furniture_and_elides_only_the_name() {
        let long = "a-very-long-group-name-that-cannot-possibly-fit";
        let narrow = group_header_row(long, 12, false, 14).to_string();
        assert!(
            narrow.contains("▾") && narrow.contains("(12)"),
            "the glyph and count survive the narrow budget: {narrow}"
        );
        assert!(
            narrow.contains('…'),
            "the name is elided instead of wrapping or vanishing: {narrow}"
        );
        assert!(
            !narrow.contains(long),
            "an over-long name must not be drawn whole at a narrow budget: {narrow}"
        );

        // A budget too small for even the furniture must not panic or wrap — the
        // name simply gets nothing.
        let starved = group_header_row(long, 12, false, 1).to_string();
        assert!(
            starved.contains("▾") && starved.contains("(12)"),
            "the fixed furniture is still drawn at the floor: {starved}"
        );
    }

    /// §2.3 + §6.1.1: `z 折叠分组` is advertised **only** when scan groups are
    /// configured. With no groups `z` has nothing to fold, so advertising it would
    /// be a dead key — the B7-02/B7-01 defect shape.
    #[test]
    fn the_fold_hint_appears_only_when_groups_are_configured() {
        let plain = dashboard_hints(false).view.to_string();
        assert!(
            plain.contains("添加目录") && plain.contains("进入仓库"),
            "the dashboard verbs are always advertised: {plain}"
        );
        assert!(
            !plain.contains("折叠分组"),
            "`z` must not be advertised without configured groups: {plain}"
        );

        let grouped = dashboard_hints(true).view.to_string();
        assert!(
            grouped.contains("折叠分组"),
            "configured groups advertise the fold verb: {grouped}"
        );
        assert!(
            grouped.contains("进入仓库") && grouped.contains("添加目录"),
            "adding the fold verb must not displace the others: {grouped}"
        );
    }

    /// §6.1.1: the panel draws headers and repositories together, and only the
    /// repository rows carry an index the key handlers can act on.
    #[test]
    fn the_list_draws_group_headers_above_their_repositories() {
        let repos = vec![make_repo("one", "main"), make_repo("two", "main")];
        let rows = vec![
            DashboardRow::Header {
                group: 0,
                name: "alpha".to_string(),
                count: 2,
                collapsed: false,
            },
            DashboardRow::Repo { repo: 0 },
            DashboardRow::Repo { repo: 1 },
        ];
        let mut list_state = ListState::default();
        list_state.select(Some(1));
        let compact = compact(&render_to_text(&mut DashboardState {
            repos: &repos,
            rows: &rows,
            load_error: None,
            list_state: &mut list_state,
            batch_log: &[],
            batch_op: "",
            batch_running: false,
            focused: true,
            focused_preview: false,
        }));
        assert!(
            compact.contains("alpha") && compact.contains("(2)"),
            "the header is drawn: {compact}"
        );
        assert!(
            compact.contains("one") && compact.contains("two"),
            "its repositories are drawn beneath it: {compact}"
        );
    }

    /// §2.3 + §6.1: the dashboard view segment is the spec-fixed verbs —
    /// `⏎ 进入仓库  n 添加目录` — and never the reverse-frozen `c/l/f/b/j/k` set
    /// that §6.1 KEYS folds into the context-tab strip (5b) or the global
    /// segment. This pins the segment so the stale dispatch surface cannot
    /// drift back in. Called with `groups = false` here; §6.1.1's conditional
    /// `z 折叠分组` has its own assertion in
    /// `the_fold_hint_appears_only_when_groups_are_configured`.
    #[test]
    fn hint_row_view_segment_follows_design_6_1() {
        let backend = TestBackend::new(120, 12);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|f| {
                crate::ui::components::footer::draw_frame(
                    f,
                    StatusRow::plain(None),
                    &dashboard_hints(false),
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
