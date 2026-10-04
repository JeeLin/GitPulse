//! Repository discovery: scan directories for git repositories, skipping symlinks.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;

use super::status::{query_status, RepoInfo};

/// Discover git repositories under the given directories.
///
/// Recursively walks each scan directory, detecting repositories by the
/// presence of a `.git` entry. Symlinks are never followed (checkpoint 5),
/// and recursion stops at repository boundaries so nested repos are not
/// visited. Per-repository query failures are skipped rather than aborting
/// the whole scan.
///
/// Overlapping scan directories (e.g. `/code` and `/code/rust`) can report
/// the same repository twice, so results are deduplicated by canonical path
/// with the first occurrence winning.
pub fn discover_repos(dirs: &[PathBuf]) -> Result<Vec<RepoInfo>> {
    let mut repos = Vec::new();
    for dir in dirs {
        discover_in_dir(dir, &mut repos);
    }
    let mut seen: HashSet<PathBuf> = HashSet::new();
    repos.retain(|repo| seen.insert(canonical_or_self(&repo.path)));
    Ok(repos)
}

/// Canonical form of `path` for deduplication and scan-dir storage.
///
/// Shared by repository discovery and the config panel's scan-dir add path so
/// both sides agree on what counts as "the same directory".
///
/// Falls back to the path itself when canonicalization fails (it vanished or
/// is unreadable between checks), so deduplication degrades to a literal
/// comparison instead of rejecting or dropping the entry outright.
pub fn canonical_or_self(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Recursively discover git repositories under `dir`.
fn discover_in_dir(dir: &Path, repos: &mut Vec<RepoInfo>) {
    // The scan directory itself may be a repository.
    if is_git_repo(dir) {
        match query_status(dir) {
            Ok(info) => repos.push(info),
            // Skip repositories that fail status query; do not abort the scan.
            Err(_err) => {}
        }
        // Do not recurse into a repository's subdirectories.
        return;
    }

    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        // Permission denied or other I/O error: skip this directory.
        Err(_err) => return,
    };

    for entry in entries.flatten() {
        // DirEntry::file_type does not traverse symlinks (checkpoint 5).
        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            discover_in_dir(&entry.path(), repos);
        }
    }
}

/// Check if a path contains a `.git` entry (file for worktrees, dir for normal repos).
fn is_git_repo(path: &Path) -> bool {
    path.join(".git").exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gitpulse-discover-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn discover_finds_git_repos() {
        let scan_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        let repos = discover_repos(&[scan_dir]).unwrap();
        assert!(repos.iter().any(|r| r.path == *env!("CARGO_MANIFEST_DIR")));
    }

    #[test]
    fn discover_dedupes_nested_scan_dirs() {
        let outer = temp_dir("nested");
        let repo_dir = outer.join("inner");
        fs::create_dir_all(&repo_dir).unwrap();
        git2::Repository::init(&repo_dir).unwrap();

        // Outer directory plus the same repository reached through a path
        // with a redundant `..` component: both walks find one repository but
        // the literal paths differ, so only canonical dedup collapses them.
        let alias = repo_dir.join("..").join("inner");
        let repos = discover_repos(&[outer.clone(), alias]).unwrap();
        let want = fs::canonicalize(&repo_dir).unwrap();
        let hits = repos
            .iter()
            .filter(|r| fs::canonicalize(&r.path).unwrap() == want)
            .count();
        assert_eq!(
            hits, 1,
            "nested repository must be reported once: {repos:?}"
        );

        let _ = fs::remove_dir_all(&outer);
    }
}
