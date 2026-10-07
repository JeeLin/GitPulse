//! Visual token system (DESIGN §5): color roles, spacing rhythm and type
//! roles.
//!
//! Every panel, overlay and the footer read their colors from here instead of
//! naming a raw [`Color`], so a theme change is one edit and no view can drift
//! into its own palette (the des-9 consistency debt this module removes).

#![allow(dead_code, unused_imports)]

use ratatui::style::{Color, Modifier};

/// Color roles of the dark theme (§5.1).
///
/// The `git_*` / `ci_*` rows are aliases of the semantic colors — file and CI
/// states never introduce a hue of their own.
pub mod color {
    use ratatui::style::Color;

    /// Application background.
    pub const BG: Color = Color::Rgb(0x1a, 0x1b, 0x26);
    /// Panel background / modal surface.
    pub const SURFACE: Color = Color::Rgb(0x24, 0x28, 0x3b);
    /// Selected row / alternating row background.
    pub const SURFACE_ALT: Color = Color::Rgb(0x2f, 0x33, 0x49);
    /// Unfocused border.
    pub const BORDER: Color = Color::Rgb(0x41, 0x48, 0x68);
    /// Focused border.
    pub const BORDER_FOCUS: Color = Color::Rgb(0x7a, 0xa2, 0xf7);
    /// Primary text.
    pub const FG: Color = Color::Rgb(0xc0, 0xca, 0xf5);
    /// Secondary text: hints, captions, inactive entries.
    pub const FG_MUTED: Color = Color::Rgb(0x56, 0x5f, 0x89);
    /// Selection / key names / active tab.
    pub const ACCENT: Color = Color::Rgb(0x7a, 0xa2, 0xf7);
    /// Success / added.
    pub const SUCCESS: Color = Color::Rgb(0x9e, 0xce, 0x6a);
    /// Error / deleted.
    pub const DANGER: Color = Color::Rgb(0xf7, 0x76, 0x8e);
    /// Modified / warning / CI pending.
    pub const WARNING: Color = Color::Rgb(0xe0, 0xaf, 0x68);
    /// Renamed / hash / progress.
    pub const INFO: Color = Color::Rgb(0x7d, 0xcf, 0xff);

    /// Working-tree file: added.
    pub const GIT_ADD: Color = SUCCESS;
    /// Working-tree file: modified.
    pub const GIT_MOD: Color = WARNING;
    /// Working-tree file: deleted.
    pub const GIT_DEL: Color = DANGER;
    /// Working-tree file: renamed.
    pub const GIT_RENAMED: Color = INFO;

    /// CI run: success.
    pub const CI_PASS: Color = SUCCESS;
    /// CI run: failure.
    pub const CI_FAIL: Color = DANGER;
    /// CI run: pending.
    pub const CI_PENDING: Color = WARNING;
    /// CI run: skipped.
    pub const CI_SKIP: Color = FG_MUTED;
}

/// Spacing rhythm (§5.2): the one-cell padding, the one-cell inter-panel
/// gutter and the fixed heights of the vertical structure.
pub mod spacing {
    /// Horizontal padding inside a panel (left and right, in cells).
    pub const PADDING_X: u16 = 1;
    /// Gutter between two panels.
    pub const GUTTER: u16 = 1;
    /// Height of the title row.
    pub const TITLE_ROWS: u16 = 1;
    /// Height of the context-tab row.
    pub const TAB_ROWS: u16 = 1;
    /// Height of the footer: status row + hint row (§4.1, always 2).
    pub const FOOTER_ROWS: u16 = 2;
    /// Two spaces between two hint pairs of the hint row (§2.3).
    pub const HINT_GAP: &str = "  ";
}

/// Canonical side-by-side panel width floors (§5.4 80 列临界地板).
///
/// §5.4: "**仅当两个面板都能保住各自 `min_width`（列表 ≥28，详情 ≥32）时才允许
/// 左右分栏**；否则回退为上下堆叠或『列表 + 焦点时内联预览』". These are the
/// single source for that rule so every screen's split gate reads one number
/// instead of inventing its own.
///
/// `PREVIEW_MAX` caps the detail pane so it can never starve the list; a screen
/// may open its split only at a HIGHER frame width than the arithmetic floor
/// (the dashboard opens at 96) — the floor is a permission, not an obligation.
pub mod width {
    /// Minimum width of a list panel that shares its row with another panel.
    pub const LIST_MIN: u16 = 28;
    /// Minimum width of the detail/preview panel beside a list.
    pub const PREVIEW_MIN: u16 = 32;
    /// Minimum frame width that may split into a list and a detail pane at all
    /// (`LIST_MIN` + `GUTTER` + `PREVIEW_MIN`).
    pub const SPLIT_FLOOR: u16 = LIST_MIN + super::spacing::GUTTER + PREVIEW_MIN;
}

/// Type roles (§5.3): the handful of text weights the whole UI draws with.
pub mod role {
    use ratatui::style::{Modifier, Style};

    use super::color;

    /// Panel / overlay title: bold accent.
    pub fn title() -> Style {
        Style::default()
            .fg(color::ACCENT)
            .add_modifier(Modifier::BOLD)
    }

    /// Section heading inside a panel: bold.
    pub fn heading() -> Style {
        Style::default().add_modifier(Modifier::BOLD)
    }

    /// Body text.
    pub fn body() -> Style {
        Style::default()
    }

    /// Caption / secondary text: muted foreground.
    pub fn caption() -> Style {
        Style::default().fg(color::FG_MUTED)
    }

    /// Panel border: `border_focus` for the one panel that holds focus,
    /// `border` for every other (§3.1). Exactly one panel is focused at a
    /// time, so this pair is how the focus becomes visible.
    pub fn border(focused: bool) -> Style {
        Style::default().fg(if focused {
            color::BORDER_FOCUS
        } else {
            color::BORDER
        })
    }

    /// Key cap in the hint row: bold accent (§2.3).
    pub fn key() -> Style {
        Style::default()
            .fg(color::ACCENT)
            .add_modifier(Modifier::BOLD)
    }

    /// Monospace run (hash, branch name); hashes take [`color::INFO`].
    pub fn code() -> Style {
        Style::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §5.1: the token values are the design's, hex by hex — a typo here
    /// would silently re-theme the whole application.
    #[test]
    fn color_tokens_match_the_design_palette() {
        assert_eq!(color::BG, Color::Rgb(0x1a, 0x1b, 0x26));
        assert_eq!(color::SURFACE, Color::Rgb(0x24, 0x28, 0x3b));
        assert_eq!(color::SURFACE_ALT, Color::Rgb(0x2f, 0x33, 0x49));
        assert_eq!(color::BORDER, Color::Rgb(0x41, 0x48, 0x68));
        assert_eq!(color::BORDER_FOCUS, Color::Rgb(0x7a, 0xa2, 0xf7));
        assert_eq!(color::FG, Color::Rgb(0xc0, 0xca, 0xf5));
        assert_eq!(color::FG_MUTED, Color::Rgb(0x56, 0x5f, 0x89));
        assert_eq!(color::ACCENT, Color::Rgb(0x7a, 0xa2, 0xf7));
        assert_eq!(color::SUCCESS, Color::Rgb(0x9e, 0xce, 0x6a));
        assert_eq!(color::DANGER, Color::Rgb(0xf7, 0x76, 0x8e));
        assert_eq!(color::WARNING, Color::Rgb(0xe0, 0xaf, 0x68));
        assert_eq!(color::INFO, Color::Rgb(0x7d, 0xcf, 0xff));
    }

    /// The git / CI rows are aliases, never independent hues.
    #[test]
    fn git_and_ci_tokens_alias_the_semantic_colors() {
        assert_eq!(color::GIT_ADD, color::SUCCESS);
        assert_eq!(color::GIT_MOD, color::WARNING);
        assert_eq!(color::GIT_DEL, color::DANGER);
        assert_eq!(color::GIT_RENAMED, color::INFO);
        assert_eq!(color::CI_PASS, color::SUCCESS);
        assert_eq!(color::CI_FAIL, color::DANGER);
        assert_eq!(color::CI_PENDING, color::WARNING);
        assert_eq!(color::CI_SKIP, color::FG_MUTED);
    }

    /// §5.2: the fixed heights the layout composes from.
    #[test]
    fn spacing_tokens_keep_the_vertical_rhythm() {
        assert_eq!(spacing::PADDING_X, 1);
        assert_eq!(spacing::GUTTER, 1);
        assert_eq!(spacing::TITLE_ROWS, 1);
        assert_eq!(spacing::TAB_ROWS, 1);
        assert_eq!(spacing::FOOTER_ROWS, 2, "footer is status + hint row");
        assert_eq!(spacing::HINT_GAP, "  ", "two spaces between hint pairs");
    }

    /// §5.3: `key` is bold accent, `caption` muted — the two roles the hint
    /// row switches between.
    #[test]
    fn type_roles_carry_the_documented_weight_and_color() {
        assert_eq!(role::key().fg, Some(color::ACCENT));
        assert!(role::key().add_modifier.contains(Modifier::BOLD));
        assert_eq!(role::caption().fg, Some(color::FG_MUTED));
        assert_eq!(role::title().fg, Some(color::ACCENT));
        assert!(role::title().add_modifier.contains(Modifier::BOLD));
        assert!(role::heading().add_modifier.contains(Modifier::BOLD));
        assert_eq!(role::body().fg, None);
    }

    /// §3.1: the focus is visible — the focused panel's border is
    /// `border_focus`, every other panel's is `border`.
    #[test]
    fn border_role_separates_the_focused_panel_from_the_rest() {
        assert_eq!(role::border(true).fg, Some(color::BORDER_FOCUS));
        assert_eq!(role::border(false).fg, Some(color::BORDER));
        assert_ne!(color::BORDER_FOCUS, color::BORDER);
    }
}
