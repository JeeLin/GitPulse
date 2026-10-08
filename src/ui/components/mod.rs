//! Shared TUI component layer (DESIGN §5–§6): visual tokens, the width budget,
//! the canonical header, the five-state panel body, the receipts spine and the
//! two-row footer.
//!
//! The components are built first and consumed by the views: every screen reads
//! its colors from [`theme`], measures rows with [`cells`], draws its title bar
//! and context-tag strip through [`header`], renders non-ready list states
//! through [`state_panel`], routes the four receipts lists through the shared
//! [`receipts`] spine, and leaves the bottom two rows to [`footer`] — so the
//! interaction grammar has exactly one implementation.

pub mod cells;
pub mod footer;
pub mod header;
pub(crate) mod receipts;
pub mod state_panel;
pub mod theme;
