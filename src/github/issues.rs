use serde::{Deserialize, Serialize};

/// Issue 信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    /// Issue 编号
    pub number: u32,
    /// Issue 标题
    pub title: String,
    /// 状态（open/closed）
    pub state: String,
    /// 作者
    pub author: String,
    /// 标签列表
    pub labels: Vec<String>,
    /// 创建时间
    pub created_at: String,
}

impl Issue {
    /// 创建新的 Issue
    pub fn new(
        number: u32,
        title: String,
        state: String,
        author: String,
        labels: Vec<String>,
        created_at: String,
    ) -> Self {
        Self {
            number,
            title,
            state,
            author,
            labels,
            created_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_issue_creation() {
        let issue = Issue::new(
            1,
            "Test Issue".to_string(),
            "open".to_string(),
            "user".to_string(),
            vec!["bug".to_string()],
            "2024-01-01".to_string(),
        );

        assert_eq!(issue.number, 1);
        assert_eq!(issue.state, "open");
    }
}
