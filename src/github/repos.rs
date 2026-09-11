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

impl RemoteRepo {
    /// 创建新的远端仓库
    pub fn new(
        name: String,
        full_name: String,
        description: Option<String>,
        language: Option<String>,
        stars: u32,
        updated_at: String,
        is_fork: bool,
        clone_url: String,
    ) -> Self {
        Self {
            name,
            full_name,
            description,
            language,
            stars,
            updated_at,
            is_fork,
            clone_url,
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
        let repo = RemoteRepo::new(
            "test-repo".to_string(),
            "user/test-repo".to_string(),
            Some("A test repository".to_string()),
            Some("Rust".to_string()),
            42,
            "2024-01-01 12:00".to_string(),
            false,
            "https://github.com/user/test-repo.git".to_string(),
        );

        assert_eq!(repo.name, "test-repo");
        assert_eq!(repo.stars, 42);
    }
}