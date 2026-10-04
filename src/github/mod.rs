//! GitHub API access: single octocrab backend (M2 *单平台远端回执*).
//!
//! gh CLI is auth-only (token resolution via [`auth`]); all transport is octocrab
//! HTTP so there is one failure surface. The client is built once and reused across
//! `r` refreshes (no `Octocrab::default()`).
//!
//! Public surface (the Lane B/C contract):
//!   - [`parse_remote_owner_repo`]: git remote URL → `(owner, repo)`
//!   - [`GithubClient`] + `list_pulls` / `get_pull` / `list_issues` / `list_notifications`
//!   - [`render_error`]: `GithubError` → user-facing line
//!   - models: [`PrSummary`], [`PrDetail`], [`CheckSummary`], [`IssueSummary`], [`NotificationSummary`]

use std::fmt::Debug;

pub mod auth;
pub mod client;
pub mod error;
pub mod models;

pub use client::GithubClient;
#[allow(unused_imports)]
pub use error::{render_error, GithubError};
#[allow(unused_imports)]
pub use models::{
    CheckSummary, IssueDetail, IssueSummary, NotificationSummary, PrDetail, PrSummary,
};

/// Parse a git remote URL into `(owner, repo)` for `github.com` remotes.
///
/// Supports `https://`, `http://`, `ssh://`, `git://`, and the scp-like
/// `git@github.com:owner/repo` forms. Non-github.com remotes (or malformed
/// ones) return [`GithubError::NotGitHub`].
#[allow(dead_code)]
pub fn parse_remote_owner_repo(url: &str) -> Result<(String, String), GithubError> {
    let trimmed = url.trim().trim_end_matches(".git").trim();
    let path = if let Some(rest) = trimmed.strip_prefix("git@github.com:") {
        rest.to_string()
    } else {
        trimmed
            .strip_prefix("https://github.com/")
            .or_else(|| trimmed.strip_prefix("http://github.com/"))
            .or_else(|| trimmed.strip_prefix("ssh://git@github.com/"))
            .or_else(|| trimmed.strip_prefix("ssh://github.com/"))
            .or_else(|| trimmed.strip_prefix("git://github.com/"))
            .ok_or_else(|| {
                GithubError::NotGitHub(format!(
                    "远端地址不是 GitHub：{trimmed}（仅支持 github.com）"
                ))
            })?
            .to_string()
    };
    split_owner_repo(&path)
}

/// Split `"owner/repo[/...extra...]"` → `("owner", "repo")`.
#[allow(dead_code)]
fn split_owner_repo(path: &str) -> Result<(String, String), GithubError> {
    let path = path.trim_start_matches('/');
    let mut parts = path.splitn(2, '/');
    let owner = parts.next().filter(|s| !s.is_empty());
    let repo = parts
        .next()
        .and_then(|r| r.split('/').next())
        .filter(|s| !s.is_empty());
    match (owner, repo) {
        (Some(o), Some(r)) => Ok((o.to_string(), r.to_string())),
        _ => Err(GithubError::NotGitHub(format!(
            "无法从远端路径解析 owner/repo：{path}"
        ))),
    }
}

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

    #[test]
    fn parses_https_with_git_suffix() {
        let (owner, repo) = parse_remote_owner_repo("https://github.com/owner/repo.git").unwrap();
        assert_eq!(owner, "owner");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn parses_https_with_extra_path() {
        let (owner, repo) =
            parse_remote_owner_repo("https://github.com/owner/repo/issues/42").unwrap();
        assert_eq!(owner, "owner");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn parses_scp_like() {
        let (owner, repo) = parse_remote_owner_repo("git@github.com:owner/repo.git").unwrap();
        assert_eq!(owner, "owner");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn parses_ssh_and_git_schemes() {
        let (owner, repo) = parse_remote_owner_repo("ssh://git@github.com/owner/repo").unwrap();
        assert_eq!(owner, "owner");
        assert_eq!(repo, "repo");

        let (owner, repo) = parse_remote_owner_repo("git://github.com/owner/repo").unwrap();
        assert_eq!(owner, "owner");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn rejects_non_github() {
        assert!(matches!(
            parse_remote_owner_repo("https://gitlab.com/owner/repo"),
            Err(GithubError::NotGitHub(_))
        ));
        assert!(matches!(
            parse_remote_owner_repo("git@gitlab.com:owner/repo"),
            Err(GithubError::NotGitHub(_))
        ));
    }

    #[test]
    fn rejects_missing_repo() {
        assert!(matches!(
            parse_remote_owner_repo("https://github.com/owner"),
            Err(GithubError::NotGitHub(_))
        ));
    }
}
