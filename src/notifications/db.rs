use crate::notifications::models::{Notification, NotificationType};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, Result};
use std::path::Path;

/// 通知数据库管理器
pub struct NotificationDb {
    conn: Connection,
}

impl NotificationDb {
    /// 创建新的数据库实例
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path)?;
        let mut db = NotificationDb { conn };
        db.init()?;
        Ok(db)
    }

    /// 初始化数据库表结构
    pub fn init(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS notifications (
                id TEXT PRIMARY KEY,
                repo TEXT NOT NULL,
                title TEXT NOT NULL,
                notification_type TEXT NOT NULL,
                unread BOOLEAN NOT NULL DEFAULT 1,
                reason TEXT NOT NULL,
                url TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            
            CREATE INDEX IF NOT EXISTS idx_notifications_repo ON notifications(repo);
            CREATE INDEX IF NOT EXISTS idx_notifications_unread ON notifications(unread);
            "#,
        )?;
        Ok(())
    }

    /// 保存通知到数据库
    pub fn save_notification(&self, notification: &Notification) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT OR REPLACE INTO notifications 
            (id, repo, title, notification_type, unread, reason, url, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![
                notification.id,
                notification.repo,
                notification.title,
                format!("{:?}", notification.notification_type).to_lowercase(),
                notification.unread as i32,
                notification.reason,
                notification.url,
                notification.updated_at.to_rfc3339()
            ],
        )?;
        Ok(())
    }

    /// 根据ID获取通知
    pub fn get_notification(&self, id: &str) -> Result<Option<Notification>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, repo, title, notification_type, unread, reason, url, updated_at
            FROM notifications WHERE id = ?
            "#,
        )?;

        let mut rows = stmt.query_map(params![id], |row| {
            Ok(Notification {
                id: row.get(0)?,
                repo: row.get(1)?,
                title: row.get(2)?,
                notification_type: {
                    let s: String = row.get(3)?;
                    match s.as_str() {
                        "issue" => NotificationType::Issue,
                        "pullrequest" => NotificationType::PullRequest,
                        "ci" => NotificationType::Ci,
                        "release" => NotificationType::Release,
                        "mention" => NotificationType::Mention,
                        _ => NotificationType::Issue, // 默认值
                    }
                },
                unread: row.get::<_, i32>(4)? != 0,
                reason: row.get(5)?,
                url: row.get(6)?,
                updated_at: {
                    let dt_str: String = row.get(7)?;
                    DateTime::parse_from_rfc3339(&dt_str)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now())
                },
            })
        })?;

        let notification = rows.next().transpose()?;
        Ok(notification)
    }

    /// 获取所有未读通知
    pub fn get_unread_notifications(&self) -> Result<Vec<Notification>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, repo, title, notification_type, unread, reason, url, updated_at
            FROM notifications WHERE unread = 1
            ORDER BY updated_at DESC
            "#,
        )?;

        let rows = stmt.query_map(params![], |row| {
            Ok(Notification {
                id: row.get(0)?,
                repo: row.get(1)?,
                title: row.get(2)?,
                notification_type: {
                    let s: String = row.get(3)?;
                    match s.as_str() {
                        "issue" => NotificationType::Issue,
                        "pullrequest" => NotificationType::PullRequest,
                        "ci" => NotificationType::Ci,
                        "release" => NotificationType::Release,
                        "mention" => NotificationType::Mention,
                        _ => NotificationType::Issue,
                    }
                },
                unread: row.get::<_, i32>(4)? != 0,
                reason: row.get(5)?,
                url: row.get(6)?,
                updated_at: {
                    let dt_str: String = row.get(7)?;
                    DateTime::parse_from_rfc3339(&dt_str)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now())
                },
            })
        })?;

        let mut notifications = Vec::new();
        for row in rows {
            notifications.push(row?);
        }
        Ok(notifications)
    }

    /// 标记通知为已读
    pub fn mark_as_read(&self, id: &str) -> Result<()> {
        self.conn.execute(
            r#"
            UPDATE notifications SET unread = 0 WHERE id = ?
            "#,
            params![id],
        )?;
        Ok(())
    }

    /// 标记所有通知为已读
    pub fn mark_all_as_read(&self) -> Result<()> {
        self.conn.execute(
            r#"
            UPDATE notifications SET unread = 0
            "#,
            [],
        )?;
        Ok(())
    }

    /// 删除指定时间的旧通知（保留最近N天）
    pub fn cleanup_old_notifications(&self, days: i64) -> Result<usize> {
        let cutoff = Utc::now() - chrono::Duration::days(days);
        let cutoff_str = cutoff.to_rfc3339();

        let count = self.conn.execute(
            r#"
            DELETE FROM notifications WHERE updated_at < ?
            "#,
            params![cutoff_str],
        )?;
        Ok(count)
    }
}
