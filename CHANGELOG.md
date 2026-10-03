# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

## [0.4.0] - 2026-10-03
### Added
- 单平台远端回据层（GitHub）：PR 列表/详情、CI checks、Issues、通知面板 + 手动刷新 `r`
- octocrab 单后端（0.54）；`gh` 仅作为 GitHub token 解析器（`GITHUB_TOKEN` → `gh auth token`）；OnceLock client 缓存
- Issue 详情面板：`#N title (state)` 头行 + meta（author / assignee / labels / updated_at） + 软袭的 `正文`，布局复用 render_pull_detail（无 checks 子窗）
- 回据 hub 键位：`g` 进入；`p`/`c`/`i`/`n` 切页 Checks/PR/Issues/Notifications；Enter 详情 / Esc 返回
- CI checks 页签取 HEAD commit 运行列表（`git head_sha` + `list_checks`，不走 get_pull）
### Changed
- 远端回据刷新改为手动 `r` 触发（不自动轮循，留待后续评估）
### Fixed
- IssueDetail 详情刷新前置 `None`，清除 stale payload 后从远端再拉

## [0.3.0] - 2026-10-02
### Added
- 提交历史视图：提交列表 + 分支/tag/HEAD 标记 + j/k 导航 + 提交详情（超 500 文件截断计数）
- DAG 图示：分支/合并拓扑连线（`layout_graph`，固定 `MAX_LANES=6` 行宽，`GRAPH_WIDTH` 单值源）
- tag 管理面板：列表、创建、删除（确认框），非 UTF-8 tag 守卫与非 commit tag 枚举（悬空/断链跳过）
- tag 推送：单 tag / 全量（`--tags`）推送 + 远端状态行运行中/成功/失败三态反馈
- 状态栏类型下沉 `src/ui/status.rs`（`StatusLine`/`RemoteStatus` 渲染）

### Changed
- `RemoteStatus::Progress` → `Running(String)`：删除冻结 0% 百分比，改为运行文本
- `HistoryState.graph` 类型收敛 `Vec<Vec<char>>`（DAG 列单点生成）
- 推送/拉取改用全 refspec `refs/heads/<branch>` / `refs/tags/<tag>`，分支/tag 名不可被解析为 git 选项
- 远端名拒绝 `-` 开头；SSH 远端追加 `-oBatchMode=yes` 防止会话级锁死
- 非交互远端操作：`GIT_TERMINAL_PROMPT=0` + askpass 清空 + 复用系统 credential helper

### Fixed
- 配置原子写：`create_new` 建临时文件 + 拒绝 planted symlink + 启动清 stale temp + 0600 权限
- 非 UTF-8 路径：NUL 字节拒绝、错误链透出、`Config::save` 原子替换防回滚
- 批量 pull 凭据阻塞：4 处 `Command::new("git")` 统一接入 `configure_git_cmd`
- HistoryState/TagsState 光标逻辑收敛为 `select_row`/`move_row`（消除重复）
- 模块文档与实现同步（进度百分比、死代码注释）

### Removed
- `git/mod.rs` 死导出：`RefMark`/`GraphCell` re-export（无非测试消费者）
- `app.rs` 死代码 `current_repo_name`

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

