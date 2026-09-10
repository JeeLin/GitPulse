use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use super::Config;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Theme {
    Dark,
    Light,
}

impl Default for Theme {
    fn default() -> Self {
        Self::Dark
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubConfig {
    pub token: Option<String>,
    pub instances: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigFile {
    #[serde(default)]
    pub scan_dirs: Vec<PathBuf>,
    #[serde(default)]
    pub groups: HashMap<String, Vec<PathBuf>>,
    #[serde(default)]
    pub theme: Theme,
    #[serde(default)]
    pub github: Option<GitHubConfig>,
}

impl From<ConfigFile> for Config {
    fn from(file: ConfigFile) -> Self {
        Self {
            scan_dirs: file.scan_dirs,
            groups: file.groups,
            theme: file.theme,
            github: file.github,
        }
    }
}

impl From<Config> for ConfigFile {
    fn from(config: Config) -> Self {
        Self {
            scan_dirs: config.scan_dirs,
            groups: config.groups,
            theme: config.theme,
            github: config.github,
        }
    }
}
