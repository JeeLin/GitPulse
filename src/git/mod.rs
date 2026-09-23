//! Git operations layer: repository discovery and status queries.

mod discover;
mod status;

pub use discover::discover_repos;
#[allow(unused_imports)] // part of the public Git API contract (M1 subtask 3)
pub use status::{query_status, RepoInfo};
