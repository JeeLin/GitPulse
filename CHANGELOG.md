# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

## [0.2.0] - 2026-09-27
### Added
- 配置管理面板：仪表盘空态引导、配置视图、添加/删除扫描目录、目录选择器
- 配置持久化与错误回滚（保存失败时回滚内存状态）
- 路径去重（canonicalization）及兜底去重（发现阶段）
- 模态键位提示切换（确认/输入期间底部提示）
- 长路径换行显示（删除确认浮层）
- 错误处理完善（anyhow + thiserror）
- 配置 IO 守护（测试隔离）

### Changed
- 配置路径守护：使用 OnceLock 覆盖而非修改 HOME
- 目录选择器与手动输入统一入口（a 打开选择器，i 手动输入）

### Fixed
- 空态文案误导（“扫描目录为空” → “暂无仓库”）
- j/k 导航重复代码抽取
- 确认/输入浮层重复代码抽取
- 保存失败回滚测试覆盖
- 长路径光标可见性（滚动偏移按显示宽度）

### Removed
- （无）

