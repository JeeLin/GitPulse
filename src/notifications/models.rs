use chrono::{DateTime, Utc};

/// 通知类型
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationType {
    /// Issue 提及或评论
    Issue,
    /// Pull Request 审核请求
    PullRequest,
    /// CI/CD 运行状态
    Ci,
    /// 发布版本
    Release,
    /// @提及
    Mention,
}

/// GitHub 通知数据模型
#[derive(Debug, Clone)]
pub struct Notification {
    /// 通知唯一标识
    pub id: String,
    /// 所属仓库名称（格式：owner/repo）
    pub repo: String,
    /// 通知标题
    pub title: String,
    /// 通知类型
    pub notification_type: NotificationType,
    /// 是否未读
    pub unread: bool,
    /// 原因（为什么产生此通知）
    pub reason: String,
    /// 通知链接URL
    pub url: String,
    /// 更新时间
    pub updated_at: DateTime<Utc>,
}

impl Notification {
    /// 创建新通知实例
    pub fn new(
        id: String,
        repo: String,
        title: String,
        notification_type: NotificationType,
        unread: bool,
        reason: String,
        url: String,
        updated_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id,
            repo,
            title,
            notification_type,
            unread,
            reason,
            url,
            updated_at,
        }
    }
}
