//! Scan-dir configuration panel: `scan_dirs` list with add/delete overlays.

use std::path::{Path, PathBuf};

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

use super::hint;
use super::repo::centered_rect;
use crate::git::show_path;
use crate::ui::components::footer::{hints as join_hints, HintRow};

/// Borrowed render state for the config panel, referencing `App.config.scan_dirs`
/// directly so no mock data source exists (checkpoint 2).
pub struct ConfigState<'a> {
    /// Live scan-dir list from `App.config.scan_dirs`.
    pub scan_dirs: &'a [PathBuf],
    /// Selection cursor shared across frames.
    pub list_state: &'a mut ListState,
    /// Whether the add-scan-dir input overlay is shown.
    pub show_input: bool,
    /// Directory path being typed in add mode (convention 9).
    pub input: &'a str,
    /// Whether the delete-scan-dir confirmation overlay is shown.
    pub show_delete_confirm: bool,
    /// Whether the interactive directory picker overlay is shown.
    pub show_picker: bool,
    /// Directory currently browsed by the picker (path line).
    pub picker_dir: &'a Path,
    /// Subdirectories listed by the picker (row 0 is "select current").
    pub picker_entries: &'a [PathBuf],
    /// Selection cursor for the picker list.
    pub picker_state: &'a mut ListState,
}

/// Render the scan-dir list plus the add-scan-dir overlays (directory picker
/// or manual input) when active.
///
/// The panel draws its content only (§4.1): the status row and the hint row
/// are both painted once by the footer — the hint row from [`config_hints`] —
/// so nothing here reserves a row for either.
pub fn render_config(f: &mut Frame, content: Rect, mut state: ConfigState<'_>) {
    render_dir_list(f, content, &mut state);

    if state.show_picker {
        render_dir_picker(
            f,
            content,
            state.picker_dir,
            state.picker_entries,
            state.picker_state,
        );
    }
    if state.show_input {
        // Single-line add-scan-dir input (convention 9): typed path plus
        // cursor, titled 添加扫描目录; Enter confirms, Esc cancels via the
        // footer's hint row.
        super::render_input_overlay(f, content, "添加扫描目录", state.input, true);
    }
    if state.show_delete_confirm {
        let dir = state
            .list_state
            .selected()
            .and_then(|i| state.scan_dirs.get(i))
            .map(|p| p.as_path());
        // Anchor on the list area, like the picker above, so the taller
        // wrapped dialog never covers the footer.
        render_delete_confirm(f, content, dir);
    }
}

/// Bottom key-hint row: picker keys while browsing, submit/cancel while
/// typing, y/n/Esc while the delete confirmation is open, list navigation
/// otherwise.
fn key_hints(picker_open: bool, input_open: bool, confirm_open: bool) -> Line<'static> {
    if confirm_open {
        // Shared row (G5): the y/n/Esc wording of the confirmation; every
        // other list key is swallowed while the dialog is open, so none of
        // them may be shown.
        super::confirm_hints()
    } else if picker_open {
        join_hints([
            hint("j/k", "移动"),
            hint("Enter", "选择"),
            hint("l/h", "进入/上级"),
            hint("i", "手动"),
            hint("Esc", "取消"),
        ])
    } else if input_open {
        // Matches the config manual input wording `Enter 确认` (G5).
        super::submit_hints("确认")
    } else {
        join_hints([
            hint("j/k", "选择"),
            hint("a", "添加"),
            hint("i", "手动"),
            hint("d", "删除"),
            hint("q", "退出"),
            hint("Esc", "返回"),
        ])
    }
}

/// Footer hint row for the config panel (§2.3): the panel's own verbs, with
/// the picker/input/confirm wording selected by the overlay state.
pub(crate) fn config_hints(
    show_picker: bool,
    show_input: bool,
    show_delete_confirm: bool,
) -> HintRow {
    HintRow::new(key_hints(show_picker, show_input, show_delete_confirm))
}

/// Interactive directory browser overlay: the current path on the first row,
/// then a virtual "select current directory" row plus each subdirectory.
/// Row 0 / highlighted subdirectory are what Enter submits; the list scrolls
/// with the cursor.
fn render_dir_picker(
    f: &mut Frame,
    area: Rect,
    dir: &Path,
    entries: &[PathBuf],
    list_state: &mut ListState,
) {
    // Fit the popup to small terminals instead of overflowing the frame.
    let height = area.height.saturating_sub(4).clamp(6, 16).min(area.height);
    let popup_area = centered_rect(70, height, area);
    f.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(format!("选择扫描目录 ({})", entries.len()))
        .borders(Borders::ALL);
    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(inner);

    // Path line, truncated by display width so long paths stay on one row
    // and the ellipsis fits within the frame.
    let path = ellipsize(&show_path(dir), inner.width as usize);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            path,
            Style::default().fg(Color::Yellow),
        ))),
        rows[0],
    );

    let mut items = vec![ListItem::new(Line::from(vec![
        Span::styled("↳ ", Style::default().fg(Color::Cyan)),
        Span::raw("选择当前目录"),
    ]))];
    items.extend(entries.iter().map(|entry| {
        let name = entry
            .file_name()
            .map(|name| show_path(Path::new(name)))
            .unwrap_or_else(|| show_path(entry));
        ListItem::new(Line::from(format!("{name}/")))
    }));

    let list = List::new(items)
        .highlight_style(super::list_highlight())
        .highlight_symbol(super::HIGHLIGHT_SYMBOL);
    f.render_stateful_widget(list, rows[1], list_state);
}

/// Shorten `text` to at most `max` display columns around a middle
/// ellipsis, so long directory paths remain readable on a single line.
/// Width is measured in terminal cells (CJK chars count as 2), matching how
/// ratatui lays out the rendered line.
fn ellipsize(text: &str, max: usize) -> String {
    let width = |s: &str| Span::raw(s.to_string()).width();
    if width(text) <= max {
        return text.to_string();
    }
    if max <= 1 {
        return "…".to_string();
    }
    let head_budget = (max - 1) / 2;

    let chars: Vec<char> = text.chars().collect();
    let mut head_end = 0;
    let mut head_width = 0;
    for (i, c) in chars.iter().enumerate() {
        let cw = width(&c.to_string());
        if head_width + cw > head_budget {
            break;
        }
        head_width += cw;
        head_end = i + 1;
    }
    // Never drop the head entirely: force one cell (or a 2-cell CJK char)
    // even when it exceeds the budget, and pay for it out of the tail so the
    // total still fits within `max` columns. When head + ellipsis cannot fit
    // at all (max == 2 with a 2-column first char), the head alone stands in
    // for the truncated form.
    if head_end == 0 && !chars.is_empty() {
        head_end = 1;
        head_width = width(&chars[0].to_string());
        if head_width + 1 > max {
            return chars[0].to_string();
        }
    }
    let tail_budget = (max - 1).saturating_sub(head_width);

    let mut tail_start = chars.len();
    let mut tail_width = 0;
    while tail_start > head_end {
        let c = chars[tail_start - 1];
        let cw = width(&c.to_string());
        if tail_width + cw > tail_budget {
            break;
        }
        tail_width += cw;
        tail_start -= 1;
    }

    let mut out: String = chars[..head_end].iter().collect();
    out.push('…');
    out.extend(chars[tail_start..].iter());
    out
}

/// Delete-confirm popup geometry: `60%` of the frame width, `7` rows so the
/// border (2 rows) plus up to three wrapped path lines, the blank spacer and
/// the `y/n` hint row (5 rows) all fit — 80 columns give 46 inner columns,
/// i.e. paths up to 138 characters stay complete.
const CONFIRM_PERCENT_X: u16 = 60;
const CONFIRM_HEIGHT: u16 = 7;

/// Delete confirmation overlay (discard-style, convention 8): y confirms,
/// n cancels; names the selected directory when one is selected. The path
/// wraps inside the popup like the repo discard dialog, so a long canonical
/// path is never clipped mid-name without a hint. `area` is the list region,
/// which keeps the status line and the y/n/Esc hints uncovered.
fn render_delete_confirm(f: &mut Frame, area: Rect, dir: Option<&Path>) {
    let mut body = vec![];
    if let Some(dir) = dir {
        body.push(Line::from(Span::styled(
            show_path(dir),
            Style::default().fg(Color::Yellow),
        )));
    }
    super::render_confirm_overlay(
        f,
        area,
        "移除该目录？",
        CONFIRM_PERCENT_X,
        CONFIRM_HEIGHT,
        body,
        true,
    );
}

/// Draw the bordered scan-dir list; the title carries the entry count.
fn render_dir_list(f: &mut Frame, area: Rect, state: &mut ConfigState<'_>) {
    let items: Vec<ListItem> = state
        .scan_dirs
        .iter()
        .map(|dir| ListItem::new(Line::raw(show_path(dir))))
        .collect();

    let block = Block::default()
        .title(format!("扫描目录配置 ({})", state.scan_dirs.len()))
        .borders(Borders::ALL);

    let list = List::new(items)
        .block(block)
        .highlight_style(super::list_highlight())
        .highlight_symbol(super::HIGHLIGHT_SYMBOL);

    f.render_stateful_widget(list, area, state.list_state);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::components::footer::StatusRow;
    use crate::ui::status::StatusLine;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// The panel draws content only (§2.3), so the frame goes through
    /// [`crate::ui::components::footer::draw_frame`] — the shape
    /// `ui::render` uses — and the hint row is the bottom row.
    fn render_to_text(state: ConfigState<'_>) -> String {
        compact(&render_to_terminal(state, 80, 12))
    }

    /// The panel draws content only (§2.3), so the frame goes through
    /// [`crate::ui::components::footer::draw_frame`] — the shape
    /// `ui::render` uses — and the footer takes the bottom two rows.
    fn render_to_terminal(
        state: ConfigState<'_>,
        width: u16,
        height: u16,
    ) -> Terminal<TestBackend> {
        let hints = config_hints(
            state.show_picker,
            state.show_input,
            state.show_delete_confirm,
        );
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                crate::ui::components::footer::draw_frame(
                    f,
                    StatusRow::plain(None),
                    &hints,
                    |f, body| render_config(f, body, state),
                );
            })
            .unwrap();
        terminal
    }

    /// Whitespace-stripped text of the whole buffer: wide CJK glyphs leave
    /// padding cells.
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

    /// Whitespace-stripped text of one row, `y` from the top.
    fn row_compact(buffer: &ratatui::buffer::Buffer, y: u16) -> String {
        let raw: String = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol().to_string())
            .collect();
        raw.chars().filter(|c| !c.is_whitespace()).collect()
    }

    /// View segment of the hint row: the bottom row without the global
    /// segment (§2.3), which is fixed on every screen and would otherwise
    /// answer for words like `Esc 返回` in the assertions below.
    fn view_segment(terminal: &Terminal<TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        let y = buffer.area.height - 1;
        let row = row_compact(buffer, y);
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

    /// Base render state: list mode, no overlays; picker fields inert.
    fn base_state<'a>(
        scan_dirs: &'a [PathBuf],
        list_state: &'a mut ListState,
        picker_state: &'a mut ListState,
    ) -> ConfigState<'a> {
        ConfigState {
            scan_dirs,
            list_state,
            show_input: false,
            input: "",
            show_delete_confirm: false,
            show_picker: false,
            picker_dir: Path::new(""),
            picker_entries: &[],
            picker_state,
        }
    }

    #[test]
    fn render_list_shows_title_dirs_and_hints() {
        let dirs = vec![
            PathBuf::from("/home/user/repos"),
            PathBuf::from("/opt/projects"),
        ];
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        let mut picker_state = ListState::default();
        let text = render_to_text(base_state(&dirs, &mut list_state, &mut picker_state));
        assert!(text.contains("扫描目录配置(2)"), "missing title: {text}");
        assert!(
            text.contains("/home/user/repos"),
            "missing first dir: {text}"
        );
        assert!(text.contains("/opt/projects"), "missing second dir: {text}");
        assert!(text.contains("j/k选择"), "missing nav hint: {text}");
        assert!(text.contains("a添加"), "missing add hint: {text}");
        assert!(text.contains("i手动"), "missing manual hint: {text}");
        assert!(text.contains("d删除"), "missing delete hint: {text}");
        assert!(text.contains("Esc返回"), "missing back hint: {text}");
    }

    #[test]
    fn render_empty_list_keeps_title_and_hints() {
        let mut list_state = ListState::default();
        let mut picker_state = ListState::default();
        let text = render_to_text(base_state(&[], &mut list_state, &mut picker_state));
        assert!(text.contains("扫描目录配置(0)"), "missing title: {text}");
        assert!(text.contains("Esc返回"), "missing back hint: {text}");
    }

    /// §4.1: the panel body draws no status row of its own — the footer is
    /// the one landing spot for an async result, so a message appears there
    /// and nowhere else, and the list keeps its own rows.
    #[test]
    fn render_error_status_reaches_the_footer_status_row_only() {
        let status = StatusLine {
            message: "目录不能为空".to_string(),
            is_error: true,
        };
        let mut list_state = ListState::default();
        let mut picker_state = ListState::default();
        let state = base_state(&[], &mut list_state, &mut picker_state);
        let hints = config_hints(false, false, false);
        let backend = TestBackend::new(80, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                crate::ui::components::footer::draw_frame(
                    f,
                    StatusRow::plain(Some(&status)),
                    &hints,
                    |f, body| render_config(f, body, state),
                );
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let rows: Vec<u16> = (0..buffer.area.height)
            .filter(|y| row_compact(buffer, *y).contains("目录不能为空"))
            .collect();
        assert_eq!(
            rows,
            vec![buffer.area.height - 2],
            "the error must land on the footer status row only"
        );
    }

    #[test]
    fn render_add_input_shows_title_typed_path_and_keys() {
        let dirs = vec![PathBuf::from("/a")];
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        let mut picker_state = ListState::default();
        let mut state = base_state(&dirs, &mut list_state, &mut picker_state);
        state.show_input = true;
        state.input = "/home/user";
        let text = render_to_text(state);
        assert!(text.contains("添加扫描目录"), "missing title: {text}");
        assert!(text.contains(">/home/user"), "missing input: {text}");
        assert!(text.contains("▌"), "missing cursor: {text}");
        assert!(text.contains("Enter确认"), "missing confirm hint: {text}");
        assert!(text.contains("Esc取消"), "missing cancel hint: {text}");
        // The list hints are replaced while typing.
        assert!(!text.contains("a添加"), "list hints leaked: {text}");
    }

    #[test]
    fn render_delete_confirm_shows_prompt_and_keys() {
        let dirs = vec![PathBuf::from("/home/user/repos")];
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        let mut picker_state = ListState::default();
        let mut state = base_state(&dirs, &mut list_state, &mut picker_state);
        state.show_delete_confirm = true;
        let terminal = render_to_terminal(state, 80, 12);
        let text = compact(&terminal);
        assert!(text.contains("移除该目录"), "missing prompt: {text}");
        assert!(
            text.contains("/home/user/repos"),
            "missing target dir: {text}"
        );
        // The view segment is replaced while the dialog is open: a/d/j/k and
        // the Esc-returns of the list row are swallowed by
        // `confirm_modal_key`. The global segment keeps its own `Esc 返回`
        // (§2.3), so the assertion reads the view segment alone.
        let hints = view_segment(&terminal);
        assert!(hints.contains("y确认"), "missing confirm key: {hints}");
        assert!(hints.contains("n取消"), "missing cancel key: {hints}");
        assert!(
            hints.contains("Esc取消对话框"),
            "missing dismiss hint on the bottom row: {hints}"
        );
        assert!(!hints.contains("a添加"), "list hints leaked: {hints}");
        assert!(!hints.contains("d删除"), "list hints leaked: {hints}");
        assert!(!hints.contains("j/k选择"), "list hints leaked: {hints}");
    }

    /// Whitespace-stripped interior of the delete-confirm popup (border
    /// excluded): the wrapped target path lands on consecutive inner rows,
    /// so joining them reassembles the path contiguously.
    fn confirm_inner(terminal: &Terminal<TestBackend>, size: (u16, u16)) -> String {
        // The popup is anchored inside the list area: the panel fills the
        // content area, so only the footer's two rows (§4.1) are left out.
        let list_area = Rect::new(0, 0, size.0, size.1.saturating_sub(2));
        let popup = centered_rect(CONFIRM_PERCENT_X, CONFIRM_HEIGHT, list_area);
        let buffer = terminal.backend().buffer();
        let mut text = String::new();
        for y in (popup.y + 1)..popup.bottom().saturating_sub(1) {
            for x in (popup.x + 1)..popup.right().saturating_sub(1) {
                text.push_str(buffer[(x, y)].symbol());
            }
        }
        text.chars().filter(|c| !c.is_whitespace()).collect()
    }

    /// G1 regression: a path wider than the popup (46 inner columns at 80,
    /// 70 at 120) must wrap onto extra rows instead of being clipped, while
    /// the prompt and the y/n/Esc wording stay where they were.
    #[test]
    fn render_delete_confirm_wraps_long_path_inside_popup() {
        let long =
            "/tmp/very/long/path/for/delete/confirm/wrapping/2026/client-projects/frontend-build-cache/node_modules";
        assert!(long.len() > 60, "fixture must be longer than one line");
        let dirs = vec![PathBuf::from(long)];

        for size in [(80u16, 12u16), (120u16, 24u16)] {
            let mut list_state = ListState::default();
            list_state.select(Some(0));
            let mut picker_state = ListState::default();
            let mut state = base_state(&dirs, &mut list_state, &mut picker_state);
            state.show_delete_confirm = true;

            let terminal = render_to_terminal(state, size.0, size.1);

            let inner = confirm_inner(&terminal, size);
            assert!(
                inner.contains(long),
                "path clipped in {}x{} popup: {inner}",
                size.0,
                size.1
            );
            assert!(inner.contains("y确认"), "missing confirm key: {inner}");
            assert!(inner.contains("n取消"), "missing cancel key: {inner}");

            let full = compact(&terminal);
            assert!(full.contains("移除该目录"), "missing prompt: {full}");
            let hints = view_segment(&terminal);
            assert!(
                hints.contains("Esc取消对话框"),
                "missing dismiss hint: {hints}"
            );
        }
    }

    #[test]
    fn render_dir_picker_shows_path_rows_and_hints() {
        let dirs = vec![PathBuf::from("/a")];
        let mut list_state = ListState::default();
        let mut picker_state = ListState::default();
        picker_state.select(Some(1));
        let entries = vec![
            PathBuf::from("/home/user/code"),
            PathBuf::from("/home/user/docs"),
        ];
        let mut state = base_state(&dirs, &mut list_state, &mut picker_state);
        state.show_picker = true;
        state.picker_dir = Path::new("/home/user");
        state.picker_entries = &entries;
        let terminal = render_to_terminal(state, 120, 12);
        let text = compact(&terminal);
        assert!(text.contains("选择扫描目录(2)"), "missing title: {text}");
        assert!(text.contains("↳选择当前目录"), "missing select row: {text}");
        assert!(text.contains("code/"), "missing subdir row: {text}");
        assert!(text.contains("docs/"), "missing subdir row: {text}");
        assert!(text.contains("►"), "missing highlight: {text}");
        assert!(text.contains("/home/user"), "missing path line: {text}");
        // The picker segment is read on the hint row itself: at 80 columns it
        // is right-truncated (§2.3), so the assertions use a frame wide
        // enough to show it whole.
        let hints = view_segment(&terminal);
        assert!(hints.contains("j/k移动"), "missing move hint: {hints}");
        assert!(hints.contains("Enter选择"), "missing select hint: {hints}");
        assert!(hints.contains("l/h进入/上级"), "missing nav hint: {hints}");
        assert!(hints.contains("i手动"), "missing manual hint: {hints}");
        assert!(hints.contains("Esc取消"), "missing cancel hint: {hints}");
        // The list hints are replaced while browsing.
        assert!(!hints.contains("j/k选择"), "list hints leaked: {hints}");
        assert!(!hints.contains("d删除"), "list hints leaked: {hints}");
    }

    #[test]
    fn render_dir_picker_truncates_long_paths() {
        let long = format!("/{}", vec!["segment"; 40].join("/"));
        assert!(long.chars().count() > 40, "fixture must be long");
        let dirs = vec![PathBuf::from("/a")];
        let mut list_state = ListState::default();
        let mut picker_state = ListState::default();
        picker_state.select(Some(0));
        let mut state = base_state(&dirs, &mut list_state, &mut picker_state);
        state.show_picker = true;
        state.picker_dir = Path::new(&long);
        let text = render_to_text(state);
        assert!(text.contains('…'), "missing ellipsis: {text}");
        assert!(!text.contains(&long), "full path must be truncated: {text}");
    }

    #[test]
    fn ellipsize_keeps_short_text_and_heads_tails_long_text() {
        assert_eq!(ellipsize("abc", 10), "abc");
        let long = "a".repeat(20);
        let out = ellipsize(&long, 9);
        assert_eq!(out.chars().count(), 9, "must fit: {out}");
        assert!(out.contains('…'), "missing ellipsis: {out}");
        assert_eq!(ellipsize("xy", 1), "…");

        // CJK chars occupy 2 columns each: measure in display width, not chars.
        let cjk = format!("/{}", ["中文目录"; 10].join("/"));
        let out = ellipsize(&cjk, 12);
        assert!(out.contains('…'), "missing ellipsis: {out}");
        assert!(
            Span::raw(out.clone()).width() <= 12,
            "display width must fit 12 cells: {out}"
        );

        // Forced-head fallback path (head does not fit its budget): the
        // result must still fit within `max` display columns.
        for (text, max) in [("abcd", 2), ("中ab", 3), ("中文目录", 4), ("a中b", 5)] {
            let out = ellipsize(text, max);
            assert!(
                Span::raw(out.clone()).width() <= max,
                "fallback must fit {max} cells: {out}"
            );
        }
        assert_eq!(
            ellipsize("中ab", 2),
            "中",
            "head alone when no ellipsis fits"
        );
    }
}
