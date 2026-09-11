use std::path::PathBuf;

/// Worktree 信息
#[derive(Debug, Clone)]
pub struct WorktreeInfo {
    /// Worktree 路径
    pub path: PathBuf,
    /// 当前分支
    pub branch: String,
    /// HEAD commit
    pub head: String,
    /// 是否为主 worktree
    pub is_main: bool,
}

impl WorktreeInfo {
    /// 创建新的 WorktreeInfo
    pub fn new(path: PathBuf, branch: String, head: String, is_main: bool) -> Self {
        Self {
            path,
            branch,
            head,
            is_main,
        }
    }

    /// 获取显示名称
    pub fn display_name(&self) -> String {
        self.path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_worktree_creation() {
        let wt = WorktreeInfo::new(
            PathBuf::from("/path/to/worktree"),
            "main".to_string(),
            "abc123".to_string(),
            true,
        );
        assert_eq!(wt.branch, "main");
        assert!(wt.is_main);
    }
}
