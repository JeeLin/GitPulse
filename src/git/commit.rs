//! Commit creation: write the index tree and commit it with a message.

use std::path::Path;

use anyhow::{Context, Result};
use git2::{Oid, Repository, Signature};

/// Create a commit from the current index and move `HEAD` to it.
///
/// The signature is taken from the repository config (`user.name` /
/// `user.email`); a missing identity or an empty message is an error, as is
/// an index whose tree matches the parent (nothing to commit).
pub fn commit(repo_path: &Path, message: &str) -> Result<Oid> {
    let message = message.trim();
    if message.is_empty() {
        anyhow::bail!("commit message 不能为空");
    }

    let repo = Repository::open(repo_path)?;
    let config = repo.config()?;
    let name = config
        .get_string("user.name")
        .context("缺少 user.name 配置")?;
    let email = config
        .get_string("user.email")
        .context("缺少 user.email 配置")?;
    let sig = Signature::now(&name, &email)?;

    let mut index = repo.index()?;
    index.write()?;
    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;

    let parent_commit = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
    if let Some(parent) = &parent_commit {
        if parent.tree_id() == tree_id {
            anyhow::bail!("没有可提交的更改");
        }
    }
    let parents: Vec<&git2::Commit> = parent_commit.iter().collect();
    let oid = repo.commit(Some("HEAD"), &sig, &sig, message, &tree, parents.as_slice())?;
    Ok(oid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gitpulse-commit-{tag}-{}", std::process::id()));
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

    #[test]
    fn commit_creates_commit_visible_in_git_log() {
        let dir = temp_dir("log");
        init_repo(&dir);

        fs::write(dir.join("hello.txt"), "hi\n").unwrap();
        crate::git::stage(&dir, Path::new("hello.txt")).unwrap();
        let oid = commit(&dir, "test commit message").unwrap();

        // The new commit is reachable from HEAD and shows up in `git log`.
        let repo = Repository::open(&dir).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head.id(), oid);
        assert_eq!(head.summary(), Some("test commit message"));

        let log = Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(["log", "--oneline"])
            .output()
            .unwrap();
        assert!(log.status.success());
        let log_text = String::from_utf8_lossy(&log.stdout);
        assert!(
            log_text.contains("test commit message"),
            "commit missing from git log: {log_text}"
        );

        // Nothing staged anymore: a second commit must be rejected.
        let err = commit(&dir, "again").unwrap_err();
        assert!(err.to_string().contains("没有可提交的更改"), "{err}");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn commit_rejects_empty_message() {
        let dir = temp_dir("empty");
        init_repo(&dir);
        let err = commit(&dir, "   ").unwrap_err();
        assert!(err.to_string().contains("不能为空"), "{err}");
        let _ = fs::remove_dir_all(&dir);
    }
}
