//! Remote operations: push / pull via the git CLI with progress feedback.
//!
//! The git2 push/pull APIs require non-trivial credential callbacks, so these
//! operations shell out to the `git` CLI inside `spawn_blocking` (design
//! checkpoint 4: keep the runtime thread free while the child process runs).
//! `pull` is strict fast-forward only (convention 7): diverged histories fail
//! with an explicit error instead of merging or rebasing.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use git2::Repository;

/// Progress update emitted while a remote operation runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    /// Completion percentage (0-100).
    pub percent: u8,
    /// Human-readable status message.
    pub message: String,
}

/// Resolve the remote to operate on: `origin` when present, else the first
/// configured remote; errors when the repository has no remote at all.
fn resolve_remote(repo_path: &Path) -> Result<String> {
    let repo = Repository::open(repo_path)?;
    let remotes = repo.remotes()?;
    let names: Vec<&str> = remotes.iter().flatten().collect();
    if names.is_empty() {
        anyhow::bail!("未配置远端，无法执行远端操作");
    }
    if names.contains(&"origin") {
        return Ok("origin".to_string());
    }
    Ok(names[0].to_string())
}

/// Resolve the current branch name; errors on detached or unborn HEAD.
fn current_branch(repo_path: &Path) -> Result<String> {
    let repo = Repository::open(repo_path)?;
    let head = repo.head().context("无法解析 HEAD")?;
    if !head.is_branch() {
        anyhow::bail!("处于 detached HEAD，无法执行远端操作");
    }
    Ok(head.shorthand().context("无法解析分支名")?.to_string())
}

/// Whether the current branch has an upstream tracking branch.
fn has_upstream(repo_path: &Path) -> bool {
    let Ok(repo) = Repository::open(repo_path) else {
        return false;
    };
    let Ok(head) = repo.head() else {
        return false;
    };
    if !head.is_branch() {
        return false;
    }
    let Some(name) = head.shorthand() else {
        return false;
    };
    let Ok(branch) = repo.find_branch(name, git2::BranchType::Local) else {
        return false;
    };
    let has_upstream = branch.upstream().is_ok();
    has_upstream
}

/// Run `git -C <repo> <args>` and return its output, or an error carrying
/// the operation name and the child's stderr (shown on the status line).
fn git_output(repo_path: &Path, args: &[&str]) -> Result<std::process::Output> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("启动 git 进程失败")?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        let op = args.first().copied().unwrap_or("git");
        anyhow::bail!("{op} 失败: {err}");
    }
    Ok(output)
}

/// Push the current branch to its remote with progress feedback.
///
/// Uses `git push -u <remote> <branch>` so the first push to a freshly
/// configured remote also establishes the upstream tracking branch.
pub async fn push(
    repo_path: &Path,
    on_progress: impl Fn(Progress) + Send + 'static,
) -> Result<()> {
    let repo_path: PathBuf = repo_path.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<()> {
        on_progress(Progress {
            percent: 0,
            message: "正在 push…".to_string(),
        });
        let remote = resolve_remote(&repo_path)?;
        let branch = current_branch(&repo_path)?;
        let args = ["push", "-u", remote.as_str(), branch.as_str()];
        git_output(&repo_path, &args)?;
        on_progress(Progress {
            percent: 100,
            message: "push 完成".to_string(),
        });
        Ok(())
    })
    .await?
}

/// Pull the current branch with progress feedback, fast-forward only.
///
/// Falls back to explicit `<remote> <branch>` arguments when the branch has
/// no upstream yet; diverged histories fail with an explicit error
/// (convention 7) instead of merging or rebasing.
pub async fn pull(
    repo_path: &Path,
    on_progress: impl Fn(Progress) + Send + 'static,
) -> Result<()> {
    let repo_path: PathBuf = repo_path.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<()> {
        on_progress(Progress {
            percent: 0,
            message: "正在 pull…".to_string(),
        });
        let remote = resolve_remote(&repo_path)?;
        let branch = current_branch(&repo_path)?;
        let args: Vec<&str> = if has_upstream(&repo_path) {
            vec!["pull", "--ff-only"]
        } else {
            vec!["pull", "--ff-only", remote.as_str(), branch.as_str()]
        };
        git_output(&repo_path, &args)?;
        on_progress(Progress {
            percent: 100,
            message: "pull 完成".to_string(),
        });
        Ok(())
    })
    .await?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::{Arc, Mutex};

    fn base_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "gitpulse-remote-{tag}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn init_repo(dir: &Path) {
        let repo = Repository::init(dir).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "GitPulse Test").unwrap();
        config.set_str("user.email", "test@gitpulse.local").unwrap();
    }

    fn commit_file(dir: &Path, name: &str, content: &str, message: &str) {
        fs::write(dir.join(name), content).unwrap();
        crate::git::stage(dir, Path::new(name)).unwrap();
        crate::git::commit(dir, message).unwrap();
    }

    #[tokio::test]
    async fn push_without_remote_reports_reason() {
        let dir = base_dir("noremote");
        init_repo(&dir);
        commit_file(&dir, "a.txt", "v1\n", "v1");

        let err = push(&dir, |_| {}).await.unwrap_err();
        assert!(
            err.to_string().contains("未配置远端"),
            "unexpected error: {err}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn push_pull_roundtrip_and_ff_only_divergence_error() {
        let root = base_dir("roundtrip");
        let origin_path = root.join("origin.git");
        Repository::init_bare(&origin_path).unwrap();
        let origin_str = origin_path.to_str().unwrap().to_string();

        // Work repo: first push must succeed and establish the upstream.
        let work = root.join("work");
        fs::create_dir_all(&work).unwrap();
        init_repo(&work);
        {
            let repo = Repository::open(&work).unwrap();
            repo.remote("origin", &origin_str).unwrap();
        }
        commit_file(&work, "a.txt", "v1\n", "v1");

        let messages: Arc<Mutex<Vec<Progress>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = messages.clone();
        push(&work, move |p| sink.lock().unwrap().push(p))
            .await
            .expect("first push must succeed");
        {
            let messages = messages.lock().unwrap();
            assert!(messages.iter().any(|p| p.percent == 100), "{messages:?}");
        }

        // Second clone advances the remote.
        let other = root.join("other");
        let repo2 = Repository::clone(&origin_str, &other).unwrap();
        let mut config2 = repo2.config().unwrap();
        config2.set_str("user.name", "GitPulse Test").unwrap();
        config2.set_str("user.email", "test@gitpulse.local").unwrap();
        drop(config2);
        commit_file(&other, "b.txt", "v2\n", "v2");
        push(&other, |_| {}).await.expect("clone push must succeed");

        // Drop the upstream so the pull exercises the explicit-args fallback.
        let untrack = Command::new("git")
            .arg("-C")
            .arg(&work)
            .arg("branch")
            .arg("--unset-upstream")
            .output()
            .unwrap();
        assert!(untrack.status.success(), "unset-upstream failed");

        pull(&work, |_| {}).await.expect("ff-only pull must succeed");
        assert!(
            work.join("b.txt").exists(),
            "pull did not bring the remote commit"
        );

        // An up-to-date push still succeeds (and re-sets the upstream).
        push(&work, |_| {}).await.expect("up-to-date push must succeed");

        // Diverged histories: local commit + advanced remote -> ff-only error.
        commit_file(&work, "c.txt", "local\n", "local-only");
        commit_file(&other, "d.txt", "remote\n", "remote-only");
        push(&other, |_| {}).await.expect("second clone push must succeed");

        let err = pull(&work, |_| {}).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("pull 失败"), "missing error reason: {msg}");

        let _ = fs::remove_dir_all(&root);
    }
}
