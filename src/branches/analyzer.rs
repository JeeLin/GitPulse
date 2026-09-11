use anyhow::Result;
use chrono::{DateTime, Local, NaiveDateTime};
use git2::{BranchType, Repository};
use std::path::Path;

use super::models::{BranchConfig, BranchInfo, BranchStatus};
use super::ignore::should_ignore;

/// 分析指定仓库的所有分支并返回智能标记结果
pub fn analyze_branches(repo_path: &Path, config: &BranchConfig) -> Result<Vec<BranchInfo>> {
    let repo = Repository::open(repo_path)?;
    let repo_name = repo_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    
    let mut branches = Vec::new();
    
    // 遍历本地分支
    let local_branches = repo.branches(Some(BranchType::Local))?;
    for branch_result in local_branches {
        let (branch, branch_type) = branch_result?;
        if let Some(info) = analyze_single_branch(&repo, branch, branch_type, repo_path, &repo_name, config)? {
            branches.push(info);
        }
    }
    
    // 遍历远程分支
    let remote_branches = repo.branches(Some(BranchType::Remote))?;
    for branch_result in remote_branches {
        let (branch, branch_type) = branch_result?;
        if let Some(info) = analyze_single_branch(&repo, branch, branch_type, repo_path, &repo_name, config)? {
            branches.push(info);
        }
    }
    
    Ok(branches)
}

/// 分析单个分支
fn analyze_single_branch(
    repo: &Repository,
    branch: git2::Branch,
    branch_type: BranchType,
    repo_path: &Path,
    repo_name: &str,
    config: &BranchConfig,
) -> Result<Option<BranchInfo>> {
    let branch_name = match branch.name() {
        Ok(Some(name)) => name.to_string(),
        _ => return Ok(None),
    };
    
    // 检查是否应该忽略（长生命周期分支）
    if should_ignore(&branch_name, config) {
        return Ok(None);
    }
    
    let is_remote = branch_type == BranchType::Remote;
    let is_head = branch.is_head();
    
    // 获取上游分支
    let upstream = branch.upstream().ok().and_then(|u| {
        u.name().ok().flatten().map(|s| s.to_string())
    });
    
    // 获取 ahead/behind（如果有上游）
    let (ahead, behind) = get_ahead_behind(repo, &branch);
    
    // 获取最后提交时间
    let last_commit_date = get_last_commit_date(repo, &branch);
    
    // 检查是否已合并（保守判断：已合并到主分支且无 ahead）
    let is_merged = check_if_merged(repo, &branch, ahead);
    
    // 确定分支状态
    let status = determine_status(
        is_merged,
        ahead,
        behind,
        &last_commit_date,
        config,
    );
    
    Ok(Some(BranchInfo::new(
        branch_name,
        repo_path.to_path_buf(),
        repo_name.to_string(),
        is_remote,
        is_head,
        upstream,
        ahead,
        behind,
        last_commit_date,
        status,
    )))
}

/// 获取 ahead/behind 数量
fn get_ahead_behind(_repo: &Repository, _branch: &git2::Branch) -> (u32, u32) {
    // 简化实现：通过 graph_ahead_behind 计算
    // 实际实现需要解析分支引用，这里先返回 0
    (0, 0)
}

/// 获取分支最后提交时间
fn get_last_commit_date(repo: &Repository, branch: &git2::Branch) -> NaiveDateTime {
    let reference = branch.get();
    if let Some(oid) = reference.target() {
        if let Ok(commit) = repo.find_commit(oid) {
            let timestamp = commit.time().seconds();
            return DateTime::from_timestamp(timestamp, 0)
                .map(|dt| dt.naive_local())
                .unwrap_or_else(|| Local::now().naive_local());
        }
    }
    Local::now().naive_local()
}

/// 检查分支是否已合并到主分支
fn check_if_merged(_repo: &Repository, _branch: &git2::Branch, ahead: u32) -> bool {
    // 简化判断：如果没有 ahead 提交，可能已合并
    // 实际实现需要检查分支是否包含在主分支历史中
    ahead == 0
}

/// 确定分支状态
fn determine_status(
    is_merged: bool,
    _ahead: u32,
    _behind: u32,
    last_commit_date: &NaiveDateTime,
    config: &BranchConfig,
) -> BranchStatus {
    // 已合并但未删除 → Merged
    if is_merged {
        return BranchStatus::Merged;
    }
    
    // 落后主干 N 个提交 → Stale
    // 最后提交超过 N 天 → Stale
    let now = Local::now().naive_local();
    let days_since_activity = (now - *last_commit_date).num_days();
    if days_since_activity >= config.min_stale_days as i64 {
        return BranchStatus::Stale;
    }
    
    // 无活动超过阈值 → Orphaned
    if days_since_activity >= config.min_orphaned_days as i64 {
        return BranchStatus::Orphaned;
    }
    
    // 默认：正常
    BranchStatus::Clean
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_determine_status_merged() {
        let config = BranchConfig::default_config();
        let now = Local::now().naive_local();
        let status = determine_status(true, 0, 0, &now, &config);
        assert_eq!(status, BranchStatus::Merged);
    }

    #[test]
    fn test_determine_status_stale() {
        let config = BranchConfig {
            min_stale_days: 5,
            ..BranchConfig::default_config()
        };
        let now = Local::now().naive_local();
        let old_date = now - chrono::Duration::days(10);
        let status = determine_status(false, 0, 0, &old_date, &config);
    }

    #[test]
    fn test_determine_status_clean() {
        let config = BranchConfig::default_config();
        let now = Local::now().naive_local();
        let status = determine_status(false, 0, 0, &now, &config);
        assert_eq!(status, BranchStatus::Clean);
    }

    #[test]
    fn test_analyze_nonexistent_repo() {
        let config = BranchConfig::default_config();
        let result = analyze_branches(&PathBuf::from("/nonexistent"), &config);
        assert!(result.is_err());
    }
}