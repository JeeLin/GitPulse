use crate::config::Config;
use crate::git::RepoInfo;
use anyhow::Result;

pub struct App {
    pub config: Config,
    pub repos: Vec<RepoInfo>,
    pub should_quit: bool,
}

impl App {
    pub fn new(config: Config) -> Result<Self> {
        Ok(Self {
            config,
            repos: Vec::new(),
            should_quit: false,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        crate::ui::run(self).await
    }
}
