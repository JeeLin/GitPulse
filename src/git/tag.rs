//! Tag operations: list / create / delete lightweight tags.
//!
//! All functions take a repository path and open the repository on the
//! calling thread, matching the other single-repo helpers in this module.

use std::path::Path;

use anyhow::{Context, Result};
use git2::{ObjectType, Repository};

use super::short_id;

/// Snapshot of one repository tag for the tag panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagInfo {
    /// Tag name (without the `refs/tags/` prefix).
    pub name: String,
    /// Short (7 character) hash of the commit the tag points at.
    pub target: String,
    /// Whether the tag carries an annotated tag object instead of pointing
    /// straight at a commit.
    pub annotated: bool,
}

/// List all tags in the repository, sorted alphabetically by name.
///
/// Returns an empty vector for repositories with no tags.
/// Returns an error if the path does not point to a valid Git repository.
pub fn list_tags(repo_path: &Path) -> Result<Vec<TagInfo>> {
    let repo = Repository::open(repo_path)?;
    let mut tags = Vec::new();
    for name_opt in repo.tag_names(None)?.iter() {
        let Some(name) = name_opt else {
            continue;
        };
        let reference = match repo.find_reference(&format!("refs/tags/{name}")) {
            Ok(r) => r,
            Err(_) => continue,
        };
        // Determine if tag is annotated by attempting to peel to a tag object.
        let annotated = reference.peel(ObjectType::Tag).is_ok();
        // Get the target commit (works for both annotated and lightweight tags).
        let Ok(commit) = reference.peel_to_commit() else {
            continue;
        };
        let target = short_id(&commit.id());
        tags.push(TagInfo {
            name: name.to_string(),
            target,
            annotated,
        });
    }
    // Sort alphabetically by tag name.
    tags.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(tags)
}

/// Create a lightweight tag at the current HEAD.
///
/// # Errors
///
/// - Returns an error if the repository path is invalid.
/// - Returns an error if the repository has no commits (HEAD cannot be resolved).
/// - Returns an error if the tag name is empty.
/// - Returns an error if a tag with the given name already exists.
pub fn create_tag(repo_path: &Path, name: &str) -> Result<()> {
    // Git-layer defence for direct API callers; the UI keeps its own empty
    // check only to give a friendly message before opening the repository.
    if name.trim().is_empty() {
        anyhow::bail!("标签名称不能为空");
    }
    let repo = Repository::open(repo_path)?;
    let head = repo.head().context("无法解析 HEAD，仓库可能尚无提交")?;
    let object = head.peel(ObjectType::Commit)?;
    // Check for duplicate tag.
    let refname = format!("refs/tags/{name}");
    if repo.find_reference(&refname).is_ok() {
        anyhow::bail!("标签 {name} 已存在");
    }
    repo.tag_lightweight(name, &object, false)
        .with_context(|| format!("创建标签 {name} 失败"))?;
    Ok(())
}

/// Delete a lightweight or annotated tag from the repository.
///
/// # Errors
///
/// - Returns an error if the repository path is invalid.
/// - Returns an error if no tag with the given name exists.
/// - Returns an error if the tag deletion fails for any other reason.
pub fn delete_tag(repo_path: &Path, name: &str) -> Result<()> {
    let repo = Repository::open(repo_path)?;
    let refname = format!("refs/tags/{name}");
    // Check existence first to provide a clear error message.
    repo.find_reference(&refname)
        .with_context(|| format!("标签 {name} 不存在"))?;
    repo.tag_delete(name)
        .with_context(|| format!("删除标签 {name} 失败"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn base_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gitpulse-tag-{tag}-{}", std::process::id()));
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
    fn list_tags_shows_name_short_hash_and_annotated() {
        let dir = base_dir("list");
        init_repo_with_commit(&dir);
        let repo = Repository::open(&dir).unwrap();
        let head = repo.head().unwrap();
        let object = head.peel(ObjectType::Commit).unwrap();

        // Create a lightweight tag.
        repo.tag_lightweight("v-light", &object, false).unwrap();
        // Create an annotated tag.
        let sig = git2::Signature::now("GitPulse Test", "test@gitpulse.local").unwrap();
        repo.tag("v-annotated", &object, &sig, "annotated tag", false)
            .unwrap();

        let tags = list_tags(&dir).unwrap();
        assert_eq!(tags.len(), 2);
        // Find each tag by name.
        let light = tags.iter().find(|t| t.name == "v-light").unwrap();
        let annotated = tags.iter().find(|t| t.name == "v-annotated").unwrap();
        // Lightweight tag: not annotated, target matches HEAD short hash.
        assert!(!light.annotated);
        let expected_target = format!("{:.7}", object.id());
        assert_eq!(light.target, expected_target);
        // Annotated tag: annotated true, same target.
        assert!(annotated.annotated);
        assert_eq!(annotated.target, expected_target);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_tag_appears_in_list() {
        let dir = base_dir("create");
        init_repo_with_commit(&dir);
        create_tag(&dir, "feature-v1").unwrap();
        let tags = list_tags(&dir).unwrap();
        assert!(tags.iter().any(|t| t.name == "feature-v1"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_tag_rejects_duplicate() {
        let dir = base_dir("dup");
        init_repo_with_commit(&dir);
        create_tag(&dir, "v1").unwrap();
        let err = create_tag(&dir, "v1").unwrap_err();
        assert!(err.to_string().contains("已存在"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_tag_rejects_empty_name() {
        let dir = base_dir("empty");
        init_repo_with_commit(&dir);
        let err = create_tag(&dir, "").unwrap_err();
        assert!(err.to_string().contains("不能为空"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_tag_removes_from_list() {
        let dir = base_dir("delete");
        init_repo_with_commit(&dir);
        create_tag(&dir, "todelete").unwrap();
        // Verify it exists.
        assert!(list_tags(&dir)
            .unwrap()
            .iter()
            .any(|t| t.name == "todelete"));
        // Delete it.
        delete_tag(&dir, "todelete").unwrap();
        // Verify it's gone.
        assert!(!list_tags(&dir)
            .unwrap()
            .iter()
            .any(|t| t.name == "todelete"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_nonexistent_tag_errors() {
        let dir = base_dir("missing");
        init_repo_with_commit(&dir);
        let err = delete_tag(&dir, "nope").unwrap_err();
        assert!(err.to_string().contains("不存在"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_tags_on_empty_repo_returns_empty() {
        let dir = base_dir("emptyrepo");
        let repo = Repository::init(&dir).unwrap();
        // Set up config so Repository operations work.
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "GitPulse Test").unwrap();
        config.set_str("user.email", "test@gitpulse.local").unwrap();
        let tags = list_tags(&dir).unwrap();
        assert!(tags.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_tags_on_non_repo_errors() {
        let dir = base_dir("norepo");
        assert!(list_tags(&dir).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
