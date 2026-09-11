use anyhow::Result;
use serde::{Deserialize, Serialize};

/// 远端仓库信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteRepo {
    /// 仓库名称
    pub name: String,
    /// 完整名称（owner/name）
    pub full_name: String,
    /// 仓库描述
    pub description: Option<String>,
    /// 主要编程语言
    pub language: Option<String>,
    /// Star 数量
    pub stars: u32,
    /// 最后更新时间
    pub updated_at: String,
    /// 是否为 Fork 仓库
    pub is_fork: bool,
    /// 克隆 URL
    pub clone_url: String,
}

/// 远端仓库构建器
pub struct RemoteRepoBuilder {
    name: String,
    full_name: String,
    description: Option<String>,
    language: Option<String>,
    stars: u32,
    updated_at: String,
    is_fork: bool,
    clone_url: String,
}

impl RemoteRepoBuilder {
    pub fn new(name: String, full_name: String) -> Self {
        Self {
            name,
            full_name,
            description: None,
            language: None,
            stars: 0,
            updated_at: String::new(),
            is_fork: false,
            clone_url: String::new(),
        }
    }

    pub fn description(mut self, val: Option<String>) -> Self {
        self.description = val;
        self
    }

    pub fn language(mut self, val: Option<String>) -> Self {
        self.language = val;
        self
    }

    pub fn stars(mut self, val: u32) -> Self {
        self.stars = val;
        self
    }

    pub fn updated_at(mut self, val: String) -> Self {
        self.updated_at = val;
        self
    }

    pub fn is_fork(mut self, val: bool) -> Self {
        self.is_fork = val;
        self
    }

    pub fn clone_url(mut self, val: String) -> Self {
        self.clone_url = val;
        self
    }

    pub fn build(self) -> RemoteRepo {
        RemoteRepo {
            name: self.name,
            full_name: self.full_name,
            description: self.description,
            language: self.language,
            stars: self.stars,
            updated_at: self.updated_at,
            is_fork: self.is_fork,
            clone_url: self.clone_url,
        }
    }
}

/// 克隆仓库到本地目录
pub fn clone_repo(url: &str, dest: &std::path::Path) -> Result<()> {
    std::process::Command::new("git")
        .args(["clone", url])
        .arg(dest.to_str().unwrap_or_default())
        .output()
        .map_err(|e| anyhow::anyhow!("Failed to clone repository: {}", e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_remote_repo_creation() {
        let repo = RemoteRepoBuilder::new(
            "test-repo".to_string(),
            "user/test-repo".to_string(),
        )
        .description(Some("A test repository".to_string()))
        .language(Some("Rust".to_string()))
        .stars(42)
        .updated_at("2024-01-01 12:00".to_string())
        .is_fork(false)
        .clone_url("https://github.com/user/test-repo.git".to_string())
        .build();

        assert_eq!(repo.name, "test-repo");
        assert_eq!(repo.stars, 42);
    }
}