/// 搜索结果
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// 仓库名称
    pub repo_name: String,
    /// 文件路径
    pub file_path: String,
    /// 行号
    pub line_number: u32,
    /// 行内容
    pub line_content: String,
}

impl SearchResult {
    /// 创建新的搜索结果
    pub fn new(
        repo_name: String,
        file_path: String,
        line_number: u32,
        line_content: String,
    ) -> Self {
        Self {
            repo_name,
            file_path,
            line_number,
            line_content,
        }
    }
}

/// 在指定仓库中搜索代码
pub fn search_in_repo(repo_path: &std::path::Path, query: &str) -> Vec<SearchResult> {
    let repo_name = repo_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    let output = std::process::Command::new("git")
        .args(["grep", "-n", query])
        .current_dir(repo_path)
        .output();

    match output {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            stdout
                .lines()
                .filter_map(|line| {
                    let parts: Vec<&str> = line.splitn(3, ':').collect();
                    if parts.len() >= 3 {
                        Some(SearchResult::new(
                            repo_name.clone(),
                            parts[0].to_string(),
                            parts[1].parse().unwrap_or(0),
                            parts[2].to_string(),
                        ))
                    } else {
                        None
                    }
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

/// 在所有仓库中搜索代码
pub fn search_all_repos(
    repos: &[std::path::PathBuf],
    query: &str,
) -> Vec<SearchResult> {
    let mut results = Vec::new();
    for repo in repos {
        let repo_results = search_in_repo(repo, query);
        results.extend(repo_results);
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_result_creation() {
        let result = SearchResult::new(
            "repo".to_string(),
            "src/main.rs".to_string(),
            42,
            "fn main()".to_string(),
        );
        assert_eq!(result.repo_name, "repo");
        assert_eq!(result.line_number, 42);
    }
}