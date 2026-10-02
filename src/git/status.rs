//! Repository status query: branch, dirty stats, ahead/behind, last commit.

use std::path::{Path, PathBuf};

use anyhow::Result;
use git2::Repository;

use super::{repo_display_name, short_id, show_bytes};

/// Statistics about uncommitted changes in a repository.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DirtyStats {
    /// Files added to index but not committed (new files).
    pub added: usize,
    /// Files modified in working tree or index.
    pub modified: usize,
    /// Files deleted from working tree or index.
    pub deleted: usize,
    /// Files not tracked by git.
    pub untracked: usize,
    /// Files with merge conflicts.
    pub conflicted: usize,
}

/// Information about the most recent commit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommitInfo {
    /// Commit summary (first line of message).
    pub summary: String,
    /// Author name.
    pub author: String,
    /// Commit timestamp (Unix epoch seconds).
    pub time: i64,
}

/// Snapshot of a git repository's current state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoInfo {
    /// Repository name (directory name).
    pub name: String,
    /// Absolute path to the repository root.
    pub path: PathBuf,
    /// Current branch name (or short hash if detached).
    pub branch: String,
    /// Count of uncommitted changes by type.
    pub dirty: DirtyStats,
    /// Commits ahead of upstream.
    pub ahead: usize,
    /// Commits behind upstream.
    pub behind: usize,
    /// Most recent commit.
    pub last_commit: CommitInfo,
}

/// Query the current status of a git repository.
pub fn query_status(repo_path: &Path) -> Result<RepoInfo> {
    let repo = Repository::open(repo_path)?;

    let branch = resolve_branch(&repo)?;
    let dirty = compute_dirty_stats(&repo)?;
    let (ahead, behind) = compute_ahead_behind(&repo)?;
    let last_commit = resolve_last_commit(&repo)?;

    let name = repo_display_name(repo_path);

    Ok(RepoInfo {
        name,
        path: repo_path.to_path_buf(),
        branch,
        dirty,
        ahead,
        behind,
        last_commit,
    })
}

/// Resolve the current branch name, detached HEAD short hash, or `unborn`.
fn resolve_branch(repo: &Repository) -> Result<String> {
    match repo.head() {
        Ok(head) => {
            if head.is_branch() {
                // A branch name that is not valid UTF-8 is rendered escaped
                // (V8): falling back to the literal "HEAD" would claim the
                // repository is not on a branch at all.
                Ok(show_bytes(head.shorthand_bytes()))
            } else {
                // Detached HEAD: show short hash.
                let commit = head.peel_to_commit()?;
                Ok(short_id(&commit.id()))
            }
        }
        // Unborn HEAD (no commits yet).
        Err(_) => Ok("unborn".to_string()),
    }
}

/// Count uncommitted changes by category from the index and working tree.
fn compute_dirty_stats(repo: &Repository) -> Result<DirtyStats> {
    let statuses = repo.statuses(None)?;
    let mut stats = DirtyStats::default();

    for entry in statuses.iter() {
        let status = entry.status();

        if status.contains(git2::Status::INDEX_NEW) {
            stats.added += 1;
        }
        if status.contains(git2::Status::WT_MODIFIED)
            || status.contains(git2::Status::INDEX_MODIFIED)
        {
            stats.modified += 1;
        }
        if status.contains(git2::Status::WT_DELETED) || status.contains(git2::Status::INDEX_DELETED)
        {
            stats.deleted += 1;
        }
        if status.contains(git2::Status::WT_NEW) {
            stats.untracked += 1;
        }
        if status.contains(git2::Status::CONFLICTED) {
            stats.conflicted += 1;
        }
    }

    Ok(stats)
}

/// Compute commits ahead/behind the upstream; `(0, 0)` when no upstream exists.
fn compute_ahead_behind(repo: &Repository) -> Result<(usize, usize)> {
    let head = match repo.head() {
        Ok(h) => h,
        Err(_) => return Ok((0, 0)),
    };
    if !head.is_branch() {
        return Ok((0, 0));
    }

    let branch = match repo.find_branch(
        head.shorthand().unwrap_or_default(),
        git2::BranchType::Local,
    ) {
        Ok(b) => b,
        // No tracking info: a branch whose name is not valid UTF-8 cannot
        // even be looked up here, so ahead/behind stays (0, 0) (V8) — the
        // dashboard simply shows no counts for it.
        Err(_) => return Ok((0, 0)),
    };
    let upstream = match branch.upstream() {
        Ok(u) => u,
        Err(_) => return Ok((0, 0)),
    };

    let head_commit = match head.peel_to_commit() {
        Ok(c) => c,
        Err(_) => return Ok((0, 0)),
    };
    let upstream_commit = match upstream.get().peel_to_commit() {
        Ok(c) => c,
        Err(_) => return Ok((0, 0)),
    };

    let (ahead, behind) = repo.graph_ahead_behind(head_commit.id(), upstream_commit.id())?;
    Ok((ahead, behind))
}

/// Extract summary, author, and time from the current HEAD commit.
fn resolve_last_commit(repo: &Repository) -> Result<CommitInfo> {
    let head = match repo.head() {
        Ok(h) => h,
        // No commits yet.
        Err(_) => return Ok(CommitInfo::default()),
    };
    let commit = head.peel_to_commit()?;
    // Byte-level fields: a message or author name that is not valid UTF-8 is
    // escaped the same way the history panel escapes it (V11) instead of
    // collapsing into an empty string.
    let summary = show_bytes(commit.summary_bytes().unwrap_or(b""));
    let author = show_bytes(commit.author().name_bytes());
    let time = commit.time().seconds();
    Ok(CommitInfo {
        summary,
        author,
        time,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::show_path;

    #[test]
    fn query_status_on_git_repo() {
        let repo_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let info = query_status(&repo_path).unwrap();
        assert!(!info.branch.is_empty());
        // The name is the checkout directory name; derive the expectation from
        // the repository path so the test holds for clones under any directory
        // name (not just "GitPulse").
        let expected = repo_path
            .file_name()
            .map(|n| show_path(Path::new(n)))
            .unwrap_or_else(|| show_path(&repo_path));
        assert_eq!(info.name, expected);
    }
}
