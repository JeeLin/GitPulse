pub mod types;

use anyhow::Result;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use crate::accounts::{AccountConfig, GitUser};

pub use types::*;

fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("gitpulse")
}

#[derive(Clone)]
pub struct Config {
    pub scan_dirs: Vec<PathBuf>,
    pub groups: HashMap<String, Vec<PathBuf>>,
    pub theme: Theme,
    pub github: Option<GitHubConfig>,
    pub user: Option<GitUser>,
    pub accounts: Option<AccountConfig>,
}

impl Config {
    pub fn load() -> Result<Self> {
        let config_file = config_dir().join("config.toml");

        if config_file.exists() {
            let content = fs::read_to_string(&config_file)?;
            let parsed: ConfigFile = toml::from_str(&content)?;
            Ok(parsed.into())
        } else {
            Ok(Self::default())
        }
    }

    #[allow(dead_code)]
    pub fn save(&self) -> Result<()> {
        let dir = config_dir();
        fs::create_dir_all(&dir)?;

        let config_file = dir.join("config.toml");
        let file_config: ConfigFile = self.clone().into();
        let content = toml::to_string_pretty(&file_config)?;
        fs::write(config_file, content)?;

        Ok(())
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            scan_dirs: Vec::new(),
            groups: HashMap::new(),
            theme: Theme::Dark,
            github: None,
            user: None,
            accounts: None,
        }
    }
}
