//! Repository discovery: scan directories for git repositories, then enumerate
//! each repository's linked worktrees via its own worktree registry (sub-task 3,
//! checkpoint ③).
//!
//! Discovery has two phases:
//! 1. A recursive directory walk collects *main repository roots* — directories
//!    that contain a `.git` entry. Symlinks are never followed (checkpoint 5),
//!    and recursion stops at a repository boundary so nested repos are not
//!    descended into.
//! 2. For every main repo, [`query_status`] snapshot its HEAD state, then
//!    [`enumerate_worktrees`] walks the repo's worktree registry
//!    (`Repository::worktrees` + `find_worktree`) to surface linked worktrees as
//!    first-class rows: a worktree is distinguished from a submodule by appearing
//!    in the *parent* registry, not by a `.git` file-vs-dir heuristic. A broken
//!    or missing registry falls back to "no worktrees" instead of failing the
//!    whole scan.
//!
//! Per-item failures (a worktree whose status query errors, an invalid
//! worktree, a registry lookup error) are collected as human-readable
//! diagnostics and returned alongside the rows; the dashboard routes them to the
//! status line (transient) per §4.3, while only a catastrophic scan failure
//! becomes the outer `Result::Err` (panel `error` state).
//!
//! A hard cap of [`WORKTREE_HARD_CAP`] linked worktrees is enforced per main
//! repository; worktrees beyond that are neither queried nor rendered, only a
//! diagnostic is recorded (checkpoint ③).

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use git2::Repository;

use super::repo_display_name;
use super::status::{query_status, query_worktree_status, RepoInfo};

/// Hard upper bound on linked worktrees surfaced per main repository.
///
/// Beyond this, worktrees are neither status-queried nor rendered — only a
/// diagnostic is recorded — so discovery cost stays bounded regardless of how
/// many worktrees a repo accumulates (checkpoint ③).
const WORKTREE_HARD_CAP: usize = 32;

/// Discover git repositories and their linked worktrees under the given
/// directories.
///
/// Returns `(repos, diagnostics)`: the discovered repository rows and a list of
/// per-item failure messages (each prefixed with the offending repo/worktree
/// name) for the dashboard to land on the status line. Overlapping scan
/// directories can report the same repository twice, so results are deduplicated
/// by canonical path with the first occurrence winning.
pub fn discover_repos(dirs: &[PathBuf]) -> Result<(Vec<RepoInfo>, Vec<String>)> {
    let mut roots: Vec<PathBuf> = Vec::new();
    for dir in dirs {
        discover_in_dir(dir, &mut roots);
    }

    let mut repos: Vec<RepoInfo> = Vec::new();
    let mut diags: Vec<String> = Vec::new();
    for root in &roots {
        let info = match query_status(root) {
            Ok(info) => info,
            Err(e) => {
                diags.push(format!(
                    "{}: 仓库状态查询失败: {e}",
                    repo_display_name(root)
                ));
                continue;
            }
        };
        repos.push(info);
        enumerate_worktrees(root, &mut repos, &mut diags);
    }

    let mut seen: HashSet<PathBuf> = HashSet::new();
    repos.retain(|repo| seen.insert(canonical_or_self(&repo.path)));

    Ok((repos, diags))
}

/// Enumerate the linked worktrees of `main_repo` and append one row per live
/// worktree to `repos`, recording any per-worktree failure in `diags`.
///
/// A linked worktree is found through the main repository's worktree registry
/// (`Repository::worktrees` + `find_worktree`), which is also what separates a
/// worktree from a submodule — a submodule appears in the index, never in the
/// worktree registry. Registry or lookup failure is transient: it is recorded as
/// a diagnostic and never fails the owning repository's row (checkpoint ③).
fn enumerate_worktrees(main_repo: &Path, repos: &mut Vec<RepoInfo>, diags: &mut Vec<String>) {
    let Ok(repo) = Repository::open(main_repo) else {
        // `query_status` already opened this repo successfully, so this is not
        // expected; treat it as "no discoverable worktrees".
        return;
    };

    let names = match repo.worktrees() {
        Ok(names) => names,
        Err(e) => {
            diags.push(format!(
                "{}: worktree 注册表查询失败: {e}",
                repo_display_name(main_repo)
            ));
            return;
        }
    };

    let canon_main = canonical_or_self(main_repo);
    let mut linked = 0usize;

    for index in 0..names.len() {
        let Some(name) = names.get(index) else {
            continue;
        };

        let wt = match repo.find_worktree(name) {
            Ok(w) => w,
            Err(e) => {
                diags.push(format!(
                    "{}: worktree `{name}` 查找失败: {e}",
                    repo_display_name(main_repo)
                ));
                continue;
            }
        };

        let wt_path = wt.path();

        // `git_worktree_list` includes the main worktree, whose path equals the
        // main repository's own working directory; skip it (already listed).
        if canonical_or_self(wt_path) == canon_main {
            continue;
        }

        if linked >= WORKTREE_HARD_CAP {
            // Beyond the hard cap we neither query nor render; record once and
            // stop enumerating this repository's remaining worktrees.
            diags.push(format!(
                "{}: 链接 worktree 超过上限 {} 个，仅显示前 {} 个",
                repo_display_name(main_repo),
                WORKTREE_HARD_CAP,
                WORKTREE_HARD_CAP
            ));
            break;
        }

        if let Err(e) = wt.validate() {
            diags.push(format!(
                "{}: worktree `{name}` 已失效: {e}",
                repo_display_name(main_repo)
            ));
            continue;
        }

        match query_worktree_status(main_repo, wt_path) {
            Ok(info) => {
                repos.push(info);
                linked += 1;
            }
            Err(e) => {
                diags.push(format!(
                    "{}: worktree `{name}` 状态查询失败: {e}",
                    repo_display_name(main_repo)
                ));
            }
        }
    }
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

/// Recursively discover main-repository roots under `dir`, accumulating their
/// paths into `out`. Each candidate is detected by the presence of a `.git`
/// entry; linked worktrees are surfaced later from the registry, not by this
/// walk, so a repository boundary still halts recursion.
fn discover_in_dir(dir: &Path, out: &mut Vec<PathBuf>) {
    // The scan directory itself may be a main repository.
    if is_git_repo(dir) {
        out.push(dir.to_path_buf());
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
            discover_in_dir(&entry.path(), out);
        }
    }
}

/// Check if a path contains a `.git` entry (a directory for a main repo; a file
/// for a worktree/submodule gitfile). Used only to locate main repository roots
/// during the walkthrough — linked worktrees are enumerated from each repo's own
/// registry (see [`enumerate_worktrees`]), not by matching `.git` files here.
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

    /// First commit on `main` for a freshly `git2::Repository::init`-ed repo,
    /// so linked worktrees can be added on top of a valid HEAD.
    fn init_first_commit(repo: &Repository, repo_dir: &Path) {
        std::fs::write(repo_dir.join("a.txt"), "hi").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("a.txt")).unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = repo.signature().unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
    }

    #[test]
    fn discover_finds_git_repos() {
        let scan_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        let (repos, _diags) = discover_repos(&[scan_dir]).unwrap();
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
        let (repos, _diags) = discover_repos(&[outer.clone(), alias]).unwrap();
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

    /// Regression hook for sub-task 3: the directory walk finds the *main*
    /// repository, and the main repo's worktree registry surfaces linked
    /// worktrees as rows carrying identity metadata. Falsifiable: if discovery
    /// missed the worktree, `wt_row` is `None` and the assertion fails.
    #[test]
    fn discover_finds_linked_worktrees_with_identity() {
        let outer = temp_dir("worktree");
        let repo_dir = outer.join("main");
        fs::create_dir_all(&repo_dir).unwrap();
        let repo = git2::Repository::init(&repo_dir).unwrap();
        init_first_commit(&repo, &repo_dir);

        // Linked worktree on a fresh sibling path (git worktree add style).
        let wt_path = outer.join("wt1");
        repo.worktree("wt1", &wt_path, None).unwrap();

        let (repos, diags) = discover_repos(std::slice::from_ref(&outer)).unwrap();
        assert!(diags.is_empty(), "no worktree failure expected: {diags:?}");

        let canon_main = fs::canonicalize(&repo_dir).unwrap();
        let main_row = repos
            .iter()
            .find(|r| fs::canonicalize(&r.path).unwrap() == canon_main)
            .expect("main repo row must exist");
        assert!(!main_row.is_worktree, "main repo is not a worktree");
        assert!(main_row.main_repo.is_none(), "main repo has no owner");

        let wt_row = repos
            .iter()
            .find(|r| r.is_worktree)
            .expect("linked worktree row must exist");
        assert_eq!(wt_row.main_repo.as_deref(), Some(canon_main.as_path()));
        assert!(
            wt_row.name.contains('⑂'),
            "name must be namespaced: {}",
            wt_row.name
        );
        assert_eq!(
            fs::canonicalize(&wt_row.path).unwrap(),
            fs::canonicalize(&wt_path).unwrap()
        );

        let _ = fs::remove_dir_all(&outer);
    }
}
