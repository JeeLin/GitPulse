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
    text::Line,
    widgets::Paragraph,
    Frame,
};

use super::{cells, theme::spacing};

/// One title row reserved at the top of every screen (§6.1/§6.2 title rows).
pub const TITLE_ROWS: u16 = spacing::TITLE_ROWS;

/// Product title that leads every breadcrumb (§6.1).
pub const PRODUCT: &str = "GitPulse";

/// Split `area` into (title bar, content, footer): one row at the top, the two
/// footer rows at the bottom (§4.1), everything between them is content (§6).
pub fn split_area(area: Rect) -> (Rect, Rect, Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(TITLE_ROWS),
            Constraint::Min(1),
            Constraint::Length(spacing::FOOTER_ROWS),
        ])
        .split(area);
    (chunks[0], chunks[1], chunks[2])
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
                let (title, _content, _footer) = split_area(f.area());
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

    /// §6.1: the title bar occupies the first row, the content sits below it and
    /// the footer keeps the bottom two rows (§4.1) — the header must not eat
    /// either end of the frame.
    #[test]
    fn split_area_reserves_one_row_on_top_and_two_on_the_bottom() {
        let area = Rect::new(0, 0, 40, 12);
        let (title, content, footer) = split_area(area);
        assert_eq!(title, Rect::new(0, 0, 40, TITLE_ROWS));
        assert_eq!(footer, Rect::new(0, 10, 40, spacing::FOOTER_ROWS));
        assert_eq!(content.height, 9, "rows between header and footer");
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
