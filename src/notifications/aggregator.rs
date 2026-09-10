use std::collections::HashMap;
use super::models::{Notification, NotificationType};

/// 按仓库分组通知
pub fn aggregate_by_repo(notifications: &[Notification]) -> HashMap<String, Vec<Notification>> {
    let mut groups = HashMap::new();
    for notification in notifications {
        groups
            .entry(notification.repo.clone())
            .or_insert_with(Vec::new)
            .push(notification.clone());
    }
    groups
}

/// 通知摘要
pub struct NotificationSummary {
    pub total: usize,
    pub by_type: HashMap<String, usize>,
    pub by_repo: HashMap<String, usize>,
}

/// 生成通知摘要
pub fn summarize_notifications(notifications: &[Notification]) -> NotificationSummary {
    let mut by_type = HashMap::new();
    let mut by_repo = HashMap::new();

    for notification in notifications {
        *by_type
            .entry(format!("{:?}", notification.notification_type))
            .or_insert(0) += 1;
        *by_repo.entry(notification.repo.clone()).or_insert(0) += 1;
    }

    NotificationSummary {
        total: notifications.len(),
        by_type,
        by_repo,
    }
}
