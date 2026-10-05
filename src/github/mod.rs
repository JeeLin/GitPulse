//! GitHub API access: the octocrab backend behind [`crate::platform`].
//!
//! gh CLI is auth-only (token resolution via [`auth`]); all transport is octocrab
//! HTTP so there is one failure surface. The client is built once and reused across
//! refreshes (no `Octocrab::default()`).
//!
//! Surface:
//!   - [`GithubClient`] implementing [`crate::platform::PlatformClient`]
//!   - [`GithubError`], re-exported as the public error type; the conversion into
//!     [`crate::platform::PlatformError`] lives in [`error`] (on this side, so the
//!     platform layer never names the GitHub backend); user-facing rendering lives
//!     in [`crate::platform::render_platform_error`]
//!   - [`models`]: re-exports of [`crate::platform::models`]

use std::fmt::Debug;

pub mod auth;
pub mod client;
pub mod error;
pub mod models;

pub use client::GithubClient;
pub use error::GithubError;

/// Debug-form of an optional enum, lowercased for stable rendering.
///
/// octocrab model enums derive `Debug` (`IssueState`, …); matching their exact
/// variant names would couple us to a specific octocrab version. The debug form
/// is lowercased (`Some(Open)` → `"open"`) and underscore-spelled GitHub values
/// are normalized back from the collapsed form.
fn debug_lower<T: Debug>(opt: &Option<T>) -> String {
    let raw = opt
        .as_ref()
        .map(|v| format!("{:?}", v).to_lowercase())
        .unwrap_or_else(|| "unknown".to_string());
    match raw.as_str() {
        "inprogress" => "in_progress".to_string(),
        "timedout" => "timed_out".to_string(),
        "actionrequired" => "action_required".to_string(),
        "startupfailure" => "startup_failure".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stand-in for an octocrab model enum: only its `Debug` form matters here.
    #[derive(Debug)]
    enum FakeState {
        Open,
        InProgress,
        ActionRequired,
    }

    #[test]
    fn debug_lower_lowercases_and_fills_missing() {
        assert_eq!(debug_lower(&Some(FakeState::Open)), "open");
        let none: Option<FakeState> = None;
        assert_eq!(debug_lower(&none), "unknown");
    }

    #[test]
    fn debug_lower_restores_underscored_variants() {
        assert_eq!(debug_lower(&Some(FakeState::InProgress)), "in_progress");
        assert_eq!(
            debug_lower(&Some(FakeState::ActionRequired)),
            "action_required"
        );
    }
}
