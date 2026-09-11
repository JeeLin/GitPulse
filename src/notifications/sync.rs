#![allow(dead_code)]
use super::db::NotificationDb;
use anyhow::Result;

/// 同步本地已读状态到 GitHub
pub async fn mark_as_read(_client: &octocrab::Octocrab, _notification_id: &str) -> Result<()> {
    // TODO: Implement actual GitHub mark as read API
    // octocrab 0.44 notifications API 需要进一步研究
    println!("[Stub] Marking notification as read (not implemented yet)");
    Ok(())
}

/// 同步本地已读状态到 GitHub 并更新本地数据库
pub async fn sync_read_status(client: &octocrab::Octocrab, db: &NotificationDb) -> Result<()> {
    let unread = db.get_unread_notifications()?;

    for notification in unread {
        if let Err(e) = mark_as_read(client, &notification.id).await {
            eprintln!(
                "Failed to mark notification {} as read: {}",
                notification.id, e
            );
            continue;
        }
        db.mark_as_read(&notification.id)?;
    }

    Ok(())
}
