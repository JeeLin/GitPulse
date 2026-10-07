//! Display-width budget (§5.4): every row is measured in terminal cells —
//! a CJK glyph costs 2, an ASCII character 1 — and cut with `…` instead of
//! wrapping or silently clipping.
//!
//! The rules live in one place so list rows, hint rows and titles all agree
//! on what "fits" means; `pulls::cells` / `pulls::ellipsize` re-export these.

#![allow(dead_code)]

use ratatui::text::{Line, Span};

/// Display width of `text` in terminal cells (a CJK glyph costs 2, ASCII 1).
pub fn width(text: &str) -> usize {
    Line::from(text).width()
}

/// Pad `text` with trailing spaces to exactly `width` display cells; text
/// that already fills (or overflows) the budget is returned unchanged so a
/// fixed-width column can never grow past its slot.
pub fn pad_cells(text: &str, width: usize) -> String {
    let used = self::width(text);
    if used >= width {
        text.to_string()
    } else {
        format!("{text}{}", " ".repeat(width - used))
    }
}

/// Cut `text` to `max` display cells, marking the cut with a trailing `…`
/// (one cell) so a truncated row reads as truncated instead of silently
/// ending mid-word.
pub fn ellipsize(text: &str, max: usize) -> String {
    if width(text) <= max {
        return text.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let budget = max - 1; // one cell for the ellipsis itself
    let mut out = String::new();
    let mut used = 0usize;
    for c in text.chars() {
        let cw = width(&c.to_string());
        if used + cw > budget {
            break;
        }
        used += cw;
        out.push(c);
    }
    out.push('…');
    out
}

/// Cut a styled [`Line`] to `max` display cells, appending `…` (in the
/// style of the last kept span) when anything was cut.
///
/// Unlike [`ellipsize`] this keeps the per-span styles, so a hint row
/// truncated at 80 columns still renders its key caps bold-accented up to the
/// cut. A grapheme wider than the remaining budget ends the walk — the cut
/// never lands mid-glyph.
pub fn fit_line(line: Line<'static>, max: usize) -> Line<'static> {
    if line.width() <= max {
        return line;
    }
    if max == 0 {
        return Line::default();
    }
    let budget = max - 1; // one cell for the ellipsis itself
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;
    let mut cut_style = ratatui::style::Style::default();
    for grapheme in line.styled_graphemes(ratatui::style::Style::default()) {
        let gw = width(grapheme.symbol);
        if used + gw > budget {
            cut_style = grapheme.style;
            break;
        }
        used += gw;
        match spans.last_mut() {
            Some(last) if last.style == grapheme.style => {
                last.content.to_mut().push_str(grapheme.symbol);
            }
            _ => spans.push(Span::styled(grapheme.symbol.to_string(), grapheme.style)),
        }
    }
    spans.push(Span::styled("…", cut_style));
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::components::theme::color;
    use ratatui::style::Style;

    #[test]
    fn width_counts_cjk_glyphs_as_two_cells() {
        assert_eq!(width("abc"), 3, "ASCII is one cell per char");
        assert_eq!(width("分支"), 4, "CJK is two cells per char");
        assert_eq!(width("分支名"), 6, "three wide glyphs in one string");
        assert_eq!(width(""), 0);
    }

    #[test]
    fn pad_cells_fills_short_text_and_never_grows_past_the_budget() {
        assert_eq!(pad_cells("ab", 5), "ab   ");
        assert_eq!(pad_cells("分支", 6), "分支  ", "wide glyphs count two");
        assert_eq!(pad_cells("abc", 3), "abc", "exact fit stays verbatim");
        assert_eq!(pad_cells("abcd", 3), "abcd", "overflow is not cut here");
    }

    #[test]
    fn ellipsize_cuts_by_display_width_not_char_count() {
        assert_eq!(ellipsize("hello", 10), "hello", "fits, stays verbatim");
        assert_eq!(ellipsize("hello", 5), "hello", "exact fit stays verbatim");
        assert_eq!(ellipsize("hello", 4), "hel…", "tail is cut, not the head");
        assert_eq!(
            ellipsize("中文标题", 5),
            "中文…",
            "wide glyphs count two, so only two fit under the mark"
        );
        assert_eq!(ellipsize("中文标题", 4), "中…", "one wide glyph + the mark");
        assert_eq!(ellipsize("hello", 0), "", "no budget, no text");
        assert_eq!(ellipsize("hello", 1), "…", "one cell only holds the mark");
    }

    #[test]
    fn fit_line_keeps_styles_and_fits_the_budget() {
        let line = Line::from(vec![
            Span::styled("Esc", Style::default().fg(color::ACCENT)),
            Span::raw(" 返回"),
        ]);
        assert_eq!(fit_line(line.clone(), 10).width(), line.width(), "fits");

        let cut = fit_line(line, 5);
        assert_eq!(cut.width(), 5, "must fit the budget: {cut:?}");
        let text: String = cut.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(text.ends_with('…'), "cut must be marked: {text}");
        assert_eq!(
            cut.spans[0].style.fg,
            Some(color::ACCENT),
            "kept spans keep their style"
        );
    }

    /// A two-cell glyph that would straddle the cut ends the walk instead of
    /// rendering half a character.
    #[test]
    fn fit_line_never_splits_a_wide_glyph() {
        let line = Line::from("分支名分支");
        let cut = fit_line(line, 4);
        assert!(cut.width() <= 4, "must fit: {:?}", cut.width());
        let text: String = cut.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(text.ends_with('…'), "cut must be marked: {text}");
        assert!(
            !text.contains('名') || text.starts_with("分"),
            "glyph cut cleanly: {text}"
        );
    }

    #[test]
    fn fit_line_drops_everything_on_a_zero_budget() {
        let cut = fit_line(Line::from("abc"), 0);
        assert_eq!(cut.width(), 0);
    }
}
