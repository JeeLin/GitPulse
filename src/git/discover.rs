//! Repository discovery: scan directories for git repositories, skipping symlinks.

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
pub fn discover_repos(dirs: &[PathBuf]) -> Result<Vec<RepoInfo>> {
    let mut repos = Vec::new();
    for dir in dirs {
        discover_in_dir(dir, &mut repos);
    }
    Ok(repos)
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

    #[test]
    fn discover_finds_git_repos() {
        let scan_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        let repos = discover_repos(&[scan_dir]).unwrap();
        assert!(repos
            .iter()
            .any(|r| r.path == PathBuf::from(env!("CARGO_MANIFEST_DIR"))));
    }
}
