use crate::ui::branches::BranchState;
use crate::config::Config;
use crate::git::RepoInfo;
use crate::notifications::{NotificationDb, NotificationState};
use anyhow::Result;

pub struct App {
    pub config: Config,
    pub repos: Vec<RepoInfo>,
    pub db: NotificationDb,
    pub notification_state: NotificationState,
    pub branch_state: BranchState,
    pub should_quit: bool,
}

impl App {
    pub fn new(config: Config) -> Result<Self> {
        let db_dir = dirs::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("gitpulse");
        let db_path = db_dir.join("state.db");
        let _ = std::fs::create_dir_all(&db_dir);
        let db = NotificationDb::new(&db_path)?;
        let notification_state = NotificationState::new();
        let branch_state = BranchState::new();
        Ok(Self {
            config,
            repos: Vec::new(),
            db,
            notification_state,
            branch_state,
            should_quit: false,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        crate::ui::run(self).await
    }
}