//! The two-row footer (§4): the status row plus the one canonical hint row.
//!
//! [`render_footer`] draws both rows into the bottom two rows of the frame;
//! [`split_area`] hands every view the content area above them, so the hint
//! row has exactly one owner — no panel draws a status line or a key-hint row
//! of its own any more (des-9: "提示行位置/格式漂移").
//!
//! §2.3 makes this module the *only* source of the hint row: [`hint`] builds
//! one key/desc pair and [`hints`] joins pairs with the two-cell
//! [`spacing::HINT_GAP`], so a view segment can never collapse into the glued
//! `j/k 选择g 进入` form. [`global_hints`], the view segment dispatched by
//! [`crate::ui::view_hints`] and the optional [`tail_hints`] are laid out by
//! [`compose_hint_row`] in the fixed order global → view → tail, the global
//! segment never truncated and the other two right-truncated with `…`.

#![allow(dead_code)]

use ratatui::{
    layout::{Constraint, Layout, Rect},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::ui::status::{RemoteStatus, StatusLine};

use super::cells;
use super::theme::{role, spacing};

/// Cells below which the tail segment is dropped instead of shown as a
/// pointless stub (`Tab …` costs more attention than it saves).
const MIN_TAIL_CELLS: usize = 6;

/// Split the frame into the content area and the footer: everything but the
/// bottom [`spacing::FOOTER_ROWS`] rows goes to the view, those rows are the
/// footer (§4.1 — the footer is always exactly 2 rows, whatever the view
/// shows).
pub fn split_area(area: Rect) -> (Rect, Rect) {
    let rows = Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(spacing::FOOTER_ROWS)])
        .split(area);
    (rows[0], rows[1])
}

/// Inputs of the status row (§4.2), identical for local git and GitHub
/// receipts: the plain status line, plus the push/pull feedback that takes
/// the row over while it is present.
///
/// Ephemeral lifetimes (`success`/`info` clear after ~3 s, `error` persists
/// until the next operation) belong to the writer of these fields — the
/// footer paints whatever it is given and never times a message out itself,
/// since the TUI only repaints on input (§4.4).
#[derive(Debug, Clone, Copy)]
pub struct StatusRow<'a> {
    /// Plain status message; errors render red and persist.
    pub status: Option<&'a StatusLine>,
    /// Push/pull feedback; takes the row over while present.
    pub remote_status: Option<&'a RemoteStatus>,
    /// Label of the running/last remote operation (`push` / `pull`).
    pub remote_op: &'a str,
}

impl<'a> StatusRow<'a> {
    /// Status row with remote feedback available (repo, tags, hub panels).
    pub fn new(
        status: Option<&'a StatusLine>,
        remote_status: Option<&'a RemoteStatus>,
        remote_op: &'a str,
    ) -> Self {
        Self {
            status,
            remote_status,
            remote_op,
        }
    }

    /// Status row of a view without remote feedback (branches, history,
    /// config, the receipt panels).
    pub fn plain(status: Option<&'a StatusLine>) -> Self {
        Self {
            status,
            remote_status: None,
            remote_op: "",
        }
    }
}

/// One key/desc pair of the hint row: the key cap in [`role::key`] (bold +
/// accent) and the action in [`role::caption`] (`fg_muted`), §2.3.
///
/// The fragment carries no separator — [`hints`] inserts the two-space gap
/// between pairs, so a row composes from `[hint(..), hint(..), ..]` without
/// hand-written padding.
pub fn hint(k: &str, desc: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(k.to_string(), role::key()),
        Span::raw(" "),
        Span::styled(desc.to_string(), role::caption()),
    ])
}

/// Join hint fragments into one segment, separated by the two-cell gap §2.3
/// prescribes between pairs.
pub fn hints(parts: impl IntoIterator<Item = Line<'static>>) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (index, part) in parts.into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw(spacing::HINT_GAP));
        }
        spans.extend(part.spans);
    }
    Line::from(spans)
}

/// The global segment of the hint row (§2.3 item 1) — the four verbs that
/// mean the same thing on every screen. It is left-aligned and **never
/// truncated**; the view and tail segments give up their cells first.
pub fn global_hints() -> Line<'static> {
    hints([
        hint("⏎", "打开"),
        hint("Esc", "返回"),
        hint("r", "刷新"),
        hint("q", "退出"),
    ])
}

/// The tail segment of the hint row (§2.3 item 3): only screens with more
/// than one panel or more than one context tab carry it, and it is the first
/// part of the row to give up cells on a narrow terminal.
pub fn tail_hints() -> Line<'static> {
    hints([hint("Tab", "面板"), hint("< >", "切换")])
}

/// The canonical hint row: a view segment (the focus panel's own verbs, in
/// priority order) plus the optional tail segment.
///
/// Views build one through their exported `{view}_hints(..)` function; the
/// footer prefixes the global segment and applies the §2.3 truncation.
#[derive(Debug, Clone)]
pub struct HintRow {
    /// View segment: ordered by importance, right-truncated first.
    pub view: Line<'static>,
    /// Tail segment (`Tab 面板  < > 切换`), present only on screens with
    /// several panels or context tabs — and dropped while a modal owns focus.
    pub tail: Option<Line<'static>>,
}

impl HintRow {
    /// Single-panel screen: view segment only.
    pub fn new(view: Line<'static>) -> Self {
        Self { view, tail: None }
    }

    /// Screen with several panels / context tabs: view segment plus the
    /// [`tail_hints`] segment.
    pub fn with_tail(view: Line<'static>) -> Self {
        Self {
            view,
            tail: Some(tail_hints()),
        }
    }
}

/// Render the 2-line footer into `area` (§4.1): the status row on top,
/// the single canonical hint row below it.
pub fn render_footer(f: &mut Frame, area: Rect, status: StatusRow<'_>, hints: &HintRow) {
    let rows = Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    crate::ui::render_status_or_remote(
        f,
        rows[0],
        status.remote_status,
        status.remote_op,
        status.status,
    );

    let line = compose_hint_row(hints, rows[1].width as usize);
    f.render_widget(Paragraph::new(line), rows[1]);
}

/// Compose the hint row (§2.3): the never-truncated global segment, then the
/// view segment, then the tail — the two of them giving up cells right to
/// left, each cut ending in `…` (§5.4), the whole row never wider than
/// `width` columns.
fn compose_hint_row(row: &HintRow, width: usize) -> Line<'static> {
    let gap = spacing::HINT_GAP.len();
    let global = global_hints();
    let mut used = global.width();
    let mut parts = vec![global];

    // The view segment gets what is left after the global segment; a frame
    // too narrow for both shows the global one alone (it never truncates).
    let avail = width.saturating_sub(used + gap);
    if avail > 0 {
        let view = cells::fit_line(row.view.clone(), avail);
        used += gap + view.width();
        parts.push(view);

        // The tail is the lowest-priority segment: it only renders when a
        // meaningful slice of cells survives the two segments before it.
        let avail = width.saturating_sub(used + gap);
        if let Some(tail) = &row.tail {
            if avail >= MIN_TAIL_CELLS {
                parts.push(cells::fit_line(tail.clone(), avail));
            }
        }
    }

    cells::fit_line(hints(parts), width)
}

/// Render one frame of a view with its footer: the content closure draws
/// into the area above the footer, [`render_footer`] takes the bottom two
/// rows.
///
/// This is the shape [`crate::ui::render`] uses in production and the view
/// tests reuse when they call a single view's renderer directly — a view
/// never renders its own status or hint row, so a test that skips this
/// helper would assert against a missing footer.
#[cfg(test)]
pub fn draw_frame(
    f: &mut Frame,
    status: StatusRow<'_>,
    hints: &HintRow,
    content: impl FnOnce(&mut Frame, Rect),
) {
    let (body, footer) = split_area(f.area());
    content(f, body);
    render_footer(f, footer, status, hints);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// Render the footer alone into a `width`×2 frame and hand back the
    /// buffer so a test can read both text and cell styles.
    fn render_buffer(status: StatusRow<'_>, row: &HintRow, width: u16) -> ratatui::buffer::Buffer {
        let backend = TestBackend::new(width, 2);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_footer(f, f.area(), status, row))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    /// Render the footer alone and return both rows as text.
    fn render_rows(status: StatusRow<'_>, row: &HintRow, width: u16) -> (String, String) {
        let buffer = render_buffer(status, row, width);
        let read = |y: u16| -> String {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect()
        };
        (read(0), read(1))
    }

    /// Display cells a rendered hint-row occupies (§2.3: the whole row is never
    /// wider than `width` columns).
    ///
    /// The ratatui buffer stores each wide (CJK) glyph in two cells: the glyph
    /// itself plus a reset *padding* cell whose symbol is `" "` — so the
    /// concatenated row string reads e.g. `"选 择"` instead of `"选择"`. That pad
    /// space would be counted once by [`cells::width`], over-counting every
    /// wide glyph by a cell. We instead drop the padding cell (the glyph already
    /// paid for its two columns) and trim the frame's trailing blanks, so the
    /// measure matches the row's true display width.
    fn row_cells(row: &str) -> usize {
        let trimmed = row.trim_end();
        let mut total = 0usize;
        let mut prev_wide = false;
        for c in trimmed.chars() {
            let w = cells::width(&c.to_string());
            // Pad space always directly follows a wide glyph in the buffer
            // string; skip it (the glyph already counted both its columns).
            if prev_wide && c == ' ' {
                prev_wide = false;
                continue;
            }
            total += w;
            prev_wide = w == 2;
        }
        total
    }

    /// Wide CJK glyphs leave a padding cell; compare whitespace-stripped.
    fn compact(text: &str) -> String {
        text.chars().filter(|c| !c.is_whitespace()).collect()
    }

    /// Foreground of the first non-blank cell of `y`.
    fn first_fg(buffer: &ratatui::buffer::Buffer, y: u16) -> Option<ratatui::style::Color> {
        (0..buffer.area.width)
            .map(|x| &buffer[(x, y)])
            .find(|cell| cell.symbol() != " ")
            .map(|cell| cell.fg)
    }

    #[test]
    fn split_area_reserves_the_bottom_two_rows_for_the_footer() {
        let (body, footer) = split_area(Rect::new(0, 0, 80, 24));
        assert_eq!(body.height, 22, "content keeps every other row");
        assert_eq!(footer.y, 22, "footer starts above the last two rows");
        assert_eq!(footer.height, 2, "footer is always 2 rows (§4.1)");
        assert_eq!(body.bottom(), footer.y, "areas are contiguous");
    }

    /// §2.3: the global segment is the fixed left edge of every hint row.
    #[test]
    fn hint_row_starts_with_the_never_truncated_global_segment() {
        let row = HintRow::new(hints([hint("j/k", "选择"), hint("n", "新建")]));
        let (_, hint_row) = render_rows(StatusRow::plain(None), &row, 80);
        let text = compact(&hint_row);
        let global: String = global_hints()
            .spans
            .iter()
            .map(|s| s.content.to_string())
            .collect();
        assert!(
            text.starts_with(&compact(&global)),
            "global segment must lead the row: {text}"
        );
        for pair in ["⏎打开", "Esc返回", "r刷新", "q退出"] {
            assert!(text.contains(pair), "missing global pair {pair}: {text}");
        }
    }

    #[test]
    fn hint_row_appends_the_view_segment_after_the_global_one() {
        let row = HintRow::new(hints([hint("j/k", "选择"), hint("n", "新建")]));
        let (_, hint_row) = render_rows(StatusRow::plain(None), &row, 80);
        let text = compact(&hint_row);
        assert!(text.contains("j/k选择"), "missing view pair: {text}");
        assert!(text.contains("n新建"), "missing view pair: {text}");
        let global_end = text.find("q退出").expect("global segment missing");
        let view_start = text.find("j/k选择").expect("view segment missing");
        assert!(global_end < view_start, "fixed order: global then view");
    }

    /// The tail segment survives on a wide frame and reports the panel/tab
    /// switches (§2.3 item 3).
    #[test]
    fn hint_row_shows_the_tail_segment_when_there_is_room() {
        let row = HintRow::with_tail(hints([hint("j/k", "选择")]));
        let (_, hint_row) = render_rows(StatusRow::plain(None), &row, 120);
        let text = compact(&hint_row);
        assert!(text.contains("Tab面板"), "missing tail: {text}");
        assert!(text.contains("<>切换"), "missing tail: {text}");
    }

    /// §2.3 truncation: the global segment holds, the tail gives up cells
    /// first, and the row never exceeds the 80-column floor.
    #[test]
    fn hint_row_truncates_the_tail_and_stays_within_80_columns() {
        let row = HintRow::with_tail(hints([
            hint("j/k", "选择"),
            hint("s/u", "暂存切换"),
            hint("c", "commit"),
            hint("p/l", "push/pull"),
            hint("b/h/t", "面板"),
        ]));
        let (_, hint_row) = render_rows(StatusRow::plain(None), &row, 80);
        assert!(row_cells(&hint_row) <= 80, "row is too wide: {hint_row}");
        let text = compact(&hint_row);
        for pair in ["⏎打开", "Esc返回", "r刷新", "q退出"] {
            assert!(text.contains(pair), "global segment truncated: {text}");
        }
        assert!(text.contains('…'), "cut must be marked: {text}");
    }

    /// A view segment longer than the 80-column floor is cut with `…` while
    /// the global segment keeps every pair.
    #[test]
    fn long_view_segment_is_right_truncated_with_an_ellipsis() {
        let row = HintRow::new(hints([
            hint("j/k", "选择"),
            hint("s/u", "暂存切换"),
            hint("x", "丢弃"),
            hint("c", "commit"),
            hint("p/l", "push/pull"),
            hint("b/h/t", "面板"),
        ]));
        let (_, hint_row) = render_rows(StatusRow::plain(None), &row, 80);
        assert!(row_cells(&hint_row) <= 80, "row is too wide: {hint_row}");
        let text = compact(&hint_row);
        assert!(text.contains('…'), "cut must be marked: {text}");
        assert!(text.contains("j/k选择"), "head of the view segment stays");
        assert!(text.ends_with('…'), "the cut is at the right edge: {text}");
    }

    /// Key caps render bold accent, actions muted (§2.3).
    #[test]
    fn hint_row_styles_key_caps_and_actions_per_the_theme() {
        use ratatui::style::Modifier;

        use crate::ui::components::theme::color;

        let row = HintRow::new(hints([hint("j/k", "选择")]));
        let buffer = render_buffer(StatusRow::plain(None), &row, 80);
        let mut key_fg = None;
        let mut key_bold = false;
        let mut action_fg = None;
        for x in 0..buffer.area.width {
            let cell = &buffer[(x, 1)];
            match cell.symbol() {
                "j" if key_fg.is_none() => {
                    key_fg = Some(cell.fg);
                    key_bold = cell.modifier.contains(Modifier::BOLD);
                }
                "选" => action_fg = Some(cell.fg),
                _ => {}
            }
        }
        assert_eq!(key_fg, Some(color::ACCENT), "key caps take the accent");
        assert!(key_bold, "key caps are bold");
        assert_eq!(action_fg, Some(color::FG_MUTED), "actions read muted");
    }

    /// A failed load lands on the status row through the shared helpers —
    /// local git and GitHub receipts take the same path (§4.3).
    #[test]
    fn status_row_renders_errors_red_and_plain_status_unstyled() {
        let error = StatusLine {
            message: "加载失败: boom".to_string(),
            is_error: true,
        };
        let buffer = render_buffer(
            StatusRow::plain(Some(&error)),
            &HintRow::new(Line::from("")),
            80,
        );
        let text: String = (0..buffer.area.width)
            .map(|x| buffer[(x, 0)].symbol().to_string())
            .collect();
        assert!(compact(&text).contains("加载失败:boom"), "missing: {text}");
        assert_eq!(
            first_fg(&buffer, 0),
            Some(crate::ui::components::theme::color::DANGER),
            "errors render on the danger token"
        );

        let plain = StatusLine {
            message: "已切换到分支 dev".to_string(),
            is_error: false,
        };
        let buffer = render_buffer(
            StatusRow::plain(Some(&plain)),
            &HintRow::new(Line::from("")),
            80,
        );
        assert_eq!(
            first_fg(&buffer, 0),
            Some(ratatui::style::Color::Reset),
            "plain status keeps the default style"
        );
    }
}
