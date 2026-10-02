//! Token resolution for the GitHub API (auth layer only).
//!
//! Single backend: `GITHUB_TOKEN` env → `gh auth token` subprocess. `gh` is
//! optional — when absent and no env token is set, returns a guided error.
//!
//! The token is resolved once and cached in a `OnceLock` (M2: no
//! `Octocrab::default()`, client reused across `r` refreshes). 401-triggered
//! re-resolve is a future additive amendment, not applied here.

use std::sync::OnceLock;

use secrecy::SecretString;

use super::error::GithubError;

/// Cached token singleton (resolved once per process).
static TOKEN: OnceLock<SecretString> = OnceLock::new();

/// Resolve a GitHub token, preferring the environment, then `gh auth token`.
///
/// The raw token is wrapped in `SecretString` only at the boundary; precedence
/// itself lives in [`pick_token`] so it is unit-testable without exposing the
/// secret.
pub(crate) fn resolve_token() -> Result<SecretString, GithubError> {
    if let Some(t) = TOKEN.get() {
        return Ok(t.clone());
    }
    let raw = pick_token(env_token(), gh_token())?;
    let secret = SecretString::new(raw.into_boxed_str());
    let _ = TOKEN.set(secret.clone());
    Ok(secret)
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
