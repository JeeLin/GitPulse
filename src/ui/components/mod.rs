//! Shared TUI component layer (DESIGN §5–§6): visual tokens, the width budget,
//! the canonical header, the five-state panel body, the receipts spine and the
//! two-row footer.
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

pub mod cells;
pub mod footer;
pub mod header;
pub(crate) mod receipts;
pub mod state_panel;
pub mod theme;
