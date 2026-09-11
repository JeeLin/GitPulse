use serde::{Deserialize, Serialize};

/// Starred 仓库信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StarredRepo {
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
    /// 收藏时间
    pub starred_at: String,
}

impl StarredRepo {
    /// 创建新的 Starred 仓库
    pub fn new(
        name: String,
        full_name: String,
        description: Option<String>,
        language: Option<String>,
        stars: u32,
        starred_at: String,
    ) -> Self {
        Self {
            name,
            full_name,
            description,
            language,
            stars,
            starred_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_starred_repo_creation() {
        let repo = StarredRepo::new(
            "test-repo".to_string(),
            "user/test-repo".to_string(),
            Some("A test repository".to_string()),
            Some("Rust".to_string()),
            1000,
            "2024-01-01".to_string(),
        );
        assert_eq!(repo.stars, 1000);
    }
}
