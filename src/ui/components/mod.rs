//! Shared TUI component layer (DESIGN §5–§6): visual tokens, the width budget,
//! the canonical header, the five-state panel body, the status-line types and
//! remote feedback, the receipts spine and the two-row footer.
//!
//! The components are built first and consumed by the views: every screen reads
//! its colors from [`theme`], measures rows with [`cells`], draws its title bar
//! and context-tag strip through [`header`], renders non-ready list states
//! through [`state_panel`], and leaves the bottom two rows to [`footer`] — so
//! the interaction grammar has exactly one implementation.
//!
//! [`receipts`] is the one crate-private member: it is the receipts *spine*
//! that the remote-hub tabs route their four lists through, reachable only
//! from within this crate (its test frame helpers are not part of the public
//! surface), not a component every screen draws like the five `pub` ones above.
//!
//! This layer sits *below* the views, so it never reaches back up into them:
//! the §3.1 cursor ([`with_cursor`]), the status row (§4.2:
//! [`footer::render_status_line`] / [`footer::render_status_or_remote`], drawn by
//! [`remote::render_remote_status`]) and the per-view status row
//! ([`view_status`]) all live here, and the views import them rather than
//! the other way round (B4-03).

pub mod cells;
pub mod footer;
pub mod header;
pub(crate) mod receipts;
pub mod remote;
pub mod state_panel;
pub mod status;
pub mod theme;

use ratatui::style::Style;
use ratatui::widgets::List;

use crate::app::{App, View};
use crate::ui::components::footer::StatusRow;

/// Footer status row for the current view (§4.2) — the one place an async
/// result reaches the screen: local git messages and GitHub receipts alike
/// land on this row, whether or not a remote is connected.
///
/// `Repo` / `Tags` / `Commit` and every receipt tab own remote feedback, so
/// they carry the remote triple and label it with their own operation name;
/// the rest (dashboard / branches / history / config / diffs) take the plain
/// status line. No panel reads these fields any more (§4.1).
///
/// This lives beside [`footer::render_footer`] because it only *builds* the row that
/// [`footer::render_footer`] paints; putting it here keeps the dispatch over
/// [`View`] inside the component layer instead of above it (B4-03).
pub(crate) fn view_status(app: &App) -> StatusRow<'_> {
    match app.view {
        View::Repo | View::Tags | View::Commit => StatusRow::new(
            app.status.as_ref(),
            app.remote_status.as_ref(),
            app.remote_op,
        ),
        View::Pulls
        | View::PullDetail
        | View::Issues
        | View::IssueDetail
        | View::Checks
        | View::Notifications => StatusRow::new(
            app.status.as_ref(),
            app.remote_status.as_ref(),
            app.receipt_op_label(),
        ),
        _ => StatusRow::plain(app.status.as_ref()),
    }
}

/// Marker rendered at the row start of the focused list's selected row
/// (§3.1: 行首 `▌` + `surface_alt` 底色). One cell wide, so a selected row
/// keeps every column the unselected rows use.
pub const HIGHLIGHT_SYMBOL: &str = "▌";

/// Shared selection style of the focused list's cursor row (§3.1): the
/// canonical `surface_alt` background, and nothing else — the design picked
/// the background over an accent foreground for the cursor.
pub fn list_highlight() -> Style {
    Style::default().bg(theme::color::SURFACE_ALT)
}

/// Wire the §3.1 cursor onto a list panel: when `focused`, the selected row
/// carries `▌` over the `surface_alt` background; otherwise the list shows
/// neither, because the cursor exists only in the focused list panel.
///
/// One place, so every list in the app draws the same cursor — a panel cannot
/// keep a private marker or a private highlight.
pub(crate) fn with_cursor(list: List<'static>, focused: bool) -> List<'static> {
    if focused {
        list.highlight_style(list_highlight())
            .highlight_symbol(HIGHLIGHT_SYMBOL)
    } else {
        list.highlight_symbol("")
    }
}
