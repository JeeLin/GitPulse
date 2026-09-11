use super::models::BranchConfig;

/// 检查分支是否应该被忽略（长生命周期分支）
pub fn should_ignore(branch_name: &str, config: &BranchConfig) -> bool {
    // 检查是否匹配排除模式
    for pattern in &config.exclude_patterns {
        if matches_pattern(branch_name, pattern) {
            return true;
        }
    }
    false
}

/// 简单的通配符模式匹配
/// 支持 * 和 ? 通配符
fn matches_pattern(name: &str, pattern: &str) -> bool {
    let pattern = pattern.trim();
    
    // 精确匹配
    if name == pattern {
        return true;
    }
    
    // 如果模式中没有通配符，直接返回
    if !pattern.contains('*') && !pattern.contains('?') {
        return false;
    }
    
    // 简单的通配符匹配
    let mut name_chars = name.chars().peekable();
    let mut pattern_chars = pattern.chars().peekable();
    
    while let Some(&p) = pattern_chars.peek() {
        match p {
            '*' => {
                pattern_chars.next();
                // 跳过多个 * 
                while pattern_chars.peek() == Some(&'*') {
                    pattern_chars.next();
                }
                
                // 如果模式结束，匹配成功
                if pattern_chars.peek().is_none() {
                    return true;
                }
                
                // 尝试匹配剩余模式
                while let Some(_) = name_chars.peek() {
                    if matches_pattern(&name_chars.clone().collect::<String>(), &pattern_chars.clone().collect::<String>()) {
                        return true;
                    }
                    name_chars.next();
                }
                return false;
            }
            '?' => {
                pattern_chars.next();
                if name_chars.next().is_none() {
                    return false;
                }
            }
            c => {
                pattern_chars.next();
                match name_chars.next() {
                    Some(nc) if nc == c => continue,
                    _ => return false,
                }
            }
        }
    }
    
    name_chars.peek().is_none()
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_excludes() {
        let config = BranchConfig::default_config();
        assert!(should_ignore("main", &config));
        assert!(should_ignore("master", &config));
        assert!(should_ignore("develop", &config));
        assert!(should_ignore("release/1.0", &config));
        assert!(should_ignore("v1.0", &config));
        assert!(should_ignore("stable", &config));
        assert!(!should_ignore("feature/login", &config));
        assert!(!should_ignore("bugfix/issue-123", &config));
    }

    #[test]
    fn test_custom_patterns() {
        let config = BranchConfig {
            exclude_patterns: vec!["custom/*".to_string(), "temp*".to_string()],
            ..BranchConfig::default_config()
        };
        assert!(should_ignore("custom/feature", &config));
        assert!(should_ignore("temporary", &config));
        assert!(!should_ignore("main", &config));
    }

    #[test]
    fn test_wildcard_matching() {
        assert!(matches_pattern("main", "main"));
        assert!(matches_pattern("feature/login", "feature/*"));
        assert!(matches_pattern("v1.0", "v*"));
        assert!(!matches_pattern("feature/login", "bugfix/*"));
    }
}