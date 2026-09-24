//! Git operations layer: repository discovery and status queries.

mod commit;
mod discover;
mod file;
mod remote;
mod status;

#[allow(unused_imports)]
pub use commit::commit;
pub use discover::discover_repos;
#[allow(unused_imports)]
pub use file::{diff_file, discard, list_file_status, stage, unstage, FileEntry, FileState};
#[allow(unused_imports)]
pub use remote::{pull, push, Progress};
#[allow(unused_imports)] // part of the public Git API contract (M1 subtask 3)
pub use status::{query_status, RepoInfo};
