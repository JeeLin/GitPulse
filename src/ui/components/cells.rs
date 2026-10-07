//! Display-width budget (§5.4): every row is measured in terminal cells —
//! a CJK glyph costs 2, an ASCII character 1 — and cut with `…` instead of
//! wrapping or silently clipping.
//!
//! The rules live in one place so list rows, hint rows and titles all agree
//! on what "fits" means. Every panel measures through these helpers; a view
//! that grows its own width arithmetic re-opens the drift this module exists
//! to close.

use ratatui::text::{Line, Span};

/// Display width of `text` in terminal cells (a CJK glyph costs 2, ASCII 1).
pub fn width(text: &str) -> usize {
    Line::from(text).width()
}

/// Longest prefix of `text` that fits in `budget` display cells, as
/// `(prefix, cells used, chars taken)`. A grapheme that would straddle the
/// budget ends the walk, so the prefix is never half a glyph.
fn take_head(text: &str, budget: usize) -> (String, usize, usize) {
    let mut out = String::new();
    let mut used = 0usize;
    let mut taken = 0usize;
    for c in text.chars() {
        let cw = width(&c.to_string());
        if used + cw > budget {
            break;
        }
        used += cw;
        taken += 1;
        out.push(c);
    }
    (out, used, taken)
}

/// Longest suffix of `text` that fits in `budget` display cells without
/// reaching back before char index `from`, as `(suffix, cells used)`. Walks
/// the tail end for [`ellipsize_middle`], so it stops on the same
/// whole-grapheme rule as [`take_head`].
fn take_tail(text: &str, from: usize, budget: usize) -> (String, usize) {
    let chars: Vec<char> = text.chars().collect();
    let mut start = chars.len();
    let mut used = 0usize;
    while start > from {
        let cw = width(&chars[start - 1].to_string());
        if used + cw > budget {
            break;
        }
        used += cw;
        start -= 1;
    }
    (chars[start..].iter().collect(), used)
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
    let (mut out, _, _) = take_head(text, max - 1); // one cell for the ellipsis
    out.push('…');
    out
}

/// Cut `text` to `max` display cells around a *middle* `…`, keeping a head and
/// a tail instead of only a head.
///
/// A path is the one case where dropping either end costs the reader more
/// than the cells do: `/home/dev/…/.config/nvim` has to keep the root prefix
/// that says *where* it lives and the entry name that says *what* is being
/// picked. Everything else — titles, branch names, PR subjects — reads left to
/// right, so [`ellipsize`] and its tail mark stay the right tool.
///
/// §5.4 still holds end to end: the budget is in display cells, the result
/// never exceeds `max`, and no glyph is cut in half.
pub fn ellipsize_middle(text: &str, max: usize) -> String {
    if width(text) <= max {
        return text.to_string();
    }
    if max == 0 {
        return String::new();
    }
    if max == 1 {
        return "…".to_string();
    }
    let (head, head_width, head_len) = take_head(text, (max - 1) / 2);
    // Never drop the head entirely: keep the first cell (or a 2-cell CJK
    // glyph) even when it overruns its share, and pay for it out of the tail
    // so the total still fits `max`. When head + `…` cannot fit at all
    // (`max == 2` with a 2-cell first glyph) the head alone stands in for the
    // truncated form.
    let (mut head, head_width, head_len) = if head.is_empty() {
        let Some(first) = text.chars().next() else {
            return String::new();
        };
        let first_width = width(&first.to_string());
        if first_width + 1 > max {
            return first.to_string();
        }
        (first.to_string(), first_width, 1)
    } else {
        (head, head_width, head_len)
    };
    let (tail, _) = take_tail(text, head_len, max - 1 - head_width);
    head.push('…');
    head.push_str(&tail);
    head
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

    /// §5.4 budgets emoji at two cells like CJK. Measured against
    /// `unicode-width` 0.2 (what ratatui lays out with): the pictographs
    /// below are 2, so a row that counts them as one drifts one cell per
    /// glyph. `●` / `★` / `⚠` / `▌` measure 1 — geometric and dingbat marks
    /// stay narrow, which is why the rule is about the glyph, not the
    /// category name.
    #[test]
    fn design_width_counts_wide_emoji_as_two_cells() {
        for glyph in ["✅", "🚀", "😀", "⚡"] {
            assert_eq!(width(glyph), 2, "{glyph} occupies two cells");
            assert_eq!(
                width(&format!("a{glyph}")),
                3,
                "{glyph} adds two cells next to ASCII"
            );
        }
        assert_eq!(
            width("✅🚀"),
            4,
            "two pictographs in one string are not three cells"
        );
        assert_eq!(width("●"), 1, "narrow geometric marks stay one cell");
    }

    /// §5.4: the tail mark marks a cut, and a two-cell emoji must never be
    /// rendered half a glyph — the walk stops before it instead.
    #[test]
    fn design_ellipsize_marks_the_cut_and_never_splits_an_emoji() {
        for glyph in ["✅", "🚀"] {
            let out = ellipsize(&format!("{glyph}build"), 3);
            assert!(
                width(&out) <= 3,
                "{glyph}: must fit the budget: {out:?} ({})",
                width(&out)
            );
            assert!(out.ends_with('…'), "cut must be marked: {out:?}");
            assert!(
                out == format!("{glyph}…"),
                "one emoji fits under the mark, the second is dropped: {out:?}"
            );

            let out = ellipsize(&format!("{glyph}build"), 2);
            assert_eq!(out, "…", "no room for the glyph, just the mark");
            assert_eq!(width(&out), 1, "the mark itself is one cell");
        }
    }

    /// §5.4 budgets in display cells for every input width, not just the
    /// ASCII and CJK rows the helpers were written for.
    #[test]
    fn design_ellipsize_never_exceeds_the_budget_for_emoji_input() {
        for max in 0..=8usize {
            for text in ["✅🚀😀", "a✅b🚀c😀d", "✅✅✅✅✅✅✅✅"] {
                let out = ellipsize(text, max);
                assert!(
                    width(&out) <= max,
                    "{text:?} at max={max} overflowed: {out:?} ({})",
                    width(&out)
                );
            }
        }
    }

    /// The middle-eliding sibling keeps both ends of a path: the root prefix
    /// that says where it lives and the entry name that says what it is.
    #[test]
    fn design_ellipsize_middle_keeps_the_path_head_and_tail() {
        assert_eq!(ellipsize_middle("abc", 10), "abc", "fits, stays verbatim");
        assert_eq!(
            ellipsize_middle("/home/dev/project/config", 13),
            "/home/…config",
            "root prefix and entry name both survive"
        );
        // Degenerate budgets are guarded, not left to the walker.
        assert_eq!(ellipsize_middle("abcdef", 0), "", "no budget, no text");
        assert_eq!(
            ellipsize_middle("abcdef", 1),
            "…",
            "one cell holds the mark"
        );
        assert_eq!(
            ellipsize_middle("中ab", 2),
            "中",
            "a two-cell head leaves no room for the mark"
        );
        // Every budget still fits, including the wide-glyph paths.
        for max in 0..=20usize {
            for text in ["/中文/目录/名字", "/a/b/c/d/e/f", "中"] {
                let out = ellipsize_middle(text, max);
                assert!(
                    width(&out) <= max,
                    "{text:?} at max={max} overflowed: {out:?} ({})",
                    width(&out)
                );
            }
        }
        // A wide glyph is dropped whole, never rendered as half a character. The head
        // takes only `/` here (the next glyph needs 2 cells and overruns the
        // head's share), so the tail still carries the last whole `中`.
        let out = ellipsize_middle("/中中中中", 5);
        assert_eq!(out, "/…中", "head and tail each keep a whole glyph");
        assert_eq!(width(&out), 4, "one cell of head, one of mark, two of tail");
    }
}
