use serde::{Deserialize, Serialize};

/// 远端平台类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Platform {
    GitHub,
    Gitea,
    GitLab,
}

impl std::fmt::Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Platform::GitHub => write!(f, "GitHub"),
            Platform::Gitea => write!(f, "Gitea"),
            Platform::GitLab => write!(f, "GitLab"),
        }
    }
}

/// 远端账户信息
#[derive(Clone, Serialize, Deserialize)]
pub struct Account {
    /// 账户名称（如 "personal"、"work"）
    pub name: String,
    /// 远端平台
    pub platform: Platform,
    /// API token
    pub token: String,
    /// 默认用户名
    pub default_username: Option<String>,
    /// 默认邮箱
    pub default_email: Option<String>,
}

impl Account {
    /// 创建新账户
    pub fn new(
        name: String,
        platform: Platform,
        token: String,
        default_username: Option<String>,
        default_email: Option<String>,
    ) -> Self {
        Self {
            name,
            platform,
            token,
            default_username,
            default_email,
        }
    }
}

impl std::fmt::Debug for Account {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Account")
            .field("name", &self.name)
            .field("platform", &self.platform)
            .field("token", &"[REDACTED]")
            .field("default_username", &self.default_username)
            .field("default_email", &self.default_email)
            .finish()
    }
}

impl Account {
    /// 获取显示名称
    pub fn display_name(&self) -> String {
        format!("{} ({})", self.name, self.platform)
    }
}

/// 仓库与账户的绑定关系
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoBinding {
    /// 仓库路径
    pub repo_path: String,
    /// 绑定的账户名称
    pub account_name: String,
}

/// 账户配置
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AccountConfig {
    /// 所有账户
    #[serde(default)]
    pub accounts: Vec<Account>,
    /// 仓库绑定关系
    #[serde(default)]
    pub repo_bindings: Vec<RepoBinding>,
}

impl AccountConfig {
    /// 根据仓库路径获取绑定的账户
    pub fn get_account_for_repo(&self, repo_path: &str) -> Option<&Account> {
        // 查找仓库绑定
        if let Some(binding) = self.repo_bindings.iter().find(|b| b.repo_path == repo_path) {
            return self.accounts.iter().find(|a| a.name == binding.account_name);
        }
        // 无绑定时返回第一个账户
        self.accounts.first()
    }

    /// 根据名称获取账户
    pub fn get_account_by_name(&self, name: &str) -> Option<&Account> {
        self.accounts.iter().find(|a| a.name == name)
    }

    /// 添加账户
    pub fn add_account(&mut self, account: Account) {
        self.accounts.push(account);
    }

    /// 删除账户
    pub fn remove_account(&mut self, name: &str) -> bool {
        let before = self.accounts.len();
        self.accounts.retain(|a| a.name != name);
        // 同时删除相关绑定
        self.repo_bindings.retain(|b| b.account_name != name);
        self.accounts.len() < before
    }

    /// 添加仓库绑定
    pub fn add_binding(&mut self, binding: RepoBinding) {
        // 移除该仓库的旧绑定
        self.repo_bindings.retain(|b| b.repo_path != binding.repo_path);
        self.repo_bindings.push(binding);
    }

    /// 移除仓库绑定
    pub fn remove_binding(&mut self, repo_path: &str) -> bool {
        let before = self.repo_bindings.len();
        self.repo_bindings.retain(|b| b.repo_path != repo_path);
        self.repo_bindings.len() < before
    }
}

/// Git 提交用户信息
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GitUser {
    /// 用户名
    pub name: String,
    /// 邮箱
    pub email: String,
}

impl GitUser {
    /// 创建新的 Git 用户
    pub fn new(name: String, email: String) -> Self {
        Self { name, email }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_account(name: &str) -> Account {
        Account::new(
            name.to_string(),
            Platform::GitHub,
            "test_token".to_string(),
            Some("testuser".to_string()),
            Some("test@example.com".to_string()),
        )
    }

    #[test]
    fn test_account_display_name() {
        let account = create_test_account("personal");
        assert_eq!(account.display_name(), "personal (GitHub)");
    }

    #[test]
    fn test_account_config_get_account_for_repo_with_binding() {
        let mut config = AccountConfig::default();
        config.add_account(create_test_account("personal"));
        config.add_account(create_test_account("work"));
        config.add_binding(RepoBinding {
            repo_path: "/projects/work-repo".to_string(),
            account_name: "work".to_string(),
        });

        let account = config.get_account_for_repo("/projects/work-repo");
        assert!(account.is_some());
        assert_eq!(account.unwrap().name, "work");
    }

    #[test]
    fn test_account_config_get_account_for_repo_without_binding() {
        let mut config = AccountConfig::default();
        config.add_account(create_test_account("personal"));

        let account = config.get_account_for_repo("/any/repo");
        assert!(account.is_some());
        assert_eq!(account.unwrap().name, "personal");
    }

    #[test]
    fn test_account_config_remove_account() {
        let mut config = AccountConfig::default();
        config.add_account(create_test_account("personal"));
        config.add_binding(RepoBinding {
            repo_path: "/test".to_string(),
            account_name: "personal".to_string(),
        });

        assert!(config.remove_account("personal"));
        assert!(config.accounts.is_empty());
        assert!(config.repo_bindings.is_empty());
    }

    #[test]
    fn test_account_config_update_binding() {
        let mut config = AccountConfig::default();
        config.add_account(create_test_account("personal"));
        config.add_account(create_test_account("work"));
        
        config.add_binding(RepoBinding {
            repo_path: "/test".to_string(),
            account_name: "personal".to_string(),
        });
        
        // 更新绑定
        config.add_binding(RepoBinding {
            repo_path: "/test".to_string(),
            account_name: "work".to_string(),
        });

        assert_eq!(config.repo_bindings.len(), 1);
        assert_eq!(config.repo_bindings[0].account_name, "work");
    }
}