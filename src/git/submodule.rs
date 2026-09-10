use anyhow::Result;
use std::path::{Path, PathBuf};
use tokio::process::Command;

use super::RepoInfo;

#[derive(Debug, Clone)]
pub struct SubmoduleInfo {
    pub name: String,
    pub path: PathBuf,
    pub current_commit: String,
    pub is_dirty: bool,
}

pub fn list_submodules(repo_path: &Path) -> Result<Vec<SubmoduleInfo>> {
    let repo = git2::Repository::open(repo_path)?;
    let mut submodules = Vec::new();
    
    for submodule in repo.submodules()? {
        let name = submodule.name().unwrap_or("unknown").to_string();
        let path = repo_path.join(submodule.path());
        
        let (current_commit, is_dirty) = if let Ok(sub_repo) = git2::Repository::open(&path) {
            let head = sub_repo.head().ok();
            let commit = head
                .and_then(|h| h.target())
                .map(|oid| oid.to_string())
                .unwrap_or_else(|| "unknown".to_string());
            
            let mut status_options = git2::StatusOptions::new();
            status_options.include_untracked(true);
            let is_dirty = sub_repo.statuses(Some(&mut status_options))
                .map(|s| !s.is_empty())
                .unwrap_or(false);
            
            (commit, is_dirty)
        } else {
            ("not initialized".to_string(), false)
        };
        
        submodules.push(SubmoduleInfo {
            name,
            path,
            current_commit,
            is_dirty,
        });
    }
    
    Ok(submodules)
}

pub async fn batch_submodule_update(repos: &[RepoInfo]) -> Vec<(String, Result<()>)> {
    let mut results = Vec::new();
    
    for repo in repos {
        let result = submodule_update(&repo.path).await;
        results.push((repo.name.clone(), result));
    }
    
    results
}

async fn submodule_update(path: &Path) -> Result<()> {
    let output = Command::new("git")
        .args(["submodule", "update", "--init", "--recursive"])
        .current_dir(path)
        .output()
        .await?;
    
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git submodule update failed: {}", stderr);
    }
}
