use chrono::NaiveDateTime;
use std::path::PathBuf;

/// Git 分支状态（智能标记）
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BranchStatus {
    /// 有活跃 PR，正常
    Clean,
    /// 落后主干 N 个提交
    Stale,
    /// 无关联 PR 且长期无活动
    Orphaned,
    /// 已合并但未删除
    Merged,
}

impl std::fmt::Display for BranchStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BranchStatus::Clean => write!(f, "🟢 正常"),
            BranchStatus::Stale => write!(f, "🟡 落后"),
            BranchStatus::Orphaned => write!(f, "🟡 废弃"),
            BranchStatus::Merged => write!(f, "🔴 已合并"),
        }
    }
}

/// Git 分支信息数据模型
#[derive(Debug, Clone)]
pub struct BranchInfo {
    /// 分支名称
    pub name: String,
    /// 仓库路径
    pub repo_path: PathBuf,
    /// 仓库名称（格式：owner/repo）
    pub repo_name: String,
    /// 是否为远程分支
    pub is_remote: bool,
    /// 是否为当前 HEAD 分支
    pub is_head: bool,
    /// 上游分支
    pub upstream: Option<String>,
    /// 领先提交数
    pub ahead: u32,
    /// 落后提交数
    pub behind: u32,
    /// 最后提交时间
    pub last_commit_date: NaiveDateTime,
    /// 分支状态
    pub status: BranchStatus,
}

impl BranchInfo {
    /// 创建新的分支信息实例
    pub fn new(
        name: String,
        repo_path: PathBuf,
        repo_name: String,
        is_remote: bool,
        is_head: bool,
        upstream: Option<String>,
        ahead: u32,
        behind: u32,
        last_commit_date: NaiveDateTime,
        status: BranchStatus,
    ) -> Self {
        Self {
            name,
            repo_path,
            repo_name,
            is_remote,
            is_head,
            upstream,
            ahead,
            behind,
            last_commit_date,
            status,
        }
    }
}

/// GitHub PR 状态信息（用于分支状态分析）
#[derive(Debug, Clone)]
pub struct PrStatus {
    /// 是否有关联的 PR
    pub has_associated_pr: bool,
    /// PR 是否已合并
    pub is_merged: bool,
    /// PR 是否处于活跃状态（未合并但有活动）
    pub is_active: bool,
    /// 最后活动时间
    pub last_activity: Option<NaiveDateTime>,
}

#[allow(dead_code)]
impl PrStatus {
    /// 无关联 PR 的默认状态
    pub fn no_pr() -> Self {
        Self {
            has_associated_pr: false,
            is_merged: false,
            is_active: false,
            last_activity: None,
        }
    }
}

/// 分支排除规则配置
#[derive(Debug, Clone, Default)]
pub struct BranchConfig {
    /// 长生命周期分支排除模式（支持通配符：main, master, develop, release/*, v*, stable）
    pub exclude_patterns: Vec<String>,
    /// 判断为 Stale 的最少天数（默认 30 天）
    pub min_stale_days: u32,
    /// 判断为 Orphaned 的最少天数（默认 60 天）
    pub min_orphaned_days: u32,
}

impl BranchConfig {
    /// 默认长生命周期分支列表
    pub fn default_excludes() -> Vec<String> {
        vec![
            "main".to_string(),
            "master".to_string(),
            "develop".to_string(),
            "release/*".to_string(),
            "v*".to_string(),
            "stable".to_string(),
        ]
    }

    /// 创建默认配置
    pub fn default_config() -> Self {
        Self {
            exclude_patterns: Self::default_excludes(),
            min_stale_days: 30,
            min_orphaned_days: 60,
        }
    }
}