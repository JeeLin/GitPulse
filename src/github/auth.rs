//! Token resolution for the GitHub API (auth layer only).
//!
//! Single backend: `GITHUB_TOKEN` env → `gh auth token` subprocess. `gh` is
//! optional — when absent and no env token is set, returns a guided error.
//!
//! The token is resolved once and cached in the per-host
//! [`HostTokenCache`] shared with [`crate::platform::auth`] (M2: no
//! `Octocrab::default()`, client reused across `r` refreshes). 401-triggered
//! re-resolve is a future additive amendment, not applied here.

use secrecy::SecretString;

use super::error::GithubError;
use crate::platform::auth::HostTokenCache;

/// Cache key standing in for GitHub's single process-wide token.
const GITHUB_CACHE_KEY: &str = "api.github.com";

/// Cached token singleton (resolved once per process).
static TOKEN: HostTokenCache = HostTokenCache::new();

/// Resolve a GitHub token, preferring the environment, then `gh auth token`.
///
/// The raw token is wrapped in `SecretString` only at the boundary; precedence
/// itself lives in [`pick_token`] so it is unit-testable without exposing the
/// secret.
pub(crate) fn resolve_token() -> Result<SecretString, GithubError> {
    TOKEN.get_or_init(GITHUB_CACHE_KEY, || pick_token(env_token(), gh_token()))
}

/// Pure precedence logic: env token wins; empty env falls back to `gh`;
/// both absent/error yields a guided [`GithubError::Token`].
pub(crate) fn pick_token(
    env: Option<String>,
    gh: Result<String, String>,
) -> Result<String, GithubError> {
    if let Some(t) = env {
        if !t.is_empty() {
            return Ok(t);
        }
    }
    match gh {
        Ok(t) if !t.is_empty() => Ok(t),
        Ok(_) => Err(GithubError::Token(
            "gh auth token 为空，请设置 GITHUB_TOKEN 环境变量".into(),
        )),
        Err(_) => Err(GithubError::Token(
            "gh 不可用，无法自动获取 token；设置 GITHUB_TOKEN 环境变量后重试".into(),
        )),
    }
}

fn env_token() -> Option<String> {
    std::env::var("GITHUB_TOKEN").ok()
}

fn gh_token() -> Result<String, String> {
    let out = std::process::Command::new("gh")
        .args(["auth", "token"])
        .output()
        .map_err(|e| format!("gh 不可用：{e}"))?;
    if !out.status.success() {
        return Err(format!(
            "gh auth token 失败（未登录？）：{}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let t = String::from_utf8(out.stdout).map_err(|e| format!("输出非 UTF-8：{e}"))?;
    Ok(t.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    #[test]
    fn shared_cache_resolves_once_and_caches_per_host() {
        let cache = HostTokenCache::new();
        let calls = std::cell::Cell::new(0);
        let resolve = || {
            calls.set(calls.get() + 1);
            Ok::<_, GithubError>("tok".to_string())
        };
        let first = cache.get_or_init("host", resolve).expect("first resolve");
        let second = cache.get_or_init("host", resolve).expect("cached resolve");
        assert_eq!(calls.get(), 1, "resolver runs once per host");
        assert_eq!(
            first.expose_secret().to_string(),
            second.expose_secret().to_string()
        );
    }

    #[test]
    fn shared_cache_does_not_cache_a_failed_resolve() {
        let cache = HostTokenCache::new();
        let err = cache
            .get_or_init("host", || {
                Err::<String, _>(GithubError::Token("nope".into()))
            })
            .expect_err("resolution fails");
        assert!(matches!(err, GithubError::Token(_)));
        let ok = cache
            .get_or_init("host", || Ok::<_, GithubError>("tok".to_string()))
            .expect("retried after failure");
        assert_eq!(ok.expose_secret().to_string(), "tok");
    }

    #[test]
    fn env_token_wins_over_gh() {
        let token =
            pick_token(Some("ghp_env".into()), Ok("ghp_gh".into())).expect("env token wins");
        assert_eq!(token, "ghp_env");
    }

    #[test]
    fn empty_env_falls_back_to_gh() {
        let token =
            pick_token(Some(String::new()), Ok("ghp_gh".into())).expect("empty env falls back");
        assert_eq!(token, "ghp_gh");
    }

    #[test]
    fn empty_gh_token_is_error() {
        let err = pick_token(None, Ok(String::new())).unwrap_err();
        match err {
            GithubError::Token(m) => assert!(m.contains("gh auth token 为空")),
            other => panic!("expected Token error, got {other:?}"),
        }
    }

    #[test]
    fn missing_env_and_gh_is_error() {
        let err = pick_token(None, Err("no gh".into())).unwrap_err();
        match err {
            GithubError::Token(m) => assert!(m.contains("gh 不可用")),
            other => panic!("expected Token error, got {other:?}"),
        }
    }
}
