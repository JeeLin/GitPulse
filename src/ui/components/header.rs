//! Screen header (§6): the one title bar that floats above every panel, carrying
//! the product title, the breadcrumb and the right-aligned status.
//!
//! §4.1 pins the footer to the bottom two rows; the header does the mirror image
//! at the top — one row, [`TITLE_ROWS`] — so every screen gets exactly one
//! canonical title bar instead of rediscovering it inside its own panel block.
//! The breadcrumb reinforces "where am I" (§1.2) and the right cell carries the
//! repo count / remote badge / op status a screen needs at a glance (§6.1).

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::{cells, theme, theme::spacing};

/// One title row reserved at the top of every screen (§6.1/§6.2 title rows).
pub const TITLE_ROWS: u16 = spacing::TITLE_ROWS;

/// One context-tab row reserved directly under the title row on the screens
/// that have sibling contexts (§5.2 vertical structure: 标题行 1 + 上下文标签行 1
/// + 内容区 + 状态行 1 + 提示行 1).
pub const CONTEXT_TAG_ROWS: u16 = spacing::TAB_ROWS;

/// Product title that leads every breadcrumb (§6.1).
pub const PRODUCT: &str = "GitPulse";

/// Separator between two sibling context tags (§6.2 draws the strip as
/// `工作区 │ 分支 │ 历史 │ 标签`).
const TAG_SEPARATOR: &str = " │ ";

/// Split `area` into (title bar, context tags, content, footer) (§5.2).
///
/// The title row is always reserved at the top and the two-line footer always
/// at the bottom (§4.1); everything between them is content (§6). When
/// `has_context_tags` is `true` the repo-workspace context-tag row (§6.2) is
/// reserved directly under the title and `content` loses that one row; when it
/// is `false` the row is absent (`None`) and `content` absorbs it, so every
/// screen without context tabs keeps the exact geometry it had before.
pub fn split_area(area: Rect, has_context_tags: bool) -> (Rect, Option<Rect>, Rect, Rect) {
    let chunks = if has_context_tags {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(TITLE_ROWS),
                Constraint::Length(CONTEXT_TAG_ROWS),
                Constraint::Min(1),
                Constraint::Length(spacing::FOOTER_ROWS),
            ])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(TITLE_ROWS),
                Constraint::Min(1),
                Constraint::Length(spacing::FOOTER_ROWS),
            ])
            .split(area)
    };
    if has_context_tags {
        (chunks[0], Some(chunks[1]), chunks[2], chunks[3])
    } else {
        (chunks[0], None, chunks[1], chunks[2])
    }
}

/// Draw the single canonical title bar (§6.1): `breadcrumb` on the left, the
/// right-flush `right` status beside it. The two never overlap — the right cell
/// is measured by display width first and the left keeps whatever cells are
/// left (§5.4).
pub fn render_title_bar(f: &mut Frame, area: Rect, breadcrumb: &str, right: &str) {
    let left = Line::from(format!("{PRODUCT}  {breadcrumb}"));
    if right.is_empty() {
        f.render_widget(Paragraph::new(left), area);
        return;
    }
    // Two cells of breathing room separate the breadcrumb from the status so a
    // long trail never touches the number beside it.
    let right_w = cells::width(right).min(area.width.saturating_sub(2) as usize) as u16;
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(1), Constraint::Length(right_w)])
        .split(area);
    f.render_widget(Paragraph::new(left), chunks[0]);
    f.render_widget(Paragraph::new(right).alignment(Alignment::Right), chunks[1]);
}

/// Render the repo-workspace context-tag strip (§5.2/§6.2): a single row of
/// sibling tags joined by ` │ `, the active tag styled as `role::key` (bold
/// accent) and the rest as `role::caption` (muted foreground).
///
/// The strip is width-budgeted with [`cells::fit_line`] so it never wraps (§5.4:
/// single line, never wraps): if it would overflow `area`, the tail is cut and
/// a `…` is appended in the style of the last kept span instead. Colors come
/// only from [`theme`] roles — no raw color values (§5 token rule).
pub fn render_context_tags(f: &mut Frame, area: Rect, tags: &[&str], active: usize) {
    if tags.is_empty() {
        return;
    }
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(tags.len() * 3);
    for (i, tag) in tags.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(TAG_SEPARATOR));
        }
        let style = if i == active {
            theme::role::key()
        } else {
            theme::role::caption()
        };
        spans.push(Span::styled((*tag).to_string(), style));
    }
    let line = cells::fit_line(Line::from(spans), area.width as usize);
    f.render_widget(Paragraph::new(line), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn buffer(width: u16, height: u16) -> ratatui::buffer::Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let (title, _tags, _content, _footer) = split_area(f.area(), false);
                render_title_bar(f, title, "仪表盘", "仓库 3");
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    /// Read row 0, collapsing the double-cell padding CJK glyphs leave behind
    /// in the test buffer (ratatui pads a wide glyph into the next cell).
    fn row0(buf: &ratatui::buffer::Buffer) -> String {
        let mut raw: String = (0..buf.area.width).map(|x| buf[(x, 0)].symbol()).collect();
        raw.retain(|c| !c.is_whitespace());
        raw
    }

    /// §6.1/§5.2: the title bar occupies the first row, the footer keeps the
    /// bottom two rows (§4.1) — the header must not eat either end of the frame.
    /// Without context tags the geometry is unchanged from the old three-region
    /// layout; with them the tag row sits directly under the title and content
    /// gives up one row.
    #[test]
    fn split_area_reserves_one_row_on_top_and_two_on_the_bottom() {
        let area = Rect::new(0, 0, 40, 12);

        // No context tags: the tag row is absent and content keeps its height.
        let (title, tags, content, footer) = split_area(area, false);
        assert_eq!(title, Rect::new(0, 0, 40, TITLE_ROWS));
        assert!(tags.is_none(), "no tag row when has_context_tags is false");
        assert_eq!(footer, Rect::new(0, 10, 40, spacing::FOOTER_ROWS));
        assert_eq!(content.height, 9, "rows between header and footer");

        // With context tags: one row under the title, content shrinks by one.
        let (title, tags, content, footer) = split_area(area, true);
        assert_eq!(title, Rect::new(0, 0, 40, TITLE_ROWS));
        let tags = tags.expect("tag row present when has_context_tags is true");
        assert_eq!(tags, Rect::new(0, TITLE_ROWS, 40, CONTEXT_TAG_ROWS));
        assert_eq!(tags.height, 1, "the tag row is exactly one row tall");
        assert_eq!(footer, Rect::new(0, 10, 40, spacing::FOOTER_ROWS));
        assert_eq!(content.height, 8, "content loses one row to the tag row");
    }

    /// §6.2/§5.4: the context-tag strip is one line, the four labels joined by
    /// `│`, the active tag bold+accent and the rest muted, and it never overflows
    /// its row on an 80-column frame.
    #[test]
    fn context_tag_strip_renders_labels_styles_and_budgets() {
        use ratatui::style::Modifier;

        use crate::ui::components::theme::color;

        let tags: &[&str] = &["工作区", "分支", "历史", "标签"];
        let backend = TestBackend::new(80, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_context_tags(f, f.area(), tags, 0))
            .unwrap();
        let buf = terminal.backend().buffer().clone();

        // All four labels present, joined by the box-light separator.
        let row = row0(&buf);
        assert!(row.contains("│"), "separator `│` missing: {row}");
        for label in tags {
            assert!(row.contains(label), "missing label {label}: {row}");
        }
        assert!(
            crate::ui::components::cells::width(&row0(&buf)) <= 80,
            "strip must not overflow an 80-column frame: {row}"
        );

        // Active tag (index 0): first glyph `工` is bold + accent.
        let mut active = None;
        // Inactive tag (index 1): first glyph `分` is muted, not bold.
        let mut inactive = None;
        for x in 0..buf.area.width {
            let cell = &buf[(x, 0)];
            let sym = cell.symbol();
            if sym == "工" {
                active = Some((cell.fg, cell.modifier.contains(Modifier::BOLD)));
            }
            if sym == "分" {
                inactive = Some((cell.fg, cell.modifier.contains(Modifier::BOLD)));
            }
        }
        let (afg, abold) = active.expect("active tag `工` must render");
        assert_eq!(afg, color::ACCENT, "active tag fg must be accent");
        assert!(abold, "active tag must be bold");
        let (ifg, ibold) = inactive.expect("inactive tag `分` must render");
        assert_eq!(ifg, color::FG_MUTED, "inactive tag fg must be muted");
        assert!(!ibold, "inactive tag must not be bold");
    }

    #[test]
    fn title_bar_leads_with_product_and_breadcrumb() {
        let row = row0(&buffer(40, 12));
        assert!(row.contains(PRODUCT), "product title missing: {row}");
        assert!(row.contains("仪表盘"), "breadcrumb missing: {row}");
    }

    /// §6.1: the right-flush status hugs the far edge without colliding with the
    /// breadcrumb.
    #[test]
    fn title_bar_right_flushes_the_status_without_overlap() {
        let row = row0(&buffer(40, 12));
        assert!(row.contains("仓库3"), "right status missing: {row}");
        assert!(
            row.ends_with("仓库3"),
            "status must sit at the right edge: {row:?}"
        );
    }
}
