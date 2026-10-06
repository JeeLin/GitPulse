//! Shared TUI component layer: visual tokens, width budget, the five-state
//! panel body and the two-row footer.
//!
//! The components are built first and consumed by the views (milestone
//! architecture note): every screen reads its colors from
//! [`theme`], measures rows with [`cells`], draws non-ready list states
//! through [`state_panel`] and leaves the bottom two rows to [`footer`], so
//! the interaction grammar has exactly one implementation.

pub mod cells;
pub mod footer;
pub mod header;
pub mod state_panel;
pub mod theme;
