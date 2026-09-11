use git2::Repository;
use std::path::PathBuf;

use super::BranchInfo;
use super::BranchStatus;

/// 分支操作结果
#[derive(Debug, Clone)]
pub struct BranchOperation {
    /// 分支名称
    pub branch_name: String,
    /// 仓库路径
    pub repo_path: PathBuf,
    /// 是否为远程分支
    pub is_remote: bool,
    /// 操作结果
    pub result: OperationResult,
}

/// 操作结果枚举
#[derive(Debug, Clone)]
pub enum OperationResult {
    /// 成功删除
    Success,
    /// 删除失败（含错误信息）
    Failure(String),
    /// 为受保护分支，跳过删除
    Skipped(String),
}

/// 获取所有已合并的分支
pub fn get_merged_branches(branches: &[BranchInfo]) -> Vec<&BranchInfo> {
    branches
        .iter()
        .filter(|b| b.status == BranchStatus::Merged)
        .collect()
}

/// 删除单个分支的辅助函数
fn delete_single_branch(repo: &Repository, branch: &BranchInfo) -> BranchOperation {
    let op = |result| BranchOperation {
        branch_name: branch.name.clone(),
        repo_path: branch.repo_path.clone(),
        is_remote: branch.is_remote,
        result,
    };

    let git_branch = match repo.find_branch(&branch.name, git2::BranchType::Local) {
        Ok(b) => b,
        Err(e) => return op(OperationResult::Failure(e.to_string())),
    };

    if git_branch.is_head() {
        return op(OperationResult::Skipped("当前分支，跳过删除".to_string()));
    }

    let mut git_branch = git_branch;
    match git_branch.delete() {
        Ok(()) => op(OperationResult::Success),
        Err(e) => op(OperationResult::Failure(e.to_string())),
    }
}

/// 批量删除已合并的分支（本地）
pub fn delete_merged_branches(
    repo: &Repository,
    branches: &[BranchInfo],
    dry_run: bool,
) -> Vec<BranchOperation> {
    branches
        .iter()
        .filter(|b| b.status == BranchStatus::Merged)
        .map(|branch| {
            if dry_run {
                BranchOperation {
                    branch_name: branch.name.clone(),
                    repo_path: branch.repo_path.clone(),
                    is_remote: branch.is_remote,
                    result: OperationResult::Success,
                }
            } else {
                delete_single_branch(repo, branch)
            }
        })
        .collect()
}

/// 获取需要删除的分支数量
pub fn count_deletable(branches: &[BranchInfo]) -> usize {
    get_merged_branches(branches).len()
}

/// 生成删除确认文本
pub fn format_confirmation(operations: &[BranchOperation]) -> String {
    let mut text = String::new();
    text.push_str("将要删除以下分支：\n\n");

    for op in operations {
        let status = match &op.result {
            OperationResult::Success => "✓",
            OperationResult::Failure(e) => &format!("✗ ({})", e),
            OperationResult::Skipped(reason) => &format!("⊘ ({})", reason),
        };

        text.push_str(&format!(
            "  {} {} ({})\n",
            status,
            op.branch_name,
            if op.is_remote { "远程" } else { "本地" }
        ));
    }

    text.push_str(&format!("\n共 {} 个分支", operations.len()));
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::branches::{BranchInfo, BranchStatus};
    use chrono::NaiveDateTime;
    use std::path::PathBuf;

    fn create_test_branch(name: &str, status: BranchStatus) -> BranchInfo {
        BranchInfo::new(
            name.to_string(),
            PathBuf::from("/test/repo"),
            "test/repo".to_string(),
            false,
            false,
            None,
            0,
            0,
            NaiveDateTime::parse_from_str("2024-01-01 00:00:00", "%Y-%m-%d %H:%M:%S").unwrap(),
            status,
        )
    }

    #[test]
    fn test_get_merged_branches() {
        let branches = vec![
            create_test_branch("main", BranchStatus::Clean),
            create_test_branch("feature-1", BranchStatus::Merged),
            create_test_branch("feature-2", BranchStatus::Merged),
            create_test_branch("stale-1", BranchStatus::Stale),
        ];

        let merged = get_merged_branches(&branches);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].name, "feature-1");
        assert_eq!(merged[1].name, "feature-2");
    }

    #[test]
    fn test_count_deletable() {
        let branches = vec![
            create_test_branch("main", BranchStatus::Clean),
            create_test_branch("feature-1", BranchStatus::Merged),
            create_test_branch("feature-2", BranchStatus::Merged),
        ];

        assert_eq!(count_deletable(&branches), 2);
    }

    #[test]
    fn test_format_confirmation() {
        let operations = vec![
            BranchOperation {
                branch_name: "feature-1".to_string(),
                repo_path: PathBuf::from("/test"),
                is_remote: false,
                result: OperationResult::Success,
            },
            BranchOperation {
                branch_name: "feature-2".to_string(),
                repo_path: PathBuf::from("/test"),
                is_remote: true,
                result: OperationResult::Skipped("当前分支".to_string()),
            },
        ];

        let text = format_confirmation(&operations);
        assert!(text.contains("feature-1"));
        assert!(text.contains("feature-2"));
        assert!(text.contains("共 2 个分支"));
    }
}