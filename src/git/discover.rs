use anyhow::Result;
use std::path::{Path, PathBuf};
use std::fs;

use super::RepoInfo;

pub fn discover_repos(dirs: &[PathBuf]) -> Result<Vec<RepoInfo>> {
    let mut repos = Vec::new();
    
    for dir in dirs {
        if !dir.exists() {
            continue;
        }
        
        discover_in_dir(dir, &mut repos)?;
    }
    
    Ok(repos)
}

fn discover_in_dir(dir: &Path, repos: &mut Vec<RepoInfo>) -> Result<()> {
    // Skip directories that are not accessible
    if !dir.is_dir() {
        return Ok(());
    }
    
    // Skip common non-repo directories
    let dir_name = dir.file_name().unwrap_or_default().to_string_lossy();
    if should_skip_dir(&dir_name) {
        return Ok(());
    }
    
    // Check if this directory is a git repository
    if dir.join(".git").exists() {
        if let Ok(repo_info) = super::query_status(dir) {
            repos.push(repo_info);
        }
        // Don't recurse into git repos
        return Ok(());
    }
    
    // Recurse into subdirectories
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // Skip symlinks to avoid infinite loops
                if path.is_symlink() {
                    continue;
                }
                discover_in_dir(&path, repos)?;
            }
        }
    }
    
    Ok(())
}

fn should_skip_dir(name: &str) -> bool {
    matches!(
        name,
        "node_modules" | "target" | ".target" | ".git" | ".svn" | ".hg"
            | "dist" | "build" | "__pycache__" | ".venv" | "venv"
    )
}
