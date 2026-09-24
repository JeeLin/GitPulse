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

    let tree_opt = head_obj_opt.as_ref().and_then(|o| o.as_tree());

    let mut diff_opts = git2::DiffOptions::new();
    diff_opts.pathspec(rel_path);

    let diff = repo
        .diff_tree_to_workdir_with_index(tree_opt, Some(&mut diff_opts))
        .with_context(|| format!("生成 diff 失败: {}", rel_path.display()))?;

    let mut diff_str = String::new();

    diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
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
            let in_head = obj
                .as_tree()
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

    let exists_in_head = head_obj
        .as_ref()
        .and_then(|o| o.as_tree())
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
}
