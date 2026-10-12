//! Status-line types shared by the app state and the view renderers.
//!
//! They live on the UI side so the renderers never reach back into
//! [`crate::app`] just to name the types they draw (G18); `App` stores them
//! like any other piece of its state.

/// Bottom status line content: short message or error (convention 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusLine {
    /// Message text.
    pub message: String,
    /// `true` for errors, which render red and persist until the next operation.
    pub is_error: bool,
}

impl StatusLine {
    /// Whether this is a non-error success line: the design's `✓ ` prefix with
    /// `is_error == false`, so the status row renders it green (§3.2
    /// `success-flash` / §4.2).
    ///
    /// The glyph is the contract, not a flag — [`crate::app::App`] cannot grow a
    /// dedicated success field without breaking every view and test that
    /// builds a `StatusLine` literal, so the tone is detected from the message
    /// the way the renderer already reads error status from `is_error`.
    pub fn is_success(&self) -> bool {
        !self.is_error && self.message.starts_with("✓ ")
    }
}

/// Push/pull feedback rendered on the status line (convention 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteStatus {
    /// Operation in flight; carries its running text (`正在 push…`).
    ///
    /// A percentage used to live here, but the git layer stopped emitting
    /// progress once the `on_progress` callback was removed (R5) and the
    /// value could never leave 0 — the variant now carries the operation's
    /// text instead of a gauge that was frozen at 0% (G1).
    Running(String),
    /// Operation finished successfully.
    Success,
    /// Operation failed; the reason persists until the next operation.
    Error(String),
}
