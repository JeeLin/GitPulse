use anyhow::Result;
use chrono::Utc;

use super::models::{Notification, NotificationType};

/// 从 GitHub API 获取通知并转换为内部格式
pub async fn fetch_notifications(_client: &octocrab::Octocrab) -> Result<Vec<Notification>> {
    // 返回一些测试通知
    let notifications = vec![
        Notification::new(
            "1".to_string(),
            "JeeLin/GitPulse".to_string(),
            "Issue: Add desktop notification support".to_string(),
            NotificationType::Issue,
            true,
            "subscribed".to_string(),
            "https://github.com/JeeLin/GitPulse/issues/1".to_string(),
            Utc::now(),
        ),
        Notification::new(
            "2".to_string(),
            "JeeLin/GitPulse".to_string(),
            "PR: Fix notification sync bug".to_string(),
            NotificationType::PullRequest,
            true,
            "review_requested".to_string(),
            "https://github.com/JeeLin/GitPulse/pull/2".to_string(),
            Utc::now(),
        ),
    ];
    Ok(notifications)
}

/// 同步通知到本地数据库
pub async fn sync_notifications(
    _client: &octocrab::Octocrab,
    db: &super::db::NotificationDb,
) -> Result<usize> {
    let notifications = fetch_notifications(_client).await?;
    let mut new_count = 0;

    for notification in &notifications {
        if let Ok(existing) = db.get_notification(&notification.id) {
            if existing.is_some() {
                continue;
            }
        }

        db.save_notification(notification)?;
        new_count += 1;
    }

    Ok(new_count)
}
