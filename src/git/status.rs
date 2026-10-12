//! Repository status query: branch, dirty stats, ahead/behind, last commit.

use std::path::{Path, PathBuf};

use anyhow::Result;
use git2::Repository;

use crate::platform::remote::PlatformKind;

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
    /// Fetch URL of the primary remote (`origin` → `upstream` → first), if any.
    ///
    /// `None` for a local-only worktree with no configured remote; the dashboard
    /// renders this as "no remote" rather than the row becoming a load error
    /// (sub-task 3, checkpoint ③). Populated in [`query_status`] by reusing the
    /// already-open `Repository` — no extra `Repository::open`.
    pub remote: Option<String>,
    /// Whether this row is a linked worktree of another repository (sub-task 3).
    ///
    /// `false` for a main or standalone repository; `true` for a linked
    /// worktree, in which case `main_repo` names the owning repository.
    pub is_worktree: bool,
    /// Absolute path of the owning main repository when `is_worktree` is true.
    ///
    /// `None` for a main repository (standalone, or the root that owns its
    /// own worktrees) (checkpoint ③).
    pub main_repo: Option<PathBuf>,
    /// Effective commit identity of this repository, or `None` when neither
    /// `user.name` nor `user.email` resolves anywhere.
    ///
    /// This is an **echo** only: `super::commit` signs commits from the same
    /// config lookup, so what the dashboard shows is what a commit here uses
    /// (v0.9.0). `None` renders as `无` / `(无本地配置)` rather than failing
    /// the row — an unreadable identity never makes a repository unlistable.
    pub author: Option<AuthorInfo>,
}

/// The commit identity a repository echoes: the name git signs commits with
/// and the email it pairs with it (v0.9.0).
///
/// Crate-scoped per sub-task 1's interface design: read through
/// [`RepoInfo::author`] and the shared `author_label`, never a standalone
/// public API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AuthorInfo {
    /// `user.name` as git resolves it for this repository.
    pub name: String,
    /// `user.email` as git resolves it for this repository.
    pub email: String,
}

/// Query the current status of a git repository.
pub fn query_status(repo_path: &Path) -> Result<RepoInfo> {
    let repo = Repository::open(repo_path)?;

    let branch = resolve_branch(&repo)?;
    let dirty = compute_dirty_stats(&repo)?;
    let (ahead, behind) = compute_ahead_behind(&repo)?;
    let last_commit = resolve_last_commit(&repo)?;
    let remote = resolve_remote(&repo);
    let author = super::author::read_author(&repo);

    let name = repo_display_name(repo_path);

    Ok(RepoInfo {
        name,
        path: repo_path.to_path_buf(),
        branch,
        dirty,
        ahead,
        behind,
        last_commit,
        remote,
        is_worktree: false,
        main_repo: None,
        author,
    })
}

/// Short platform label for a repository row's `远端` column (sub-task 4).
///
/// DESIGN §6.1's sample row renders `远端: GitHub`, so the column shows a
/// *platform*, not a URL. The label is derived from the fetch URL's host: known
/// forges map to their product name, any other host falls back to the host
/// itself (an `ssh://` or `git@host:path` remote still identifies where the
/// code lives), and a repository with no remote renders `无` rather than
/// failing the row (checkpoint ③).
pub fn remote_platform(url: &str) -> String {
    let host = bare_host(url);
    if host.is_empty() {
        return "无".to_string();
    }
    match PlatformKind::from_host(&host) {
        PlatformKind::SelfHosted => host,
        known => known.as_str().to_string(),
    }
}

/// The host of a fetch URL, port-free and lowercase — the shape
/// [`PlatformKind::from_host`] compares against.
///
/// Handles the scp-like `git@host:owner/repo` form as well as
/// `scheme://[user@]host[:port]/path`; anything unrecognized yields the trimmed
/// input so the caller still has something to display.
fn bare_host(url: &str) -> String {
    let trimmed = url.trim();
    // scp-like `[user@]host:path` carries no `://`, so it takes the first arm.
    let scp = trimmed
        .split_once("://")
        .is_none()
        .then(|| trimmed.split_once(':'))
        .flatten();
    let host_port = match scp {
        Some((head, _path)) => head,
        None => trimmed
            .split_once("://")
            .map(|(_scheme, rest)| rest.split('/').next().unwrap_or(rest))
            .unwrap_or(trimmed),
    };
    let host = host_port.rsplit('@').next().unwrap_or(host_port);
    host.split(':').next().unwrap_or(host).to_ascii_lowercase()
}

/// The dashboard's `远端` cell: the platform label, or `无` when the repository
/// has no remote (sub-task 4).
pub fn remote_label(repo: &RepoInfo) -> String {
    match repo.remote.as_deref() {
        Some(url) => remote_platform(url),
        None => "无".to_string(),
    }
}

/// Query status for a linked worktree, flagging identity and namespacing the
/// display name as `{main}⑂{worktree}` per checkpoint ⑯ (sub-task 3).
///
/// `main_repo` is the owning main repository's working directory; `wt_path` is
/// the worktree's checked-out path (as returned by `Worktree::path`). The
/// caller has already resolved the worktree via the main repo's own worktree
/// registry, so a submodule (which appears in the index, not the registry) is
/// never confused with a worktree (checkpoint ③).
pub(crate) fn query_worktree_status(main_repo: &Path, wt_path: &Path) -> Result<RepoInfo> {
    let mut info = query_status(wt_path)?;
    info.name = format!(
        "{}⑂{}",
        repo_display_name(main_repo),
        repo_display_name(wt_path)
    );
    info.is_worktree = true;
    info.main_repo = Some(main_repo.to_path_buf());
    Ok(info)
}

/// Fetch URL of the repository's primary remote (`origin` → `upstream` → first
/// configured remote), if configured and valid UTF-8.
///
/// Returns `None` (rather than erroring) when the repository has no remote or
/// its fetch URL is not valid UTF-8 — a local worktree's normal state is "no
/// remote", and the dashboard must still render that row. This deliberately does
/// *not* reuse the `remote_url` helper, which bails on a remote-less repo
/// (`bail!("未配置远端")`); the dashboard's steady state is precisely the case
/// that has to surface as `None` (sub-task 3, checkpoint ③). Reuses the
/// already-open `Repository` handle, so there is no additional `Repository::open`.
fn resolve_remote(repo: &Repository) -> Option<String> {
    let remotes = repo.remotes().ok()?;
    // Walk by index (V5: `iter().flatten()` would silently drop non-UTF-8
    // remote names, making a repo look remote-less or picking a wrong one).
    let mut names: Vec<&str> = Vec::with_capacity(remotes.len());
    for index in 0..remotes.len() {
        if let Some(name) = remotes.get(index) {
            names.push(name);
        }
    }
    // Priority: origin → upstream → first configured remote.
    let name = names
        .iter()
        .find(|n| **n == "origin")
        .or_else(|| names.iter().find(|n| **n == "upstream"))
        .or_else(|| names.first())?;
    let remote = repo.find_remote(name).ok()?;
    remote.url().map(str::to_string)
}

/// SHA-1 of the current `HEAD` commit (full 40 hex), used by the Checks tab to
/// fetch head-commit check runs. A missing repository or an unborn HEAD (no
/// commits yet) returns a readable error rather than a raw git message.
pub fn head_sha(repo_path: &Path) -> Result<String> {
    let repo = Repository::open(repo_path).map_err(|e| anyhow::anyhow!("打开仓库失败: {e}"))?;
    let head = repo
        .head()
        .map_err(|_| anyhow::anyhow!("无法解析 HEAD（可能是空仓库）"))?;
    let commit = head
        .peel_to_commit()
        .map_err(|e| anyhow::anyhow!("解析 HEAD 提交失败: {e}"))?;
    Ok(commit.id().to_string())
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

    #[test]
    fn head_sha_returns_40_hex_of_head_commit() {
        let repo_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let sha = head_sha(&repo_path).unwrap();
        assert_eq!(sha.len(), 40);
        assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn head_sha_missing_repo_errors_readably() {
        let err = head_sha(Path::new("/nonexistent/gitpulse")).unwrap_err();
        assert!(err.to_string().contains("打开仓库失败"));
    }

    /// Empty temp git repo under a unique path (used by remote-fixture tests).
    fn temp_repo(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gitpulse-status-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn query_status_remote_resolution() {
        let dir = temp_repo("remote");
        let repo = Repository::init(&dir).unwrap();

        // No remote configured: the row must still render — `query_status`
        // returns Ok and surfaces `None`, never bailing like `remote_url` would
        // (sub-task 3 regression hook).
        let info = query_status(&dir).unwrap();
        assert_eq!(info.remote, None);

        // Priority: origin → upstream → first configured remote.
        repo.remote("origin", "https://github.com/o/origin.git")
            .unwrap();
        repo.remote("upstream", "https://github.com/o/upstream.git")
            .unwrap();
        repo.remote("fork", "https://github.com/o/fork.git")
            .unwrap();
        let info = query_status(&dir).unwrap();
        assert_eq!(
            info.remote.as_deref(),
            Some("https://github.com/o/origin.git")
        );

        // origin removed entirely → upstream wins.
        repo.remote_delete("origin").unwrap();
        let info = query_status(&dir).unwrap();
        assert_eq!(
            info.remote.as_deref(),
            Some("https://github.com/o/upstream.git")
        );

        // no origin/upstream → first configured remote wins.
        repo.remote_delete("upstream").unwrap();
        let info = query_status(&dir).unwrap();
        assert_eq!(
            info.remote.as_deref(),
            Some("https://github.com/o/fork.git")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Regression hook for sub-task 4: the dashboard's `远端` column shows a
    /// platform, not the raw URL. Falsifiable: mapping `github.com` to the
    /// URL itself breaks the first assertion.
    #[test]
    fn remote_platform_labels_known_forges() {
        assert_eq!(remote_platform("https://github.com/o/r.git"), "GitHub");
        assert_eq!(remote_platform("git@github.com:o/r.git"), "GitHub");
        assert_eq!(remote_platform("https://gitlab.com/o/r.git"), "GitLab");
        assert_eq!(remote_platform("https://gitea.com/o/r.git"), "Gitea");
        assert_eq!(remote_platform("ssh://git@gitea.com/group/r.git"), "Gitea");
        // A host the app's own SaaS list does not own falls back to the host
        // spelling — the row still says where the code lives, not "无".
        assert_eq!(
            remote_platform("https://git.internal.corp/gp/r.git"),
            "git.internal.corp"
        );
        assert_eq!(remote_platform("git@10.0.0.5:group/r.git"), "10.0.0.5");
    }

    #[test]
    fn remote_label_names_the_repo_without_a_remote() {
        let dir = temp_repo("remote-label");
        Repository::init(&dir).unwrap();
        let info = query_status(&dir).unwrap();
        assert_eq!(remote_label(&info), "无");

        let repo = Repository::open(&dir).unwrap();
        repo.remote("origin", "https://github.com/o/origin.git")
            .unwrap();
        let info = query_status(&dir).unwrap();
        assert_eq!(remote_label(&info), "GitHub");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
