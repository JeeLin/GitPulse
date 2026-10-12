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

/// Outcome of [`fit_name_and_tail`]: the `name` the row may draw, and the
/// droppable tail it may draw — or `None` when the row cannot fund it.
///
/// §5.4's rule is the same wherever a row has an identity plus optional
/// metadata: the high-priority `name` is always shown, the low-priority tail
/// disappears **whole** when the row cannot pay for it, and the name absorbs
/// what is left by being ellipsized. The tail is never partially truncated —
/// a half-drawn `→ upstream` or `→ ab34cd` reads as a different value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameAndTail {
    /// The name, ellipsized to whatever the row can fund.
    pub name: String,
    /// The tail, or `None` when the row could not fund it.
    pub tail: Option<String>,
}

/// Split `name` and its optional `tail` across `width` display cells under
/// §5.4's "low priority disappears before the identity shrinks" rule
/// (B4-10).
///
/// `fixed` is the width of the row's **non-droppable** furniture — the branch
/// `* ` marker, the tag kind suffix — which is paid for before the tail is
/// even considered; it is not part of `name`'s budget.
///
/// The tail is kept only when `fixed + name + tail` fits `width` exactly;
/// otherwise it drops whole and the name is ellipsized into
/// `width - fixed`. An absent tail is simply an absent tail, never a
/// zero-width one.
///
/// Callers build their own spans around the result — the row's identity style
/// and the tail's colour are panel-specific, and folding them in here would
/// make this helper a second place to change for one panel's sake.
pub fn fit_name_and_tail(
    name: &str,
    tail: Option<&str>,
    fixed: usize,
    width: usize,
) -> NameAndTail {
    let tail = tail.filter(|tail| fixed + self::width(name) + self::width(tail) <= width);
    let name = match tail {
        Some(_) => name.to_string(),
        None => ellipsize(name, width.saturating_sub(fixed)),
    };
    NameAndTail {
        name,
        tail: tail.map(str::to_string),
    }
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

/// Whitespace-stripped text of a whole rendered frame.
///
/// A wide (CJK) glyph occupies two buffer cells: the glyph plus a reset
/// *padding* cell whose symbol is `" "`, so the raw concatenation reads
/// `" 选择 "` rather than `" 选择"`. Stripping every whitespace removes the
/// padding and the layout gaps alike, leaving the text the frame actually
/// says — which is what a view test asserts on.
///
/// One implementation for every view: a screen test that re-derived this would
/// drift the moment the buffer layout changes.
#[cfg(test)]
pub(crate) fn compact(buffer: &ratatui::buffer::Buffer) -> String {
    let mut raw = String::new();
    for cell in buffer.content() {
        raw.push_str(cell.symbol());
    }
    raw.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Row indices of a rendered frame whose whitespace-stripped text contains
/// `needle` — how a view test proves one message landed on the footer status
/// row (`height - 2`) and nowhere else (§4.1), or that a panel title is still
/// on screen at all.
#[cfg(test)]
pub(crate) fn rows_with(buffer: &ratatui::buffer::Buffer, needle: &str) -> Vec<u16> {
    (0..buffer.area.height)
        .filter(|y| {
            let row: String = (0..buffer.area.width)
                .map(|x| buffer[(x, *y)].symbol().to_string())
                .collect();
            let row: String = row.chars().filter(|c| !c.is_whitespace()).collect();
            row.contains(needle)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::components::theme::color;
    use ratatui::style::Style;

    /// B4-10: the budget's upper edge — a width that exactly funds
    /// `fixed + name + tail` keeps the tail. One cell less drops it whole.
    #[test]
    fn fit_name_and_tail_keeps_the_tail_exactly_at_the_boundary() {
        let name = "main";
        let tail = "  → origin/main";
        let fixed = 2; // the branch marker's two cells
        let exact = fixed + width(name) + width(tail);

        let kept = fit_name_and_tail(name, Some(tail), fixed, exact);
        assert_eq!(
            kept,
            NameAndTail {
                name: name.to_string(),
                tail: Some(tail.to_string()),
            },
            "a width that exactly funds the tail must keep it"
        );

        let dropped = fit_name_and_tail(name, Some(tail), fixed, exact - 1);
        assert_eq!(
            dropped.tail, None,
            "one cell short, the tail must drop whole rather than truncate"
        );
        assert_eq!(
            dropped.name, name,
            "the name still fits without the tail, so it is not ellipsized"
        );
    }

    /// B4-10: when the tail drops, the name is ellipsized against
    /// `width - fixed` — the fixed furniture is paid for before the name's
    /// budget, so a wide marker must shorten the name, not be paid out of it.
    #[test]
    fn fit_name_and_tail_ellipsizes_the_name_around_the_fixed_cost() {
        let name = "a-very-long-branch-name";

        let narrow_marker = fit_name_and_tail(name, None, 2, 10);
        let wide_marker = fit_name_and_tail(name, None, 8, 10);

        assert_eq!(
            width(&wide_marker.name),
            2,
            "the wider fixed cost must leave the name only 2 cells"
        );
        assert_eq!(width(&narrow_marker.name), 8);
        assert!(
            narrow_marker.name.ends_with('…') && wide_marker.name.ends_with('…'),
            "a cut name is marked with …, never silently clipped: {narrow_marker:?}"
        );
        assert_eq!(wide_marker.tail, None, "no tail was offered");
    }

    /// B4-10: the fixed cost alone can exceed the row, and the helper must not
    /// wrap — `saturating_sub` yields 0 and the name disappears rather than
    /// underflowing into a huge budget.
    #[test]
    fn fit_name_and_tail_saturates_when_the_fixed_cost_exceeds_the_row() {
        let fitted = fit_name_and_tail("main", Some("  → origin/main"), 20, 8);
        assert_eq!(fitted.name, "", "a name with no cells left is empty");
        assert_eq!(fitted.tail, None);
    }

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
