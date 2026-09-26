//! Configuration loading and persistence.
//!
//! All filesystem paths come from [`path`]; only that module may call `dirs` (checkpoint 3).

mod path;
mod types;

pub use path::{config_path, home_dir};
pub use types::Config;

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use thiserror::Error;

/// Errors produced while reading or writing the configuration file.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// The config file exists but could not be read.
    #[error("读取配置失败 {path}: {source}")]
    Read {
        /// Path that could not be read.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },
    /// The config file is not valid TOML.
    #[error("解析配置失败 {path}: {source}")]
    Parse {
        /// Path that could not be parsed.
        path: PathBuf,
        /// Underlying TOML error.
        source: toml::de::Error,
    },
    /// The config could not be serialized to TOML.
    #[error("序列化配置失败: {source}")]
    Serialize {
        /// Underlying serialization error.
        source: toml::ser::Error,
    },
    /// The config directory could not be created.
    #[error("创建配置目录失败 {path}: {source}")]
    CreateDir {
        /// Parent directory that could not be created.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },
    /// The config file could not be written.
    #[error("写入配置失败 {path}: {source}")]
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

/// Serialize tests that exercise the shared, redirected config file.
///
/// Every test that calls [`Config::save`], [`Config::load`], or otherwise
/// resolves the config path must hold this guard for its whole body: they all
/// resolve the same throwaway [`config_path`] override, so concurrent save /
/// load pairs would race on that one file.
///
/// The redirect itself is installed through
/// [`set_config_path_override`](path::set_config_path_override) rather than by
/// mutating `HOME`: `dirs` and libgit2 keep reading a stable environment, so
/// there is no `setenv`/`getenv` data race and no timing dependency on when
/// the first guarded test runs.
#[cfg(test)]
pub(crate) fn config_io_guard() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    let guard = LOCK
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    path::set_config_path_override(test_config_path());
    guard
}

/// Throwaway config file shared by every [`config_io_guard`] holder.
#[cfg(test)]
fn test_config_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "gitpulse-test-config-{}/config.toml",
        std::process::id()
    ))
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

    #[test]
    fn config_save_persists_scan_dirs_via_public_api() {
        // Holds the I/O guard: it points `config_path` at a throwaway file and
        // serializes access to that shared redirect.
        let _io = super::config_io_guard();
        let config = Config {
            scan_dirs: vec![
                PathBuf::from("/home/user/repos"),
                PathBuf::from("/opt/projects"),
            ],
        };
        config.save().expect("save");
        let loaded = Config::load().expect("load");
        assert_eq!(loaded, config);
    }
}
