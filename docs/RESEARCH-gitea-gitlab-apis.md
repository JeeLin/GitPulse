# Research: Gitea & GitLab REST APIs for Multi-Platform Git Remote Receipts

**Goal:** Extend GitPulse (currently GitHub-only via `octocrab`) to also fetch
PRs/MRs, issues, CI-check status, and notifications from **Gitea** and **GitLab**,
and to resolve tokens/auth the same "env first → CLI fall-back" pattern the app
uses for GitHub (`GITHUB_TOKEN` → `gh auth token`).

**Method:** Live OpenAPI specs fetched from the running instances, official docs
(docs.gitlab.com, docs.gitea.com) queried via Context7, and real Rust client
implementations mined from GitHub (`git-ai`, `Tabby`, `self_update`,
`gitlab-tracker`, `gitea-go-sdk`).

> TL;DR — **reqwest is the right tool for Gitea** (no maintained Rust SDK); for
> **GitLab** the typed `gitlab` crate (rust-gitlab) is mature and covers the 6
> needed calls, but `reqwest` is a fine uniform baseline. **octocrab is GitHub-only.**

---

## 0. How this maps to GitPulse today

`src/github/client.rs` exposes a 6-call contract behind `GithubClient`:

| method | GitHub (octocrab) |
|---|---|
| `list_pulls` | open PRs (100/page) |
| `get_pull(n)` | PR detail + head-SHA check runs |
| `list_issues` | open issues, **PRs filtered out** (`pull_request.is_none()`) |
| `get_issue(n)` | issue detail (summary + markdown body) |
| `list_notifications` | repo-scoped notification threads |
| `list_checks(sha)` | check runs for a commit |

Models in `src/github/models.rs`: `PrSummary`, `PrDetail`, `IssueSummary`,
`IssueDetail`, `NotificationSummary`, `CheckSummary`.

**Key design takeaway to mirror:** each platform client should expose the same
6 call signatures and map into the *same* summary/detail model enum (so the TUI
panel code is provider-agnostic).

---

## 1. Gitea API

**Live spec source:** `https://gitea.com/swagger.v1.json` (342 paths).
**SDK source:** `gitea.com/gitea/go-sdk` (mirrored as `github.com/Madh93/gitea-go-sdk`).
**REST is deliberately GitHub-compatible** — see `docs/guidelines-backend.md`:
"Gitea's API should use the same endpoints and fields as the GitHub API where
possible … Existing fields should not be removed …"
This means the existing octocrab model mapping transfers almost verbatim.

### 1.1 Base URL

```
https://<host>/api/v1
```
Examples: `https://gitea.com/api/v1`, `https://codeberg.org/api/v1`,
`https://git.example.com/api/v1`.

⚠️ **Self-hosted caveat:** Gitea honours `AppSubURL`/`ROOT_URL`, so the API may
live under a sub-path: `https://<host>/<subpath>/api/v1`. The `APIURL()` helper
builds `AppSubURL + "/api/v1/repos/..."` (`models/repo/repo.go`). The `tea`
CLI likewise lets you point at any base URL per "login".

### 1.2 Authentication

- Header: `Authorization: token <token>` — **confirmed in source**
  `gitea-go-sdk/gitea/client.go:352`:
  ```go
  if len(c.accessToken) != 0 {
      req.Header.Set("Authorization", "token "+c.accessToken)
  }
  ```
  (identical scheme to GitHub's PATs).
- OAuth2: also accepts `Authorization: Bearer <token>` — confirmed in
  `go-gitea/gitea/services/auth/oauth2.go` `parseToken`: it reads the
  `Authorization` header via `httpauth.ParseAuthorizationHeader` and uses
  `parsed.BearerToken`.
- Query-param fallback: `?access_token=<token>` (only when
  `DISABLE_QUERY_AUTH_TOKEN=false`).
- **Env var convention:** `GITEA_TOKEN` — found in `jaemk/self_update`,
  `updatecli`, `ghorg`, and Gitea itself sets `GITEA_TOKEN` (identical to
  `GITHUB_TOKEN`) in Actions task context
  (`go-gitea/gitea/services/actions/task.go`).
- MFA/OTP: optional `X-GITEA-OTP` header.
- **CLI tool:** official `tea` — `gitea.com/gitea/tea` (`"A command line tool to
  interact with Gitea servers"`). Uses `gitea.dev/sdk`. Tokens are added via
  `tea login add` (stored in keyring/config, ~no single global env var the way
  `gh` exposes `GITHUB_TOKEN`). For a TUI, prefer `GITEA_TOKEN` env → fall back
  to `tea` (read `tea`'s stored token? not standard; document env only).

### 1.3 List pull requests

```
GET /repos/{owner}/{repo}/pulls
```
Query: `state` (`open`/`closed`/`all`), `base_branch`, `sort`
(`created_at`/`updated_at`/`index`/`longitudinally`), `milestone`,
`labels`, `poster`, `page`, `limit` (default 30, max 50 on gitea.com).
Returns `PullRequest[]`.

**Response shape** (`definitions.PullRequest` from swagger):

| field | type | notes |
|---|---|---|
| `number` | int64 | PR number (GitHub-style — **this is the "index"**) |
| `id` | int64 | global DB id |
| `title` | string | |
| `body` | string | markdown |
| `state` | string | `open`/`closed` |
| `draft` | boolean | ✓ |
| `mergeable` | boolean | |
| `merged` | boolean | |
| `head` | `PRBranchInfo` | `{label, ref, sha, repo, repo_id}` |
| `base` | `PRBranchInfo` | `{label, ref, sha, repo, repo_id}` |
| `user` | `User` | author |
| `assignee`/`assignees` | `User` / `User[]` | |
| `labels` | array | |
| `milestone` | `Milestone` | |
| `created_at`/`updated_at`/`closed_at`/`merged_at` | string(date-time) | |
| `merge_base`, `merge_commit_sha` | string | |
| `html_url`, `url`, `diff_url`, `patch_url` | string | |
| `requested_reviewers`, `merged_by` | — | |

### 1.4 Get pull request detail

```
GET /repos/{owner}/{repo}/pulls/{index}
```
Returns the **full** `PullRequest` above (single call gives body + labels —
no need for a separate detail fetch of markdown). Draft is the `draft` boolean
(NOT only a title prefix; the `CreatePullRequestOption` has no `draft` field,
status is set by title prefix on write, but is read as a field on read).
`mergeable` is computed (can be `null` during async recalculation).

### 1.5 List issues (⚠️ includes PRs)

```
GET /repos/{owner}/{repo}/issues
```
Query: `state` (`open`/`closed`/`all`), `page`, `limit`, `assignee`, `creator`,
`mentioned`, `labels` (ints), `milestone`, `since`, `before`, `sort`, `order`.
**Important difference from GitLab:** Gitea (like GitHub) returns PRs in the
issues list. Distinguish with the `pull_request` field.

**Response shape** (`definitions.Issue`):

| field | type | notes |
|---|---|---|
| `number` | int64 | issue number |
| `title` | string | |
| `body` | string | markdown |
| `state` | string | `open`/`closed` |
| `user` | `User` | author |
| `assignee`/`assignees` | `User` / `User[]` | |
| `labels` | array | |
| `milestone` | `Milestone` | |
| `pull_request` | `PullRequestMeta`\|null | **null → real issue; present → it is a PR** |
| `updated_at`/`created_at`/`closed_at` | string(date-time) | |
| `html_url`, `url`, `ref` | string | |
| `repository` | `RepositoryMeta` | |
| `projects`, `time_estimate` | — | |

`PullRequestMeta` = `{draft, html_url, merged, merged_at}` — minimal, just
enough to filter PRs out. Client-side filter: `.filter(|i| i.pull_request.is_none())`.

### 1.6 Get issue detail

```
GET /repos/{owner}/{repo}/issues/{index}
```
Returns the same `Issue` object; `body` is the markdown.

### 1.7 CI status (commit statuses, **no "check runs"**)

Gitea mirrors **GitHub's commit-status model** (not GitHub's Checks API):

```
GET /repos/{owner}/{repo}/commits/{ref}/status          → CombinedStatus
GET /repos/{owner}/{repo}/statuses/{sha}                → CommitStatus[]
```

`CombinedStatus` = `{state, statuses[], total_count, sha, url, commit_url, repository}`
`CommitStatus` = `{state, context, description, target_url, url, created_at, updated_at, creator}`

`state` ∈ `pending`/`running`/`success`/`failure`/`error`. Good news: this lines
up almost exactly with octocrab's `list_check_runs_for_git_ref` mapping — you can
map `context`→name, `state`→status, and synthesize a `conclusion` like
`crate::github::client.rs:check_to_summary` already does.

> Gitea does **not** expose a Checks-like API with formatted annotations. If you
> later need diff annotations, fetch the PR files/comments separately.

### 1.8 Notifications

```
GET /notifications                                 query: all, status-types, subject-type, since, before, page, limit
GET /repos/{owner}/{repo}/notifications              (repo-scoped variant)
GET /notifications/threads/{id}
GET /notifications/new                             (count of unread)
```

`NotificationThread` = `{id, unread, pinned, repository, subject{type,title,state,url,html_url,latest_comment_url,latest_comment_html_url}, updated_at, url}`.
`subject.type` ∈ `Issue`/`PullRequest`/`Release`/`Repository`/`Discussion`/`Commit`
etc. Matches GitHub's `Notification` shape closely (`reason` is absent; Gitea
omits `reason`).

### 1.9 Pagination & rate limiting

- **Pagination:** `page` (1-based) + `limit` query; response header
  `X-Total-Count` (stated as mandatory for list endpoints in
  `docs/guidelines-backend.md: “Endpoints returning lists must support
  pagination (page and limit) and set the X-Total-Count header”`).
- **Rate limiting:** `429` per-IP for anonymous budgets. Header
  `X-RateLimit-Limit` / `X-RateLimit-Remaining` (GitHub-compatible). The
  `self_update` crate surfaces a `RateLimited` error and notes anonymous
  budgets are counted *per source IP*.

### 1.10 Response field naming

**snake_case** end-to-end (`full_name`, `html_url`, `mergeable`, `created_at`).
Because Gitea mirrors the GitHub schema, you can reuse `serde` aliases that match
octocrab structs where field names coincide; otherwise the snake_case names map
straight onto existing model field names.

---

## 2. GitLab API

**Docs:** `docs.gitlab.com/api/*` (92k snippets via Context7). Version referenced
in the current docs is **GitLab 19.x** (2026).

### 2.1 Base URL

```
https://gitlab.com/api/v4                         (SaaS)
https://<host>/api/v4                             (self-managed)
https://<host>/<subpath>/api/v4                   (relative_url_root)
```
From docs: "Your API URL is your GitLab instance URL with `/api/v4` appended."
`GITLAB_BASE_URL` overrides the SaaS default for the `glab` CLI.

### 2.2 Authentication

- Recommended: header `PRIVATE-TOKEN: <token>`
- Also valid (OAuth-compliant): `Authorization: Bearer <token>`
  (confirmed used by `TabbyML/tabby/ee/tabby-webserver/src/oauth/gitlab.rs`)
- CI job token: `JOB-TOKEN: <token>` header
- Scope: `api` (full) or granular (`read_api`, `read_merged`)
- **Env var:** `GITLAB_TOKEN` — set/expected by `glab`, and by `git-ai`’s
  `src/ci/gitlab.rs` (falls back to `CI_JOB_TOKEN`).
- **CLI tool:** `glab` (`gitlab-org/cli` / `https://gitlab.com/gitlab-org/cli`).
  `glab` sends the token in `PRIVATE-TOKEN`. ⚠️ quirk: `glab` does **not** work
  with `$CI_JOB_TOKEN` for the Releases API (needs `GLAB_ENABLE_CI_AUTOLOGIN`).

### 2.3 Remote URL parsing (group/subgroup)

GitLab project path = full namespace: `gitlab.com/group/subgroup/project`.
In API calls `:id` accepts the **URL-encoded full path**:
`GET /projects/group%2Fsubgroup%2Fproject/merge_requests`. This is the one
non-trivial difference vs. GitHub — GitPulse's `owner/repo` splitter must
instead take everything before the final segment as the namespace and URL-encode
slashes.

### 2.4 List merge requests

```
GET /projects/:id/merge_requests
```
Query: `state` (`all`/`opened`/`closed`/`locked`/`merged`, default `all`),
`source_branch`, `target_branch`, `draft` (bool), `labels`, `iids[]`,
`author_id`/`author_username`, `assignee_id`/`assignee_username`, `milestone`,
`updated_after`/`updated_before` (ISO-8601), `search`,
`with_merge_status_recheck` (bool — refresh `merge_status`), `view=simple`,
`sort`, `page`, `per_page` (default 20, max 100).

**Response (list)** — `definitions.ProjectMergeRequestLight` / full MR:

| field | type | notes |
|---|---|---|
| `id` | int | global MR id |
| `iid` | int | **project-scoped MR number** (== `number`) |
| `title` | string | |
| `description` | string | markdown |
| `state` | string | `opened`/`closed`/`merged`/`locked` |
| `author` | user | `{id,name,username,state,avatar_url,web_url,bot}` |
| `assignee` (deprecated) / `assignees[]` / `reviewers[]` | user | |
| `source_branch` / `target_branch` | string | ✓ (`source`→head, `target`→base) |
| `draft` (bool) | boolean | `work_in_progress` is the deprecated alias |
| `merge_status` | string | `unchecked`/`checking`/`can_be_merged`/`cannot_be_merged`/`cannot_be_merged_recheck`（**GitLab 15.6 起 deprecated**，改用 `detailed_merge_status`）|
| `detailed_merge_status` | string | 全 24 值（官方 Merge requests API §Merge status 逐条列出）：`mergeable`（唯一可合并）·阻塞态 `not_open`/`conflict`/`ci_must_pass`/`ci_still_running`/`discussions_not_resolved`/`need_rebase`/`draft_status`/`not_approved`/`merge_request_blocked`/`commits_status`/`requested_changes`/`jira_association_missing`/`status_checks_must_pass`/`security_policy_pipeline_check`/`security_policy_violations`/`locked_paths`/`locked_lfs_files`/`title_regex`/`merge_time`/`approvals_syncing`·无结论态 `unchecked`/`checking`/`preparing`。**注意**：阻塞态真身是 `merge_request_blocked`；`blocked` 与 `blocked_status` 均**不是** REST 取值（后者是 GraphQL 枚举成员名）；`not_fast_forward`/`ready` 属 GitHub 说法 |
| `sha`, `merge_commit_sha`, `squash_commit_sha` | string | `sha`=source HEAD |
| `labels` | string[] | **array of strings, not objects** (differs from Gitea/GitHub!) |
| `created_at`/`updated_at`/`merged_at`/`closed_at` | string | ISO-8601 |
| `has_conflicts` | boolean | only true when `merge_status=="cannot_be_merged"` |
| `web_url`, `references{short,relative,full}` | string/obj | `!1` style refs |
| `time_stats`, `task_completion_status` | object | `{count, completed_count}` |

> The list endpoint does **not** proactively refresh `merge_status`/`has_conflicts`
> (expensive). Add `with_merge_status_recheck=true` if you need current values.

### 2.5 Get MR detail

```
GET /projects/:id/merge_requests/:merge_request_iid
```
Query: `render_html`, `include_rebase_in_progress`, `include_diverged_commits_count`.
Adds to the fields above: `diff_refs` `{base_sha, start_sha, head_sha}` **async, may
be null on first read** (`git-ai` fetches this separately from the list endpoint
exactly for this reason — see comment in `git-ai/src/ci/gitlab.rs`). The MR
response does **not** embed CI pipeline here by default; fetch it via 2.8.

### 2.6 List issues — ⚠️ GitLab EXCLUDES MRs

```
GET /projects/:id/issues
```
Query: `state` (`opened`/`closed`), `assignee_id`, `author_id`, `labels`,
`milestone`, `my_reaction_emoji`, `search`, `in`, `iids[]`, `confidential`.

**GitLab issues are a separate resource from MRs** — `merge_requests` are NOT
returned here (unlike GitHub/Gitea). No client-side PR filter needed.

Response fields per `ProjectIssue`: `iid`, `title`, `description` (markdown),
`state`, `author`, `assignee`/`assignees`, `labels` (strings), `milestone`,
`created_at`, `updated_at`, `closed_at`, `due_date`, `weight`, `web_url`,
`time_stats`, `references`, etc. (20/page default, paginated).

### 2.7 Get issue detail

```
GET /projects/:id/issues/:issue_iid
```
Same object; `description` is the markdown body.

### 2.8 CI/CD pipelines & commit status (no "checks" API)

GitLab has **no GitHub equivalent of check runs**; CI status = **pipelines +
commit statuses**:

```
GET /projects/:id/pipelines?sha=<sha>          → Pipeline[]  (filter by sha/ref)
GET /projects/:id/pipelines/:pipeline_id/jobs   → Job[]
GET /projects/:id/statuses/:sha                 → CommitStatus[]  (legacy GitHub-style)
GET /projects/:id/commit/:sha                   → includes last_pipeline {id,sha,status}
```

`Pipeline` = `{id, iid, project_id, status, source, ref, sha, name, web_url, created_at, updated_at, user}`.
Valid `status`: `created`/`waiting_for_resource`/`preparing`/`pending`/`running`/`success`/`failed`/`canceled`/`canceling`/`skipped`/`manual`/`scheduled`/`waiting_for_callback`.
This maps cleanly onto GitPulse's `CheckSummary` (`status` + `conclusion`),
using `name`=pipeline id / `web_url`.

> For an MR, grab `diff_refs.head_sha` (or `sha`) → query pipelines for that sha
> → take latest `status`. `git-ai` confirms this exact pattern.

External/status-check services (e.g. external CI reporting status back):
`GET /projects/:id/merge_requests/:iid/status_checks` → `{id, name, external_url, status}`.

### 2.9 Notifications

GitLab's notification model is **preferences/settings** rather than a thread
list (it deprecated bulk notification listing in favour of To-Dos):

- `GET /notification_settings` → `{level, notification_email}` (global)
- `GET /projects/:id/notification_settings`, `GET /groups/:id/notification_settings`
- `GET /todos` → **To-Do list** items `{id, project, author, action_name, target_type, target, target_url, body, resolved, created_at}` —
  the closest analogue to “unread notifications”.
  (source: `docs.gitlab.com/api/todos/index.md`)

So a GitLab panel ≈ "open To-Dos" rather than GitHub-style notification threads.
GitLab does still expose per-project `GET /projects/:id/notifications` for
settings, but not a rich unread thread feed.

### 2.10 Pagination & rate limiting

- **Pagination:** offset-based, `page` + `per_page`; **Link** header
  (`rel="prev|next|first|last"`) + `x-next-page`, `x-page`, `x-per-page`,
  `x-prev-page`, `x-total`, `x-total-pages` (the latter two omitted for >10k
  results on GitLab.com).
- **Rate limiting:** `429 Too Many Requests` + `Retry-After` +
  `RateLimit-ResetTime`; `RateLimit-Limit`/`RateLimit-Remaining` on responses.
  Rack::Attack at HTTP layer; anonymous per-IP. ⚠️ headers can lag —
  429 can occur even when headers showed remaining quota.

### 2.11 Response field naming

**snake_case** (identical to GitHub/Gitea): `source_branch`, `target_branch`,
`merge_status`, `created_at`, `web_url`, `detailed_merge_status`.

---

## 3. Common patterns

### 3.1 HTTP status codes

| Code | GitHub | Gitea | GitLab |
|---|---|---|---|
| 401 | invalid token | invalid token | invalid token |
| 403 | insufficient scope | insufficient scope / admin-only | insufficient scope |
| 404 | not found | not found | **not found** (private project hidden → 404, not 403) |
| 429 | rate limited | rate limited | rate limited |

### 3.2 Rate limiting

- All three return `429` (+ `Retry-After` for GitLab/GitHub).
- Headers: GitHub `X-RateLimit-*`; GitLab `RateLimit-Limit/Remaining/ResetTime`;
  Gitea `X-RateLimit-*` (GitHub-compatible).
- All throttle **anonymous** traffic per source IP (Gitea notably so).

### 3.3 Pagination

- **GitHub:** `per_page` (max 100) + `Link` header (`octocrab` `Page` auto-paginates).
- **Gitea:** `page` + `limit` (max 50 on gitea.com); `X-Total-Count` header.
- **GitLab:** `page` + `per_page` (max 100) + `Link` header + `x-next-page` block.

### 3.4 Field naming

**snake_case universally** (GitHub, Gitea, GitLab). Gitea goes further — it
mirrors GitHub's schema, so field-for-field mapping is the path of least
resistance.

### 3.5 Issues-vs-PRs semantics (the one real semantic fork)

| Platform | `/issues` list | Distinguish |
|---|---|---|
| GitHub | includes PRs | `pull_request != null` |
| Gitea | **includes PRs** | `pull_request != null` (`PullRequestMeta`) |
| GitLab | **excludes PRs** | (separate `/merge_requests`) — no filter needed |

GitPulse's existing `list_issues` filter (`.filter(|i| i.pull_request.is_none())`)
is correct for GitHub and Gitea; for GitLab it's a harmless no-op.

### 3.6 CI model

| Platform | CI signal | Endpoint | Mapped to `CheckSummary` |
|---|---|---|---|
| GitHub | check runs | `GET /repos/{o}/{r}/commits/{ref}/check-runs` | via octocrab |
| Gitea | commit statuses | `GET /repos/{o}/{r}/commits/{ref}/status` | `context`→name, `state`→status/`conclusion` |
| GitLab | pipelines | `GET /projects/:id/pipelines?sha=<sha>` | `name`/id → name, `status` → status/conclusion |

---

## 4. Rust ecosystem survey

### 4.1 Gitea clients (crates.io)

| crate | version | last update | verdict |
|---|---|---|---|
| `gitea` | 0.2.0 | **2020-07-09** (2 versions, 14 recent dl) | ❌ **Stale / unmaintained.** |
| `forgejo-mcp-rs` | 0.20.4 (2026) | active | ❌ Not an API client — it's an MCP server. |

**Conclusion:** No maintained Rust Gitea client. → **Use `reqwest` directly.**
The `gitea-go-sdk` (`gitea/client.go`) confirms the exact 3-line auth:
`req.Header.Set("Authorization", "token "+c.accessToken)`, and base path
`c.url + "/api/v1" + path`.

### 4.2 GitLab clients (crates.io)

| crate | version | last update | verdict |
|---|---|---|---|
| `gitlab` (rust-gitlab) | **0.1904.0** | **2026-09-25** (165 versions, ~5M dl, ~monthly) | ✅ **Actively maintained.** Hosted at `gitlab.kitware.com/utils/rust-gitlab`. |
| `gitlab-api` | 0.6.0 | 2017 (API v3) | ❌ Abandoned. |

**rust-gitlab API surface (v0.1904.0 source tree confirms):**
- `api/merge_requests/{merge_request, merge_requests}` ✓
- `api/projects/merge_requests/{…}` (approvals, discussions, diffs, notes, pipelines, create/edit/merge) ✓
- `api/projects/issues/{issues, issue, create, edit, delete, notes, awards, links}` ✓
- `api/projects/pipelines/{pipelines, pipeline, jobs, create, retry, cancel}` ✓
- `api/projects/repository/commits/{commits, commit, statuses, create_status}` ✓
- Auth: `Gitlab::new(host, token)` or `.personal_access_token(token)`;
  sets `PRIVATE-TOKEN` internally; configurable `rest_url` (self-hosted/relative root).
- **No** notifications module (GitLab's weakest fit anyway — use `/todos`).

So `gitlab` (rust-gitlab) covers all 5 substantive calls (MRs, issues, pipelines)
but **not notifications/To-Dos**. A hybrid — SDK for the typed calls + a
hand-rolled `/todos` fetch — is viable.

### 4.3 octocrab is GitHub-only

Confirmed from the autodocs: handlers are `repos()`, `issues()`, `pulls()`,
`users()`, `orgs()`, `search()`, `gists()`, `current()`, `activity()`,
`apps()`, `teams()`, `commits()`, `actions()`, `workflows()`, `ratelimit()`, ….
The lower-level `get(url, ..)` path supports GitHub Enterprise by overriding the
base URI, but **there is no multi-provider support** — octocrab cannot target
Gitea/GitLab.

### 4.4 reqwest

- **Current version:** `0.13.5` (crates.io, 2026-09-08; 767 M downloads). Context7's
  snapshot said 0.12.9 — use 0.13.x.
- Pattern: `Client::builder().default_headers(h).build()`, then
  `.header("X", v)`, `.bearer_auth(t)`, `.query(&json!({..}))`,
  `.json(&body)`, `.send().await?.error_for_status()?`,
  `.json::<T>()`, `.headers()` (`&HeaderMap`).
- Reading pagination/total headers is direct: `resp.headers().get("x-total-count")`.

A reference Rust GitLab impl exists: `git-ai/src/ci/gitlab.rs`:
```rust
fn gitlab_api_get(endpoint, auth_header_name, auth_token) {
    let agent = http::build_agent(Some(30));
    agent.get(endpoint)
        .header(auth_header_name, auth_token)        // "PRIVATE-TOKEN" | "JOB-TOKEN"
        .header("User-Agent", &format!("git-ai/{}", version))
        .send()?;
}
```
Token source: `GITLAB_TOKEN` env, falling back to `CI_JOB_TOKEN`.

### 4.5 Tradeoffs: typed SDK vs reqwest (for ~6 calls/platform)

| | typed SDK (rust-gitlab / octocrab-style) | reqwest (raw) |
|---|---|---|
| Per-call code | builder + typed response; less boilerplate | hand-build URL + deserialize |
| Schema safety | ✅ compile-time field checks | ❌ silent on rename until runtime |
| Auth/pagination plumbing | ✅ built-in | must hand-roll header + `page` loop |
| Field completeness | ✅ full struct | you choose a subset struct |
| Gitea coverage | ❌ no maintained crate | ✅ only option |
| GitLab coverage | ✅ covers MRs/issues/pipelines | ✅ only option (no SDK for Gitea) |
| TUI fit | SDK = extra dep + async model coupling | one client for both; minimal diff to existing |
| **Verdict for GitPulse** | consider for **GitLab** only | **recommended as the uniform baseline** for Gitea; strong case to use reqwest for both to keep one auth/pagination path |

Because Gitea has **no** Rust SDK, the cleanest architecture is a single
**reqwest-based transport** shared by Gitea + GitLab, with small per-platform
response structs. octocrab can stay **only** for GitHub, OR (for uniformity)
GitHub can also move to reqwest — the Gitea spec is GitHub-compatible, so a
unified `GitHub/Gitea/GitLab` response-normalizer is feasible.

---

## 5. Recommendations for the GitPulse extension

1. **New module layout** mirroring `src/github/`:
   `src/gitea/{client.rs, auth.rs, error.rs, models.rs}` and
   `src/gitlab/{…}`. Re-use the `PrSummary/PrDetail/IssueSummary/IssueDetail/
   NotificationSummary/CheckSummary` models — field names already align.
2. **Auth per platform** (mirror `src/github/auth.rs` precedence):
   - Gitea: `GITEA_TOKEN` env → (no single standard CLI; document that `tea`
     stores tokens in its own config/keyring, so just document env). Header:
     `Authorization: token <t>`.
   - GitLab: `GITLAB_TOKEN` env → `glab` (`glab api --cache --raw` is awkward;
     prefer env; `GITLAB_BASE_URL` for self-hosted base). Header:
     `PRIVATE-TOKEN: <t>`.
   - Both: cache the resolved token in a `OnceLock` like `GithubClient`.
3. **Issue-vs-PR filtering:** apply `pull_request` filter for GitHub + Gitea;
   a no-op for GitLab (issues exclude MRs).
4. **CI mapping:** Gitea `commit/status` → `CheckSummary` exactly like the
   GitHub `check_to_summary` helper; GitLab `pipelines?sha=` → same.
5. **Notifications:** GitHub/Gitea have real notification threads; GitLab →
   map the `/todos` list (open) onto `NotificationSummary` and label the panel
   "To-Dos" for GitLab.
6. **Dependency decision:** add `reqwest` (likely already present transitively)
   for Gitea; for GitLab, prefer the typed `gitlab` crate (0.1904.0) for
   MRs/issues/pipelines and a 4-line reqwest call for `/todos`, OR go pure
   reqwest for a single shared transport. Either is sound; pure-reqwest keeps
   the diff to `error.rs`/models trivially small.

---

## Sources

- Gitea OpenAPI/Swagger spec: `https://gitea.com/swagger.v1.json`
- Gitea SDK auth (`Authorization: token`): `https://raw.githubusercontent.com/Madh93/gitea-go-sdk/main/gitea/client.go` (lines 345-365)
- Gitea GitHub-API-compatibility guidelines: `go-gitea/gitea docs/guidelines-backend.md`
- Gitea `tea` CLI: `https://gitea.com/gitea/tea`
- Gitea env vars (`GITEA_TOKEN`): `jaemk/self_update`, `updatecli`, `ghorg`, `go-gitea/gitea/services/actions`
- GitLab REST API docs: `https://docs.gitlab.com/api/` (merge_requests, issues, pipelines, jobs, todos, notification_settings, rate limiting)
- GitLab `glab` CLI: `https://gitlab.com/gitlab-org/cli`
- GitLab auth (`PRIVATE-TOKEN` / `Bearer`): `docs.gitlab.com/api/rest/authentication`
- Real Rust GitLab client: `https://github.com/git-ai-project/git-ai/blob/main/src/ci/gitlab.rs`
- Rust GitLab OAuth sample: `TabbyML/tabby/ee/tabby-webserver/src/oauth/gitlab.rs`
- rust-gitlab SDK (types + modules): `crates.io/crates/gitlab` v0.1904.0
- gitlab-tracker (reqwest TUI reference): `julien-langlois/gitlab-tracker` (`gitlab-tracker-core/src/provider.rs`)
- octocrab (GitHub-only): `crates.io/crates/octocrab`
- reqwest: `crates.io/crates/reqwest` v0.13.5
- Gitea pagination `X-Total-Count` rule: `go-gitea/gitea docs/guidelines-backend.md`
