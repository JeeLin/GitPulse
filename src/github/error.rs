//! Error type and user-facing rendering for GitHub API access.

use thiserror::Error;

/// Errors produced by [`crate::github::GithubClient`].
///
/// Network/HTTP failures from octocrab are normalized into the [`Http`], [`Network`], and
/// [`Other`] variants so the UI can render a stable, human-readable message through
/// [`render_error`] regardless of octocrab's internal error shape.
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

/// Render an error as a short, user-facing line.
///
/// M2's single text channel: the caller passes the result to `App::set_error`.
/// Status-aware mapping per design step2 (401/404/403/rate-limit/5xx/network).
#[allow(dead_code)]
pub fn render_error(e: &GithubError) -> String {
    match e {
        GithubError::Token(m) | GithubError::NotGitHub(m) => m.clone(),
        GithubError::Network(_) => "网络错误，请检查网络连接".to_string(),
        GithubError::Other(m) => format!("GitHub 错误: {m}"),
        GithubError::Http {
            status,
            message,
            rate_limited,
        } => {
            if *rate_limited || *status == 429 {
                return "GitHub 速率限制，请稍后重试".to_string();
            }
            match *status {
                401 => "GitHub 认证失败，请检查 GITHUB_TOKEN / gh auth token".to_string(),
                403 => "无权限访问该资源".to_string(),
                404 => "仓库/资源不存在或无权限".to_string(),
                500..=599 => "GitHub 服务端错误，请稍后重试".to_string(),
                _ => {
                    if message.is_empty() {
                        format!("GitHub HTTP {status}")
                    } else {
                        message.clone()
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_token_and_not_github_passthrough() {
        assert_eq!(
            render_error(&GithubError::Token("需要 token".into())),
            "需要 token"
        );
        assert_eq!(
            render_error(&GithubError::NotGitHub("非 GitHub 远端".into())),
            "非 GitHub 远端"
        );
    }

    #[test]
    fn render_network_is_generic() {
        assert_eq!(
            render_error(&GithubError::Network("dns fail".into())),
            "网络错误，请检查网络连接"
        );
    }

    #[test]
    fn render_other_keeps_detail() {
        assert_eq!(
            render_error(&GithubError::Other("boom".into())),
            "GitHub 错误: boom"
        );
    }

    #[test]
    fn render_status_specific_messages() {
        let cases = [
            (
                401,
                false,
                "GitHub 认证失败，请检查 GITHUB_TOKEN / gh auth token",
            ),
            (403, false, "无权限访问该资源"),
            (404, false, "仓库/资源不存在或无权限"),
            (503, false, "GitHub 服务端错误，请稍后重试"),
            (429, false, "GitHub 速率限制，请稍后重试"),
            (403, true, "GitHub 速率限制，请稍后重试"),
        ];
        for (status, rate_limited, want) in cases {
            let got = render_error(&GithubError::Http {
                status,
                message: "ignored".into(),
                rate_limited,
            });
            assert_eq!(got, want, "status={status} rate_limited={rate_limited}");
        }
    }

    #[test]
    fn render_unknown_status_falls_back_to_message() {
        assert_eq!(
            render_error(&GithubError::Http {
                status: 418,
                message: "teapot".into(),
                rate_limited: false,
            }),
            "teapot"
        );
        assert_eq!(
            render_error(&GithubError::Http {
                status: 418,
                message: String::new(),
                rate_limited: false,
            }),
            "GitHub HTTP 418"
        );
    }
}
