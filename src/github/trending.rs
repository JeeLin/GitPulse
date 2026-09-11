use serde::{Deserialize, Serialize};

/// Trending 仓库信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendingRepo {
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
    /// 今日 Star 增量
    pub stars_today: u32,
    /// Fork 数量
    pub forks: u32,
}

impl TrendingRepo {
    /// 创建新的 Trending 仓库
    pub fn new(
        name: String,
        full_name: String,
        description: Option<String>,
        language: Option<String>,
        stars: u32,
        stars_today: u32,
        forks: u32,
    ) -> Self {
        Self {
            name,
            full_name,
            description,
            language,
            stars,
            stars_today,
            forks,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trending_repo_creation() {
        let repo = TrendingRepo::new(
            "test-repo".to_string(),
            "user/test-repo".to_string(),
            Some("A test repository".to_string()),
            Some("Rust".to_string()),
            1000,
            50,
            100,
        );
        assert_eq!(repo.stars, 1000);
        assert_eq!(repo.stars_today, 50);
    }
}