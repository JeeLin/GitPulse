//! Configuration type definitions.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Persistent GitPulse configuration stored in `~/.gitpulse/config.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Directories scanned for git repositories at startup.
    pub scan_dirs: Vec<PathBuf>,
}
