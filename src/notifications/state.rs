use super::models::Notification;

/// 通知面板状态
pub struct NotificationState {
    pub notifications: Vec<Notification>,
    pub list_selected: Option<usize>,
    pub is_visible: bool,
}

impl NotificationState {
    pub fn new() -> Self {
        Self {
            notifications: Vec::new(),
            list_selected: Some(0),
            is_visible: false,
        }
    }

    pub fn toggle_visibility(&mut self) {
        self.is_visible = !self.is_visible;
    }

    pub fn update_notifications(&mut self, notifications: Vec<Notification>) {
        self.notifications = notifications;
        if !self.notifications.is_empty() {
            self.list_selected = Some(0);
        }
    }

    pub fn selected_notification(&self) -> Option<&Notification> {
        self.list_selected.and_then(|i| self.notifications.get(i))
    }
}
