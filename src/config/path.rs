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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_dir_ends_with_gitpulse() {
        let dir = config_dir();
        assert_eq!(
            dir.file_name().and_then(|n| n.to_str()),
            Some(".gitpulse"),
            "unexpected config dir: {}",
            dir.display()
        );
    }

    #[test]
    fn config_path_is_config_toml_inside_config_dir() {
        let path = config_path();
        assert_eq!(
            path.file_name().and_then(|n| n.to_str()),
            Some("config.toml"),
            "unexpected config path: {}",
            path.display()
        );
        assert_eq!(path.parent(), Some(config_dir().as_path()));
    }
}
