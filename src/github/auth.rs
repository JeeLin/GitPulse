use anyhow::Result;
use std::fs;

pub async fn create_client() -> Result<octocrab::Octocrab> {
    let token = find_github_token()?;

    let octocrab = octocrab::OctocrabBuilder::default()
        .personal_token(token)
        .build()?;

    Ok(octocrab)
}

fn find_github_token() -> Result<String> {
    // 1. Check GITHUB_TOKEN environment variable
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        return Ok(token);
    }

    // 2. Check gh CLI config
    if let Some(token) = find_gh_cli_token() {
        return Ok(token);
    }

    // 3. Check gitpulse config
    if let Some(token) = find_gitpulse_token() {
        return Ok(token);
    }

    anyhow::bail!("No GitHub token found. Set GITHUB_TOKEN, run gh auth login, or configure in ~/.gitpulse/config.toml")
}

fn find_gh_cli_token() -> Option<String> {
    let config_dir = dirs::config_dir()?;
    let gh_config = config_dir.join("gh").join("hosts.yml");

    if !gh_config.exists() {
        return None;
    }

    let content = fs::read_to_string(gh_config).ok()?;

    // Simple YAML parsing for oauth_token
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("oauth_token:") {
            let token = line.strip_prefix("oauth_token:")?.trim();
            // Remove quotes if present
            let token = token.trim_matches(|c| c == '"' || c == '\'');
            return Some(token.to_string());
        }
    }

    None
}

fn find_gitpulse_token() -> Option<String> {
    let config_dir = dirs::config_dir()?;
    let config_file = config_dir.join("gitpulse").join("config.toml");

    if !config_file.exists() {
        return None;
    }

    let content = fs::read_to_string(config_file).ok()?;
    let config: crate::config::ConfigFile = toml::from_str(&content).ok()?;

    config.github?.token
}
