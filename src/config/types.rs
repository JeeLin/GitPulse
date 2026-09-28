//! Configuration type definitions.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Persistent GitPulse configuration stored in `~/.gitpulse/config.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Directories scanned for git repositories at startup.
    ///
    /// Encoded entry by entry so non-UTF-8 paths persist too: UTF-8 paths are
    /// plain TOML strings (the original format), anything else uses the
    /// additive `{ path_hex = .. }` marker documented in the config encoding
    /// module.
    #[serde(with = "crate::config::encoding")]
    pub scan_dirs: Vec<PathBuf>,
}
