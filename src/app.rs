use crate::config::Config;
use crate::git::RepoInfo;
use crate::notifications::NotificationDb;
use anyhow::Result;
use std::path::PathBuf;

pub struct App {
    pub config: Config,
    pub repos: Vec<RepoInfo>,
    pub db: NotificationDb,
    pub should_quit: bool,
}

impl App {
    pub fn new(config: Config) -> Result<Self> {
        // Initialize notification database
        let db_dir = dirs::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("gitpulse");
        let db_path = db_dir.join("state.db");
        
        // Ensure directory exists
        let _ = std::fs::create_dir_all(&db_dir);
        
        let db = crate::notifications::NotificationDb::new(&db_path)?;
        
        Ok(Self {
            config,
            repos: Vec::new(),
            db,
            should_quit: false,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        crate::ui::run(self).await
    }
}
