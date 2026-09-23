//! The single source of truth for GitPulse filesystem paths.
//!
//! No other module may call `dirs` directly (design checkpoint 3).

use std::path::PathBuf;

/// The only place that resolves the GitPulse config directory (`~/.gitpulse`).
pub fn config_dir() -> PathBuf {
    dirs::home_dir()
        .map(|home| home.join(".gitpulse"))
        .unwrap_or_else(|| PathBuf::from(".gitpulse"))
}

/// The only place that resolves the GitPulse config file path (`~/.gitpulse/config.toml`).
pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}
