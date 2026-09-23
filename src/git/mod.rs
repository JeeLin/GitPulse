//! Git operations layer: repository discovery and status queries.

mod discover;
mod file;
mod status;

pub use discover::discover_repos;
#[allow(unused_imports)]
pub use file::{diff_file, discard, list_file_status, stage, unstage, FileEntry, FileState};
#[allow(unused_imports)] // part of the public Git API contract (M1 subtask 3)
pub use status::{query_status, RepoInfo};
