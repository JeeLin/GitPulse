# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

## [Unreleased]

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
