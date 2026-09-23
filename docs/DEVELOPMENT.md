# GitPulse — 开发设计文档

## 新起点说明

next 分支为孤儿分支重写，旧版 v0.1-v0.7 实现归档于 master，因功能未真正接线整体废弃。

## 技术栈与质量门禁

- 技术栈：Rust / ratatui / git2-rs / octocrab / tokio / rusqlite / clap / anyhow + thiserror；工具链用 mise 管理
- 质量门禁：`cargo check`、`cargo clippy -- -D warnings`、`cargo test` 全绿才能进入下一步

## 里程碑划分

预估为相对规模：S = 小，M = 中，L = 大。

### M1 本地核心闭环

- **核心功能**：项目骨架、配置管理（统一 `~/.gitpulse/` 单一路径来源）、仓库发现与状态查询（TUI 启动时真实加载）、状态仪表盘渲染真数据、单仓库视图（文件列表 + diff 查看 + stage/unstage + commit + push/pull）、批量 pull/fetch、分支面板（列表/创建/切换/删除）
- **端到端验收**：在 TUI 里对真实仓库完成一次 edit→stage→commit→push
- **子任务预估**：

  | # | 子任务 | 预估 |
  |---|--------|------|
  | 1 | 项目骨架：Cargo.toml + 目录结构 + 基础依赖 + .gitignore | S |
  | 2 | 配置管理：~/.gitpulse/config.toml 读写（路径全项目唯一来源） | S |
  | 3 | Git 操作层：仓库发现 + 状态查询（git2-rs），TUI 启动即调用 | M |
  | 4 | 状态仪表盘：渲染真实仓库数据 + 分组外的进入单仓库交互 | M |
  | 5 | 单仓库工作流 I：文件状态列表 + diff 查看 + stage/unstage | M |
  | 6 | 单仓库工作流 II：commit + push/pull + 进度反馈 | M |
  | 7 | 批量操作 + 分支面板：批量 pull/fetch；分支列表/创建/切换/删除 | M |

- **依赖**：无（新定位首个里程碑）
- **状态**：⬜ 未开始

### M2 单平台远端回执

- **核心功能**：PR 列表/详情、CI checks、Issues、通知；全部走真实 API，禁止 mock（GitHub 先行）
- **端到端验收**：TUI 中看到自己仓库的 PR/checks 并能刷新出最新状态
- **子任务预估**：

  | # | 子任务 | 预估 |
  |---|--------|------|
  | 1 | GitHub 认证与 octocrab client 初始化 | S |
  | 2 | PR 列表 / 详情面板 | M |
  | 3 | CI checks 面板 | M |
  | 4 | Issues 面板 | M |
  | 5 | 通知面板与手动刷新 | M |

- **依赖**：M1（远端回执层挂在单仓库视图上，复用其仓库状态、配置与 UI 骨架）
- **状态**：⬜ 未开始

### M3 多平台与收尾

- **核心功能**：Gitea/GitLab 适配层、token 按 remote URL 匹配、按需补 worktree/分组
- **端到端验收**：非 GitHub 仓库在同一界面看到它的 PR/CI
- **子任务预估**：

  | # | 子任务 | 预估 |
  |---|--------|------|
  | 1 | 平台适配层抽象（GitHub / Gitea / GitLab 统一接口） | M |
  | 2 | token 按 remote URL 自动匹配 | S |
  | 3 | worktree（按需） | 待评估 |
  | 4 | 分组（按需） | 待评估 |

- **依赖**：M2（在 GitHub 实现之上抽出适配层）、M1（配置与 token 路径基础）
- **状态**：⬜ 未开始
