//! GitLab platform client: REST API v4 implementation of [`PlatformClient`].
//!
//! Uses reqwest directly (the `gitlab` crate is missing notifications and we want
//! uniform transport across all six endpoints). Auth is the `PRIVATE-TOKEN` header,
//! resolved via `platform::auth`.
//!
//! Base URL: `{origin}/api/v4` (see [`PlatformRemote::api_base`]), so a remote on
//! a non-default port keeps that port. Project ids are the URL-encoded full
//! namespace path (`group%2Fsubgroup%2Fproject`) so subgroups work. Responses
//! are parsed as `serde_json::Value` and mapped to the shared models in
//! [`crate::platform::models`].

use std::sync::Arc;

use async_trait::async_trait;
use reqwest::{header, Client, RequestBuilder};
use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;

use crate::platform::auth::resolve_token;
use crate::platform::client::PlatformClient;
use crate::platform::error::PlatformError;
use crate::platform::http::{cached_client, fetch_json, fetch_page};
use crate::platform::models::{
    CheckSummary, IssueDetail, IssueSummary, NotificationSummary, PrDetail, PrSummary,
};
use crate::platform::remote::{PlatformKind, PlatformRemote};

/// Platform label used in error messages.
const PLATFORM: &str = PlatformKind::GitLab.as_str();

/// Hard cap on `/todos` paging.
///
/// GitLab has no project-scoped notification endpoint (only a global `/todos`),
/// so the client pages through the list and filters locally. The cap keeps a
/// pathological `x-next-page` chain (or a huge todo list) from stalling the UI.
const MAX_TODO_PAGES: u32 = 5;

/// GitLab REST API v4 client (async, reqwest-backed).
pub struct GitLabClient {
    http: Arc<Client>,
    remote: PlatformRemote,
    token: SecretString,
    base_url: String,
    /// URL-encoded full project path: `group%2Fsub%2Fproject` (subgroup-safe).
    project_id: String,
}

impl GitLabClient {
    /// Build a client for `remote`, resolving a token via `platform::auth`.
    ///
    /// The `reqwest::Client` (and its connection pool) is shared per host, so
    /// rebuilding this client on every refresh is cheap.
    pub fn new(remote: &PlatformRemote) -> Result<Self, PlatformError> {
        let token = resolve_token(&remote.host, remote.kind)?;
        let http = cached_client(PLATFORM, &remote.host)?;
        Ok(Self {
            http,
            remote: remote.clone(),
            token,
            base_url: remote.api_base("v4"),
            project_id: encode_project_id(&remote.path),
        })
    }

    /// The `PRIVATE-TOKEN` header value for this client.
    fn auth_header(&self) -> Result<header::HeaderValue, PlatformError> {
        auth_header_value(self.token.expose_secret())
    }

    /// Build an authenticated GET request for `path`.
    fn get(&self, path: &str) -> Result<RequestBuilder, PlatformError> {
        let url = format!("{}{}", self.base_url, path);
        Ok(self.http.get(&url).header(
            header::HeaderName::from_static("private-token"),
            self.auth_header()?,
        ))
    }

    /// GET `path` and decode the JSON body.
    async fn fetch(&self, path: &str) -> Result<Value, PlatformError> {
        fetch_json(PLATFORM, self.get(path)?).await
    }
}

/// Build the `PRIVATE-TOKEN` header value.
///
/// GitLab's private-token header is not GitHub/Gitea's
/// `Authorization: token …`, so the two adapters build their headers
/// separately; only the transport lives in `platform::http`.
///
/// A malformed token (CR/LF or other control characters) is an error, never a
/// panic.
fn auth_header_value(token: &str) -> Result<header::HeaderValue, PlatformError> {
    header::HeaderValue::from_str(token)
        .map_err(|e| PlatformError::Token(format!("GitLab token 含非法字符，无法作为请求头：{e}")))
}

/// URL-encode a full namespace path into a GitLab project id.
///
/// GitLab identifies a project by its URL-encoded full path, so
/// `group/sub/project` becomes `group%2Fsub%2Fproject` and subgroups resolve to
/// the right project instead of a sibling group.
fn encode_project_id(path: &str) -> String {
    path.replace('/', "%2F")
}

/// `/projects/{id}/merge_requests` (open MRs).
fn mr_list_path(project_id: &str) -> String {
    format!("/projects/{project_id}/merge_requests?state=opened&per_page=100")
}

/// `/projects/{id}/merge_requests/{iid}`.
fn mr_path(project_id: &str, number: u64) -> String {
    format!("/projects/{project_id}/merge_requests/{number}")
}

/// `/projects/{id}/pipelines?sha=…`, or `None` when `sha` is unknown.
///
/// An empty `sha=` would make GitLab answer with the project's most recent
/// pipeline instead of the MR head's, so the caller must skip the request and
/// report "no checks" rather than show unrelated CI.
fn checks_path(project_id: &str, sha: &str) -> Option<String> {
    if sha.is_empty() {
        return None;
    }
    Some(format!(
        "/projects/{project_id}/pipelines?sha={sha}&per_page=100"
    ))
}

/// `/projects/{id}/issues?state=opened` (project issues exclude MRs).
fn issue_list_path(project_id: &str) -> String {
    format!("/projects/{project_id}/issues?state=opened&per_page=100")
}

/// `/projects/{id}/issues/{iid}`.
fn issue_path(project_id: &str, number: u64) -> String {
    format!("/projects/{project_id}/issues/{number}")
}

/// `/todos?per_page=100&page=…` — global, not project-scoped (see D6).
fn todos_path(page: u32) -> String {
    format!("/todos?per_page=100&page={page}")
}

// ---------------------------------------------------------------------------
// Trait impl
// ---------------------------------------------------------------------------

#[async_trait]
impl PlatformClient for GitLabClient {
    async fn list_pulls(&self) -> Result<Vec<PrSummary>, PlatformError> {
        let json = self.fetch(&mr_list_path(&self.project_id)).await?;
        Ok(json
            .as_array()
            .map(|arr| arr.iter().map(mr_to_summary).collect())
            .unwrap_or_default())
    }

    async fn get_pull(&self, number: u64) -> Result<PrDetail, PlatformError> {
        let json = self.fetch(&mr_path(&self.project_id, number)).await?;
        // Use diff_refs.head_sha for the CI pipeline lookup.
        let sha = json["diff_refs"]["head_sha"].as_str().unwrap_or("");
        // Best-effort checks: a failure here leaves the list empty, not an error,
        // and an unknown head sha skips the request entirely (see `checks_path`).
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
        let Some(path) = checks_path(&self.project_id, sha) else {
            return Ok(Vec::new());
        };
        let json = self.fetch(&path).await?;
        Ok(json
            .as_array()
            .map(|arr| arr.iter().map(pipeline_to_check).collect())
            .unwrap_or_default())
    }

    async fn list_issues(&self) -> Result<Vec<IssueSummary>, PlatformError> {
        // GitLab `/issues` already excludes merge requests — no PR filter needed.
        let json = self.fetch(&issue_list_path(&self.project_id)).await?;
        Ok(json
            .as_array()
            .map(|arr| arr.iter().map(issue_to_summary).collect())
            .unwrap_or_default())
    }

    async fn get_issue(&self, number: u64) -> Result<IssueDetail, PlatformError> {
        let json = self.fetch(&issue_path(&self.project_id, number)).await?;
        Ok(IssueDetail {
            summary: issue_to_summary(&json),
            body: json["description"].as_str().unwrap_or("").to_string(),
        })
    }

    async fn list_notifications(&self) -> Result<Vec<NotificationSummary>, PlatformError> {
        // GitLab has no repo-scoped notification endpoint: `/todos` is the whole
        // user's todo list. Follow the `x-next-page` cursor for at most
        // MAX_TODO_PAGES hops (so the panel never stalls) and filter to this
        // project client-side, comparing against the remote's full
        // `group/sub/repo` path.
        let full = self.remote.path.clone();
        let mut items: Vec<NotificationSummary> = Vec::new();
        let mut page = Some(1u32);
        let mut hops = 0u32;
        while let Some(current) = page {
            if hops >= MAX_TODO_PAGES {
                break;
            }
            hops += 1;
            let body = fetch_page(PLATFORM, self.get(&todos_path(current))?).await?;
            if let Some(arr) = body.body.as_array() {
                items.extend(arr.iter().map(todo_to_notification));
            }
            // Only follow a strictly forward cursor: a stale/equal header would
            // otherwise loop forever inside the hop budget.
            page = body
                .next_page
                .as_deref()
                .and_then(|v| v.parse::<u32>().ok())
                .filter(|&next| next > current);
        }
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

/// Display name for a pipeline row.
///
/// GitLab added a user-defined pipeline `name`; older instances only report
/// `source` (`push`, `web`, …), so fall back to the numeric id and then to
/// `source` rather than showing an empty column.
fn pipeline_name(v: &Value) -> String {
    if let Some(name) = v["name"].as_str().filter(|s| !s.is_empty()) {
        return name.to_string();
    }
    if let Some(id) = v["id"]
        .as_u64()
        .or_else(|| v["id"].as_i64().map(|n| n as u64))
    {
        return id.to_string();
    }
    v["source"].as_str().unwrap_or("").to_string()
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
        name: pipeline_name(v),
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
/// Field mapping follows GitLab's documented todo response:
///   - `repo` comes from the top-level `project.path_with_namespace`; `target` is
///     the Issue/MR itself and carries no namespace.
///   - `reason` comes from `action_name` (`assigned`, `directly_addressed`, …);
///     `action` is the older name and only a fallback.
///   - `subject_title` prefers the target's own `title` and falls back to the
///     todo `body`, which is free-form message text and may span several lines —
///     it is passed through verbatim rather than reduced to its first line.
///   - `subject_type` derives from `target_type` (`MergeRequest` →
///     `PullRequest`) so the panel's styling matches the other platforms.
fn todo_to_notification(v: &Value) -> NotificationSummary {
    let subject_title = v["target"]["title"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| v["body"].as_str().filter(|s| !s.is_empty()))
        .unwrap_or("")
        .to_string();
    let subject_type = match v["target_type"].as_str().unwrap_or("") {
        "MergeRequest" => "PullRequest",
        "Issue" => "Issue",
        other => other,
    }
    .to_string();
    NotificationSummary {
        id: v["id"]
            .as_i64()
            .map(|n| n.to_string())
            .or_else(|| v["id"].as_str().map(|s| s.to_string()))
            .unwrap_or_default(),
        repo: v["project"]["path_with_namespace"]
            .as_str()
            .unwrap_or("")
            .to_string(),
        subject_title,
        subject_type,
        reason: v["action_name"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| v["action"].as_str())
            .unwrap_or("")
            .to_string(),
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
    use std::io::{Read, Write};

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
            "name": "CI/CD pipeline",
            "source": "push",
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

    /// Shape per GitLab's documented `GET /todos` response: the project is a
    /// top-level object and the todo `body` is free-form message text.
    fn todo_json() -> Value {
        json!({
            "id": 102i64,
            "project": {
                "id": 1u64,
                "name": "Gitlab Test",
                "name_with_organization": "Gitlab Org / Gitlab Test",
                "path": "gitlab-test",
                "path_with_namespace": "gitlab-org/gitlab-test",
                "created_at": "2024-06-03T00:00:00Z"
            },
            "author": { "id": 1u64, "username": "john_doe", "name": "John Doe" },
            "action_name": "assigned",
            "target_type": "Issue",
            "target": {
                "id": 173u64,
                "iid": 1u64,
                "project_id": 6u64,
                "title": "Issues with auth",
                "description": "Fix login",
                "state": "opened",
                "web_url": "https://gitlab.com/gitlab-org/gitlab-test/issues/1"
            },
            "target_url": "https://gitlab.com/gitlab-org/gitlab-test/issues/1",
            "body": "test",
            "state": "pending",
            "created_at": "2024-06-03T00:00:00Z",
            "updated_at": "2024-06-03T00:00:00Z"
        })
    }

    #[test]
    fn encode_project_id_covers_subgroup_paths() {
        assert_eq!(encode_project_id("group/repo"), "group%2Frepo");
        assert_eq!(encode_project_id("group/sub/repo"), "group%2Fsub%2Frepo");
        assert_eq!(encode_project_id("a/b/c/repo"), "a%2Fb%2Fc%2Frepo");
    }

    #[test]
    fn new_builds_base_url_from_the_remote_origin() {
        if std::env::var("GITLAB_TOKEN").is_err() {
            std::env::set_var("GITLAB_TOKEN", "test-token");
        }
        let remote = crate::platform::parse_remote("https://gitlab.com/group/sub/repo")
            .expect("parse remote");
        let client = GitLabClient::new(&remote).expect("gitlab client");
        assert_eq!(client.base_url, "https://gitlab.com/api/v4");
        assert_eq!(client.base_url, remote.api_base("v4"));
        assert_eq!(client.project_id, "group%2Fsub%2Frepo");
    }

    #[test]
    fn new_keeps_a_non_default_port_in_the_base_url() {
        if std::env::var("GITLAB_TOKEN").is_err() {
            std::env::set_var("GITLAB_TOKEN", "test-token");
        }
        // A self-hosted host is rejected by `build_client`, but the adapter must
        // still derive its base URL from the remote (scheme + host + port).
        let mut remote = crate::platform::parse_remote("https://gitlab.example.com:8443/o/r")
            .expect("parse remote");
        remote.kind = PlatformKind::GitLab;
        let client = GitLabClient::new(&remote).expect("gitlab client");
        assert_eq!(client.base_url, "https://gitlab.example.com:8443/api/v4");
    }

    #[test]
    fn mr_and_issue_paths_use_the_encoded_project_id() {
        let id = encode_project_id("group/sub/repo");
        assert_eq!(
            mr_list_path(&id),
            "/projects/group%2Fsub%2Frepo/merge_requests?state=opened&per_page=100"
        );
        assert_eq!(
            mr_path(&id, 15),
            "/projects/group%2Fsub%2Frepo/merge_requests/15"
        );
        assert_eq!(
            issue_list_path(&id),
            "/projects/group%2Fsub%2Frepo/issues?state=opened&per_page=100"
        );
        assert_eq!(issue_path(&id, 3), "/projects/group%2Fsub%2Frepo/issues/3");
    }

    #[test]
    fn issue_list_requests_state_opened_not_scope() {
        // `scope=opened` is not a legal GitLab value (legal: created_by_me,
        // assigned_to_me, all), so the filter must use `state`.
        let path = issue_list_path(&encode_project_id("group/repo"));
        assert!(path.contains("state=opened"), "unexpected: {path}");
        assert!(!path.contains("scope="), "unexpected: {path}");
    }

    #[test]
    fn checks_path_requires_a_sha() {
        let id = encode_project_id("group/repo");
        assert_eq!(
            checks_path(&id, "abc123def"),
            Some("/projects/group%2Frepo/pipelines?sha=abc123def&per_page=100".to_string())
        );
        // No head sha: skipping beats asking for an unrelated pipeline.
        assert_eq!(checks_path(&id, ""), None);
    }

    #[test]
    fn list_checks_is_empty_without_a_sha() {
        if std::env::var("GITLAB_TOKEN").is_err() {
            std::env::set_var("GITLAB_TOKEN", "test-token");
        }
        let remote =
            crate::platform::parse_remote("https://gitlab.com/group/repo").expect("parse remote");
        let client = GitLabClient::new(&remote).expect("gitlab client");
        let checks = block_on(client.list_checks("")).expect("no request, no error");
        assert!(checks.is_empty());
    }

    #[test]
    fn todos_path_carries_the_page_cursor() {
        assert_eq!(todos_path(1), "/todos?per_page=100&page=1");
        assert_eq!(todos_path(5), "/todos?per_page=100&page=5");
    }

    #[test]
    fn max_todo_pages_bounds_the_pagination() {
        assert_eq!(
            MAX_TODO_PAGES, 5,
            "the pagination cap is part of the contract"
        );
    }

    #[test]
    fn list_notifications_follows_the_cursor_up_to_the_page_cap() {
        let origin = spawn_todo_server();
        let mut remote = crate::platform::parse_remote("https://gitlab.com/group/repo").unwrap();
        remote.scheme = "http".to_string();
        remote.host = origin.trim_start_matches("http://").to_string();
        remote.port = None;
        let client = GitLabClient {
            http: cached_client(PLATFORM, &remote.host).expect("client"),
            token: SecretString::new("t".to_string().into_boxed_str()),
            base_url: origin,
            project_id: encode_project_id(&remote.path),
            remote,
        };

        let notes = block_on(client.list_notifications()).expect("todos");
        // Every page holds one todo for this repo plus one for another repo, so a
        // repo leak or a page drop is visible in the counts.
        assert_eq!(notes.len(), MAX_TODO_PAGES as usize, "one todo per page");
        assert!(
            notes.iter().all(|n| n.repo == "group/repo"),
            "todos from other projects leaked in"
        );
        assert_eq!(notes[0].id, "1");
        assert_eq!(notes[1].id, "2");
    }

    /// Serves `/todos` pages 1..=8 (each advertising a next page) so the client
    /// must stop at [`MAX_TODO_PAGES`].
    fn spawn_todo_server() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind localhost");
        let addr = listener.local_addr().expect("local addr");
        std::thread::spawn(move || {
            for page in 1..=8u32 {
                let Ok((mut sock, _)) = listener.accept() else {
                    return;
                };
                let mut buf = [0u8; 8192];
                let _ = sock.read(&mut buf);
                let body = format!(
                    r#"[{{"id":{page},"project":{{"path_with_namespace":"group/repo"}},"action_name":"assigned","target_type":"Issue","target":{{"title":"t{page}"}},"state":"pending","updated_at":"2024-01-01T00:00:00Z"}},{{"id":{other},"project":{{"path_with_namespace":"other/repo"}},"action_name":"assigned","target_type":"Issue","target":{{"title":"x"}},"state":"pending","updated_at":"2024-01-01T00:00:00Z"}}]"#,
                    other = page * 100
                );
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nx-next-page: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    page + 1,
                    body.len()
                );
                let _ = sock.write_all(resp.as_bytes());
            }
        });
        format!("http://{addr}")
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
        assert_eq!(c.name, "CI/CD pipeline");
        assert_eq!(c.status, "completed");
        assert_eq!(c.conclusion, "success");
        assert!(c.html_url.contains("pipelines/100"));
    }

    #[test]
    fn pipeline_to_check_running() {
        let v = json!({ "id": 7u64, "status": "running", "source": "push", "web_url": "" });
        let c = pipeline_to_check(&v);
        assert_eq!(c.status, "in_progress");
        assert_eq!(c.conclusion, "unknown");
    }

    #[test]
    fn pipeline_to_check_failed() {
        let v = json!({ "id": 7u64, "status": "failed", "source": "web", "web_url": "" });
        let c = pipeline_to_check(&v);
        assert_eq!(c.status, "completed");
        assert_eq!(c.conclusion, "failure");
    }

    #[test]
    fn pipeline_name_falls_back_from_name_to_id_to_source() {
        assert_eq!(
            pipeline_name(&json!({ "name": "Nightly", "id": 1u64, "source": "web" })),
            "Nightly"
        );
        // Empty name falls through to the id.
        assert_eq!(pipeline_name(&json!({ "name": "", "id": 42u64 })), "42");
        assert_eq!(pipeline_name(&json!({ "id": 42u64 })), "42");
        // No id either (string ids are not expected, but must not panic).
        assert_eq!(pipeline_name(&json!({ "source": "push" })), "push");
        assert_eq!(pipeline_name(&Value::Null), "");
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
        assert_eq!(n.id, "102");
        assert!(n.unread);
        assert_eq!(n.subject_type, "Issue");
        // target.title wins over the free-form body.
        assert_eq!(n.subject_title, "Issues with auth");
        assert_eq!(n.reason, "assigned");
        assert_eq!(n.updated_at, "2024-06-03T00:00:00Z");
        // The project (not the target) carries the full namespace path.
        assert_eq!(n.repo, "gitlab-org/gitlab-test");
    }

    #[test]
    fn todo_to_notification_mr_type() {
        let mut v = todo_json();
        v["target_type"] = json!("MergeRequest");
        let n = todo_to_notification(&v);
        assert_eq!(n.subject_type, "PullRequest");
    }

    #[test]
    fn todo_to_notification_reads_a_subgroup_project_path() {
        let mut v = todo_json();
        v["project"]["path_with_namespace"] = json!("group/sub/repo");
        let n = todo_to_notification(&v);
        assert_eq!(n.repo, "group/sub/repo");
    }

    #[test]
    fn todo_to_notification_falls_back_to_body_when_target_has_no_title() {
        let mut v = todo_json();
        v["target"]["title"] = Value::Null;
        v["body"] = json!("mentioned you in\nan issue comment");
        let n = todo_to_notification(&v);
        // Multi-line body is passed through verbatim, not clipped to line 1.
        assert_eq!(n.subject_title, "mentioned you in\nan issue comment");
    }

    #[test]
    fn todo_to_notification_keeps_the_whole_body_when_both_are_empty() {
        let mut v = todo_json();
        v["target"]["title"] = Value::Null;
        v["body"] = Value::Null;
        let n = todo_to_notification(&v);
        assert_eq!(n.subject_title, "");
    }

    #[test]
    fn todo_to_notification_reason_prefers_action_name() {
        let mut v = todo_json();
        v["action"] = json!("mentioned");
        v["action_name"] = json!("directly_addressed");
        let n = todo_to_notification(&v);
        assert_eq!(n.reason, "directly_addressed");

        // Older instances only report `action`.
        v["action_name"] = Value::Null;
        let n = todo_to_notification(&v);
        assert_eq!(n.reason, "mentioned");

        v["action"] = Value::Null;
        let n = todo_to_notification(&v);
        assert_eq!(n.reason, "");
    }

    #[test]
    fn todo_to_notification_tolerates_a_missing_project() {
        let mut v = todo_json();
        v["project"] = Value::Null;
        let n = todo_to_notification(&v);
        assert_eq!(n.repo, "");
    }

    #[test]
    fn auth_header_value_rejects_control_characters() {
        assert!(auth_header_value("glpat-x").is_ok());
        let err = auth_header_value("bad\ntoken").expect_err("a newline must not be accepted");
        match err {
            PlatformError::Token(m) => assert!(m.contains("非法字符"), "unexpected: {m}"),
            other => panic!("expected Token error, got {other:?}"),
        }
        assert!(matches!(
            auth_header_value("bad\rtoken"),
            Err(PlatformError::Token(_))
        ));
    }

    #[test]
    fn get_rejects_a_malformed_token_instead_of_panicking() {
        let remote = crate::platform::parse_remote("https://gitlab.com/group/repo").unwrap();
        let client = GitLabClient {
            http: cached_client(PLATFORM, "gitlab.com").expect("client"),
            remote: remote.clone(),
            token: SecretString::new("bad\ntoken".to_string().into_boxed_str()),
            base_url: remote.api_base("v4"),
            project_id: encode_project_id(&remote.path),
        };
        let err = client.get("/todos").expect_err("malformed token must fail");
        assert!(matches!(err, PlatformError::Token(_)));
    }

    /// Minimal runtime for the one non-mapper test that calls into an async fn.
    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime")
            .block_on(f)
    }
}
