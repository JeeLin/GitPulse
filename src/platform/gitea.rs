//! Gitea platform client: REST API implementation of [`PlatformClient`].
//!
//! Uses reqwest directly (the `gitea-rs` crate is stale). Auth is the GitHub-compatible
//! `Authorization: token {token}` scheme, resolved via [`PlatformAuth`] / `platform::auth`.
//!
//! Base URL: `https://{host}/api/v1`. Responses are parsed as `serde_json::Value`
//! and mapped to the shared models in `crate::github::models`.

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
const PLATFORM: &str = "Gitea";

/// Gitea REST API v1 client (async, reqwest-backed).
pub struct GiteaClient {
    http: Arc<Client>,
    remote: PlatformRemote,
    token: SecretString,
    base_url: String,
}

impl GiteaClient {
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
        let base_url = format!("https://{}/api/v1", remote.host);
        Ok(Self {
            http: Arc::new(client),
            remote: remote.clone(),
            token,
            base_url,
        })
    }

    /// The `Authorization: token {token}` header value.
    fn auth_header(&self) -> header::HeaderValue {
        let val = format!("token {}", self.token.expose_secret());
        header::HeaderValue::from_str(&val).expect("token header value is valid")
    }

    /// Build an authenticated GET request for `path`.
    fn get(&self, path: &str) -> RequestBuilder {
        let url = format!("{}{}", self.base_url, path);
        self.http
            .get(&url)
            .header(header::AUTHORIZATION, self.auth_header())
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
impl PlatformClient for GiteaClient {
    async fn list_pulls(&self) -> Result<Vec<PrSummary>, PlatformError> {
        let path = format!(
            "/repos/{}/{}/pulls?state=open&limit=100",
            self.remote.owner, self.remote.repo,
        );
        let json = self.fetch_json(self.get(&path)).await?;
        Ok(json
            .as_array()
            .map(|arr| arr.iter().map(pr_to_summary).collect())
            .unwrap_or_default())
    }

    async fn get_pull(&self, number: u64) -> Result<PrDetail, PlatformError> {
        let path = format!(
            "/repos/{}/{}/pulls/{}",
            self.remote.owner, self.remote.repo, number
        );
        let json = self.fetch_json(self.get(&path)).await?;
        let sha = json["head"]["sha"].as_str().unwrap_or("");
        // Best-effort checks: a failure here leaves the list empty, not an error.
        let checks = self.list_checks(sha).await.unwrap_or_default();
        Ok(PrDetail {
            summary: pr_to_summary(&json),
            body: json["body"].as_str().unwrap_or("").to_string(),
            labels: extract_labels(&json["labels"]),
            mergeable: json["mergeable"].as_bool(),
            checks,
        })
    }

    async fn list_checks(&self, sha: &str) -> Result<Vec<CheckSummary>, PlatformError> {
        let path = format!(
            "/repos/{}/{}/commits/{}/status",
            self.remote.owner, self.remote.repo, sha
        );
        let json = self.fetch_json(self.get(&path)).await?;
        Ok(json["statuses"]
            .as_array()
            .map(|arr| arr.iter().map(check_to_summary).collect())
            .unwrap_or_default())
    }

    async fn list_issues(&self) -> Result<Vec<IssueSummary>, PlatformError> {
        let path = format!(
            "/repos/{}/{}/issues?state=open&limit=100",
            self.remote.owner, self.remote.repo,
        );
        let json = self.fetch_json(self.get(&path)).await?;
        Ok(json
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter(|v| v["pull_request"].is_null())
                    .map(issue_to_summary)
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn get_issue(&self, number: u64) -> Result<IssueDetail, PlatformError> {
        let path = format!(
            "/repos/{}/{}/issues/{}",
            self.remote.owner, self.remote.repo, number
        );
        let json = self.fetch_json(self.get(&path)).await?;
        Ok(IssueDetail {
            summary: issue_to_summary(&json),
            body: json["body"].as_str().unwrap_or("").to_string(),
        })
    }

    async fn list_notifications(&self) -> Result<Vec<NotificationSummary>, PlatformError> {
        let full_name = format!("{}/{}", self.remote.owner, self.remote.repo);
        let json = self
            .fetch_json(self.get("/notifications?limit=100"))
            .await?;
        let items: Vec<NotificationSummary> = json
            .as_array()
            .map(|arr| arr.iter().map(notification_to_summary).collect())
            .unwrap_or_default();
        Ok(items.into_iter().filter(|n| n.repo == full_name).collect())
    }
}

// ---------------------------------------------------------------------------
// Mapper functions
// ---------------------------------------------------------------------------

/// Extract label names from a Gitea `labels` value.
///
/// Gitea returns labels as objects with a `name` field, but may also return
/// plain strings — handle both.
fn extract_labels(labels: &Value) -> Vec<String> {
    labels
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|l| {
                    if let Some(name) = l["name"].as_str() {
                        name.to_string()
                    } else if let Some(s) = l.as_str() {
                        s.to_string()
                    } else {
                        String::new()
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn pr_to_summary(v: &Value) -> PrSummary {
    PrSummary {
        number: v["number"].as_u64().unwrap_or(0),
        title: v["title"].as_str().unwrap_or("").to_string(),
        state: v["state"].as_str().unwrap_or("").to_lowercase(),
        draft: v["draft"].as_bool().unwrap_or(false),
        author: v["user"]["login"].as_str().unwrap_or("").to_string(),
        head_ref: v["head"]["ref"].as_str().unwrap_or("").to_string(),
        base_ref: v["base"]["ref"].as_str().unwrap_or("").to_string(),
        updated_at: v["updated_at"].as_str().unwrap_or("").to_string(),
    }
}

fn check_to_summary(v: &Value) -> CheckSummary {
    let state = v["state"].as_str().unwrap_or("");
    let (status, conclusion) = match state {
        "pending" => ("in_progress".to_string(), "unknown".to_string()),
        "success" => ("completed".to_string(), "success".to_string()),
        "failure" | "error" => ("completed".to_string(), "failure".to_string()),
        "warning" => ("completed".to_string(), "neutral".to_string()),
        _ => ("unknown".to_string(), "unknown".to_string()),
    };
    CheckSummary {
        name: v["context"].as_str().unwrap_or("").to_string(),
        status,
        conclusion,
        html_url: v["target_url"].as_str().unwrap_or("").to_string(),
    }
}

fn issue_to_summary(v: &Value) -> IssueSummary {
    IssueSummary {
        number: v["number"].as_u64().unwrap_or(0),
        title: v["title"].as_str().unwrap_or("").to_string(),
        state: v["state"].as_str().unwrap_or("").to_lowercase(),
        author: v["user"]["login"].as_str().unwrap_or("").to_string(),
        assignee: v["assignee"]["login"].as_str().map(|s| s.to_string()),
        updated_at: v["updated_at"].as_str().unwrap_or("").to_string(),
        labels: extract_labels(&v["labels"]),
    }
}

fn notification_to_summary(v: &Value) -> NotificationSummary {
    NotificationSummary {
        id: v["id"].as_str().unwrap_or("").to_string(),
        repo: v["repository"]["full_name"]
            .as_str()
            .unwrap_or("")
            .to_string(),
        subject_title: v["subject"]["title"].as_str().unwrap_or("").to_string(),
        subject_type: v["subject"]["type"]
            .as_str()
            .unwrap_or("unknown")
            .to_string(),
        reason: String::new(),
        updated_at: v["updated_at"].as_str().unwrap_or("").to_string(),
        unread: v["unread"].as_bool().unwrap_or(false),
    }
}

// ---------------------------------------------------------------------------
// Unit tests for mappers
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pr_json() -> Value {
        json!({
            "number": 42u64,
            "title": "Fix bug",
            "state": "open",
            "draft": true,
            "user": { "login": "alice" },
            "head": { "ref": "feature", "sha": "abc123" },
            "base": { "ref": "main" },
            "body": "description",
            "updated_at": "2024-01-01T00:00:00Z",
            "mergeable": true,
            "labels": [{ "name": "bug" }, { "name": "critical" }]
        })
    }

    fn check_json() -> Value {
        json!({
            "state": "success",
            "context": "ci/test",
            "target_url": "https://ci.example.com/builds/1",
        })
    }

    fn issue_json() -> Value {
        json!({
            "number": 7u64,
            "title": "Bug report",
            "state": "open",
            "user": { "login": "bob" },
            "assignee": { "login": "carol" },
            "updated_at": "2024-01-02T00:00:00Z",
            "labels": [{ "name": "bug" }]
        })
    }

    fn issue_without_assignee() -> Value {
        json!({
            "number": 8u64,
            "title": "No assignee",
            "state": "closed",
            "user": { "login": "dave" },
            "assignee": null,
            "updated_at": "2024-01-03T00:00:00Z",
            "labels": []
        })
    }

    fn notification_json() -> Value {
        json!({
            "id": "nt-1",
            "unread": true,
            "subject": { "type": "PullRequest", "title": "PR #1" },
            "repository": { "full_name": "owner/repo" },
            "updated_at": "2024-01-04T00:00:00Z"
        })
    }

    #[test]
    fn pr_to_summary_extracts_fields() {
        let s = pr_to_summary(&pr_json());
        assert_eq!(s.number, 42);
        assert_eq!(s.title, "Fix bug");
        assert_eq!(s.state, "open");
        assert!(s.draft);
        assert_eq!(s.author, "alice");
        assert_eq!(s.head_ref, "feature");
        assert_eq!(s.base_ref, "main");
        assert_eq!(s.updated_at, "2024-01-01T00:00:00Z");
    }

    #[test]
    fn check_to_summary_success() {
        let c = check_to_summary(&check_json());
        assert_eq!(c.name, "ci/test");
        assert_eq!(c.status, "completed");
        assert_eq!(c.conclusion, "success");
        assert_eq!(c.html_url, "https://ci.example.com/builds/1");
    }

    #[test]
    fn check_to_summary_pending() {
        let v = json!({ "state": "pending", "context": "ci/x" });
        let c = check_to_summary(&v);
        assert_eq!(c.status, "in_progress");
        assert_eq!(c.conclusion, "unknown");
    }

    #[test]
    fn check_to_summary_failure() {
        let v = json!({ "state": "failure", "context": "ci/x" });
        let c = check_to_summary(&v);
        assert_eq!(c.status, "completed");
        assert_eq!(c.conclusion, "failure");
    }

    #[test]
    fn issue_to_summary_with_assignee() {
        let s = issue_to_summary(&issue_json());
        assert_eq!(s.number, 7);
        assert_eq!(s.author, "bob");
        assert_eq!(s.assignee.as_deref(), Some("carol"));
        assert_eq!(s.labels, vec!["bug".to_string()]);
    }

    #[test]
    fn issue_to_summary_without_assignee() {
        let s = issue_to_summary(&issue_without_assignee());
        assert_eq!(s.number, 8);
        assert_eq!(s.assignee, None);
        assert_eq!(s.labels, Vec::<String>::new());
    }

    #[test]
    fn extract_labels_handles_string_and_object() {
        let mixed = json!(["string-label", { "name": "obj-label", "color": "#fff" }]);
        assert_eq!(extract_labels(&mixed), vec!["string-label", "obj-label"]);
    }

    #[test]
    fn extract_labels_empty() {
        assert_eq!(extract_labels(&json!([])), Vec::<String>::new());
        assert_eq!(extract_labels(&Value::Null), Vec::<String>::new());
    }

    #[test]
    fn notification_to_summary_extracts_fields() {
        let n = notification_to_summary(&notification_json());
        assert_eq!(n.id, "nt-1");
        assert!(n.unread);
        assert_eq!(n.repo, "owner/repo");
        assert_eq!(n.subject_type, "PullRequest");
        assert_eq!(n.subject_title, "PR #1");
    }
}
