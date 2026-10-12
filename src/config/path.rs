//! The single source of truth for GitPulse filesystem paths.
//!
//! No other module may call `dirs` directly (design checkpoint 3).

use std::path::PathBuf;

/// Test-only redirect for [`config_path`], so tests share a throwaway config
/// file instead of mutating `HOME`.
///
/// Set once process-wide (first call wins) by [`set_config_path_override`];
/// every test that then reads or writes the config file must hold
/// [`crate::config::config_io_guard`] to serialize that shared file.
#[cfg(test)]
static CONFIG_PATH_OVERRIDE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Test-only: point [`config_path`] at `path` (the first call wins).
///
/// Because the override is process-wide, callers must hold
/// [`crate::config::config_io_guard`] whenever the resulting file is touched.
#[cfg(test)]
pub(crate) fn set_config_path_override(path: PathBuf) {
    let _ = CONFIG_PATH_OVERRIDE.set(path);
}

/// The only place that resolves the GitPulse config directory (`~/.gitpulse`).
pub fn config_dir() -> PathBuf {
    dirs::home_dir()
        .map(|home| home.join(".gitpulse"))
        .unwrap_or_else(|| PathBuf::from(".gitpulse"))
}

/// The un-overridden config path: `<config_dir>/config.toml`.
fn default_config_path() -> PathBuf {
    config_dir().join("config.toml")
}

/// The only place that resolves the GitPulse config file path (`~/.gitpulse/config.toml`).
///
/// Tests redirect it through `set_config_path_override`; production always
/// resolves [`default_config_path`].
pub fn config_path() -> PathBuf {
    #[cfg(test)]
    {
        if let Some(overridden) = CONFIG_PATH_OVERRIDE.get() {
            return overridden.clone();
        }
    }
    default_config_path()
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
        // `config_dir` only reads the real HOME: tests never mutate the
        // environment, so there is nothing to serialize here.
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
        let dir = home_dir();
        assert!(
            dir.is_absolute(),
            "home must be absolute: {}",
            dir.display()
        );
    }

    #[test]
    fn default_config_path_is_config_toml_inside_config_dir() {
        // The default (never-overridden) path is `config.toml` inside the
        // config dir.
        let path = default_config_path();
        assert_eq!(
            path.file_name().and_then(|n| n.to_str()),
            Some("config.toml"),
            "unexpected config path: {}",
            path.display()
        );
        assert_eq!(path.parent(), Some(config_dir().as_path()));
    }

    #[test]
    fn config_path_prefers_test_override() {
        // Holds the I/O guard: it installs the throwaway override on first
        // use, and the guard keeps that value stable for this body.
        let _io = config_io_guard();
        let overridden = CONFIG_PATH_OVERRIDE.get().expect("guard installs override");
        assert_eq!(config_path(), *overridden);
        assert_eq!(
            overridden.file_name().and_then(|n| n.to_str()),
            Some("config.toml"),
            "override must keep the config file name: {}",
            overridden.display()
        );
        assert!(
            overridden.starts_with(std::env::temp_dir()),
            "override must stay out of the real home: {}",
            overridden.display()
        );
    }
}
