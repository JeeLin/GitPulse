use anyhow::Result;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct RepoInfo {
    pub name: String,
    pub path: std::path::PathBuf,
    pub branch: String,
    pub dirty: DirtyStats,
    pub ahead: usize,
    pub behind: usize,
    pub last_commit: CommitInfo,
}

#[derive(Debug, Clone, Default)]
pub struct DirtyStats {
    pub modified: usize,
    pub staged: usize,
    pub untracked: usize,
}

impl DirtyStats {
    pub fn total(&self) -> usize {
        self.modified + self.staged + self.untracked
    }
    
    pub fn is_clean(&self) -> bool {
        self.total() == 0
    }
}

#[derive(Debug, Clone)]
pub struct CommitInfo {
    pub hash: String,
    pub message: String,
    pub time: String,
}

pub fn query_status(repo_path: &Path) -> Result<RepoInfo> {
    let repo = git2::Repository::open(repo_path)?;
    
    let name = repo_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    
    let branch = get_current_branch(&repo)?;
    let dirty = get_dirty_stats(&repo)?;
    let (ahead, behind) = get_ahead_behind(&repo)?;
    let last_commit = get_last_commit(&repo)?;
    
    Ok(RepoInfo {
        name,
        path: repo_path.to_path_buf(),
        branch,
        dirty,
        ahead,
        behind,
        last_commit,
    })
}

fn get_current_branch(repo: &git2::Repository) -> Result<String> {
    let head = repo.head()?;
    if let Some(branch) = head.shorthand() {
        Ok(branch.to_string())
    } else {
        Ok("HEAD".to_string())
    }
}

fn get_dirty_stats(repo: &git2::Repository) -> Result<DirtyStats> {
    let mut status_options = git2::StatusOptions::new();
    status_options.include_untracked(true);
    
    let statuses = repo.statuses(Some(&mut status_options))?;
    
    let mut stats = DirtyStats::default();
    
    for entry in statuses.iter() {
        let status = entry.status();
        
        if status.contains(git2::Status::WT_MODIFIED) {
            stats.modified += 1;
        }
        if status.contains(git2::Status::INDEX_NEW)
            || status.contains(git2::Status::INDEX_MODIFIED)
        {
            stats.staged += 1;
        }
        if status.contains(git2::Status::WT_NEW) {
            stats.untracked += 1;
        }
    }
    
    Ok(stats)
}

fn get_ahead_behind(repo: &git2::Repository) -> Result<(usize, usize)> {
    let head = match repo.head() {
        Ok(head) => head,
        Err(_) => return Ok((0, 0)),
    };
    
    let head_oid = match head.target() {
        Some(oid) => oid,
        None => return Ok((0, 0)),
    };
    
    let branch_name = match head.shorthand() {
        Some(name) => name,
        None => return Ok((0, 0)),
    };
    
    let upstream_name = format!("refs/remotes/origin/{}", branch_name);
    
    let upstream = match repo.refname_to_id(&upstream_name) {
        Ok(oid) => oid,
        Err(_) => return Ok((0, 0)),
    };
    
    let (ahead, behind) = repo.graph_ahead_behind(head_oid, upstream)?;
    
    Ok((ahead, behind))
}

fn get_last_commit(repo: &git2::Repository) -> Result<CommitInfo> {
    let head = repo.head()?;
    let commit = head.peel_to_commit()?;
    
    let hash = commit.id().to_string();
    let message = commit.summary().unwrap_or("").to_string();
    
    let time = commit.time();
    let seconds = time.seconds();
    let dt = chrono::DateTime::from_timestamp(seconds, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "unknown".to_string());
    
    Ok(CommitInfo {
        hash,
        message,
        time: dt,
    })
}
