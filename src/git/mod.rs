//! Git operations layer: repository discovery and status queries.

use std::fmt::Write as _;
use std::path::Path;

mod batch;
mod branch;
mod commit;
mod discover;
mod file;
pub(crate) mod history;
mod remote;
mod status;
mod tag;

pub use batch::{batch_fetch, batch_pull};
pub use branch::{create_branch, delete_branch, list_branches, switch_branch, BranchInfo};
pub use commit::commit;
pub use discover::{canonical_or_self, discover_repos};
pub use file::{diff_file, discard, list_file_status, stage, unstage, FileEntry, FileState};
// `RefMark` is intentionally not re-exported: outside `history.rs` nothing
// names the type in non-test code (the UI reads `CommitRow::refs` through
// `RefKind` and the fields), so the re-export was a dead API surface kept
// alive only by `#[allow(unused_imports)]` (G35/I1). The type stays public
// inside `history` for its own tests and badge builders; add a `pub use`
// back only when a non-test consumer appears. `GraphCell` is gone for the
// same reason: `layout_graph` now yields positioned rows of `Vec<char>`, so
// no consumer names a cell type any more (G8).
pub use history::{
    commit_detail, layout_graph, list_commits, CommitDetail, CommitRow, RefKind, MAX_LANES,
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

/// Repository display name derived from `path`: the final path component,
/// falling back to the whole path when there is none (a bare `/` or `..`).
///
/// Shared by the dashboard, the config panel and the batch helpers so a
/// directory name is spelled the same way everywhere (G31); a name that is
/// not valid UTF-8 takes the [`show_path`] escaping.
pub fn repo_display_name(path: &Path) -> String {
    let component = path.file_name().map(Path::new);
    show_path(component.unwrap_or(path))
}

/// Display form of `path` for user-facing messages: the exact UTF-8
/// spelling when the path is valid Unicode, otherwise the `Debug` rendering
/// so invalid bytes escape as `\xe9` instead of the lossy U+FFFD
/// replacement produced by [`std::path::Path::display`].
pub fn show_path(path: &Path) -> String {
    path.to_str()
        .map(str::to_string)
        .unwrap_or_else(|| format!("{path:?}"))
}

/// Display form of a raw git byte string (reference name, commit message,
/// author name) for user-facing messages: valid UTF-8 comes back verbatim,
/// non-UTF-8 bytes are escaped exactly the way [`show_path`] escapes a
/// non-UTF-8 path (`\xE9` style) instead of collapsing into the lossy
/// U+FFFD replacement.
pub fn show_bytes(bytes: &[u8]) -> String {
    let Ok(text) = std::str::from_utf8(bytes) else {
        let mut out = String::with_capacity(bytes.len() + 2);
        out.push('"');
        for chunk in bytes.utf8_chunks() {
            let valid = chunk.valid();
            if !valid.is_empty() {
                // `{:?}` on a `&str` wraps it in quotes; only the escaped
                // interior belongs here, the outer quotes are added once.
                let escaped = format!("{valid:?}");
                out.push_str(&escaped[1..escaped.len() - 1]);
            }
            for byte in chunk.invalid() {
                let _ = write!(out, "\\x{byte:02X}");
            }
        }
        out.push('"');
        return out;
    };
    text.to_string()
}
