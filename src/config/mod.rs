//! Configuration loading and persistence.
//!
//! All filesystem paths come from [`path`]; only that module may call `dirs` (checkpoint 3).

mod path;
mod types;

pub use path::config_path;
pub use types::Config;

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use thiserror::Error;

/// Errors produced while reading or writing the configuration file.
// Save-path variants are constructed once `Config::save` gains callers.
#[allow(dead_code)]
#[derive(Debug, Error)]
pub enum ConfigError {
    /// The config file exists but could not be read.
    #[error("failed to read config at {path}: {source}")]
    Read {
        /// Path that could not be read.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },
    /// The config file is not valid TOML.
    #[error("failed to parse config at {path}: {source}")]
    Parse {
        /// Path that could not be parsed.
        path: PathBuf,
        /// Underlying TOML error.
        source: toml::de::Error,
    },
    /// The config could not be serialized to TOML.
    #[error("failed to serialize config: {source}")]
    Serialize {
        /// Underlying serialization error.
        source: toml::ser::Error,
    },
    /// The config directory could not be created.
    #[error("failed to create config dir at {path}: {source}")]
    CreateDir {
        /// Parent directory that could not be created.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },
    /// The config file could not be written.
    #[error("failed to write config at {path}: {source}")]
    Write {
        /// Path that could not be written.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },
}

impl Config {
    /// Load the config from [`config_path`]; a missing file yields [`Config::default`].
    pub fn load() -> Result<Self> {
        load_from(&config_path())
    }

    /// Save the config to [`config_path`], creating the config dir when missing.
    // Callers land with the config editing UI in a later subtask.
    #[allow(dead_code)]
    pub fn save(&self) -> Result<()> {
        save_to(self, &config_path())
    }
}

/// Load a config from the given file path; missing files yield [`Config::default`].
fn load_from(path: &Path) -> Result<Config> {
    let contents = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(source) => {
            return Err(ConfigError::Read {
                path: path.to_path_buf(),
                source,
            }
            .into())
        }
    };
    toml::from_str(&contents)
        .map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })
        .map_err(Into::into)
}

/// Save a config to the given file path, creating parent directories as needed.
// Reached via `Config::save`; exercised directly by unit tests.
#[allow(dead_code)]
fn save_to(config: &Config, path: &Path) -> Result<()> {
    let contents =
        toml::to_string_pretty(config).map_err(|source| ConfigError::Serialize { source })?;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|source| ConfigError::CreateDir {
                path: parent.to_path_buf(),
                source,
            })?;
        }
    }
    fs::write(path, contents).map_err(|source| ConfigError::Write {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            use std::time::{SystemTime, UNIX_EPOCH};
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let ns = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "gitpulse-test-{tag}-{}-{ns}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed),
            ));
            fs::create_dir_all(&path).expect("create temp dir");
            Self(path)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn load_missing_file_returns_default() {
        let dir = TempDir::new("missing");
        let path = dir.path().join("nonexistent.toml");
        let config = load_from(&path).expect("missing file yields default");
        assert_eq!(config, Config::default());
        assert!(config.scan_dirs.is_empty());
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = TempDir::new("roundtrip");
        let path = dir.path().join("nested").join("config.toml");
        let config = Config {
            scan_dirs: vec![
                PathBuf::from("/home/user/repos"),
                PathBuf::from("/opt/projects"),
            ],
        };
        save_to(&config, &path).expect("save");
        let loaded = load_from(&path).expect("load");
        assert_eq!(loaded, config);
    }
}
