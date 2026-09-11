use serde::{Deserialize, Serialize};

/// Workflow Run 信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRun {
    /// Run ID
    pub id: u64,
    /// Run 名称
    pub name: String,
    /// 状态（completed/in_progress/queued）
    pub status: String,
    /// 结果（success/failure/cancelled）
    pub conclusion: Option<String>,
    /// 创建时间
    pub created_at: String,
    /// 更新时间
    pub updated_at: String,
}

impl WorkflowRun {
    /// 创建新的 Workflow Run
    pub fn new(
        id: u64,
        name: String,
        status: String,
        conclusion: Option<String>,
        created_at: String,
        updated_at: String,
    ) -> Self {
        Self {
            id,
            name,
            status,
            conclusion,
            created_at,
            updated_at,
        }
    }

    /// 获取状态图标
    pub fn status_icon(&self) -> &str {
        match self.conclusion.as_deref() {
            Some("success") => "✅",
            Some("failure") => "❌",
            Some("cancelled") => "⚠️",
            _ => match self.status.as_str() {
                "in_progress" => "🔄",
                "queued" => "⏳",
                _ => "❓",
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workflow_run_creation() {
        let run = WorkflowRun::new(
            12345,
            "CI".to_string(),
            "completed".to_string(),
            Some("success".to_string()),
            "2024-01-01".to_string(),
            "2024-01-01".to_string(),
        );

        assert_eq!(run.id, 12345);
        assert_eq!(run.status_icon(), "✅");
    }

    #[test]
    fn test_workflow_run_status_icons() {
        let success_run = WorkflowRun::new(
            1,
            "CI".to_string(),
            "completed".to_string(),
            Some("success".to_string()),
            "2024-01-01".to_string(),
            "2024-01-01".to_string(),
        );
        assert_eq!(success_run.status_icon(), "✅");

        let failure_run = WorkflowRun::new(
            2,
            "CI".to_string(),
            "completed".to_string(),
            Some("failure".to_string()),
            "2024-01-01".to_string(),
            "2024-01-01".to_string(),
        );
        assert_eq!(failure_run.status_icon(), "❌");

        let running_run = WorkflowRun::new(
            3,
            "CI".to_string(),
            "in_progress".to_string(),
            None,
            "2024-01-01".to_string(),
            "2024-01-01".to_string(),
        );
        assert_eq!(running_run.status_icon(), "🔄");
    }
}