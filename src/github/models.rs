//! Compatibility shim: the shared models live in [`crate::platform::models`].
//!
//! Production code and views depend on `crate::platform::*`; this re-export
//! keeps `crate::github::models::*` working for existing imports.

// Compatibility shim: nothing in the crate imports from here anymore, but the
// module is kept so the `crate::github::models::*` path stays valid.
#[allow(unused_imports)]
pub use crate::platform::models::{
    CheckSummary, IssueDetail, IssueSummary, NotificationSummary, PrDetail, PrSummary,
};
