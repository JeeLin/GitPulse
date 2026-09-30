//! Git operations layer: repository discovery and status queries.

mod batch;
mod branch;
mod commit;
mod discover;
mod file;
mod history;
mod remote;
mod status;
mod tag;

pub use batch::{batch_fetch, batch_pull};
pub use branch::{create_branch, delete_branch, list_branches, switch_branch, BranchInfo};
pub use commit::commit;
pub use discover::{canonical_or_self, discover_repos};
pub use file::{diff_file, discard, list_file_status, stage, unstage, FileEntry, FileState};
// `RefMark` is part of the exported API but has no non-test consumer in the
// binary yet (tests build reference badges from it directly).
#[allow(unused_imports)]
pub use history::RefMark;
pub use history::{
    commit_detail, layout_graph, list_commits, CommitDetail, CommitRow, GraphCell, RefKind,
    MAX_LANES,
};
pub use remote::{pull, push, push_all_tags, push_tag};
// part of the public Git API contract (M1 subtask 3)
pub use status::{query_status, RepoInfo};
pub use tag::{create_tag, delete_tag, list_tags, TagInfo};

/// Abbreviate an object id to the 7 hex characters `git log --oneline` shows.
///
/// Every short-id rendering in GitPulse goes through this helper so the width
/// stays consistent across the history list, the tag panel and the status
/// line.
pub fn short_id(oid: &git2::Oid) -> String {
    format!("{oid:.7}")
}

/// Display form of `path` for user-facing messages: the exact UTF-8
/// spelling when the path is valid Unicode, otherwise the `Debug` rendering
/// so invalid bytes escape as `\xe9` instead of the lossy U+FFFD
/// replacement produced by [`std::path::Path::display`].
pub fn show_path(path: &std::path::Path) -> String {
    path.to_str()
        .map(str::to_string)
        .unwrap_or_else(|| format!("{path:?}"))
}
