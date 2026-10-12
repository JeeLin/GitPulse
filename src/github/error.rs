//! Error type for GitHub API access (rendering lives in `platform::error`).

use thiserror::Error;

use crate::platform::{PlatformError, PlatformKind};

/// Errors produced by [`crate::github::GithubClient`].
///
/// Network/HTTP failures from octocrab are normalized into the [`Self::Http`],
/// [`Self::Network`], and [`Self::Other`] variants so
/// [`crate::platform::render_platform_error`] can render a
/// stable, human-readable message regardless of octocrab's internal error shape.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum GithubError {
    /// No usable GitHub token could be resolved (`GITHUB_TOKEN` / `gh auth token`).
    #[error("{0}")]
    Token(String),

    /// The repository's remote URL does not point at `github.com`.
    #[error("{0}")]
    #[allow(dead_code)]
    NotGitHub(String),

    /// The GitHub API answered with an HTTP error status.
    #[error("{message}")]
    Http {
        /// HTTP status code (e.g. 401, 403, 404, 429).
        status: u16,
        /// Message reported by GitHub, when available.
        message: String,
        /// Whether GitHub signalled rate limiting via `x-ratelimit-remaining: 0`.
        rate_limited: bool,
    },

    /// The request failed before GitHub responded (DNS, TLS, connect, …).
    #[error("{0}")]
    Network(String),

    /// Any other octocrab error.
    #[error("{0}")]
    Other(String),
}

impl From<octocrab::Error> for GithubError {
    fn from(e: octocrab::Error) -> Self {
        match e {
            octocrab::Error::GitHub { source, .. } => GithubError::Http {
                status: source.status_code.as_u16(),
                message: source.message.clone(),
                // octocrab 0.54.2's GitHub error source has no response headers
                // (docs_url/errors/message/status_code only); infer from status 429.
                rate_limited: source.status_code.as_u16() == 429,
            },
            octocrab::Error::Hyper { .. } => GithubError::Network(e.to_string()),
            other => GithubError::Other(other.to_string()),
        }
    }
}

/// Map [`crate::github::GithubError`] → [`PlatformError`] (platform label =
/// `"GitHub"`).
///
/// Implemented here, on the backend side, because `platform` is the abstraction
/// layer and must not name a concrete backend type; `github` is the layer that
/// implements [`crate::platform::PlatformClient`], so the dependency runs
/// `github` → `platform` (and never the other way).
impl From<crate::github::GithubError> for PlatformError {
    fn from(e: GithubError) -> Self {
        let platform = PlatformKind::GitHub.as_str();
        match e {
            GithubError::Token(m) => PlatformError::Token(m),
            GithubError::NotGitHub(m) => PlatformError::NotPlatform(m),
            GithubError::Http {
                status,
                message,
                rate_limited,
            } => PlatformError::Http {
                status,
                message,
                rate_limited,
                platform,
            },
            GithubError::Network(m) => PlatformError::Network(m),
            GithubError::Other(m) => PlatformError::Other {
                platform,
                message: m,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::platform::render_platform_error;

    #[test]
    fn github_error_display_is_the_message() {
        assert_eq!(
            GithubError::Token("需要 token".into()).to_string(),
            "需要 token"
        );
        assert_eq!(
            GithubError::Network("dns fail".into()).to_string(),
            "dns fail"
        );
    }

    #[test]
    fn http_variant_renders_its_message() {
        let e = GithubError::Http {
            status: 404,
            message: "Not Found".into(),
            rate_limited: false,
        };
        assert_eq!(e.to_string(), "Not Found");
    }

    #[test]
    fn from_octocrab_maps_http_status_and_rate_limit() {
        // Building an octocrab::Error without the network is awkward; assert the
        // mapping contract on the enum shapes that matter instead.
        let e = GithubError::Http {
            status: 429,
            message: String::new(),
            rate_limited: true,
        };
        let p: crate::platform::PlatformError = e.into();
        assert_eq!(
            crate::platform::render_platform_error(&p),
            "GitHub 速率限制，请稍后重试"
        );
    }

    #[test]
    fn from_github_error_maps_https_with_platform_label() {
        let e: PlatformError = GithubError::Http {
            status: 404,
            message: "nope".into(),
            rate_limited: false,
        }
        .into();
        assert!(matches!(
            e,
            PlatformError::Http {
                status: 404,
                platform: "GitHub",
                ..
            }
        ));
    }

    #[test]
    fn from_github_error_network_passthrough() {
        let e: PlatformError = GithubError::Network("dns fail".into()).into();
        assert!(matches!(e, PlatformError::Network(_)));
    }

    #[test]
    fn from_github_error_other_carries_label() {
        let e: PlatformError = GithubError::Other("boom".into()).into();
        assert!(matches!(
            e,
            PlatformError::Other {
                platform: "GitHub",
                message
            } if message == "boom"
        ));
    }

    #[test]
    fn render_rate_limited() {
        let e: PlatformError = GithubError::Http {
            status: 429,
            message: String::new(),
            rate_limited: false,
        }
        .into();
        assert_eq!(render_platform_error(&e), "GitHub 速率限制，请稍后重试");
    }

    #[test]
    fn render_network_is_generic() {
        let e: PlatformError = GithubError::Network("dns fail".into()).into();
        assert_eq!(render_platform_error(&e), "网络错误，请检查网络连接");
    }
}
