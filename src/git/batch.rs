use anyhow::Result;
use std::path::Path;
use tokio::process::Command;

use super::status::RepoInfo;

pub type BatchResult = Vec<(String, Result<()>)>;

macro_rules! batch_async {
    ($repos:expr, $progress:expr, $op:expr) => {{
        let mut results = BatchResult::new();
        for (i, repo) in $repos.iter().enumerate() {
            $progress(i, &repo.name);
            let result = $op(&repo.path).await;
            results.push((repo.name.clone(), result));
        }
        results
    }};
}

pub async fn batch_pull(repos: &[RepoInfo], progress: impl Fn(usize, &str)) -> BatchResult {
    batch_async!(repos, progress, pull_repo)
}

pub async fn batch_fetch(repos: &[RepoInfo], progress: impl Fn(usize, &str)) -> BatchResult {
    batch_async!(repos, progress, fetch_repo)
}

pub async fn batch_stash(repos: &[RepoInfo], progress: impl Fn(usize, &str)) -> BatchResult {
    batch_async!(repos, progress, stash_repo)
}

pub fn batch_clean(repos: &[RepoInfo], progress: impl Fn(usize, &str)) -> BatchResult {
    let mut results = BatchResult::new();
    for (i, repo) in repos.iter().enumerate() {
        progress(i, &repo.name);
        results.push((repo.name.clone(), clean_repo(&repo.path)));
    }
    results
}

async fn pull_repo(path: &Path) -> Result<()> {
    let output = Command::new("git")
        .arg("pull")
        .current_dir(path)
        .output()
        .await?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git pull failed: {}", stderr);
    }
}

async fn fetch_repo(path: &Path) -> Result<()> {
    let output = Command::new("git")
        .arg("fetch")
        .current_dir(path)
        .output()
        .await?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git fetch failed: {}", stderr);
    }
}

async fn stash_repo(path: &Path) -> Result<()> {
    let output = Command::new("git")
        .args(["stash", "push", "-m", "gitpulse-stash"])
        .current_dir(path)
        .output()
        .await?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git stash failed: {}", stderr);
    }
}

fn clean_repo(path: &Path) -> Result<()> {
    let output = std::process::Command::new("git")
        .args(["clean", "-fd"])
        .current_dir(path)
        .output()?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git clean failed: {}", stderr);
    }
}
