//! Batch remote operations: pull/fetch across many repositories.
//!
//! Each repository runs sequentially so progress is deterministic. `pull`
//! reuses [`super::pull`], which already runs the git CLI inside
//! `spawn_blocking` (checkpoint 4) and is fast-forward only (convention 7);
//! `fetch` shells out to the git CLI in `spawn_blocking` as well, so no
//! non-`Send` git2 `Repository` ever crosses a thread boundary.
//!
//! Progress messages follow a fixed prefix convention so the dashboard can
//! style each line: `进行中` while a repository runs, `成功` when it finishes,
//! `失败: <reason>` on error.

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::remote;
use super::status::RepoInfo;

/// Run `git fetch --all --prune` for one repository (blocking; call from
/// `spawn_blocking`).
///
/// Reuses the shared [`remote::git_output`] so a failing fetch is reported
/// exactly like a failing push/pull (G29).
fn fetch_blocking(repo_path: &Path) -> Result<()> {
    remote::git_output(repo_path, &["fetch", "--all", "--prune"])?;
    Ok(())
}

/// Run `git fetch --all --prune` for one repository off the runtime thread
/// (checkpoint 4: git work never blocks the UI loop).
async fn fetch_one(repo_path: &Path) -> Result<()> {
    let repo_path: PathBuf = repo_path.to_path_buf();
    tokio::task::spawn_blocking(move || fetch_blocking(&repo_path)).await?
}

/// Pull every repository fast-forward only, reporting progress per repository.
///
/// `progress(index, message)` is called twice per repository: once with
/// `进行中` when it starts, then with `成功` or `失败: <reason>` when it
/// finishes. Results come back in input order.
pub async fn batch_pull(
    repos: &[RepoInfo],
    progress: impl Fn(usize, &str),
) -> Vec<(String, Result<()>)> {
    let mut results = Vec::with_capacity(repos.len());
    for (i, repo) in repos.iter().enumerate() {
        progress(i, "进行中");
        let result = remote::pull(&repo.path).await;
        let message = match &result {
            Ok(()) => "成功".to_string(),
            Err(err) => format!("失败: {err}"),
        };
        progress(i, &message);
        results.push((repo.name.clone(), result));
    }
    results
}

/// Fetch every repository, reporting progress per repository.
///
/// Same progress contract as [`batch_pull`].
pub async fn batch_fetch(
    repos: &[RepoInfo],
    progress: impl Fn(usize, &str),
) -> Vec<(String, Result<()>)> {
    let mut results = Vec::with_capacity(repos.len());
    for (i, repo) in repos.iter().enumerate() {
        progress(i, "进行中");
        let result = fetch_one(&repo.path).await;
        let message = match &result {
            Ok(()) => "成功".to_string(),
            Err(err) => format!("失败: {err}"),
        };
        progress(i, &message);
        results.push((repo.name.clone(), result));
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::show_path;
    use crate::git::RepoInfo;
    use std::fs;
    use std::sync::{Arc, Mutex};

    fn base_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gitpulse-batch-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn init_repo(dir: &Path) {
        let repo = git2::Repository::init(dir).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "GitPulse Test").unwrap();
        config.set_str("user.email", "test@gitpulse.local").unwrap();
    }

    fn repo_info(path: &Path) -> RepoInfo {
        RepoInfo {
            name: path
                .file_name()
                .map(|n| show_path(std::path::Path::new(n)))
                .unwrap_or_else(|| show_path(path)),
            path: path.to_path_buf(),
            branch: "main".to_string(),
            dirty: Default::default(),
            ahead: 0,
            behind: 0,
            last_commit: Default::default(),
            remote: None,
            is_worktree: false,
            main_repo: None,
            author: None,
        }
    }

    fn commit_file(dir: &Path, name: &str, content: &str, message: &str) {
        fs::write(dir.join(name), content).unwrap();
        crate::git::stage(dir, Path::new(name)).unwrap();
        crate::git::commit(dir, message).unwrap();
    }

    #[tokio::test]
    async fn batch_pull_without_remote_reports_reason_per_repo() {
        let dir = base_dir("noremote");
        init_repo(&dir);
        commit_file(&dir, "a.txt", "v1\n", "v1");
        let repos = [repo_info(&dir)];

        let messages: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = messages.clone();
        let results = batch_pull(&repos, move |_, msg| {
            sink.lock().unwrap().push(msg.to_string());
        })
        .await;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, dir.file_name().unwrap().to_string_lossy());
        let err = results[0].1.as_ref().expect_err("no remote must fail");
        assert!(
            err.to_string().contains("未配置远端"),
            "unexpected error: {err}"
        );

        let messages = messages.lock().unwrap();
        assert_eq!(messages.len(), 2, "{messages:?}");
        assert_eq!(messages[0], "进行中");
        assert!(messages[1].starts_with("失败: "), "{messages:?}");

        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn batch_pull_and_fetch_succeed_against_origin() {
        let root = base_dir("roundtrip");
        let origin_path = root.join("origin.git");
        git2::Repository::init_bare(&origin_path).unwrap();
        let origin_str = origin_path.to_str().unwrap().to_string();

        let work = root.join("work");
        fs::create_dir_all(&work).unwrap();
        init_repo(&work);
        {
            let repo = git2::Repository::open(&work).unwrap();
            repo.remote("origin", &origin_str).unwrap();
        }
        commit_file(&work, "a.txt", "v1\n", "v1");
        crate::git::push(&work)
            .await
            .expect("initial push must succeed");

        let repos = [repo_info(&work)];

        let pull_messages: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = pull_messages.clone();
        let results = batch_pull(&repos, move |_, msg| {
            sink.lock().unwrap().push(msg.to_string());
        })
        .await;
        assert!(results[0].1.is_ok(), "{:?}", results[0].1);
        {
            let messages = pull_messages.lock().unwrap();
            assert_eq!(messages.as_slice(), ["进行中", "成功"], "{messages:?}");
        }

        let fetch_messages: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = fetch_messages.clone();
        let results = batch_fetch(&repos, move |_, msg| {
            sink.lock().unwrap().push(msg.to_string());
        })
        .await;
        assert!(results[0].1.is_ok(), "{:?}", results[0].1);
        let messages = fetch_messages.lock().unwrap();
        assert_eq!(messages.as_slice(), ["进行中", "成功"], "{messages:?}");

        let _ = fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn batch_pull_empty_repo_slice_is_a_noop() {
        let results = batch_pull(&[], |_, _| {}).await;
        assert!(results.is_empty());
        let results = batch_fetch(&[], |_, _| {}).await;
        assert!(results.is_empty());
    }
}
