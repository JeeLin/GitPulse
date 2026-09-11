# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

## [Unreleased]

## [0.3.0] - 2026-09-12

### Added
- 远端账户管理：多远端账户配置（GitHub/Gitea/GitLab）、账户切换 TUI 面板
- 仓库账户绑定：仓库与账户的绑定关系，支持自动选择对应账户
- Git 提交用户维护：全局默认用户和按仓库覆盖的提交用户管理
- 账户安全：Account Debug 输出自动脱敏 token 字段

## [0.2.0] - 2026-09-12

### Added
- 分支数据模型：定义 BranchInfo 结构体和 BranchStatus 枚举（Clean/Stale/Orphaned/Merged）
- 分支状态分析：使用 git2-rs 遍历本地和远程分支，智能标记状态
- 分支列表 TUI：按仓库分组展示分支列表，支持键盘导航和状态过滤
- 批量清理操作：一键删除已合并分支，支持 dry-run 预览
- 分支过滤视图：按状态过滤分支（数字键 1-4 切换，0 取消）
- 分支排除规则：支持通配符模式匹配（main/master/develop/release/*/v*/stable）
## [0.1.1] - 2026-09-11

### Added
- 通知数据模型：定义 Notification 结构体和数据库表
- 通知获取模块：调用 GitHub API 获取通知（含 mock 数据）
- 通知聚合逻辑：按仓库、类型、时间聚合
- TUI 通知中心：集成到现有仪表盘，支持 n 键切换、j/k 导航、m 标记已读、a 全部已读
- 桌面通知集成：使用 notify-rust 显示弹窗（stub 实现）
- 通知状态同步：标记已读并同步到 GitHub（stub 实现）

## [0.1.0] - 2026-09-10

### Added
- 仓库发现：扫描指定目录，自动找到所有 git 仓库
- 状态仪表盘：展示所有仓库的状态一览
- 批量操作：pull / fetch / stash / clean
- GitHub 认证：复用 gh CLI token
- 配置持久化：~/.gitpulse/config.toml
- Submodule 状态显示 + 批量 update
- 仓库分组：按业务分组管理
