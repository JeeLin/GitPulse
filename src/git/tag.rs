//! Tag operations: list / create / delete lightweight tags.
//!
//! All functions take a repository path and open the repository on the
//! calling thread, matching the other single-repo helpers in this module.

use std::path::Path;

use anyhow::{Context, Result};
use git2::{ObjectType, Oid, Repository};

use super::{short_id, show_bytes};

/// Snapshot of one repository tag for the tag panel.
#[derive(Debug)]
pub struct TagInfo {
    /// Tag name (without the `refs/tags/` prefix); a name that is not valid
    /// UTF-8 is rendered with the same escaping as [`super::show_path`],
    /// e.g. `bad-\xFF-tag`, instead of being dropped.
    pub name: String,
    /// Raw name bytes (without the `refs/tags/` prefix), the round-trip-safe
    /// counterpart of [`TagInfo::name`]: the escaped display form is not
    /// injective and can never be fed back as a ref name, so operations that
    /// need the real ref check these bytes for valid UTF-8 first (H1/V2).
    pub name_bytes: Vec<u8>,
    /// Short (7 character) hash of the commit the tag points at, or
    /// `<object type> <short id>` (e.g. `tree abc1234`) when the tag points
    /// at a non-commit object such as `git tag t-tree HEAD^{tree}` (H10).
    pub target: String,
    /// Whether the tag carries an annotated tag object instead of pointing
    /// straight at a commit.
    pub annotated: bool,
}

/// What a tag reference ultimately resolves to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TagTarget {
    /// The reference peels to a commit (the common case).
    Commit(Oid),
    /// The reference points at a non-commit object (`git tag t-tree
    /// HEAD^{tree}` is legal) — kept visible in the panel instead of being
    /// silently skipped (H10).
    Other {
        /// Object type label, e.g. `tree`.
        kind: &'static str,
        /// Object id the reference resolves to.
        id: Oid,
    },
}

/// One `refs/tags/*` reference produced by [`walk_tag_refs`].
pub(super) struct WalkedTag {
    /// Short name without the prefix; non-UTF-8 names are escaped.
    pub name: String,
    /// Raw short-name bytes without the prefix, kept beside the escaped
    /// display form so callers can round-trip the ref (H1/V2).
    pub name_bytes: Vec<u8>,
    /// Whether the reference resolves through an annotated tag object.
    pub annotated: bool,
    /// Where the reference ultimately points.
    pub target: TagTarget,
}

/// Label for the object type of a [`TagTarget::Other`] target.
fn kind_label(kind: ObjectType) -> &'static str {
    match kind {
        ObjectType::Commit => "commit",
        ObjectType::Tree => "tree",
        ObjectType::Blob => "blob",
        ObjectType::Tag => "tag",
        _ => "object",
    }
}

/// Walk every `refs/tags/*` reference at the byte level, sorted by name.
///
/// This is the single enumeration shared by [`list_tags`] and the history
/// ref marks (G33): byte-level iteration keeps a tag whose name is not valid
/// UTF-8 visible instead of the `None` entries `tag_names` yields for such
/// names, and packed refs are covered by `Repository::references`. A
/// reference whose object is missing on disk (dangling) is skipped instead
/// of failing the whole list.
pub(super) fn walk_tag_refs(repo: &Repository) -> Result<Vec<WalkedTag>> {
    let mut tags = Vec::new();
    for reference in repo.references()?.flatten() {
        if !reference.name_bytes().starts_with(b"refs/tags/") {
            continue;
        }
        // Shorthand is byte-identical to `shorthand()` minus the prefix; a
        // non-UTF-8 tag name is rendered escaped instead of being dropped,
        // while the raw bytes stay available for write paths (H1/V2).
        let name_bytes = reference.shorthand_bytes().to_vec();
        let name = show_bytes(&name_bytes);
        // Determine if tag is annotated by attempting to peel to a tag object.
        let annotated = reference.peel(ObjectType::Tag).is_ok();
        let target = match reference.peel_to_commit() {
            Ok(commit) => TagTarget::Commit(commit.id()),
            // Not a commit: record the resolved object (H10). Take the
            // reference's direct target and follow annotated tag objects
            // until the first non-tag one; a dangling reference (object
            // missing on disk) or a broken tag chain is skipped.
            Err(_) => {
                let Some(id) = reference.target() else {
                    continue;
                };
                let Some(mut object) = repo.find_object(id, None).ok() else {
                    continue;
                };
                while let Some(tag) = object.as_tag() {
                    match tag.target() {
                        Ok(next) => object = next,
                        Err(_) => break,
                    }
                }
                match object.kind() {
                    Some(kind) if kind != ObjectType::Tag => TagTarget::Other {
                        kind: kind_label(kind),
                        id: object.id(),
                    },
                    _ => continue,
                }
            }
        };
        tags.push(WalkedTag {
            name,
            name_bytes,
            annotated,
            target,
        });
    }
    tags.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(tags)
}

/// List all tags in the repository, sorted alphabetically by name.
///
/// Each entry carries the escaped display name ([`TagInfo::name`]) next to
/// the raw name bytes ([`TagInfo::name_bytes`]).
///
/// Returns an empty vector for repositories with no tags.
/// Returns an error if the path does not point to a valid Git repository.
pub fn list_tags(repo_path: &Path) -> Result<Vec<TagInfo>> {
    let repo = Repository::open(repo_path)?;
    let tags = walk_tag_refs(&repo)?
        .into_iter()
        .map(|tag| TagInfo {
            name: tag.name,
            name_bytes: tag.name_bytes,
            target: match tag.target {
                TagTarget::Commit(id) => short_id(&id),
                TagTarget::Other { kind, id } => format!("{kind} {}", short_id(&id)),
            },
            annotated: tag.annotated,
        })
        .collect();
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

    /// A tag whose name is not valid UTF-8 must still reach the tag panel,
    /// escaped like `show_path` escapes a non-UTF-8 path, while an ordinary
    /// UTF-8 tag next to it stays byte-identical. Both keep their raw ref
    /// bytes in `name_bytes` so the panel can round-trip them (H1/V2).
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
        let utf8 = tags.iter().find(|t| t.name == "utf8-tag").unwrap();
        assert_eq!(utf8.name_bytes, b"utf8-tag");
        let bad = tags.iter().find(|t| t.name.contains("bad-")).unwrap();
        assert_eq!(bad.name, "\"bad-\\xFF-tag\"");
        // H1/V2: `name_bytes` keeps the untouched ref bytes beside the
        // escaped display form, so the panel can round-trip the tag.
        assert_eq!(bad.name_bytes, raw_name);
        assert_ne!(
            bad.name.as_bytes(),
            bad.name_bytes.as_slice(),
            "the escaped display form must not double as the ref name"
        );
        assert!(!bad.annotated);
        let expected_target = format!("{:.7}", object.id());
        assert_eq!(bad.target, expected_target);
        let _ = fs::remove_dir_all(&dir);
    }

    /// A tag pointing at a non-commit object (`git tag t-tree HEAD^{tree}`
    /// is legal) must stay visible in the panel with an object-type target
    /// and remain deletable instead of being skipped (H10).
    #[test]
    fn list_tags_shows_non_commit_target_tag() {
        let dir = base_dir("tree-target");
        init_repo_with_commit(&dir);
        let repo = Repository::open(&dir).unwrap();
        let head = repo.head().unwrap();
        let tree = head.peel(ObjectType::Tree).unwrap();
        repo.tag_lightweight("t-tree", &tree, false).unwrap();

        let tags = list_tags(&dir).unwrap();
        let tree_tag = tags.iter().find(|t| t.name == "t-tree").unwrap();
        assert!(
            tree_tag.target.starts_with("tree "),
            "expected object-type target, got {:?}",
            tree_tag.target
        );
        assert!(!tree_tag.annotated);

        delete_tag(&dir, "t-tree").unwrap();
        assert!(!list_tags(&dir).unwrap().iter().any(|t| t.name == "t-tree"));
        let _ = fs::remove_dir_all(&dir);
    }
}
