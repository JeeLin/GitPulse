//! Local branch operations: list / create / switch / delete.
//!
//! All functions take a repository path and open the repository on the
//! calling thread, matching the other single-repo helpers in this module;
//! only batch operations (checkpoint 4) need `spawn_blocking`.

use std::path::Path;

use anyhow::{Context, Result};
use git2::{BranchType, ObjectType, Repository};

/// Snapshot of one local branch for the branch panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchInfo {
    /// Branch name (without the `refs/heads/` prefix).
    pub name: String,
    /// Whether this branch HEAD points at.
    pub is_head: bool,
    /// Upstream branch name (e.g. `origin/main`), when configured.
    pub upstream: Option<String>,
}

/// List all local branches, current branch first, then alphabetically.
///
/// Branches that cannot be read are skipped rather than failing the whole
/// list; upstream lookup failures simply yield `None`.
pub fn list_branches(repo_path: &Path) -> Result<Vec<BranchInfo>> {
    let repo = Repository::open(repo_path)?;
    let mut branches = Vec::new();
    let iter = repo.branches(Some(BranchType::Local))?;
    for entry in iter.flatten() {
        let (branch, _) = entry;
        let Some(name) = branch.name()?.map(str::to_string) else {
            continue;
        };
        let is_head = branch.is_head();
        let upstream = branch
            .upstream()
            .ok()
            .and_then(|up| up.get().name().map(str::to_string));
        branches.push(BranchInfo {
            name,
            is_head,
            upstream,
        });
    }
    branches.sort_by(|a, b| b.is_head.cmp(&a.is_head).then_with(|| a.name.cmp(&b.name)));
    Ok(branches)
}

/// Create a new branch at the current HEAD without switching to it.
pub fn create_branch(repo_path: &Path, name: &str) -> Result<()> {
    let repo = Repository::open(repo_path)?;
    let head = repo.head().context("无法解析 HEAD，仓库可能尚无提交")?;
    let commit = head.peel_to_commit()?;
    repo.branch(name, &commit, false)
        .with_context(|| format!("创建分支 {name} 失败"))?;
    Ok(())
}

/// Switch the working tree to the named branch.
///
/// Uses a safe checkout: local modifications that would be overwritten by
/// the switch abort with an error instead of being discarded.
pub fn switch_branch(repo_path: &Path, name: &str) -> Result<()> {
    let repo = Repository::open(repo_path)?;
    let branch = repo
        .find_branch(name, BranchType::Local)
        .with_context(|| format!("分支 {name} 不存在"))?;
    branch
        .get()
        .peel(ObjectType::Commit)
        .context("无法解析分支指向的提交")?;
    repo.set_head(&format!("refs/heads/{name}"))
        .with_context(|| format!("切换到 {name} 失败"))?;
    let mut opts = git2::build::CheckoutBuilder::new();
    repo.checkout_head(Some(&mut opts))
        .context("更新工作区失败")?;
    Ok(())
}

/// Delete a local branch; refuses to delete the branch HEAD points at.
pub fn delete_branch(repo_path: &Path, name: &str) -> Result<()> {
    let repo = Repository::open(repo_path)?;
    let mut branch = repo
        .find_branch(name, BranchType::Local)
        .with_context(|| format!("分支 {name} 不存在"))?;
    if branch.is_head() {
        anyhow::bail!("无法删除当前分支 {name}");
    }
    branch
        .delete()
        .with_context(|| format!("删除分支 {name} 失败"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn base_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gitpulse-branch-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn init_repo_with_commit(dir: &Path) {
        let repo = Repository::init(dir).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "GitPulse Test").unwrap();
        config.set_str("user.email", "test@gitpulse.local").unwrap();
        drop(config);
        fs::write(dir.join("a.txt"), "v1\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("a.txt")).unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = git2::Signature::now("GitPulse Test", "test@gitpulse.local").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "v1", &tree, &[])
            .unwrap();
    }

    #[test]
    fn list_branches_shows_head_first() {
        let dir = base_dir("list");
        init_repo_with_commit(&dir);

        let branches = list_branches(&dir).unwrap();
        assert_eq!(branches.len(), 1);
        assert!(branches[0].is_head);
        assert_eq!(branches[0].upstream, None);
        assert!(!branches[0].name.is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_switch_delete_lifecycle() {
        let dir = base_dir("lifecycle");
        init_repo_with_commit(&dir);
        let head_name = list_branches(&dir).unwrap()[0].name.clone();

        // Create: appears in the list, HEAD stays on the original branch.
        create_branch(&dir, "feature").unwrap();
        let branches = list_branches(&dir).unwrap();
        assert!(branches.iter().any(|b| b.name == "feature" && !b.is_head));
        assert!(branches.iter().any(|b| b.name == head_name && b.is_head));

        // Switch: HEAD moves and the working tree follows.
        switch_branch(&dir, "feature").unwrap();
        let branches = list_branches(&dir).unwrap();
        assert!(branches.iter().any(|b| b.name == "feature" && b.is_head));
        assert!(branches.iter().any(|b| b.name == head_name && !b.is_head));

        // Cannot delete the current branch.
        let err = delete_branch(&dir, "feature").unwrap_err();
        assert!(err.to_string().contains("无法删除当前分支"), "{err}");

        // Switch back, then delete succeeds.
        switch_branch(&dir, &head_name).unwrap();
        delete_branch(&dir, "feature").unwrap();
        let branches = list_branches(&dir).unwrap();
        assert!(!branches.iter().any(|b| b.name == "feature"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn switch_to_missing_branch_reports_reason() {
        let dir = base_dir("missing");
        init_repo_with_commit(&dir);

        let err = switch_branch(&dir, "nope").unwrap_err();
        assert!(err.to_string().contains("不存在"), "{err}");

        let err = delete_branch(&dir, "nope").unwrap_err();
        assert!(err.to_string().contains("不存在"), "{err}");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_branches_on_non_repo_reports_error() {
        let dir = base_dir("norepo");
        assert!(list_branches(&dir).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
