//! Single-repo file operations: status list, diff, stage/unstage, discard.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use git2::{ObjectType, Repository, Status, StatusOptions};

/// State classification for a file in the working tree or index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileState {
    /// Modified in working tree or index.
    Modified,
    /// Added to index (staged new file).
    Added,
    /// Deleted from working tree or index.
    Deleted,
    /// Untracked new file.
    Untracked,
    /// Merge conflict.
    Conflicted,
}

/// A single file entry in the repository status list.
#[derive(Debug, Clone)]
pub struct FileEntry {
    /// Path relative to the repository working directory.
    pub path: PathBuf,
    /// Current state classification.
    pub state: FileState,
}

/// List all changed/untracked files in the repository.
pub fn list_file_status(repo_path: &Path) -> Result<Vec<FileEntry>> {
    let repo = Repository::open(repo_path)?;
    let mut opts = StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false);

    let statuses = repo.statuses(Some(&mut opts))?;
    let entries = statuses
        .iter()
        .filter_map(|entry| {
            let status = entry.status();
            let path = entry.path()?.to_owned();
            let state = if status.contains(Status::CONFLICTED) {
                FileState::Conflicted
            } else if status.contains(Status::INDEX_NEW) {
                FileState::Added
            } else if status.contains(Status::WT_NEW) {
                FileState::Untracked
            } else if status.contains(Status::INDEX_DELETED) || status.contains(Status::WT_DELETED)
            {
                FileState::Deleted
            } else if status.contains(Status::INDEX_MODIFIED)
                || status.contains(Status::WT_MODIFIED)
            {
                FileState::Modified
            } else {
                return None;
            };
            Some(FileEntry {
                path: path.into(),
                state,
            })
        })
        .collect();
    Ok(entries)
}

/// Produce the unified diff for a single file (HEAD tree → working directory, blended with index).
pub fn diff_file(repo_path: &Path, rel_path: &Path) -> Result<String> {
    let repo = Repository::open(repo_path)?;
    let head_obj_opt = repo
        .head()
        .ok()
        .and_then(|h| h.peel(ObjectType::Commit).ok());

    // Peel down to the tree: a non-peeling `as_tree()` cast on the commit
    // itself would return `None` and diff against an empty tree.
    let head_tree = head_obj_opt.as_ref().and_then(|o| o.peel_to_tree().ok());

    let mut diff_opts = git2::DiffOptions::new();
    diff_opts.pathspec(rel_path);

    let diff = repo
        .diff_tree_to_workdir_with_index(head_tree.as_ref(), Some(&mut diff_opts))
        .with_context(|| format!("生成 diff 失败: {}", rel_path.display()))?;

    let mut diff_str = String::new();

    diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
        // libgit2 keeps the +/-/space origin outside `content()`; re-attach it
        // for change lines so the UI can colour them (headers have no origin).
        if matches!(
            line.origin_value(),
            git2::DiffLineType::Context
                | git2::DiffLineType::Addition
                | git2::DiffLineType::Deletion
        ) {
            diff_str.push(line.origin());
        }
        diff_str.push_str(std::str::from_utf8(line.content()).unwrap_or(""));
        true
    })?;

    if diff_str.is_empty() {
        anyhow::bail!("文件无更改或不在 diff 中");
    }
    Ok(diff_str)
}

/// Stage (add to index) a file. Handles both existing files and staged deletions.
pub fn stage(repo_path: &Path, rel_path: &Path) -> Result<()> {
    let repo = Repository::open(repo_path)?;
    let mut index = repo.index()?;

    let workdir = repo.workdir().map(Path::to_path_buf);
    let full_path = workdir
        .as_ref()
        .map(|w| w.join(rel_path))
        .unwrap_or_default();

    if full_path.exists() {
        index.add_path(rel_path)?;
    } else {
        index.remove_path(rel_path)?;
    }
    index.write()?;
    Ok(())
}

/// Unstage a file: reset the index entry back to HEAD (or remove it from index if not in HEAD).
///
/// A path absent from both HEAD and the index has nothing to unstage and
/// returns `Ok` (not an error); real reset failures propagate so the UI
/// never reports success for a failed operation.
pub fn unstage(repo_path: &Path, rel_path: &Path) -> Result<()> {
    let repo = Repository::open(repo_path)?;

    // Try to get HEAD commit object
    let head_obj = repo
        .head()
        .ok()
        .and_then(|h| h.peel(ObjectType::Commit).ok());

    match head_obj {
        Some(obj) => {
            let head_tree = obj.peel_to_tree().ok();
            let in_head = head_tree
                .as_ref()
                .and_then(|t| t.get_path(rel_path).ok())
                .is_some();
            let in_index = repo.index()?.get_path(rel_path, 0).is_some();
            if !in_head && !in_index {
                return Ok(());
            }
            repo.reset_default(Some(&obj), [rel_path])?;
            Ok(())
        }
        None => {
            // No HEAD (unborn): just remove the path from the index entirely
            let mut index = repo.index()?;
            if index.get_path(rel_path, 0).is_some() {
                index.remove_path(rel_path)?;
                index.write()?;
            }
            Ok(())
        }
    }
}

/// Discard all changes to a file: unstage it, restore from HEAD to working tree, and remove untracked files.
pub fn discard(repo_path: &Path, rel_path: &Path) -> Result<()> {
    let repo = Repository::open(repo_path)?;

    // 1. Unstage if staged
    unstage(repo_path, rel_path)?;

    // 2. If file exists in HEAD, checkout the HEAD version to working tree (force).
    //    If not in HEAD (was untracked or newly added), remove it from working tree.
    let head_obj = repo
        .head()
        .ok()
        .and_then(|h| h.peel(ObjectType::Commit).ok());

    let head_tree = head_obj.as_ref().and_then(|o| o.peel_to_tree().ok());
    let exists_in_head = head_tree
        .as_ref()
        .and_then(|t| t.get_path(rel_path).ok())
        .is_some();

    if exists_in_head {
        let mut checkout_opts = git2::build::CheckoutBuilder::new();
        checkout_opts.path(rel_path).force();
        repo.checkout_index(None, Some(&mut checkout_opts))?;
    } else {
        let workdir = repo.workdir().map(Path::to_path_buf);
        if let Some(wd) = workdir {
            let full_path = wd.join(rel_path);
            if full_path.exists() {
                std::fs::remove_file(&full_path)?;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::fs;
    use std::process::Command;

    #[test]
    fn list_file_status_does_not_panic_on_current_repo() {
        let repo_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let entries = list_file_status(&repo_path);
        assert!(
            entries.is_ok(),
            "list_file_status failed: {:?}",
            entries.err()
        );
        // The repo should either be clean or have some entries, but not panic.
        let _ = entries.unwrap();
    }

    #[test]
    fn file_state_debug_impl() {
        // Ensure FileState derives Debug for test output
        let states = [
            FileState::Modified,
            FileState::Added,
            FileState::Deleted,
            FileState::Untracked,
            FileState::Conflicted,
        ];
        for s in states {
            let _ = format!("{s:?}");
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("gitpulse-file-{tag}-{}", std::process::id()));
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

    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn commit_file(dir: &Path, name: &str, content: &str, message: &str) {
        fs::write(dir.join(name), content).unwrap();
        git(dir, &["add", "--", name]);
        git(dir, &["commit", "-m", message]);
    }

    /// Raw git2 status flags for a single path (empty status when absent).
    fn status_of(dir: &Path, name: &str) -> Status {
        let repo = Repository::open(dir).unwrap();
        let mut opts = StatusOptions::new();
        opts.include_untracked(true);
        let statuses = repo.statuses(Some(&mut opts)).unwrap();
        statuses
            .iter()
            .find(|entry| entry.path() == Some(name))
            .map(|entry| entry.status())
            .unwrap_or(Status::CURRENT)
    }

    #[test]
    fn stage_moves_tracked_modification_into_index() {
        let dir = temp_dir("stage-mod");
        init_repo(&dir);
        commit_file(&dir, "a.txt", "v1\n", "v1");
        fs::write(dir.join("a.txt"), "v2 modified\n").unwrap();

        let before = status_of(&dir, "a.txt");
        assert!(before.contains(Status::WT_MODIFIED), "{before:?}");
        assert!(!before.contains(Status::INDEX_MODIFIED), "{before:?}");

        stage(&dir, Path::new("a.txt")).unwrap();

        let after = status_of(&dir, "a.txt");
        assert!(after.contains(Status::INDEX_MODIFIED), "{after:?}");
        assert!(!after.contains(Status::WT_MODIFIED), "{after:?}");
        let entries = list_file_status(&dir).unwrap();
        assert!(
            entries
                .iter()
                .any(|e| e.path == Path::new("a.txt") && e.state == FileState::Modified),
            "{entries:?}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn stage_removes_deleted_tracked_file_from_index() {
        let dir = temp_dir("stage-del");
        init_repo(&dir);
        commit_file(&dir, "a.txt", "v1\n", "v1");
        fs::remove_file(dir.join("a.txt")).unwrap();

        stage(&dir, Path::new("a.txt")).unwrap();

        let repo = Repository::open(&dir).unwrap();
        assert!(repo
            .index()
            .unwrap()
            .get_path(Path::new("a.txt"), 0)
            .is_none());
        let entries = list_file_status(&dir).unwrap();
        assert!(
            entries
                .iter()
                .any(|e| e.path == Path::new("a.txt") && e.state == FileState::Deleted),
            "{entries:?}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unstage_returns_staged_modification_to_worktree() {
        let dir = temp_dir("unstage-mod");
        init_repo(&dir);
        commit_file(&dir, "a.txt", "v1\n", "v1");
        fs::write(dir.join("a.txt"), "v2 modified\n").unwrap();
        stage(&dir, Path::new("a.txt")).unwrap();
        assert!(status_of(&dir, "a.txt").contains(Status::INDEX_MODIFIED));

        unstage(&dir, Path::new("a.txt")).unwrap();

        let status = status_of(&dir, "a.txt");
        assert!(status.contains(Status::WT_MODIFIED), "{status:?}");
        assert!(!status.contains(Status::INDEX_MODIFIED), "{status:?}");
        assert_eq!(
            fs::read_to_string(dir.join("a.txt")).unwrap(),
            "v2 modified\n"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unstage_removes_staged_new_file_from_index() {
        let dir = temp_dir("unstage-new");
        init_repo(&dir);
        commit_file(&dir, "a.txt", "v1\n", "v1");
        fs::write(dir.join("b.txt"), "new\n").unwrap();
        stage(&dir, Path::new("b.txt")).unwrap();
        let entries = list_file_status(&dir).unwrap();
        assert!(
            entries
                .iter()
                .any(|e| e.path == Path::new("b.txt") && e.state == FileState::Added),
            "{entries:?}"
        );

        unstage(&dir, Path::new("b.txt")).unwrap();

        let repo = Repository::open(&dir).unwrap();
        assert!(repo
            .index()
            .unwrap()
            .get_path(Path::new("b.txt"), 0)
            .is_none());
        assert!(dir.join("b.txt").exists());
        let entries = list_file_status(&dir).unwrap();
        assert!(
            entries
                .iter()
                .any(|e| e.path == Path::new("b.txt") && e.state == FileState::Untracked),
            "{entries:?}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unstage_ignores_path_absent_from_head_and_index() {
        let dir = temp_dir("unstage-ghost");
        init_repo(&dir);
        commit_file(&dir, "a.txt", "v1\n", "v1");

        unstage(&dir, Path::new("ghost.txt")).unwrap();

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unstage_on_unborn_head_clears_index_entry() {
        let dir = temp_dir("unstage-unborn");
        init_repo(&dir);
        fs::write(dir.join("a.txt"), "v1\n").unwrap();
        stage(&dir, Path::new("a.txt")).unwrap();
        let repo = Repository::open(&dir).unwrap();
        assert!(repo
            .index()
            .unwrap()
            .get_path(Path::new("a.txt"), 0)
            .is_some());

        unstage(&dir, Path::new("a.txt")).unwrap();

        let repo = Repository::open(&dir).unwrap();
        assert!(repo
            .index()
            .unwrap()
            .get_path(Path::new("a.txt"), 0)
            .is_none());
        assert!(
            dir.join("a.txt").exists(),
            "worktree file must survive unstage"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn discard_restores_modified_tracked_file() {
        let dir = temp_dir("discard-mod");
        init_repo(&dir);
        commit_file(&dir, "a.txt", "v1\n", "v1");
        fs::write(dir.join("a.txt"), "v2 modified\n").unwrap();

        discard(&dir, Path::new("a.txt")).unwrap();

        assert_eq!(fs::read_to_string(dir.join("a.txt")).unwrap(), "v1\n");
        assert!(list_file_status(&dir).unwrap().is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn discard_removes_untracked_file() {
        let dir = temp_dir("discard-untracked");
        init_repo(&dir);
        commit_file(&dir, "a.txt", "v1\n", "v1");
        fs::write(dir.join("b.txt"), "new\n").unwrap();
        assert!(list_file_status(&dir)
            .unwrap()
            .iter()
            .any(|e| e.state == FileState::Untracked));

        discard(&dir, Path::new("b.txt")).unwrap();

        assert!(!dir.join("b.txt").exists());
        assert!(list_file_status(&dir).unwrap().is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn discard_removes_staged_new_file() {
        let dir = temp_dir("discard-staged-new");
        init_repo(&dir);
        commit_file(&dir, "a.txt", "v1\n", "v1");
        fs::write(dir.join("b.txt"), "new\n").unwrap();
        stage(&dir, Path::new("b.txt")).unwrap();

        discard(&dir, Path::new("b.txt")).unwrap();

        assert!(!dir.join("b.txt").exists());
        let repo = Repository::open(&dir).unwrap();
        assert!(repo
            .index()
            .unwrap()
            .get_path(Path::new("b.txt"), 0)
            .is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn diff_file_returns_patch_with_added_and_removed_lines() {
        let dir = temp_dir("diff");
        init_repo(&dir);
        commit_file(&dir, "a.txt", "v1\n", "v1");
        fs::write(dir.join("a.txt"), "v2 modified\n").unwrap();

        let diff = diff_file(&dir, Path::new("a.txt")).unwrap();
        assert!(diff.contains("-v1"), "missing removal line: {diff}");
        assert!(
            diff.contains("+v2 modified"),
            "missing addition line: {diff}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn diff_file_errors_when_file_has_no_changes() {
        let dir = temp_dir("diff-clean");
        init_repo(&dir);
        commit_file(&dir, "a.txt", "v1\n", "v1");

        let err = diff_file(&dir, Path::new("a.txt")).unwrap_err();
        assert!(
            err.to_string().contains("文件无更改或不在 diff 中"),
            "{err}"
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
