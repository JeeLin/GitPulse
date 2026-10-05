//! Gitea platform client: REST API implementation of [`PlatformClient`].
//!
//! Uses reqwest directly (the `gitea-rs` crate is stale). Auth is the GitHub-compatible
//! `Authorization: token {token}` scheme, resolved via [`crate::platform::auth`].
//!
//! Base URL: `{origin}/api/v1` (see [`PlatformRemote::api_base`]). Responses are
//! parsed as `serde_json::Value` and mapped to the shared models in
//! [`crate::platform::models`].

use std::sync::Arc;

use async_trait::async_trait;
use reqwest::{header, Client, RequestBuilder};
use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;

use crate::platform::auth::resolve_token;
use crate::platform::client::PlatformClient;
use crate::platform::error::PlatformError;
use crate::platform::http::{cached_client, fetch_json};
use crate::platform::models::{
    CheckSummary, IssueDetail, IssueSummary, NotificationSummary, PrDetail, PrSummary,
};
use crate::platform::remote::{PlatformKind, PlatformRemote};

/// Platform label used in error messages.
const PLATFORM: &str = PlatformKind::Gitea.as_str();

/// Gitea REST API v1 client (async, reqwest-backed).
pub struct GiteaClient {
    http: Arc<Client>,
    remote: PlatformRemote,
    token: SecretString,
    base_url: String,
}

impl GiteaClient {
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
            base_url: remote.api_base("v1"),
        })
    }

    /// The `Authorization: token {token}` header value for this client.
    fn auth_header(&self) -> Result<header::HeaderValue, PlatformError> {
        auth_header_value(self.token.expose_secret())
    }

    /// Build an authenticated GET request for `path`.
    fn get(&self, path: &str) -> Result<RequestBuilder, PlatformError> {
        let url = format!("{}{}", self.base_url, path);
        Ok(self
            .http
            .get(&url)
            .header(header::AUTHORIZATION, self.auth_header()?))
    }

    /// GET `path` and decode the JSON body.
    async fn fetch(&self, path: &str) -> Result<Value, PlatformError> {
        fetch_json(PLATFORM, self.get(path)?).await
    }
}

/// Build the `Authorization: token {token}` header value.
///
/// Gitea shares GitHub's `Authorization: token …` scheme while GitLab uses
/// `PRIVATE-TOKEN`, so the two adapters build their headers separately; only the
/// transport lives in `platform::http`.
///
/// A malformed token (CR/LF or other control characters) is an error, never a
/// panic.
fn auth_header_value(token: &str) -> Result<header::HeaderValue, PlatformError> {
    header::HeaderValue::from_str(&format!("token {token}"))
        .map_err(|e| PlatformError::Token(format!("Gitea token 含非法字符，无法作为请求头：{e}")))
}

/// Repo-scoped notification endpoint.
///
/// Gitea ships a repo-scoped variant (`GET /repos/{owner}/{repo}/notifications`),
/// so the panel never has to download the whole user's notification list.
fn notifications_path(owner: &str, repo: &str) -> String {
    format!("/repos/{owner}/{repo}/notifications?limit=100")
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
        let json = self.fetch(&path).await?;
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
        let json = self.fetch(&path).await?;
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
        let json = self.fetch(&path).await?;
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
        let json = self.fetch(&path).await?;
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
        let json = self.fetch(&path).await?;
        Ok(IssueDetail {
            summary: issue_to_summary(&json),
            body: json["body"].as_str().unwrap_or("").to_string(),
        })
    }

    async fn list_notifications(&self) -> Result<Vec<NotificationSummary>, PlatformError> {
        let path = notifications_path(&self.remote.owner, &self.remote.repo);
        let json = self.fetch(&path).await?;
        // The repo-scoped endpoint already narrows to this repository upstream,
        // so no client-side filter: a `full_name` that differs in case from the
        // remote path would otherwise drop every row.
        Ok(json
            .as_array()
            .map(|arr| arr.iter().map(notification_to_summary).collect())
            .unwrap_or_default())
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
    // RESEARCH §1.7: `state` ∈ pending/running/success/failure/error. The two
    // in-flight states share GitHub's `in_progress` shape so the renderer keeps
    // painting them as work in flight instead of DarkGray `unknown`.
    let (status, conclusion) = match state {
        "pending" | "running" => ("in_progress".to_string(), "unknown".to_string()),
        "success" => ("completed".to_string(), "success".to_string()),
        "failure" | "error" => ("completed".to_string(), "failure".to_string()),
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
        id: notification_id(&v["id"]),
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

/// Gitea notification thread ids are `int64`; accept a numeric or string id.
fn notification_id(v: &Value) -> String {
    v.as_i64()
        .map(|i| i.to_string())
        .or_else(|| v.as_str().map(|s| s.to_string()))
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Unit tests for mappers
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Mutex;

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

    /// Shape per Gitea's `NotificationThread` (int64 `id`, no `reason` field).
    fn notification_json() -> Value {
        json!({
            "id": 1234i64,
            "unread": true,
            "pinned": false,
            "subject": { "type": "PullRequest", "title": "PR #1", "state": "open" },
            "repository": { "full_name": "owner/repo" },
            "updated_at": "2024-01-04T00:00:00Z"
        })
    }

    /// `pr_json` under a different number, so a list test can tell "every row
    /// arrived" from "a row was dropped".
    fn pr_json_numbered(number: u64) -> Value {
        let mut v = pr_json();
        v["number"] = json!(number);
        v["title"] = json!(format!("PR {number}"));
        v
    }

    /// The request target [`GiteaClient`] builds for `owner`/`repo`: the API
    /// base, the remote namespace, then `tail` (carrying any query string).
    fn target(owner: &str, repo: &str, tail: &str) -> String {
        format!("/api/v1/repos/{owner}/{repo}/{tail}")
    }

    /// A minimal `200 OK` JSON response.
    fn ok_response(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    /// Serve `routes` (request target → JSON body) on a random localhost port,
    /// answering up to eight requests and recording every request target so a
    /// test can assert the exact URL the adapter asked for. An unknown target
    /// gets a `404` instead of an empty body, so a wrong path fails loudly
    /// rather than looking like an empty panel.
    fn spawn_server(routes: Vec<(String, String)>) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind localhost");
        let addr = listener.local_addr().expect("local addr");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&seen);
        std::thread::spawn(move || {
            for _ in 0..8 {
                let Ok((mut sock, _)) = listener.accept() else {
                    return;
                };
                let mut buf = [0u8; 8192];
                let read = sock.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..read]).to_string();
                let asked = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or_default()
                    .to_string();
                recorded
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(asked.clone());
                let resp = match routes.iter().find(|(route, _)| *route == asked) {
                    Some((_, body)) => ok_response(body),
                    None => {
                        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_string()
                    }
                };
                let _ = sock.write_all(resp.as_bytes());
            }
        });
        (format!("http://{addr}"), seen)
    }

    /// A [`GiteaClient`] whose base URL points at `origin` while its remote
    /// namespace still comes from `remote_url`, so requests keep their real
    /// `/api/v1/repos/{owner}/{repo}/…` shape.
    fn client_for(origin: &str, remote_url: &str) -> GiteaClient {
        let mut remote = crate::platform::parse_remote(remote_url).expect("parse remote");
        remote.scheme = "http".to_string();
        remote.host = origin.trim_start_matches("http://").to_string();
        remote.port = None;
        GiteaClient {
            http: cached_client(PLATFORM, &remote.host).expect("client"),
            remote: remote.clone(),
            token: SecretString::new("t".to_string().into_boxed_str()),
            base_url: remote.api_base("v1"),
        }
    }

    /// The request targets the server recorded, in order.
    fn requested(seen: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
        seen.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Minimal runtime for the tests that call into an async trait method.
    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime")
            .block_on(f)
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
    fn check_to_summary_running_is_in_progress_not_unknown() {
        let c = check_to_summary(&json!({ "state": "running", "context": "ci/x" }));
        assert_eq!(c.status, "in_progress");
        assert_eq!(c.conclusion, "unknown");
    }

    /// Every `state` Gitea documents (RESEARCH §1.7) must reach a real
    /// status/conclusion pair — a new one would silently render as `unknown`.
    #[test]
    fn check_to_summary_covers_every_documented_state() {
        for state in ["pending", "running", "success", "failure", "error"] {
            let c = check_to_summary(&json!({ "state": state, "context": "ci/x" }));
            assert!(
                !(c.status == "unknown" && c.conclusion == "unknown"),
                "{state} fell through to unknown/unknown"
            );
        }
        // Undocumented / missing values stay on the defensive fallback.
        let c = check_to_summary(&json!({ "state": "wat", "context": "ci/x" }));
        assert_eq!(
            (c.status.as_str(), c.conclusion.as_str()),
            ("unknown", "unknown")
        );
        let c = check_to_summary(&json!({ "context": "ci/x" }));
        assert_eq!(
            (c.status.as_str(), c.conclusion.as_str()),
            ("unknown", "unknown")
        );
    }

    #[test]
    fn check_to_summary_failure() {
        let v = json!({ "state": "failure", "context": "ci/x" });
        let c = check_to_summary(&v);
        assert_eq!(c.status, "completed");
        assert_eq!(c.conclusion, "failure");
        // `error` is the other documented failure state.
        let v = json!({ "state": "error", "context": "ci/x" });
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
        assert_eq!(n.id, "1234");
        assert!(n.unread);
        assert_eq!(n.repo, "owner/repo");
        assert_eq!(n.subject_type, "PullRequest");
        assert_eq!(n.subject_title, "PR #1");
    }

    #[test]
    fn notification_id_accepts_numbers_strings_and_missing() {
        assert_eq!(notification_id(&json!(4321)), "4321");
        assert_eq!(notification_id(&json!("nt-1")), "nt-1");
        assert_eq!(notification_id(&Value::Null), "");
    }

    #[test]
    fn notifications_path_is_repo_scoped() {
        assert_eq!(
            notifications_path("owner", "repo"),
            "/repos/owner/repo/notifications?limit=100"
        );
        // Built from the parsed remote, so the endpoint names this repository:
        // that upstream narrowing is what replaces the client-side filter.
        let remote = crate::platform::parse_remote("https://gitea.com/Owner/Repo.git").unwrap();
        assert_eq!(
            notifications_path(&remote.owner, &remote.repo),
            "/repos/Owner/Repo/notifications?limit=100"
        );
        assert_eq!(
            remote.path, "Owner/Repo",
            "the .git suffix must not reach the repo-scoped path"
        );
    }

    #[test]
    fn auth_header_value_rejects_control_characters() {
        assert!(auth_header_value("t0ken").is_ok());
        let err = auth_header_value("bad\ntoken").expect_err("a newline must not be accepted");
        match err {
            PlatformError::Token(m) => assert!(m.contains("非法字符"), "unexpected: {m}"),
            other => panic!("expected Token error, got {other:?}"),
        }
        let err = auth_header_value("bad\rtoken").expect_err("a CR must not be accepted");
        assert!(matches!(err, PlatformError::Token(_)));
    }

    #[test]
    fn new_uses_the_parsed_api_base() {
        if std::env::var("GITEA_TOKEN").is_err() {
            std::env::set_var("GITEA_TOKEN", "test-token");
        }
        let remote = crate::platform::parse_remote("https://gitea.com/owner/repo").unwrap();
        let client = GiteaClient::new(&remote).expect("gitea client");
        assert_eq!(client.base_url, "https://gitea.com/api/v1");
        assert_eq!(client.base_url, remote.api_base("v1"));
    }

    #[test]
    fn get_rejects_a_malformed_token_instead_of_panicking() {
        let remote = crate::platform::parse_remote("https://gitea.com/owner/repo").unwrap();
        let client = GiteaClient {
            http: cached_client(PLATFORM, "gitea.com").expect("client"),
            remote: remote.clone(),
            token: SecretString::new("bad\ntoken".to_string().into_boxed_str()),
            base_url: remote.api_base("v1"),
        };
        let err = client.get("/x").expect_err("malformed token must fail");
        assert!(matches!(err, PlatformError::Token(_)));
    }

    /// Gitea reports `repository.full_name` in the repository's own casing,
    /// which need not match the remote URL. Filtering the response on
    /// `full_name` therefore drops every row for a remote spelled
    /// `Owner/Repo` while the server answers `owner/repo`, and the panel goes
    /// empty — the bug this repo-scoped endpoint was adopted to fix. This test
    /// fails the moment a `full_name` filter (case-sensitive or not) is added
    /// back, which is the regression it guards.
    #[test]
    fn list_notifications_keeps_a_thread_whose_repo_case_differs() {
        let mut thread = notification_json();
        thread["repository"]["full_name"] = json!("owner/repo");
        let notifications = target("Owner", "Repo", "notifications?limit=100");
        let (origin, seen) =
            spawn_server(vec![(notifications.clone(), json!([thread]).to_string())]);
        // A `.git` remote too: the suffix must not reach the repo-scoped path.
        let client = client_for(&origin, "https://gitea.com/Owner/Repo.git");

        let notes = block_on(client.list_notifications()).expect("notifications");
        assert_eq!(
            notes.len(),
            1,
            "a full_name differing only in case must not drop the row"
        );
        assert_eq!(notes[0].repo, "owner/repo");
        assert_eq!(notes[0].subject_title, "PR #1");
        assert!(notes[0].unread);
        // The repo-scoped endpoint is what replaces the client-side filter, so
        // the request itself must name this repository.
        assert_eq!(requested(&seen), vec![notifications]);
    }

    #[test]
    fn list_pulls_asks_for_this_repo_open_pulls() {
        let pulls = target("Owner", "Repo", "pulls?state=open&limit=100");
        let body = json!([pr_json_numbered(42), pr_json_numbered(43)]).to_string();
        let (origin, seen) = spawn_server(vec![(pulls.clone(), body)]);
        let client = client_for(&origin, "https://gitea.com/Owner/Repo");

        let rows = block_on(client.list_pulls()).expect("pulls");
        assert_eq!(rows.len(), 2, "every returned PR must survive mapping");
        assert_eq!(rows.iter().map(|p| p.number).collect::<Vec<_>>(), [42, 43]);
        // `state=open` is what keeps closed PRs off the panel and `limit=100`
        // the page size; both are part of the request contract.
        assert_eq!(requested(&seen), vec![pulls]);
    }

    #[test]
    fn list_issues_asks_for_open_issues_and_drops_pull_request_rows() {
        let mut pr_row = issue_json();
        pr_row["number"] = json!(9);
        pr_row["pull_request"] = json!({ "merged": false });
        let issues = target("Owner", "Repo", "issues?state=open&limit=100");
        let body = json!([issue_json(), pr_row]).to_string();
        let (origin, seen) = spawn_server(vec![(issues.clone(), body)]);
        let client = client_for(&origin, "https://gitea.com/Owner/Repo");

        let rows = block_on(client.list_issues()).expect("issues");
        // Gitea's issue list also carries PRs; they belong to the PR panel.
        assert_eq!(rows.len(), 1, "a PR row leaked into the issue panel");
        assert_eq!(rows[0].number, 7);
        assert_eq!(rows[0].labels, vec!["bug".to_string()]);
        assert_eq!(requested(&seen), vec![issues]);
    }

    #[test]
    fn get_pull_asks_for_the_numbered_pull_and_its_commit_status() {
        let pull = target("Owner", "Repo", "pulls/42");
        // `pr_json`'s head sha drives the best-effort status lookup; that call
        // swallows its errors, so the second URL is asserted rather than left
        // to an empty check list.
        let status = target("Owner", "Repo", "commits/abc123/status");
        let body = json!({ "statuses": [check_json()] }).to_string();
        let (origin, seen) = spawn_server(vec![
            (pull.clone(), pr_json().to_string()),
            (status.clone(), body),
        ]);
        let client = client_for(&origin, "https://gitea.com/Owner/Repo");

        let detail = block_on(client.get_pull(42)).expect("pull");
        assert_eq!(detail.summary.number, 42);
        assert_eq!(detail.body, "description");
        assert_eq!(
            detail.labels,
            vec!["bug".to_string(), "critical".to_string()]
        );
        assert_eq!(detail.mergeable, Some(true));
        assert_eq!(detail.checks.len(), 1);
        assert_eq!(detail.checks[0].name, "ci/test");
        assert_eq!(requested(&seen), vec![pull, status]);
    }

    #[test]
    fn get_issue_asks_for_the_numbered_issue() {
        let issue = target("Owner", "Repo", "issues/7");
        let (origin, seen) = spawn_server(vec![(issue.clone(), issue_json().to_string())]);
        let client = client_for(&origin, "https://gitea.com/Owner/Repo");

        let detail = block_on(client.get_issue(7)).expect("issue");
        assert_eq!(detail.summary.number, 7);
        assert_eq!(detail.summary.title, "Bug report");
        assert_eq!(detail.summary.assignee.as_deref(), Some("carol"));
        assert_eq!(requested(&seen), vec![issue]);
    }

    #[test]
    fn list_checks_asks_for_the_commit_status_endpoint() {
        let status = target("Owner", "Repo", "commits/abc123/status");
        let body = json!({ "statuses": [check_json()] }).to_string();
        let (origin, seen) = spawn_server(vec![(status.clone(), body)]);
        let client = client_for(&origin, "https://gitea.com/Owner/Repo");

        let checks = block_on(client.list_checks("abc123")).expect("checks");
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].name, "ci/test");
        assert_eq!(checks[0].status, "completed");
        assert_eq!(checks[0].conclusion, "success");
        assert_eq!(requested(&seen), vec![status]);
    }
}
