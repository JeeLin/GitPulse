//! Git operations layer: repository discovery and status queries.

mod batch;
mod branch;
mod commit;
mod discover;
mod file;
mod remote;
mod status;

pub use batch::{batch_fetch, batch_pull};
pub use branch::{create_branch, delete_branch, list_branches, switch_branch, BranchInfo};
pub use commit::commit;
pub use discover::discover_repos;
pub use file::{diff_file, discard, list_file_status, stage, unstage, FileEntry, FileState};
pub use remote::{pull, push, Progress};
// part of the public Git API contract (M1 subtask 3)
pub use status::{query_status, RepoInfo};
