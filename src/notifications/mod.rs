pub mod models;
pub mod db;
pub mod fetcher;
pub mod aggregator;
pub mod desktop;
pub mod sync;
pub mod state;

pub use models::{Notification, NotificationType};
pub use db::NotificationDb;
pub use state::NotificationState;
pub use fetcher::{fetch_notifications, sync_notifications};
