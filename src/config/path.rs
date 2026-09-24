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

/// The user's home directory, falling back to `/` when it cannot be resolved.
///
/// Used as the directory picker's start-dir fallback; like [`config_dir`],
/// this is the only place that may call `dirs`.
pub fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::config_io_guard;

    #[test]
    fn config_dir_ends_with_gitpulse() {
        // Serialized with the one-time HOME redirect in `config_io_guard`:
        // reading HOME while another test calls `set_var` would race.
        let _io = config_io_guard();
        let dir = config_dir();
        assert_eq!(
            dir.file_name().and_then(|n| n.to_str()),
            Some(".gitpulse"),
            "unexpected config dir: {}",
            dir.display()
        );
    }

    #[test]
    fn home_dir_is_absolute_or_root_fallback() {
        // Serialized with the one-time HOME redirect in `config_io_guard`:
        // reading HOME while another test calls `set_var` would race.
        let _io = config_io_guard();
        let dir = home_dir();
        assert!(
            dir.is_absolute(),
            "home must be absolute: {}",
            dir.display()
        );
    }

    #[test]
    fn config_path_is_config_toml_inside_config_dir() {
        let _io = config_io_guard();
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
