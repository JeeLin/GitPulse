//! Data models shared by [`crate::github`] and the M2 panels.
//!
//! These mirror the GitHub REST shapes the UI needs. Production fetch paths
//! (octocrab, typed) populate them through the `from_*` mappers in
//! [`crate::github::client`]; unit tests build them directly. Keeping them out
//! of the octocrab type graph lets the panels render without depending on
//! octocrab's enum variants.

/// One pull request row in the list panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrSummary {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub draft: bool,
    pub author: String,
    pub head_ref: String,
    pub base_ref: String,
    /// ISO-8601 `updated_at`; kept raw until the UI learns relative time.
    pub updated_at: String,
}

/// Detail of a pull request, including its head-commit checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrDetail {
    /// Core summary fields reused verbatim by the detail panel.
    pub summary: PrSummary,
    /// Markdown body (possibly empty).
    pub body: String,
    /// Mergeability: `None` means GitHub has not computed it yet.
    pub mergeable: Option<bool>,
    /// Check runs for the head commit (M2 step2: `PrDetail.checks`).
    pub checks: Vec<CheckSummary>,
}

/// One CI check run for a commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckSummary {
    pub name: String,
    /// `queued` / `in_progress` / `completed` / `unknown`.
    pub status: String,
    /// `success` / `failure` / `neutral` / `cancelled` / `skipped` / `unknown`.
    pub conclusion: String,
    pub html_url: String,
}

/// One issue row in the issues panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueSummary {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub author: String,
    pub updated_at: String,
    /// Label names; empty when the issue has none.
    pub labels: Vec<String>,
}

/// One notification row (repo-filtered to the current repo).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationSummary {
    /// Opaque GitHub notification id.
    pub id: String,
    /// `owner/repo` of the repository the notification belongs to.
    pub repo: String,
    pub subject_title: String,
    /// `PullRequest` / `Issue` / `CheckSuite` / `Commit` / `Release` / `unknown`.
    pub subject_type: String,
    pub reason: String,
    pub updated_at: String,
    pub unread: bool,
}
