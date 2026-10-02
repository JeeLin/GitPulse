//! Configuration loading and persistence.
//!
//! All filesystem paths come from [`path`]; only that module may call `dirs` (checkpoint 3).

mod encoding;
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
    ///
    /// Both scan-dir wire forms are accepted: the legacy plain strings and
    /// the `{ path_hex = .. }` markers for non-UTF-8 paths (documented in
    /// the `config::encoding` module).
    ///
    /// Any temp file a crashed earlier run left beside the config
    /// (`<file name>.tmp-*`) is swept first (V7); see
    /// [`cleanup_stale_temps`].
    pub fn load() -> Result<Self> {
        let path = config_path();
        cleanup_stale_temps(&path);
        load_from(&path)
    }

    /// Save the config to [`config_path`], creating the config dir when missing.
    ///
    /// The document is serialized before the filesystem is touched and then
    /// replaced atomically, so a failed save leaves the previous file
    /// byte-for-byte intact — the in-memory rollbacks around `Config::save`
    /// rely on that.
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
///
/// Serialization happens first and writes go through [`write_atomically`], so
/// no failure mode reaches the target file with partial contents.
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
    write_atomically(path, &contents)
}

/// Write `contents` to `path` through a sibling temp file renamed into place.
///
/// A plain `fs::write` truncates the target first, so a failing write would
/// leave a half-written config behind while the caller only rolls back
/// memory. Renaming over the target keeps the old bytes until the new ones
/// are complete; the temp file is removed on any failure.
///
/// The temp file is created with mode `0600` instead of the usual
/// `0666 & !umask`, so the rename can never widen a `chmod 600` config —
/// this file is where credentials such as the GitHub token will live — and
/// it is `sync_all`ed before the rename so a crash cannot leave an empty or
/// half-written config in place. The rename stays within the directory — no
/// cross-device window.
fn write_atomically(path: &Path, contents: &str) -> Result<()> {
    let temp = temp_path(path);
    write_temp(&temp, contents)
        .and_then(|_| fs::rename(&temp, path))
        // The temp file is best-effort cleanup: this arm already carries the
        // original `fs::write`/`fs::rename` error, and that is the error
        // worth reporting — a failed cleanup is deliberately swallowed so
        // the caller still sees why the *save* failed (G37).
        .map_err(|source| {
            let _ = fs::remove_file(&temp);
            ConfigError::Write {
                path: path.to_path_buf(),
                source,
            }
        })?;
    Ok(())
}

/// Create `temp` with owner-only permissions, write `contents` into it, and
/// flush it to disk so the following rename publishes complete bytes.
///
/// The file is opened with `create_new` (V7): an existing path — including
/// a symlink planted at a predicted temp name — is never followed or
/// truncated. The only legitimate conflict is a leftover temp, so the
/// offending entry is removed once and the open retried; `remove_file` on a
/// symlink drops the link itself, not its target.
fn write_temp(temp: &Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write as _;

    let mut file = open_temp(temp).or_else(|err| {
        if err.kind() == std::io::ErrorKind::AlreadyExists {
            let _ = fs::remove_file(temp);
            open_temp(temp)
        } else {
            Err(err)
        }
    })?;

    // `mode` only applies when the file is created; a leftover temp file
    // from an earlier run keeps its old mode, so re-assert `0600` before
    // the rename carries it onto the config.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }

    file.write_all(contents.as_bytes())?;
    file.sync_all()
}

/// Open a fresh owner-only temp file, failing when `temp` already exists.
fn open_temp(temp: &Path) -> std::io::Result<fs::File> {
    #[cfg(unix)]
    let file = {
        use std::os::unix::fs::OpenOptionsExt as _;
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(temp)?
    };
    #[cfg(not(unix))]
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temp)?;
    Ok(file)
}

/// The sibling temp file used by [`write_atomically`]: `<name>.tmp-<pid>`.
///
/// The name stays deterministic for a given run — the threat model is a
/// same-permission local writer (V7), and predictability alone cannot
/// redirect the write: [`write_temp`] opens with `create_new`, never
/// following a planted symlink, and removes a same-pid leftover once before
/// retrying. The `tmp-` marker names the prefix [`cleanup_stale_temps`]
/// sweeps at startup so a crash cannot litter the config dir forever.
fn temp_path(path: &Path) -> PathBuf {
    let mut name = match path.file_name() {
        Some(name) => name.to_os_string(),
        None => std::ffi::OsString::from("config.toml"),
    };
    name.push(format!(".tmp-{}", std::process::id()));
    path.with_file_name(name)
}

/// Best-effort removal of the temp files a crashed run left beside `path`:
/// every sibling named `<config file name>.tmp-*`.
///
/// Called once from [`Config::load`] at startup (V7). All failures are
/// swallowed — a stale temp is inert, and a read-only config directory must
/// not keep the config from loading.
fn cleanup_stale_temps(path: &Path) {
    let Some(dir) = path.parent() else {
        return;
    };
    let Some(name) = path.file_name() else {
        return;
    };
    if dir.as_os_str().is_empty() {
        return;
    }
    let mut prefix = name.to_os_string();
    prefix.push(".tmp-");
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry
            .file_name()
            .as_encoded_bytes()
            .starts_with(prefix.as_encoded_bytes())
        {
            let _ = fs::remove_file(entry.path());
        }
    }
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

    #[test]
    fn load_legacy_plain_string_config() {
        // Files written before the marker encoding existed must keep loading
        // unchanged (backward compatibility of the string form).
        let dir = TempDir::new("legacy");
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            "scan_dirs = [\"/home/user/repos\", \"/opt/projects\"]\n",
        )
        .expect("write legacy config");
        let loaded = load_from(&path).expect("legacy config loads");
        assert_eq!(
            loaded.scan_dirs,
            vec![
                PathBuf::from("/home/user/repos"),
                PathBuf::from("/opt/projects"),
            ]
        );
    }

    #[test]
    fn load_empty_config_file_returns_default() {
        let dir = TempDir::new("empty");
        let path = dir.path().join("config.toml");
        fs::write(&path, "").expect("write empty config");
        let loaded = load_from(&path).expect("empty config loads");
        assert_eq!(loaded, Config::default());
        assert!(loaded.scan_dirs.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn save_load_round_trips_non_utf8_scan_dirs() {
        use std::os::unix::ffi::OsStrExt as _;
        let dir = TempDir::new("non-utf8");
        let path = dir.path().join("config.toml");
        let non_utf8 = PathBuf::from(std::ffi::OsStr::from_bytes(b"/tmp/\xff\xfe-not-utf8"));
        let config = Config {
            scan_dirs: vec![PathBuf::from("/home/user/repos"), non_utf8],
        };
        save_to(&config, &path).expect("save non-UTF-8 config");
        let loaded = load_from(&path).expect("load non-UTF-8 config");
        assert_eq!(loaded, config);
    }

    #[cfg(unix)]
    #[test]
    fn loads_legacy_and_marker_entries_side_by_side() {
        use std::os::unix::ffi::OsStrExt as _;
        let dir = TempDir::new("mixed");
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            "scan_dirs = [\"/home/user/repos\", { path_hex = \"2fffe5\" }]\n",
        )
        .expect("write mixed config");
        let loaded = load_from(&path).expect("mixed config loads");
        assert_eq!(
            loaded.scan_dirs,
            vec![
                PathBuf::from("/home/user/repos"),
                PathBuf::from(std::ffi::OsStr::from_bytes(b"/\xff\xe5")),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn config_save_persists_non_utf8_scan_dir_via_public_api() {
        // Holds the I/O guard: the save/load pair below uses the shared
        // redirect installed on that config file.
        use std::os::unix::ffi::OsStrExt as _;
        let _io = super::config_io_guard();
        let config = Config {
            scan_dirs: vec![PathBuf::from(std::ffi::OsStr::from_bytes(
                b"/tmp/\xff\xfe_picker",
            ))],
        };
        config.save().expect("save non-UTF-8 config");
        let loaded = Config::load().expect("load non-UTF-8 config");
        assert_eq!(loaded, config);
    }

    #[test]
    fn failed_save_preserves_the_previous_config_file() {
        // The temp path is blocked, so the write fails before the rename and
        // the seeded config must survive untouched (no partial contents).
        let dir = TempDir::new("atomic");
        let path = dir.path().join("config.toml");
        let seeded = Config {
            scan_dirs: vec![PathBuf::from("/home/user/repos")],
        };
        save_to(&seeded, &path).expect("seed config");
        let before = fs::read(&path).expect("read seeded config");

        let temp = temp_path(&path);
        fs::create_dir_all(&temp).expect("block temp path");
        let other = Config {
            scan_dirs: vec![PathBuf::from("/opt/projects")],
        };
        save_to(&other, &path).expect_err("blocked save must fail");

        let after = fs::read(&path).expect("config still readable");
        assert_eq!(after, before, "failed save must not touch the config");
        fs::remove_dir_all(&temp).expect("clean up blocked temp path");
    }

    #[test]
    fn failed_save_removes_its_temp_file() {
        // The target itself is blocked (a directory at `config.toml`), so the
        // rename fails: the temp file written beforehand must be cleaned up.
        let dir = TempDir::new("atomic-litter");
        let path = dir.path().join("config.toml");
        fs::create_dir_all(&path).expect("block target");
        let config = Config {
            scan_dirs: vec![PathBuf::from("/opt/projects")],
        };
        save_to(&config, &path).expect_err("blocked save must fail");

        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .expect("list config dir")
            .map(|entry| entry.expect("dir entry").file_name())
            .collect();
        assert_eq!(
            leftovers.len(),
            1,
            "only the blocking target may remain: {leftovers:?}"
        );
        fs::remove_dir_all(&path).expect("clean up blocked target");
    }

    #[cfg(unix)]
    #[test]
    fn save_keeps_owner_only_permissions_on_the_config() {
        // The config file is meant to hold credentials (the GitHub token):
        // the atomic temp file must be created 0600, otherwise the rename
        // would publish it as 0644 (`0666 & !umask`) and widen a
        // `chmod 600` config.
        use std::os::unix::fs::PermissionsExt as _;

        let dir = TempDir::new("perms");
        let path = dir.path().join("config.toml");
        let config = Config {
            scan_dirs: vec![PathBuf::from("/home/user/repos")],
        };
        save_to(&config, &path).expect("save");
        let mode = fs::metadata(&path)
            .expect("config metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "config must stay owner-only");
    }

    #[test]
    fn cleanup_stale_temps_removes_siblings_and_keeps_the_config() {
        // A crashed run leaves `<name>.tmp-*` beside the config; startup
        // sweeps exactly that prefix (V7) without touching the config
        // itself or unrelated files.
        let dir = TempDir::new("stale-temp");
        let path = dir.path().join("config.toml");
        fs::write(&path, "scan_dirs = []\n").expect("config");
        let stale_one = dir.path().join("config.toml.tmp-111-dead");
        let stale_two = dir.path().join("config.toml.tmp-999-beef");
        let unrelated = dir.path().join("other.toml.tmp-1-1");
        fs::write(&stale_one, "junk").expect("stale one");
        fs::write(&stale_two, "junk").expect("stale two");
        fs::write(&unrelated, "keep").expect("unrelated");

        cleanup_stale_temps(&path);

        assert!(!stale_one.exists(), "stale temp must go");
        assert!(!stale_two.exists(), "stale temp must go");
        assert!(path.exists(), "the config itself must stay");
        assert!(unrelated.exists(), "only this config's temps are swept");
        fs::remove_dir_all(&dir.path().to_path_buf()).expect("cleanup");
    }

    /// A temp path pre-planted as a symlink must never be written through:
    /// `create_new` fails on the existing link, the retry removes the link
    /// itself (not its target) and creates a fresh regular file (V7).
    #[cfg(unix)]
    #[test]
    fn write_temp_refuses_a_planted_symlink() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new("planted-symlink");
        let temp = dir.path().join("config.toml.tmp-planted");
        let victim = dir.path().join("victim.txt");
        fs::write(&victim, "precious").expect("victim");
        symlink(&victim, &temp).expect("plant link");

        write_temp(&temp, "new bytes").expect("retry creates a fresh file");

        assert!(
            !fs::symlink_metadata(&temp)
                .expect("temp metadata")
                .file_type()
                .is_symlink(),
            "the planted link must be replaced, not written through"
        );
        assert_eq!(
            fs::read_to_string(&victim).expect("victim content"),
            "precious",
            "the symlink target must stay untouched"
        );
        fs::remove_file(&temp).expect("remove temp");
        fs::remove_dir_all(&dir.path().to_path_buf()).expect("cleanup");
    }
}
