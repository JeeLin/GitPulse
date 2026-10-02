//! octocrab backend: typed GitHub fetches + octocrab→model mapping.
//!
//! Lane B/C contract: `GithubClient::{list_pulls,get_pull,list_issues,list_notifications}`.
//! Fetchers are async (octocrab is async); the App layer polls per frame.

use std::sync::Arc;

use octocrab::models::pulls::PullRequest;
use octocrab::{models, Octocrab, Page};

use super::auth::resolve_token;
use super::debug_lower;
use super::error::GithubError;
use super::models::{CheckSummary, IssueSummary, NotificationSummary, PrDetail, PrSummary};

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

    /// Open PRs (first page, 100).
    pub async fn list_pulls(&self) -> Result<Vec<PrSummary>, GithubError> {
        let page: Page<PullRequest> = self
            .octocrab
            .pulls(self.owner.as_str(), self.repo.as_str())
            .list()
            .state(octocrab::params::State::Open)
            .per_page(100)
            .send()
            .await?;
        Ok(page.items.into_iter().map(|p| pr_to_summary(&p)).collect())
    }

    /// PR detail + head-commit checks (M2: `PrDetail.checks`).
    pub async fn get_pull(&self, number: u64) -> Result<PrDetail, GithubError> {
        let pr: PullRequest = self
            .octocrab
            .pulls(self.owner.as_str(), self.repo.as_str())
            .get(number)
            .await?;
        let sha = pr.head.sha.clone();
        // Best effort: checks failing to load leaves the list empty, not an error.
        let checks = self.check_runs(&sha).await.unwrap_or_default();
        let summary = pr_to_summary(&pr);
        Ok(PrDetail {
            summary,
            body: pr.body.clone().unwrap_or_default(),
            mergeable: pr.mergeable,
            checks,
        })
    }

    /// Check runs for a commit SHA (`Commitish`, not `Reference`).
    async fn check_runs(&self, sha: &str) -> Result<Vec<CheckSummary>, GithubError> {
        let res = self
            .octocrab
            .checks(self.owner.as_str(), self.repo.as_str())
            .list_check_runs_for_git_ref(octocrab::params::repos::Commitish(sha.to_string()))
            .send()
            .await?;
        Ok(res
            .check_runs
            .into_iter()
            .map(|c| check_to_summary(&c))
            .collect())
    }

    /// Open issues excluding PRs (`pull_request.is_none()`).
    pub async fn list_issues(&self) -> Result<Vec<IssueSummary>, GithubError> {
        let page: Page<models::issues::Issue> = self
            .octocrab
            .issues(self.owner.as_str(), self.repo.as_str())
            .list()
            .state(octocrab::params::State::Open)
            .per_page(100)
            .send()
            .await?;
        Ok(page
            .items
            .into_iter()
            .filter(|i| i.pull_request.is_none())
            .map(issue_to_summary)
            .collect())
    }

    /// Repo-scoped notifications (read-only: never marked-as-read).
    pub async fn list_notifications(&self) -> Result<Vec<NotificationSummary>, GithubError> {
        let page: Page<models::activity::Notification> = self
            .octocrab
            .activity()
            .notifications()
            .list_for_repo(self.owner.as_str(), self.repo.as_str())
            .send()
            .await?;
        Ok(page
            .items
            .into_iter()
            .map(notification_to_summary)
            .collect())
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
