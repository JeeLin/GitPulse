//! octocrab backend: typed GitHub fetches + octocrab→model mapping.
//!
//! `GithubClient` implements [`PlatformClient`], the single async surface the App
//! layer drives for every platform; octocrab is the only transport, so its errors
//! are normalized into [`PlatformError`] (which carries the platform label the UI
//! renders).

use std::sync::Arc;

use async_trait::async_trait;
use octocrab::models::pulls::PullRequest;
use octocrab::{models, Octocrab, Page};

use super::auth::resolve_token;
use super::debug_lower;
use super::error::GithubError;
use crate::platform::models::{
    CheckSummary, IssueDetail, IssueSummary, NotificationSummary, PrDetail, PrSummary,
};
use crate::platform::{PlatformClient, PlatformError};

/// GitHub API client (single backend; token resolved once via [`resolve_token`]).
pub struct GithubClient {
    octocrab: Arc<Octocrab>,
    owner: String,
    repo: String,
}

impl GithubClient {
    /// Build a client for `owner/repo`.
    pub fn new(owner: &str, repo: &str) -> Result<Self, GithubError> {
        let token = resolve_token()?;
        let crab = Octocrab::builder()
            .personal_token(token)
            .build()
            .map_err(|e| GithubError::Other(e.to_string()))?;
        Ok(Self {
            octocrab: Arc::new(crab),
            owner: owner.to_string(),
            repo: repo.to_string(),
        })
    }

    /// Keep only the notifications that belong to `full_name` (`owner/repo`).
    ///
    /// The `list_for_repo` endpoint is already repo-scoped, but we filter on the
    /// client too so a future switch to the org-wide list cannot leak other
    /// repositories' rows into the panel (covered by a unit test).
    pub(crate) fn filter_repo(
        items: Vec<NotificationSummary>,
        full_name: &str,
    ) -> Vec<NotificationSummary> {
        items.into_iter().filter(|n| n.repo == full_name).collect()
    }
}

#[async_trait]
impl PlatformClient for GithubClient {
    /// Open PRs (first page, 100).
    async fn list_pulls(&self) -> Result<Vec<PrSummary>, PlatformError> {
        let page: Page<PullRequest> = self
            .octocrab
            .pulls(self.owner.as_str(), self.repo.as_str())
            .list()
            .state(octocrab::params::State::Open)
            .per_page(100)
            .send()
            .await
            .map_err(GithubError::from)?;
        Ok(page.items.into_iter().map(|p| pr_to_summary(&p)).collect())
    }

    /// PR detail + head-commit checks.
    async fn get_pull(&self, number: u64) -> Result<PrDetail, PlatformError> {
        let pr: PullRequest = self
            .octocrab
            .pulls(self.owner.as_str(), self.repo.as_str())
            .get(number)
            .await
            .map_err(GithubError::from)?;
        let sha = pr.head.sha.clone();
        // Best effort: checks failing to load leaves the list empty, not an error.
        let checks = self.list_checks(&sha).await.unwrap_or_default();
        let summary = pr_to_summary(&pr);
        Ok(PrDetail {
            summary,
            body: pr.body.clone().unwrap_or_default(),
            labels: pr.labels.iter().flatten().map(|l| l.name.clone()).collect(),
            mergeable: pr.mergeable,
            checks,
        })
    }

    /// Check runs for a commit SHA (`Commitish`, not `Reference`).
    ///
    /// Shared by `get_pull` (head-commit snapshot) and the Checks tab, which
    /// fetches the local `HEAD` SHA via `crate::git::head_sha`.
    async fn list_checks(&self, sha: &str) -> Result<Vec<CheckSummary>, PlatformError> {
        let res = self
            .octocrab
            .checks(self.owner.as_str(), self.repo.as_str())
            .list_check_runs_for_git_ref(octocrab::params::repos::Commitish(sha.to_string()))
            .send()
            .await
            .map_err(GithubError::from)?;
        Ok(res
            .check_runs
            .into_iter()
            .map(|c| check_to_summary(&c))
            .collect())
    }

    /// Open issues excluding PRs (`pull_request.is_none()`).
    async fn list_issues(&self) -> Result<Vec<IssueSummary>, PlatformError> {
        let page: Page<models::issues::Issue> = self
            .octocrab
            .issues(self.owner.as_str(), self.repo.as_str())
            .list()
            .state(octocrab::params::State::Open)
            .per_page(100)
            .send()
            .await
            .map_err(GithubError::from)?;
        Ok(page
            .items
            .into_iter()
            .filter(|i| i.pull_request.is_none())
            .map(issue_to_summary)
            .collect())
    }

    /// Single issue detail (summary fields + markdown body).
    async fn get_issue(&self, number: u64) -> Result<IssueDetail, PlatformError> {
        let issue: models::issues::Issue = self
            .octocrab
            .issues(self.owner.as_str(), self.repo.as_str())
            .get(number)
            .await
            .map_err(GithubError::from)?;
        let body = issue.body.clone().unwrap_or_default();
        Ok(IssueDetail {
            summary: issue_to_summary(issue),
            body,
        })
    }

    /// Repo-scoped notifications (read-only: never marked-as-read).
    async fn list_notifications(&self) -> Result<Vec<NotificationSummary>, PlatformError> {
        let page: Page<models::activity::Notification> = self
            .octocrab
            .activity()
            .notifications()
            .list_for_repo(self.owner.as_str(), self.repo.as_str())
            .send()
            .await
            .map_err(GithubError::from)?;
        let full = format!("{}/{}", self.owner, self.repo);
        Ok(Self::filter_repo(
            page.items
                .into_iter()
                .map(notification_to_summary)
                .collect(),
            &full,
        ))
    }
}

fn pr_to_summary(p: &models::pulls::PullRequest) -> PrSummary {
    PrSummary {
        number: p.number,
        title: p.title.as_deref().unwrap_or_default().to_string(),
        state: debug_lower(&p.state),
        draft: p.draft.unwrap_or(false),
        author: p.user.as_ref().map(|u| u.login.clone()).unwrap_or_default(),
        head_ref: p.head.ref_field.clone(),
        base_ref: p.base.ref_field.clone(),
        updated_at: p
            .updated_at
            .as_ref()
            .map(|dt| format!("{}", dt))
            .unwrap_or_default(),
    }
}

fn issue_to_summary(i: models::issues::Issue) -> IssueSummary {
    IssueSummary {
        number: i.number,
        title: i.title.clone(),
        state: debug_lower(&Some(i.state)),
        author: i.user.login.clone(),
        assignee: i.assignee.as_ref().map(|u| u.login.clone()),
        updated_at: format!("{}", i.updated_at),
        labels: i.labels.iter().map(|l| l.name.clone()).collect(),
    }
}

fn check_to_summary(c: &models::checks::CheckRun) -> CheckSummary {
    // octocrab 0.54.2's `CheckRun` exposes `conclusion` but not `status`; derive
    // a running/completed badge from whether GitHub reported a conclusion.
    CheckSummary {
        name: c.name.clone(),
        status: if c.conclusion.is_some() {
            "completed".to_string()
        } else {
            "in_progress".to_string()
        },
        conclusion: c.conclusion.clone().unwrap_or_default(),
        html_url: c.html_url.clone().unwrap_or_default(),
    }
}

fn notification_to_summary(n: models::activity::Notification) -> NotificationSummary {
    NotificationSummary {
        id: n.id.to_string(),
        repo: n.repository.full_name.clone().unwrap_or_default(),
        subject_title: n.subject.title.clone(),
        subject_type: n.subject.r#type.clone(),
        reason: n.reason.clone(),
        updated_at: format!("{}", n.updated_at),
        unread: n.unread,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: &str, repo: &str) -> NotificationSummary {
        NotificationSummary {
            id: id.to_string(),
            repo: repo.to_string(),
            subject_title: String::new(),
            subject_type: String::new(),
            reason: String::new(),
            updated_at: String::new(),
            unread: false,
        }
    }

    #[test]
    fn filter_repo_keeps_only_current_repo() {
        let items = vec![note("1", "octocat/repo"), note("2", "other/repo")];
        let kept = GithubClient::filter_repo(items, "octocat/repo");
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].id, "1");
    }

    #[test]
    fn filter_repo_empty_when_no_match() {
        let items = vec![note("1", "octocat/repo")];
        assert_eq!(GithubClient::filter_repo(items, "nope/nada").len(), 0);
    }

    /// `CheckRun` is `#[non_exhaustive]` and has no `Default`, so build one from
    /// the JSON the API actually returns instead of a struct literal.
    fn check_run(conclusion: Option<&str>, html_url: Option<&str>) -> models::checks::CheckRun {
        serde_json::from_value(serde_json::json!({
            "id": 1u64,
            "node_id": "MDg6Q2hlY2tSdW4x",
            "head_sha": "abc123",
            "url": "https://api.github.com/repos/octocat/repo/check-runs/1",
            "html_url": html_url,
            "conclusion": conclusion,
            "output": {
                "title": null,
                "summary": null,
                "text": null,
                "annotations_count": 0u64,
                "annotations_url": "https://api.github.com/repos/octocat/repo/check-runs/1/annotations"
            },
            "name": "ci",
            "pull_requests": [],
        }))
        .expect("CheckRun decodes from the API payload")
    }

    #[test]
    fn check_to_summary_derives_status_from_conclusion() {
        let done = check_run(Some("success"), Some("https://ci.example.com/1"));
        let c = check_to_summary(&done);
        assert_eq!(c.name, "ci");
        assert_eq!(c.status, "completed");
        assert_eq!(c.conclusion, "success");
        assert_eq!(c.html_url, "https://ci.example.com/1");

        let running = check_run(None, None);
        let c = check_to_summary(&running);
        assert_eq!(c.status, "in_progress");
        assert_eq!(c.conclusion, "");
    }
}
