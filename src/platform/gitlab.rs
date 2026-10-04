//! GitLab platform client: REST API v4 implementation of [`PlatformClient`].
//!
//! Uses reqwest directly (the `gitlab` crate is missing notifications and we want
//! uniform transport across all six endpoints). Auth is the `PRIVATE-TOKEN` header,
//! resolved via `platform::auth`.
//!
//! Base URL: `https://{host}/api/v4` (defaults to `gitlab.com`). Project IDs are
//! the URL-encoded full path (`owner%2Frepo`) so subgroups work. Responses are
//! parsed as `serde_json::Value` and mapped to shared models.

use std::sync::Arc;

use async_trait::async_trait;
use reqwest::{header, Client, RequestBuilder};
use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;

use crate::github::models::{
    CheckSummary, IssueDetail, IssueSummary, NotificationSummary, PrDetail, PrSummary,
};
use crate::platform::auth::resolve_token;
use crate::platform::client::PlatformClient;
use crate::platform::error::PlatformError;
use crate::platform::remote::PlatformRemote;

/// Platform label used in error messages.
const PLATFORM: &str = "GitLab";

/// GitLab REST API v4 client (async, reqwest-backed).
pub struct GitLabClient {
    http: Arc<Client>,
    remote: PlatformRemote,
    token: SecretString,
    base_url: String,
    /// URL-encoded project ID: `owner%2Frepo` (subgroup-safe).
    project_id: String,
}

impl GitLabClient {
    /// Build a client for `remote`, resolving a token via `platform::auth`.
    pub fn new(remote: &PlatformRemote) -> Result<Self, PlatformError> {
        let token = resolve_token(&remote.host, remote.kind)?;
        let client = Client::builder()
            .user_agent("gitpulse")
            .build()
            .map_err(|e| PlatformError::Other {
                platform: PLATFORM,
                message: e.to_string(),
            })?;
        let base_url = format!("https://{}/api/v4", remote.host);
        // URL-encode the owner/repo path so subgroups (`group%2Fsub%2Fproject`) work.
        let project_id = format!("{}%2F{}", remote.owner, remote.repo);
        Ok(Self {
            http: Arc::new(client),
            remote: remote.clone(),
            token,
            base_url,
            project_id,
        })
    }

    /// The `PRIVATE-TOKEN` header value.
    fn auth_header(&self) -> header::HeaderValue {
        header::HeaderValue::from_str(self.token.expose_secret())
            .expect("token header value is valid")
    }

    /// Build an authenticated GET request for `path`.
    fn get(&self, path: &str) -> RequestBuilder {
        let url = format!("{}{}", self.base_url, path);
        self.http.get(&url).header(
            header::HeaderName::from_static("private-token"),
            self.auth_header(),
        )
    }

    /// Execute a GET, returning the JSON body or mapping errors to `PlatformError`.
    async fn fetch_json(&self, req: RequestBuilder) -> Result<Value, PlatformError> {
        let resp = req.send().await.map_err(reqwest_to_platform)?;
        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            let message = resp.text().await.unwrap_or_default();
            return Err(PlatformError::Http {
                status,
                message,
                rate_limited: status == 429,
                platform: PLATFORM,
            });
        }
        resp.json::<Value>()
            .await
            .map_err(|e| PlatformError::Other {
                platform: PLATFORM,
                message: e.to_string(),
            })
    }
}

/// Convert a `reqwest::Error` into a `PlatformError`.
///
/// Connect/timeout/build errors → `Network`; everything else (including decode
/// and JSON errors) → `Other`.
fn reqwest_to_platform(e: reqwest::Error) -> PlatformError {
    if e.is_connect() || e.is_timeout() || e.is_builder() {
        PlatformError::Network(e.to_string())
    } else {
        PlatformError::Other {
            platform: PLATFORM,
            message: e.to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Trait impl
// ---------------------------------------------------------------------------

#[async_trait]
impl PlatformClient for GitLabClient {
    async fn list_pulls(&self) -> Result<Vec<PrSummary>, PlatformError> {
        let path = format!(
            "/projects/{}/merge_requests?state=opened&per_page=100",
            self.project_id,
        );
        let json = self.fetch_json(self.get(&path)).await?;
        Ok(json
            .as_array()
            .map(|arr| arr.iter().map(mr_to_summary).collect())
            .unwrap_or_default())
    }

    async fn get_pull(&self, number: u64) -> Result<PrDetail, PlatformError> {
        let path = format!("/projects/{}/merge_requests/{}", self.project_id, number);
        let json = self.fetch_json(self.get(&path)).await?;
        // Use diff_refs.head_sha for the CI pipeline lookup.
        let sha = json["diff_refs"]["head_sha"].as_str().unwrap_or("");
        let checks = self.list_checks(sha).await.unwrap_or_default();
        Ok(PrDetail {
            summary: mr_to_summary(&json),
            body: json["description"].as_str().unwrap_or("").to_string(),
            labels: extract_labels(&json["labels"]),
            mergeable: map_merge_status(json["detailed_merge_status"].as_str()),
            checks,
        })
    }

    async fn list_checks(&self, sha: &str) -> Result<Vec<CheckSummary>, PlatformError> {
        let path = format!(
            "/projects/{}/pipelines?sha={}&per_page=100",
            self.project_id, sha,
        );
        let json = self.fetch_json(self.get(&path)).await?;
        Ok(json
            .as_array()
            .map(|arr| arr.iter().map(pipeline_to_check).collect())
            .unwrap_or_default())
    }

    async fn list_issues(&self) -> Result<Vec<IssueSummary>, PlatformError> {
        // GitLab `/issues` already excludes merge requests — no PR filter needed.
        let path = format!(
            "/projects/{}/issues?scope=opened&per_page=100",
            self.project_id
        );
        let json = self.fetch_json(self.get(&path)).await?;
        Ok(json
            .as_array()
            .map(|arr| arr.iter().map(issue_to_summary).collect())
            .unwrap_or_default())
    }

    async fn get_issue(&self, number: u64) -> Result<IssueDetail, PlatformError> {
        let path = format!("/projects/{}/issues/{}", self.project_id, number);
        let json = self.fetch_json(self.get(&path)).await?;
        Ok(IssueDetail {
            summary: issue_to_summary(&json),
            body: json["description"].as_str().unwrap_or("").to_string(),
        })
    }

    async fn list_notifications(&self) -> Result<Vec<NotificationSummary>, PlatformError> {
        let full = format!("{}/{}", self.remote.owner, self.remote.repo);
        let json = self.fetch_json(self.get("/todos?per_page=100")).await?;
        let items: Vec<NotificationSummary> = json
            .as_array()
            .map(|arr| arr.iter().map(todo_to_notification).collect())
            .unwrap_or_default();
        Ok(items.into_iter().filter(|n| n.repo == full).collect())
    }
}

// ---------------------------------------------------------------------------
// Mapper functions
// ---------------------------------------------------------------------------

/// Map GitLab `detailed_merge_status` to `Option<bool>`.
///
/// `mergeable` / `ready` → `Some(true)`; `blocked` / `conflict` / `cannot_be_merged`
/// → `Some(false)`; anything else (e.g. `unchecked`, `checking`) → `None` (not
/// computed yet).
fn map_merge_status(status: Option<&str>) -> Option<bool> {
    match status {
        Some("mergeable" | "ready" | "can_be_merged") => Some(true),
        Some("cannot_be_merged" | "blocked" | "conflict" | "not_fast_forward") => Some(false),
        _ => None,
    }
}

/// Extract label names from a GitLab `labels` value.
///
/// GitLab returns labels as an array of strings (not objects like GitHub/Gitea).
fn extract_labels(labels: &Value) -> Vec<String> {
    labels
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|l| l.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

fn mr_to_summary(v: &Value) -> PrSummary {
    PrSummary {
        number: v["iid"].as_u64().unwrap_or(0),
        title: v["title"].as_str().unwrap_or("").to_string(),
        state: v["state"].as_str().unwrap_or("").to_string(),
        draft: v["work_in_progress"].as_bool().unwrap_or(false),
        author: v["author"]["username"].as_str().unwrap_or("").to_string(),
        head_ref: v["source_branch"].as_str().unwrap_or("").to_string(),
        base_ref: v["target_branch"].as_str().unwrap_or("").to_string(),
        updated_at: v["updated_at"].as_str().unwrap_or("").to_string(),
    }
}

fn pipeline_to_check(v: &Value) -> CheckSummary {
    let raw = v["status"].as_str().unwrap_or("");
    let (status, conclusion) = match raw {
        "pending"
        | "running"
        | "created"
        | "waiting_for_manual"
        | "waiting_for_resource"
        | "manual"
        | "scheduled"
        | "blocked" => ("in_progress".to_string(), "unknown".to_string()),
        "success" => ("completed".to_string(), "success".to_string()),
        "failed" => ("completed".to_string(), "failure".to_string()),
        "canceled" => ("completed".to_string(), "cancelled".to_string()),
        "skipped" => ("completed".to_string(), "skipped".to_string()),
        _ => ("unknown".to_string(), "unknown".to_string()),
    };
    CheckSummary {
        name: v["source"].as_str().unwrap_or("").to_string(),
        status,
        conclusion,
        html_url: v["web_url"].as_str().unwrap_or("").to_string(),
    }
}

fn issue_to_summary(v: &Value) -> IssueSummary {
    IssueSummary {
        number: v["iid"].as_u64().unwrap_or(0),
        title: v["title"].as_str().unwrap_or("").to_string(),
        state: v["state"].as_str().unwrap_or("").to_string(),
        author: v["author"]["username"].as_str().unwrap_or("").to_string(),
        assignee: v["assignee"]["username"].as_str().map(|s| s.to_string()),
        updated_at: v["updated_at"].as_str().unwrap_or("").to_string(),
        labels: extract_labels(&v["labels"]),
    }
}

/// Map a GitLab todo to a [`NotificationSummary`].
///
/// GitLab has no thread-feed / notification API; `/todos` is the closest match.
/// The `subject_type` is derived from `target_type` (`MergeRequest` → `PullRequest`).
fn todo_to_notification(v: &Value) -> NotificationSummary {
    let repo = format!(
        "{}/{}",
        v["target"]["namespace"]["full_path"].as_str().unwrap_or(""),
        v["target"]["path"].as_str().unwrap_or("")
    );
    let repo = if repo == "/" { String::new() } else { repo };
    let subject_title = v["body"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| v["target"]["title"].as_str())
        .unwrap_or("")
        .to_string();
    let subject_type = match v["target_type"].as_str().unwrap_or("") {
        "MergeRequest" => "PullRequest",
        "Issue" => "Issue",
        other => other,
    }
    .to_string();
    NotificationSummary {
        id: v["id"].as_i64().map(|n| n.to_string()).unwrap_or_default(),
        repo,
        subject_title,
        subject_type,
        reason: v["action"].as_str().unwrap_or("").to_string(),
        updated_at: v["updated_at"]
            .as_str()
            .or_else(|| v["created_at"].as_str())
            .unwrap_or("")
            .to_string(),
        unread: v["state"].as_str() == Some("pending"),
    }
}

// ---------------------------------------------------------------------------
// Unit tests for mappers
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn mr_json() -> Value {
        json!({
            "iid": 15u64,
            "title": "Add feature",
            "state": "opened",
            "work_in_progress": false,
            "author": { "username": "dev" },
            "source_branch": "feature",
            "target_branch": "main",
            "updated_at": "2024-06-01T00:00:00Z",
            "labels": ["bug", "enhancement"],
            "description": "Detailed description",
            "detailed_merge_status": "mergeable",
            "diff_refs": { "head_sha": "abc123def" }
        })
    }

    fn pipeline_json() -> Value {
        json!({
            "id": 100u64,
            "sha": "abc123",
            "ref": "main",
            "status": "success",
            "source": "pipeline",
            "web_url": "https://gitlab.com/group/repo/pipelines/100"
        })
    }

    fn issue_json() -> Value {
        json!({
            "iid": 3u64,
            "title": "Issue title",
            "state": "opened",
            "author": { "username": "alice" },
            "assignee": { "username": "bob" },
            "updated_at": "2024-06-02T00:00:00Z",
            "labels": ["bug"],
            "description": "Issue body"
        })
    }

    fn todo_json() -> Value {
        json!({
            "id": 99u64,
            "body": "mentioned you",
            "state": "pending",
            "action": "assigned",
            "target_type": "Issue",
            "target": {
                "title": "Issue title",
                "namespace": { "full_path": "group/repo" },
                "path": "repo"
            },
            "created_at": "2024-06-03T00:00:00Z",
            "updated_at": "2024-06-03T00:00:00Z"
        })
    }

    #[test]
    fn mr_to_summary_extracts_fields() {
        let s = mr_to_summary(&mr_json());
        assert_eq!(s.number, 15);
        assert_eq!(s.title, "Add feature");
        assert_eq!(s.state, "opened");
        assert!(!s.draft);
        assert_eq!(s.author, "dev");
        assert_eq!(s.head_ref, "feature");
        assert_eq!(s.base_ref, "main");
        assert_eq!(s.updated_at, "2024-06-01T00:00:00Z");
    }

    #[test]
    fn mr_to_summary_maps_work_in_progress() {
        let mut v = mr_json();
        v["work_in_progress"] = json!(true);
        let s = mr_to_summary(&v);
        assert!(s.draft);
    }

    #[test]
    fn pipeline_to_check_success() {
        let c = pipeline_to_check(&pipeline_json());
        assert_eq!(c.name, "pipeline");
        assert_eq!(c.status, "completed");
        assert_eq!(c.conclusion, "success");
        assert!(c.html_url.contains("pipelines/100"));
    }

    #[test]
    fn pipeline_to_check_running() {
        let v = json!({ "status": "running", "source": "pipeline", "web_url": "" });
        let c = pipeline_to_check(&v);
        assert_eq!(c.status, "in_progress");
        assert_eq!(c.conclusion, "unknown");
    }

    #[test]
    fn pipeline_to_check_failed() {
        let v = json!({ "status": "failed", "source": "ci", "web_url": "" });
        let c = pipeline_to_check(&v);
        assert_eq!(c.status, "completed");
        assert_eq!(c.conclusion, "failure");
    }

    #[test]
    fn map_merge_status_variants() {
        assert_eq!(map_merge_status(Some("mergeable")), Some(true));
        assert_eq!(map_merge_status(Some("ready")), Some(true));
        assert_eq!(map_merge_status(Some("can_be_merged")), Some(true));
        assert_eq!(map_merge_status(Some("cannot_be_merged")), Some(false));
        assert_eq!(map_merge_status(Some("blocked")), Some(false));
        assert_eq!(map_merge_status(Some("conflict")), Some(false));
        assert_eq!(map_merge_status(Some("unchecked")), None);
        assert_eq!(map_merge_status(Some("checking")), None);
        assert_eq!(map_merge_status(None), None);
    }

    #[test]
    fn extract_labels_handles_strings() {
        let v = json!(["bug", "enhancement"]);
        assert_eq!(
            extract_labels(&v),
            vec!["bug".to_string(), "enhancement".to_string()]
        );
    }

    #[test]
    fn extract_labels_empty() {
        assert_eq!(extract_labels(&json!([])), Vec::<String>::new());
        assert_eq!(extract_labels(&Value::Null), Vec::<String>::new());
    }

    #[test]
    fn issue_to_summary_with_assignee() {
        let s = issue_to_summary(&issue_json());
        assert_eq!(s.number, 3);
        assert_eq!(s.author, "alice");
        assert_eq!(s.assignee.as_deref(), Some("bob"));
        assert_eq!(s.labels, vec!["bug".to_string()]);
    }

    #[test]
    fn issue_to_summary_without_assignee() {
        let mut v = issue_json();
        v["assignee"] = Value::Null;
        let s = issue_to_summary(&v);
        assert_eq!(s.assignee, None);
    }

    #[test]
    fn todo_to_notification_extracts_fields() {
        let n = todo_to_notification(&todo_json());
        assert_eq!(n.id, "99");
        assert!(n.unread);
        assert_eq!(n.subject_type, "Issue");
        assert_eq!(n.subject_title, "mentioned you");
        assert_eq!(n.reason, "assigned");
        assert_eq!(n.updated_at, "2024-06-03T00:00:00Z");
    }

    #[test]
    fn todo_to_notification_mr_type() {
        let mut v = todo_json();
        v["target_type"] = json!("MergeRequest");
        let n = todo_to_notification(&v);
        assert_eq!(n.subject_type, "PullRequest");
    }
}
