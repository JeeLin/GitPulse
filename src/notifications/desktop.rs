use anyhow::Result;

/// 显示桌面通知（使用 notify-rust）
#[allow(dead_code)]
pub fn show_desktop_notification(title: &str, body: &str, _icon: Option<&str>) -> Result<()> {
    // TODO: 实现 notify-rust 桌面通知
    println!("[Desktop Notification] {}: {}", title, body);
    Ok(())
}

/// 检查新通知并显示桌面通知
#[allow(dead_code)]
pub fn check_and_show_new_notifications(
    notifications: &[super::models::Notification],
) -> Result<()> {
    for notification in notifications {
        if notification.unread {
            show_desktop_notification(
                &format!("{} - {}", notification.repo, notification.title),
                &notification.reason,
                None,
            )?;
        }
    }
    Ok(())
}
