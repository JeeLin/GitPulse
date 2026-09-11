use serde::{Deserialize, Serialize};

/// Pull Request 信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullRequest {
    /// PR 编号
    pub number: u32,
    /// PR 标题
    pub title: String,
    /// 状态（open/closed/merged）
    pub state: String,
    /// 作者
    pub author: String,
    /// CI 状态
    pub ci_status: Option<String>,
    /// 创建时间
    pub created_at: String,
}

impl PullRequest {
    /// 创建新的 Pull Request
    pub fn new(
        number: u32,
        title: String,
        state: String,
        author: String,
        ci_status: Option<String>,
        created_at: String,
    ) -> Self {
        Self {
            number,
            title,
            state,
            author,
            ci_status,
            created_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pull_request_creation() {
        let pr = PullRequest::new(
            1,
            "Test PR".to_string(),
            "open".to_string(),
            "user".to_string(),
            Some("success".to_string()),
            "2024-01-01".to_string(),
        );

        assert_eq!(pr.number, 1);
        assert_eq!(pr.ci_status, Some("success".to_string()));
    }
}