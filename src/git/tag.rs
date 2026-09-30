//! Tag operations: list / create / delete lightweight tags.
//!
//! All functions take a repository path and open the repository on the
//! calling thread, matching the other single-repo helpers in this module.

use std::fmt::Write;
use std::path::Path;

use anyhow::{Context, Result};
use git2::{ObjectType, Repository};

use super::short_id;

/// Snapshot of one repository tag for the tag panel.
#[derive(Debug)]
pub struct TagInfo {
    /// Tag name (without the `refs/tags/` prefix); a name that is not valid
    /// UTF-8 is rendered with the same escaping as [`super::show_path`],
    /// e.g. `bad-\xFF-tag`, instead of being dropped.
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
    // Byte-level walk instead of `tag_names`, whose `StringArray` yields
    // `None` for a tag name that is not valid UTF-8 and would silently hide
    // the tag from the panel.
    for reference in repo.references()?.flatten() {
        if !reference.name_bytes().starts_with(b"refs/tags/") {
            continue;
        }
        // Shorthand is byte-identical to `shorthand()` minus the prefix; a
        // non-UTF-8 tag name is rendered escaped instead of being dropped.
        let name = show_bytes(reference.shorthand_bytes());
        // Determine if tag is annotated by attempting to peel to a tag object.
        let annotated = reference.peel(ObjectType::Tag).is_ok();
        // Get the target commit (works for both annotated and lightweight tags).
        let Ok(commit) = reference.peel_to_commit() else {
            continue;
        };
        let target = short_id(&commit.id());
        tags.push(TagInfo {
            name,
            target,
            annotated,
        });
    }
    // Sort alphabetically by tag name.
    tags.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(tags)
}

/// Display form of a raw git reference name for user-facing text: the exact
/// UTF-8 spelling when `bytes` are valid Unicode, otherwise the `Debug`
/// escaping [`super::show_path`] applies to a non-UTF-8 path, so invalid
/// bytes surface as `\xFF` instead of being dropped.
///
/// Kept in step with the identical `show_bytes` helper private to the
/// `history` module of this crate — change both together.
fn show_bytes(bytes: &[u8]) -> String {
    let Ok(text) = std::str::from_utf8(bytes) else {
        let mut out = String::with_capacity(bytes.len() + 2);
        out.push('"');
        for chunk in bytes.utf8_chunks() {
            let valid = chunk.valid();
            if !valid.is_empty() {
                // `{:?}` on a `&str` wraps it in quotes; only the escaped
                // interior belongs here, the outer quotes are added once.
                let escaped = format!("{valid:?}");
                out.push_str(&escaped[1..escaped.len() - 1]);
            }
            for byte in chunk.invalid() {
                let _ = write!(out, "\\x{byte:02X}");
            }
        }
        out.push('"');
        return out;
    };
    text.to_string()
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

    /// A tag whose name is not valid UTF-8 must still reach the tag panel,
    /// escaped like `show_path` escapes a non-UTF-8 path, while an ordinary
    /// UTF-8 tag next to it stays byte-identical.
    #[cfg(unix)]
    #[test]
    fn list_tags_shows_non_utf8_tag_escaped() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let dir = base_dir("nonutf8");
        init_repo_with_commit(&dir);
        let repo = Repository::open(&dir).unwrap();
        let head = repo.head().unwrap();
        let object = head.peel(ObjectType::Commit).unwrap();
        repo.tag_lightweight("utf8-tag", &object, false).unwrap();

        // `git2` only accepts `&str` reference names, so the loose ref file
        // is written directly with a name that is not valid UTF-8.
        let raw_name = b"bad-\xFF-tag";
        let ref_dir = repo.path().join("refs/tags");
        fs::create_dir_all(&ref_dir).unwrap();
        fs::write(
            ref_dir.join(OsStr::from_bytes(raw_name)),
            format!("{}\n", object.id()),
        )
        .unwrap();

        let tags = list_tags(&dir).unwrap();
        assert_eq!(tags.len(), 2);
        assert!(tags.iter().any(|t| t.name == "utf8-tag"));
        let bad = tags.iter().find(|t| t.name.contains("bad-")).unwrap();
        assert_eq!(bad.name, "\"bad-\\xFF-tag\"");
        assert!(!bad.annotated);
        let expected_target = format!("{:.7}", object.id());
        assert_eq!(bad.target, expected_target);
        let _ = fs::remove_dir_all(&dir);
    }
}
