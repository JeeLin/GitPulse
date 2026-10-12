use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use super::components::state_panel::{self, PanelState};
use super::components::theme::{color, role, spacing, width as theme_width};
use super::{hint, pulls, render_confirm_overlay};
use crate::git::FileEntry;
use crate::ui::components::cells;
use crate::ui::components::footer::{hints as join_hints, HintRow};

/// State for rendering the single-repo view.
///
/// The panel draws its content only (§4.1): the status row and the hint row
/// are both painted once by the footer, so no async *result* is read here. A
/// failed *load* is not an async result — it belongs to the panel, and arrives
/// as [`RepoState::load_error`].
pub struct RepoState<'a> {
    /// Working-tree entries to list.
    pub entries: &'a [FileEntry],
    /// Selection cursor shared across frames.
    pub list_state: &'a mut ListState,
    /// Whether the discard confirmation is open.
    pub show_discard_confirm: bool,
    /// File the confirmation targets, when open.
    pub confirm_file: Option<&'a FileEntry>,
    /// Load failure of the file list / repository status (§4.3): the panel
    /// shows its `error` state instead of the list.
    pub load_error: Option<&'a str>,
    /// Whether this panel holds the screen's focus (§3.1): it picks the
    /// border color and whether the list draws the `▌` / `surface_alt` cursor.
    pub focused: bool,

    // §6.2.1 inline preview pane (feature A).
    /// Per-entry added/removed line counts, aligned 1:1 with `entries`;
    /// `None` is an entry whose diff could not be read (an untracked file).
    pub line_counts: &'a [Option<(usize, usize)>],
    /// Diff text the preview pane renders, or `None` for the placeholder.
    pub preview_content: Option<&'a str>,
    /// Scroll offset of the preview pane, driven by `j`/`k` while it holds
    /// focus (§3.1).
    pub preview_scroll: u16,
    /// Whether the preview pane holds the focus (§3.1): picks its border and
    /// whether the cursor `▌` is drawn.
    pub focused_preview: bool,
}

/// Render the single-repo file status list beside its inline diff preview,
/// plus the discard confirmation overlay when it is open.
///
/// The split is gated by width (§5.4): the preview pane only opens once the
/// frame funds both floors. A failed load paints the *whole screen's* `error`
/// state (§4.3) instead of the list — an operation failure leaves the list
/// exactly as it was.
pub fn render_repo(f: &mut Frame, content: Rect, mut state: RepoState<'_>) {
    if let Some(message) = state.load_error {
        // §4.3: the failure replaces the list. §3.1 still holds — the failed
        // panel keeps the focus border, only the cursor is gone (an `error`
        // state has none).
        state_panel::render_state_panel_with_focus(
            f,
            content,
            "文件状态",
            &PanelState::Error { message },
            state.focused,
        );
        return;
    }

    let (list_area, preview_area) = split_repo(content);

    render_file_list(f, list_area, &mut state);
    if let Some(preview) = preview_area {
        render_preview(f, preview, &state);
    }

    // Overlay discard confirmation dialog
    if state.show_discard_confirm {
        if let Some(file) = state.confirm_file {
            render_discard_dialog(f, content, file);
        }
    }
}

/// §5.4 split gate for the repo view (§6.2.1): the file list and its inline
/// preview stand side by side only once the frame funds both floors — the
/// list at [`theme_width::LIST_MIN`] and the preview at
/// [`theme_width::PREVIEW_MIN`], with one [`spacing::GUTTER`] between them.
/// That is exactly [`theme_width::SPLIT_FLOOR`], the canonical permission to
/// split at all.
///
/// Below the floor there is no second layout to fall back to: the list keeps
/// the full width and the preview is simply absent, the reading of §5.4's
/// 「窄屏隐藏」 the repo view specifies — the detail pane is never allowed to
/// squeeze the list below its floor, and never gets an inline preview of its
/// own here (that is the dashboard's arrangement, §6.1).
fn split_repo(content: Rect) -> (Rect, Option<Rect>) {
    if content.width < theme_width::SPLIT_FLOOR {
        return (content, None);
    }
    // The preview is floored at `PREVIEW_MIN` and the list grows into the
    // slack, so the list is never the panel that gets starved.
    let preview_w = theme_width::PREVIEW_MIN;
    let list_w = content.width.saturating_sub(preview_w + spacing::GUTTER);
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(list_w),          // file list
            Constraint::Length(spacing::GUTTER), // gap
            Constraint::Length(preview_w),       // inline preview
        ])
        .split(content);
    (chunks[0], Some(chunks[2]))
}

/// Bottom key-hint row: y/n/Esc while the discard confirmation is open (every
/// other list key is swallowed then), the file-list actions otherwise.
///
/// The pairs are §6.2.1's 提示行视图段 verbatim, in §2.3's priority order, with
/// the two keys that share one action folded together (`⏎ 查看diff`, `p/P`,
/// `b/h/t`) so the view segment keeps its cells inside the 80-column line.
/// `A 切换身份` (v0.9.0) sits in its own cap because it is a distinct key from
/// `a` (stage-all): folding them would advertise a single cap for two verbs,
/// and §2.3 requires every advertised cap to dispatch. The commit message
/// input owns its own row, see [`crate::ui::commit::commit_hints`].
fn key_hints(discard_confirm_open: bool) -> Line<'static> {
    if discard_confirm_open {
        // Shared row (G5): the y/n/Esc wording of the confirmation, matching
        // the popup title `确认丢弃更改`.
        super::confirm_hints()
    } else {
        join_hints([
            hint("⏎", "查看diff"),
            hint("s/u", "暂存"),
            hint("c", "提交"),
            hint("A", "切换身份"),
            hint("p/P", "推送/拉取"),
            hint("b/h/t", "面板"),
        ])
    }
}

/// Footer hint row for the single-repo view (§2.3): the list actions, or the
/// y/n/Esc row while the discard dialog owns focus. The row itself is composed
/// by `crate::ui::components::footer::compose_hint_row` through
/// [`join_hints`], so the key/desc pairs keep the two-cell gap §2.3 fixes.
pub(crate) fn repo_hints(show_discard_confirm: bool) -> HintRow {
    HintRow::new(key_hints(show_discard_confirm))
}

/// The file panel's §3.2 `empty` state: a clean working tree has no files to
/// list, and no key can change that — every verb below (`s` / `c` / `p`) acts
/// on a row that is not there — so the panel states the fact and its `r`.
fn empty_files_state() -> state_panel::PanelState<'static> {
    state_panel::empty_state("工作区干净", "所有更改已提交", Some(hint("r", "重新读取")))
}

/// Title of the §6.2.1 inline preview pane. Mirrors the diff sub-screen's
/// `文件差异` so the two views read as one diff affordance.
const PREVIEW_TITLE: &str = "文件差异预览";

/// Placeholder text the preview pane shows when there is no diff to render —
/// no file selected, or a file with no readable diff (untracked) — rather than
/// an `error` state (§4.3: a missing preview is never a failed load).
const PREVIEW_EMPTY: &str = "选择一个文件查看 diff 摘要";

/// Render file list
fn render_file_list(f: &mut Frame, area: Rect, state: &mut RepoState<'_>) {
    let title = format!("文件状态 ({} 文件)", state.entries.len());

    if state.entries.is_empty() {
        state_panel::render_state_panel_with_focus(
            f,
            area,
            &title,
            &empty_files_state(),
            state.focused,
        );
        return;
    }

    let width = pulls::row_width(area);
    // Copy the slice reference out before the mapping closure so the cursor
    // borrow below is the only thing touching `state` afterwards (§3.1).
    let line_counts = state.line_counts;
    // §5.4 orders the columns by what may be sacrificed, and the counts column
    // is the low-priority one: it disappears *whole* rather than being clipped
    // mid-number. `row_width` has already taken the cursor and the border off,
    // so what is left must still fund the two-cell symbol plus the counts
    // column before any path cell is worth keeping.
    let digits = counts_digit_width(line_counts).min(funded_counts_digits(width));
    let items: Vec<ListItem> = state
        .entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let counts = line_counts.get(index).copied().flatten();
            file_item(entry, counts, digits, width)
        })
        .collect();

    // §3.2 `ready`: the shell is painted once and the rows fill its inner rect.
    // Operation results no longer flash inside the panel — they ride on the
    // status row (§4.2), so the list always renders its plain `ready` shell.
    let inner = state_panel::render_state_panel_with_focus(
        f,
        area,
        &title,
        &PanelState::Ready,
        state.focused,
    );

    let list = super::with_cursor(List::new(items), state.focused);
    f.render_stateful_widget(list, inner, state.list_state);
}

/// Widest digit run any of the list's counts needs (§6.2.1 `增删行数`).
///
/// The column is padded to one shared width across every row so the numbers
/// line up column-wise; taking the max of the whole list is what makes a
/// single-digit `+9` and a triple-digit `+128` share one column instead of
/// each filling its own. A list with no counts at all measures `0`, which is
/// how [`file_item`] knows to drop the column entirely.
fn counts_digit_width(counts: &[Option<(usize, usize)>]) -> usize {
    counts
        .iter()
        .flatten()
        .map(|(added, removed)| digits_of(*added).max(digits_of(*removed)))
        .max()
        .unwrap_or(0)
}

/// Decimal digit count of `n`, at least one — `0` is one digit, not none.
fn digits_of(n: usize) -> usize {
    n.to_string().len().max(1)
}

/// Cells the whole right-hand `增删行数` tail takes at `digit_width`: the
/// `+NN -NN` column itself (`+`, N digits, a space, `-`, N digits = 2N + 3)
/// plus the one-cell gap that separates it from the path.
fn counts_column_cells(digit_width: usize) -> usize {
    2 * digit_width + 4
}

/// Cells the row's cursor marker occupies on top of its content.
///
/// `row_width` already took the block borders off, but ratatui's
/// `highlight_symbol` writes the `▌` inside the area on top of whatever the
/// item renders. A row that budgets the full `row_width` for symbol, path and
/// counts therefore overflows by this much, and its rightmost cell (the
/// removed-line count) gets clipped. Reserving it here is what keeps the counts
/// column whole instead of half-rendered.
const CURSOR_CELLS: usize = 1;

/// Widest counts column `width` can fund without squeezing the path out of
/// existence (§5.4).
///
/// The row spends its width in the design's own order — two-cell state symbol,
/// then the path, then the counts column — so the column is only kept while
/// the symbol and a path worth reading both still fit beside it. Anything
/// narrower and the column is dropped whole, which is what `counts_column_cells`
/// returning 0 (via a zero `digit_width`) already means to [`file_item`].
fn funded_counts_digits(width: usize) -> usize {
    // Two cells for the state symbol, and a path that has to stay long enough
    // to identify a file before a line count is worth anything.
    const SYMBOL_CELLS: usize = 2;
    /// Minimum cells a path must keep (§5.4) before a `+N -M` counts column is
    /// worth the width it costs — below it the column is dropped whole.
    ///
    /// The derivation, in display cells, is reproducible from the row layout
    /// [`file_item`] draws. A row that shows *both* the shortest still
    /// identifiable path and one-digit counts needs:
    ///
    /// ```text
    /// src/            4   shortest identifiable prefix (§6.2.1 paths are repo-relative)
    /// ab              4   shortest still-distinguishable file stem
    /// +1 -1           6   one-digit counts column (`counts_column_cells(1)`)
    /// gap             2   the separator between path and counts
    /// ─────────────────
    ///                16   path + column
    /// ```
    ///
    /// 16 is too wide to demand: on an 80-column frame the repo pane is half
    /// the terminal, so requiring 16 cells of path *plus* the two-cell symbol
    /// would drop the counts column on ordinary paths and shrink the path on
    /// the few that do fit. 12 is the compromise — a path stays readable
    /// (`src/main.rs` survives 12 cells intact) while the counts column still
    /// appears on the wide rows it is worth showing on. It is an
    /// implementation parameter, not a design constant: §5.4 fixes the
    /// *rule* (low-priority column disappears before the path shrinks) but
    /// leaves this floor to the implementation, and the two assertions
    /// `funded_counts_digits_drops_the_column_at_the_floor` /
    /// `..._funds_one_digit_above_it` pin the resulting behaviour so the
    /// number cannot drift unnoticed.
    const MIN_PATH_CELLS: usize = 12;
    let spare = width.saturating_sub(SYMBOL_CELLS + MIN_PATH_CELLS);
    // `counts_column_cells` is `2 * digits + 4`, so invert it and clamp at 0.
    spare.saturating_sub(4) / 2
}

/// One working-tree row: state symbol, path, then the right-aligned `+`/`-`
/// line counts (§6.2.1 `▌ | 状态字母 | 文件路径（可省略号）| 增删行数`).
///
/// §5.4 budget, in the design's own order of sacrifice: the state symbol is
/// two cells and never cut; the path gives up cells from the right with `…`;
/// and the counts column is the Low-priority column, so it is the first thing
/// that disappears — whole — when the pane cannot fund it, rather than
/// shrinking into unreadable digits. An entry whose diff could not be read
/// (`None`) keeps the column's width but prints no number, so the counts of
/// its neighbours stay aligned.
fn file_item(
    entry: &FileEntry,
    counts: Option<(usize, usize)>,
    digit_width: usize,
    width: usize,
) -> ListItem<'static> {
    let (symbol, style) = super::file_state_symbol(entry.state);

    if digit_width == 0 {
        // No entry reports counts, so there is no column to align to: the row
        // is the symbol plus a path that may use the whole budget.
        let path = cells::ellipsize(
            &entry.path.display().to_string(),
            width.saturating_sub(CURSOR_CELLS),
        );
        return ListItem::new(Line::from(vec![
            Span::styled(symbol, style),
            Span::raw(path),
        ]));
    }

    let column = counts_column_cells(digit_width);
    let path_budget = width.saturating_sub(column + CURSOR_CELLS);
    let path = cells::pad_cells(
        &cells::ellipsize(&entry.path.display().to_string(), path_budget),
        path_budget,
    );

    let tail = match counts {
        Some((added, removed)) => format!(
            " +{added:>digit_width$} -{removed:>digit_width$}",
            digit_width = digit_width
        ),
        None => " ".repeat(column),
    };

    ListItem::new(Line::from(vec![
        Span::styled(symbol, style),
        Span::raw(path),
        Span::styled(tail, role::caption()),
    ]))
}

/// Added and removed line counts of a unified diff body, as
/// `(added, removed)`.
///
/// The patch's `+++` / `---` lines are file headers rather than changes, and
/// a `\`-prefixed line is the "no newline at end of file" marker — counting
/// either would overstate the change by a line the reader never sees as one.
/// This is the pure arithmetic behind the §6.2.1 `增删行数` column, kept
/// separate from the render so it can be tested without a frame.
pub(crate) fn patch_counts(diff: &str) -> (usize, usize) {
    let mut added = 0;
    let mut removed = 0;
    for line in diff.lines() {
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        if line.starts_with('+') {
            added += 1;
        } else if line.starts_with('-') {
            removed += 1;
        }
    }
    (added, removed)
}

/// The inline preview pane of the repo view (§6.2.1): the highlighted file's
/// diff summary, which replaces the need to open the full diff sub-screen to
/// see what a file changed.
///
/// A file with nothing to preview is a valid state rather than a failure —
/// no selection, or a file whose diff cannot be read (an untracked one) — so
/// the pane draws its placeholder and never an `error` state; a failed *load*
/// belongs to the file list, which [`render_repo`] already paints (§4.3).
fn render_preview(f: &mut Frame, area: Rect, state: &RepoState<'_>) {
    let block = Block::default()
        .title(PREVIEW_TITLE)
        .borders(Borders::ALL)
        .border_style(role::border(state.focused_preview));

    let paragraph = match state.preview_content {
        None => Paragraph::new(Span::styled(PREVIEW_EMPTY, role::caption())).block(block),
        Some(diff) => {
            // §5.4: a single line never wraps — a wide diff line is cut with
            // `…`, and `fit_line` keeps each span's color up to the cut.
            let lines: Vec<Line> = diff
                .lines()
                .map(|line| {
                    cells::fit_line(patch_line(line), area.width.saturating_sub(2) as usize)
                })
                .collect();
            Paragraph::new(lines)
                .block(block)
                .scroll((state.preview_scroll, 0))
        }
    };
    f.render_widget(paragraph, area);
}

/// One diff line, colored by what it is.
///
/// `+++` / `---` are the patch's file headers rather than changed lines, so
/// they stay plain — coloring them would read as one removal and one addition
/// that never happened. The colors are the §5.1 tokens the full diff viewer
/// uses, kept here as well because `diff.rs` is frozen this release and the
/// preview may not borrow its private helper.
fn patch_line(line: &str) -> Line<'static> {
    let styled = if line.starts_with("+++") || line.starts_with("---") {
        Line::from(Span::raw(line.to_string()))
    } else if line.starts_with('+') {
        Line::from(Span::styled(
            line.to_string(),
            Style::default().fg(color::SUCCESS),
        ))
    } else if line.starts_with('-') {
        Line::from(Span::styled(
            line.to_string(),
            Style::default().fg(color::DANGER),
        ))
    } else if line.starts_with("@@") {
        Line::from(Span::styled(
            line.to_string(),
            Style::default().fg(color::INFO),
        ))
    } else {
        Line::from(Span::raw(line.to_string()))
    };
    styled
}

fn render_discard_dialog(f: &mut Frame, area: Rect, file: &FileEntry) {
    let body = vec![Line::from(vec![
        Span::raw("确定丢弃 "),
        Span::styled(
            file.path.display().to_string(),
            Style::default().fg(color::WARNING),
        ),
        Span::raw(" 的更改?"),
    ])];
    render_confirm_overlay(f, area, "确认丢弃更改", 50, 10, body, true);
}

/// Create a centered rectangle within the given area.
pub fn centered_rect(percent_x: u16, height: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - height) / 2),
            Constraint::Length(height),
            Constraint::Percentage((100 - height) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::FileState;
    use crate::ui::components::footer::StatusRow;
    use crate::ui::components::theme::color;
    use crate::ui::status::{RemoteStatus, StatusLine};
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;
    use ratatui::Terminal;

    fn entry(path: &str, state: FileState) -> FileEntry {
        FileEntry {
            path: std::path::PathBuf::from(path),
            state,
        }
    }

    fn base_state<'a>(entries: &'a [FileEntry], list_state: &'a mut ListState) -> RepoState<'a> {
        RepoState {
            entries,
            list_state,
            show_discard_confirm: false,
            confirm_file: None,
            load_error: None,
            focused: true,
            line_counts: &[],
            preview_content: None,
            preview_scroll: 0,
            focused_preview: false,
        }
    }

    /// The panel draws content only (§4.1), so the frame goes through
    /// [`crate::ui::components::footer::draw_frame`] — the shape
    /// `ui::render` uses — and the footer takes the bottom two rows.
    fn render_to_terminal(state: RepoState<'_>) -> Terminal<TestBackend> {
        render_with_footer(state, StatusRow::plain(None), 80, 24)
    }

    fn render_with_footer(
        state: RepoState<'_>,
        status: StatusRow<'_>,
        width: u16,
        height: u16,
    ) -> Terminal<TestBackend> {
        let hints = repo_hints(state.show_discard_confirm);
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                crate::ui::components::footer::draw_frame(f, status, &hints, |f, body| {
                    render_repo(f, body, state)
                });
            })
            .unwrap();
        terminal
    }

    /// Wide CJK glyphs leave padding cells; compare whitespace-stripped text.
    fn compact(terminal: &Terminal<TestBackend>) -> String {
        let raw: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        raw.chars().filter(|c| !c.is_whitespace()).collect()
    }

    /// Foreground color of the footer's status row: §4.1 puts it at
    /// `height - 2`, directly above the hint row.
    fn status_fg(terminal: &Terminal<TestBackend>) -> Color {
        let buffer = terminal.backend().buffer();
        buffer[(0, buffer.area.height - 2)].fg
    }

    /// Whitespace-stripped text of a single row; the hint row is `height-1`.
    fn row_compact(terminal: &Terminal<TestBackend>, y: u16) -> String {
        let buffer = terminal.backend().buffer();
        let raw: String = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol().to_string())
            .collect();
        raw.chars().filter(|c| !c.is_whitespace()).collect()
    }

    /// Render a hint row alone into an 80x24 frame, so a test can read the
    /// view segment the footer would compose from it.
    fn render_hints_row(row: &HintRow) -> Terminal<TestBackend> {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                crate::ui::components::footer::draw_frame(
                    f,
                    StatusRow::plain(None),
                    row,
                    |_, _| {},
                );
            })
            .unwrap();
        terminal
    }

    /// View segment of the hint row: the bottom row without the global
    /// segment (§2.3), which is fixed on every screen and would otherwise
    /// answer for words like `返回` in the assertions below.
    fn view_segment(terminal: &Terminal<TestBackend>) -> String {
        let row = row_compact(terminal, terminal.backend().buffer().area.height - 1);
        let prefix: String = crate::ui::components::footer::global_hints()
            .spans
            .iter()
            .map(|s| s.content.to_string())
            .collect();
        let prefix: String = prefix.chars().filter(|c| !c.is_whitespace()).collect();
        row.strip_prefix(&prefix)
            .unwrap_or(row.as_str())
            .to_string()
    }

    /// §4.1: the file list and the footer status row, the panel body keeping
    /// no status row of its own.
    #[test]
    fn render_file_list_shows_paths_states_and_status_row() {
        let entries = vec![
            entry("src/lib.rs", FileState::Modified),
            entry("notes.txt", FileState::Untracked),
        ];
        let status = StatusLine {
            message: "工作区有 2 处变更".to_string(),
            is_error: false,
        };
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        let terminal = render_with_footer(
            base_state(&entries, &mut list_state),
            StatusRow::plain(Some(&status)),
            80,
            24,
        );
        let text = compact(&terminal);
        assert!(text.contains("文件状态(2文件)"), "missing title: {text}");
        assert!(
            text.contains("Msrc/lib.rs"),
            "missing modified path: {text}"
        );
        assert!(
            text.contains("?notes.txt"),
            "missing untracked path: {text}"
        );
        assert!(text.contains("工作区有2处变更"), "missing status: {text}");
        assert_eq!(
            rows_with(&terminal, "工作区有2处变更"),
            vec![22],
            "the status must land on the footer status row only"
        );
        // Non-error status renders with the default (unstyled) foreground.
        assert_eq!(status_fg(&terminal), Color::Reset, "status must be plain");
    }

    #[test]
    fn render_file_list_covers_remaining_state_symbols() {
        let entries = vec![
            entry("added.txt", FileState::Added),
            entry("deleted.txt", FileState::Deleted),
            entry("conflict.txt", FileState::Conflicted),
        ];
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        let terminal = render_to_terminal(base_state(&entries, &mut list_state));
        let text = compact(&terminal);
        assert!(text.contains("Aadded.txt"), "missing added symbol: {text}");
        assert!(
            text.contains("Ddeleted.txt"),
            "missing deleted symbol: {text}"
        );
        assert!(
            text.contains("Uconflict.txt"),
            "missing conflicted symbol: {text}"
        );
    }

    /// §3.2: a clean working tree has nothing to list, which is a valid state
    /// rather than a failure — the panel shows the three-part `empty` shape
    /// (title, one line of why, and the key that re-reads the tree) instead of
    /// an empty bordered list.
    #[test]
    fn render_clean_working_tree_shows_the_three_part_empty_state() {
        let mut list_state = ListState::default();
        let terminal = render_to_terminal(base_state(&[], &mut list_state));
        let text = compact(&terminal);
        assert!(text.contains("工作区干净"), "missing empty title: {text}");
        assert!(
            text.contains("所有更改已提交"),
            "missing empty description: {text}"
        );
        assert!(text.contains("r重新读取"), "missing empty action: {text}");
    }

    /// §4.3: a failed load paints the panel's `error` state (`加载失败` + the
    /// reason + `按 r 重试`) and never the footer's status row.
    #[test]
    fn render_load_error_shows_the_panel_error_state_not_the_status_row() {
        let mut list_state = ListState::default();
        let mut state = base_state(&[], &mut list_state);
        state.load_error = Some("加载文件列表失败: boom");
        let terminal = render_with_footer(state, StatusRow::plain(None), 80, 24);
        let text = compact(&terminal);
        assert!(text.contains("加载失败"), "missing error title: {text}");
        assert!(
            text.contains("加载文件列表失败:boom"),
            "missing reason: {text}"
        );
        assert!(text.contains("按r重试"), "missing retry hint: {text}");
        assert!(
            !rows_with(&terminal, "加载文件列表失败:boom").contains(&22),
            "the load failure must not reach the footer status row: {text}"
        );
        // The stale operation message on the status row is a different face:
        // it stays where §4.2 put it and never paints the panel.
        let status = StatusLine {
            message: "stage 失败: boom".to_string(),
            is_error: true,
        };
        let mut list_state = ListState::default();
        let entries = vec![entry("src/lib.rs", FileState::Modified)];
        let terminal = render_with_footer(
            base_state(&entries, &mut list_state),
            StatusRow::plain(Some(&status)),
            80,
            24,
        );
        let text = compact(&terminal);
        assert!(
            text.contains("Msrc/lib.rs"),
            "the list must survive: {text}"
        );
        assert_eq!(
            rows_with(&terminal, "stage失败:boom"),
            vec![22],
            "an operation result stays on the status row only: {text}"
        );
        assert_eq!(
            status_fg(&terminal),
            color::DANGER,
            "error must render danger"
        );
    }

    #[test]
    fn render_discard_confirm_shows_title_file_and_keys() {
        let entries = vec![entry("src/lib.rs", FileState::Modified)];
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        let mut state = base_state(&entries, &mut list_state);
        state.show_discard_confirm = true;
        state.confirm_file = entries.first();
        let terminal = render_to_terminal(state);
        let text = compact(&terminal);
        assert!(
            text.contains("确认丢弃更改"),
            "missing dialog title: {text}"
        );
        assert!(text.contains("src/lib.rs"), "missing file name: {text}");
        assert!(text.contains("y确认"), "missing confirm key: {text}");
        assert!(text.contains("n取消"), "missing cancel key: {text}");
    }

    #[test]
    fn render_discard_confirm_switches_bottom_key_hints() {
        let entries = vec![entry("src/lib.rs", FileState::Modified)];
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        let mut state = base_state(&entries, &mut list_state);
        state.show_discard_confirm = true;
        state.confirm_file = entries.first();
        let terminal = render_to_terminal(state);
        let hints = view_segment(&terminal);
        // The dialog swallows every list key: none of them may be advertised.
        for swallowed in [
            "j/k",
            "diff",
            "暂存切换",
            "丢弃",
            "commit",
            "push",
            "pull",
            "面板",
        ] {
            assert!(
                !hints.contains(swallowed),
                "swallowed key {swallowed} still hinted: {hints}"
            );
        }
        // Only the keys the confirmation actually handles are shown.
        assert!(hints.contains("y确认"), "missing confirm hint: {hints}");
        assert!(hints.contains("n取消"), "missing cancel hint: {hints}");
        assert!(hints.contains("Esc取消对话框"), "missing Esc hint: {hints}");
    }

    /// The commit popup routes every press through `input_modal_key`, so the
    /// list actions behind it may not be advertised either. The popup is
    /// drawn by [`crate::ui::commit`] over this panel, so the switched row is
    /// [`crate::ui::commit::commit_hints`]' own output.
    #[test]
    fn render_commit_input_switches_bottom_key_hints() {
        let terminal = render_hints_row(&crate::ui::commit::commit_hints());
        let hints = view_segment(&terminal);
        for swallowed in [
            "j/k",
            "diff",
            "暂存切换",
            "丢弃",
            "commit",
            "push",
            "pull",
            "面板",
        ] {
            assert!(
                !hints.contains(swallowed),
                "swallowed key {swallowed} still hinted: {hints}"
            );
        }
        assert!(hints.contains("Enter提交"), "missing submit hint: {hints}");
        assert!(hints.contains("Esc取消"), "missing cancel hint: {hints}");
    }

    /// The status row used to drop a pending status whenever the running
    /// feedback was on screen, which hid the `已有远端操作进行中` notice of a
    /// rejected second push (F1). Both now share the row: the running
    /// message keeps its own cell and the status renders beside it — and
    /// with no progress source left (G1), the row carries the message
    /// rather than a frozen percentage.
    /// §4.1: a running push takes the status row over while the panel keeps
    /// its rows — the old data stays and no progress source is frozen.
    #[test]
    fn render_remote_running_shows_message_and_pending_status_row() {
        let status = StatusLine {
            message: "底层状态行".to_string(),
            is_error: false,
        };
        let running = RemoteStatus::Running("正在 push…".to_string());
        let mut list_state = ListState::default();
        let state = base_state(&[], &mut list_state);
        let terminal = render_with_footer(
            state,
            StatusRow::new(Some(&status), Some(&running), "push"),
            80,
            24,
        );
        let text = compact(&terminal);
        assert!(text.contains("正在push"), "missing running message: {text}");
        assert!(
            !text.contains('%'),
            "frozen progress percent must be gone: {text}"
        );
        assert!(
            text.contains("底层状态行"),
            "pending status hidden by the running line: {text}"
        );
        assert_eq!(
            rows_with(&terminal, "正在push"),
            vec![22],
            "the running message must appear once, on the status row"
        );
    }

    #[test]
    fn render_remote_success_names_operation_in_green() {
        let mut list_state = ListState::default();
        let state = base_state(&[], &mut list_state);
        let terminal = render_with_footer(
            state,
            StatusRow::new(None, Some(&RemoteStatus::Success), "push"),
            80,
            24,
        );
        let text = compact(&terminal);
        assert!(text.contains("push成功"), "missing success text: {text}");
        assert_eq!(
            status_fg(&terminal),
            color::SUCCESS,
            "success must read success"
        );
    }

    #[test]
    fn render_remote_error_shows_reason_in_red() {
        let error = RemoteStatus::Error("pull 失败: diverged".to_string());
        let mut list_state = ListState::default();
        let state = base_state(&[], &mut list_state);
        let terminal =
            render_with_footer(state, StatusRow::new(None, Some(&error), "pull"), 80, 24);
        let text = compact(&terminal);
        assert!(text.contains("diverged"), "missing reason: {text}");
        assert_eq!(status_fg(&terminal), color::DANGER, "error must be danger");
    }

    /// Rows whose whitespace-stripped text contains `needle` — how a test
    /// counts how often one async result reached the screen (§4.1).
    fn rows_with(terminal: &Terminal<TestBackend>, needle: &str) -> Vec<u16> {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .filter(|y| row_compact(terminal, *y).contains(needle))
            .collect()
    }

    /// A 120-column frame keeps the whole view segment on screen: at 80 the
    /// footer truncates it (§2.3), which {@link key_hints_fit_within_80_columns}
    /// pins separately.
    #[test]
    fn render_key_hints_show_action_keys() {
        let entries = vec![entry("src/lib.rs", FileState::Modified)];
        let mut list_state = ListState::default();
        let terminal = render_with_footer(
            base_state(&entries, &mut list_state),
            StatusRow::plain(None),
            120,
            24,
        );
        let text = compact(&terminal);
        // §6.2.1 提示行视图段: ⏎ 查看diff  s 暂存  u 取消  c 提交
        //  p 推送  P 拉取 — the row folds the two keys of one action
        // together so it fits (§2.3 / §5.4), and it never restates the
        // global `Esc 返回` / `q 退出`.
        for pair in ["⏎查看diff", "s/u暂存", "c提交", "p/P推送/拉取", "b/h/t面板"] {
            assert!(text.contains(pair), "missing view pair {pair}: {text}");
        }
        assert!(text.contains("Esc返回"), "missing global back hint: {text}");
    }

    /// The hint row must never overflow an 80-column terminal. The repo
    /// view segment alone is wider than the 24 cells the global segment
    /// keeps, so the row ends in `…` (§2.3) rather than wrapping — and the
    /// global segment survives whole.
    #[test]
    fn key_hints_fit_within_80_columns() {
        let terminal = render_hints_row(&repo_hints(false));
        let hint = view_segment(&terminal);
        let hint = format!("⏎打开Esc返回r刷新q退出{hint}");
        assert!(
            hint.starts_with("⏎打开Esc返回r刷新q退出"),
            "global segment must stay whole: {hint}"
        );
        assert!(hint.ends_with('…'), "the cut must be marked: {hint}");
        assert!(
            hint.contains("⏎查看diff"),
            "the head of the view segment stays: {hint}"
        );
    }

    /// v0.9.0: the repo view advertises `A 切换身份` in its own cap. §2.3 makes
    /// an advertised key a promise — `App::handle_repo_key` binds `Char('A')`,
    /// and `every_advertised_key_on_every_screen_dispatches_an_action` presses
    /// it. This pins the *copy*, so a rename cannot silently drop the verb.
    #[test]
    fn repo_hint_row_advertises_the_identity_switch() {
        let terminal = render_hints_row(&repo_hints(false));
        let text = row_compact(&terminal, terminal.backend().buffer().area.height - 1);
        assert!(
            text.contains("切换身份"),
            "the repo hint row must advertise the identity switch: {text}"
        );
    }

    #[test]
    fn centered_rect_is_horizontally_centered_within_parent() {
        let r = centered_rect(50, 10, Rect::new(0, 0, 80, 24));
        assert_eq!(r.x, 20, "must be horizontally centered: {r:?}");
        assert_eq!(r.width, 40, "must take 50% of the width: {r:?}");
        assert!(r.height > 0, "height must be positive: {r:?}");
        assert!(r.y + r.height <= 24, "must stay inside the parent: {r:?}");
    }

    // === Feature A: §6.2.1 `增删行数` column, §5.4 split gate, inline preview ===

    /// `+++`/`---` are file headers, not changes, and a `\`-prefixed line is
    /// the "no newline at end of file" marker — neither must be counted.
    #[test]
    fn patch_counts_excludes_headers_and_newline_markers() {
        let diff = "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1,3 +1,4 @@\n context\n-removed\n+added\n+added2\n\\ No newline at end of file\n";
        assert_eq!(patch_counts(diff), (2, 1));
    }

    #[test]
    fn patch_counts_handles_empty_and_plain_add_remove_bodies() {
        assert_eq!(patch_counts(""), (0, 0));
        assert_eq!(patch_counts("--- a/x\n+++ b/x\n"), (0, 0));
        assert_eq!(patch_counts("+a\n+b\n-c\n"), (2, 1));
    }

    #[test]
    fn counts_column_cells_grows_with_digit_width() {
        // `+N -N` (2N + 3) plus the one-cell gap that leads the column.
        assert_eq!(counts_column_cells(1), 6);
        assert_eq!(counts_column_cells(2), 8);
        assert_eq!(counts_column_cells(3), 10);
    }

    #[test]
    fn counts_digit_width_is_the_widest_count_in_the_list() {
        let counts: [Option<(usize, usize)>; 3] = [Some((3, 1)), Some((128, 5)), None];
        assert_eq!(counts_digit_width(&counts), 3);
        let empty: [Option<(usize, usize)>; 0] = [];
        assert_eq!(counts_digit_width(&empty), 0);
    }

    /// B7-03: `MIN_PATH_CELLS`'s lower edge, pinned as a measurement instead
    /// of a claim. At the floor (12 cells) the two-cell symbol already eats a
    /// quarter of the row, so nothing is left to fund `counts_column_cells(1)`
    /// and the whole `增删行数` column disappears — §5.4's "low-priority column
    /// disappears before the path shrinks" in its strongest form.
    #[test]
    fn funded_counts_digits_drops_the_column_at_the_floor() {
        assert_eq!(
            funded_counts_digits(12),
            0,
            "at the 12-cell floor the counts column must vanish whole"
        );
        // The floor is a floor: nothing narrower may fund a digit either, and
        // `saturating_sub` must not wrap a tiny width back up into a column.
        for width in 0..=12 {
            assert_eq!(
                funded_counts_digits(width),
                0,
                "width {width} is at or below the floor and must fund no column"
            );
        }
    }

    /// B7-03: the first width that funds one digit, derived rather than
    /// guessed: `SYMBOL_CELLS(2) + MIN_PATH_CELLS(12) + gap/column head(4) +
    /// 2 cells per digit` = 20. One cell narrower still funds nothing, which
    /// is what makes 12 a threshold and not a soft preference.
    ///
    /// Width 18 is pinned explicitly because it is the width a reader
    /// eyeballing the derivation reaches for, and it funds *nothing*: at 18 the
    /// floor plus the symbol already leave `18 - 14 = 4` cells, which the
    /// column's own 4-cell head (`+`, gap, `-`, gap) consumes before a single
    /// digit fits. Funding a digit at 18 would need `MIN_PATH_CELLS = 10`,
    /// which would shorten the shortest path the floor is there to protect —
    /// so 20, not 18, is the real edge and this assertion says so.
    #[test]
    fn funded_counts_digits_funds_one_digit_above_the_floor() {
        assert_eq!(
            funded_counts_digits(18),
            0,
            "18 cells buys the column head only, not a digit"
        );
        assert_eq!(
            funded_counts_digits(19),
            0,
            "19 cells is one short of a one-digit column"
        );
        assert_eq!(
            funded_counts_digits(20),
            1,
            "20 cells funds exactly one digit of the counts column"
        );
        // Every further pair of cells buys one more digit, monotonically.
        assert_eq!(funded_counts_digits(21), 1);
        assert_eq!(funded_counts_digits(22), 2);
        assert_eq!(funded_counts_digits(30), 6);
    }

    /// §5.4: the preview opens once the frame funds both floors and hides
    /// below the floor, leaving the list the full width.
    #[test]
    fn split_repo_opens_the_preview_above_the_floor_and_hides_it_below() {
        // SPLIT_FLOOR = LIST_MIN(28) + GUTTER(1) + PREVIEW_MIN(32) = 61.
        let (list, preview) = split_repo(Rect::new(0, 0, 80, 24));
        let preview = preview.expect("a wide frame must open the preview");
        assert!(
            list.width >= theme_width::LIST_MIN,
            "list must keep its floor"
        );
        assert_eq!(preview.width, theme_width::PREVIEW_MIN);
        assert_eq!(
            list.width + spacing::GUTTER + preview.width,
            80,
            "one gutter between list and preview"
        );

        let (list, preview) = split_repo(Rect::new(0, 0, 50, 24));
        assert!(preview.is_none(), "a narrow frame must hide the preview");
        assert_eq!(list.width, 50, "list takes the whole width when narrow");
    }

    /// The `增删行数` column renders only when an entry reports counts, and the
    /// right-aligned `+N -M` form is emitted; an entry with no diff (untracked)
    /// leaves the column blank but keeps its width.
    #[test]
    fn render_shows_counts_column_when_counts_are_present() {
        let entries = vec![
            entry("src/a.rs", FileState::Modified),
            entry("notes.txt", FileState::Untracked),
        ];
        let counts: [Option<(usize, usize)>; 2] = [Some((3, 1)), None];
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        let mut state = base_state(&entries, &mut list_state);
        state.line_counts = &counts;
        let terminal = render_to_terminal(state);
        let text = compact(&terminal);
        assert!(text.contains("Msrc/a.rs"), "missing modified row: {text}");
        assert!(text.contains("?notes.txt"), "missing untracked row: {text}");
        assert!(text.contains("+3-1"), "missing counts column: {text}");
    }

    /// With no counts anywhere, the column is dropped and the row is the symbol
    /// plus an ellipsized path — the pre-existing layout, untouched.
    #[test]
    fn render_drops_the_counts_column_when_no_counts_are_present() {
        let entries = vec![entry("src/a.rs", FileState::Modified)];
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        let state = base_state(&entries, &mut list_state);
        // base_state already ships an empty `line_counts`.
        let terminal = render_to_terminal(state);
        let text = compact(&terminal);
        assert!(text.contains("Msrc/a.rs"), "missing path: {text}");
        assert!(
            !text.contains("Msrc/a.rs+"),
            "no counts column should be drawn: {text}"
        );
    }

    /// §5.4: a file with nothing to preview is a valid state — the preview pane
    /// shows its placeholder and never an `error` state (§4.3). A file with a
    /// readable diff renders the styled `+`/`-`/`@@` lines.
    #[test]
    fn render_preview_shows_placeholder_then_styled_diff() {
        let entries = vec![entry("src/a.rs", FileState::Modified)];

        // No diff to preview: the placeholder, never an error.
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        let state = base_state(&entries, &mut list_state);
        let terminal = render_with_footer(state, StatusRow::plain(None), 80, 24);
        let text = compact(&terminal);
        assert!(
            text.contains("选择一个文件查看diff摘要"),
            "missing preview placeholder: {text}"
        );
        assert!(
            !text.contains("加载失败"),
            "no diff is never an error: {text}"
        );

        // With a readable diff: the change lines paint in the pane.
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        let mut state = base_state(&entries, &mut list_state);
        state.preview_content = Some("-removed\n+added\n@@ -1,2 +1,2 @@\n");
        let terminal = render_with_footer(state, StatusRow::plain(None), 80, 24);
        let text = compact(&terminal);
        assert!(text.contains("+added"), "missing added line: {text}");
        assert!(text.contains("-removed"), "missing removed line: {text}");
        assert!(text.contains("@@-1,2+1,2@@"), "missing hunk header: {text}");
    }

    /// The counts column survives the narrow (stacked) fall-back: the preview
    /// hides but the list and its `增删行数` do not.
    #[test]
    fn render_counts_column_survives_the_narrow_split_gate() {
        let entries = vec![entry("src/a.rs", FileState::Modified)];
        let counts: [Option<(usize, usize)>; 1] = [Some((12, 3))];
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        let mut state = base_state(&entries, &mut list_state);
        state.line_counts = &counts;
        let terminal = render_with_footer(state, StatusRow::plain(None), 50, 24);
        let text = compact(&terminal);
        assert!(
            text.contains("Msrc/a.rs"),
            "list must survive narrow: {text}"
        );
        assert!(text.contains("+12-3"), "counts must survive narrow: {text}");
        assert!(
            !text.contains("选择一个文件查看diff摘要"),
            "preview must not render when narrow: {text}"
        );
    }
}
