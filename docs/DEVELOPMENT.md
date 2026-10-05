# GitPulse — 开发设计文档

## 新起点说明

next 分支为孤儿分支重写，旧版 v0.1-v0.7 实现归档于 master，因功能未真正接线整体废弃。

## 技术栈与质量门禁

- 技术栈：Rust / ratatui / git2-rs / octocrab / tokio / rusqlite / clap / anyhow + thiserror；工具链用 mise 管理
- 质量门禁：`cargo check`、`cargo clippy -- -D warnings`、`cargo test` 全绿才能进入下一步

## 里程碑划分

预估为相对规模：S = 小，M = 中，L = 大。

### M1 本地核心闭环

- **核心功能**：项目骨架、配置管理（统一 `~/.gitpulse/` 单一路径来源）、仓库发现与状态查询（TUI 启动时真实加载）、状态仪表盘渲染真数据、单仓库视图（文件列表 + diff 查看 + stage/unstage + discard 确认框 + commit 单行输入 + push/pull 快进）、批量 pull/fetch、分支面板（列表/创建/切换/删除）
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
- **状态**：✅ 完成

### M1.1 配置管理面板

- **核心功能**：空配置引导 + 配置管理面板——仪表盘空态提示（引导用户添加扫描目录）、程序内查看/添加/删除 `scan_dirs` 并持久化到 `~/.gitpulse/config.toml`，免去手编配置文件
- **端到端验收**：首启动 0 仓库时按引导在 TUI 内添加扫描目录，立即刷出仓库列表；重启后配置仍在
- **子任务预估**：

  | # | 子任务 | 预估 |
  |---|--------|------|
  | 1 | 仪表盘空态引导：仓库列表为空时显示提示与入口 | S |
  | 2 | 配置视图骨架：查看 scan_dirs 列表 + 键位/状态机接入 | M |
  | 3 | 添加目录：输入校验（存在性/去重）+ 保存 + 重扫 | M |
  | 4 | 删除目录：确认框 + 保存 + 重扫 | S |

- **依赖**：M1（复用其 View 状态机、配置读写与 UI 骨架）
- **状态**：✅ 完成
- **版本**：0.2.0

### M1.2 本地历史与标签

- **核心功能**：提交历史视图（提交列表 + 分支/tag/HEAD 标记 + DAG 图示展示分支合并拓扑 + j/k 导航查看提交详情）与 tag 管理面板（列表/创建/删除确认/推送到远端）
- **端到端验收**：在 TUI 中查看某仓库的提交 DAG 图并导航到指定提交；创建一个 tag 推送到远端后在 GitHub 页面可见
- **子任务预估**：

  | # | 子任务 | 预估 |
  |---|--------|------|
  | 1 | 提交历史视图：提交列表 + 分支/tag/HEAD 标记 + 提交详情 | M |
  | 2 | DAG 图示：分支/合并拓扑连线渲染 | M |
  | 3 | tag 管理面板：列表/创建/删除（确认框） | M |
  | 4 | tag 推送：单 tag / 全量推送 + 进度反馈 | S |

- **依赖**：M1（git2-rs 操作层与 View 状态机）、M1.1（面板与模态交互模式）
- **状态**：✅ 完成
- **版本**：0.3.0

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
- **状态**：✅ 完成

### M3 交互改版落地（Adopt Interaction Design）

- **前提**：完成从头的交互设计规范 `docs/DESIGN.md`（统一 q/Esc/Enter 文法、五态面板、2 行页脚、视觉 Token、≤80 列 CJK 裁剪）。
- **核心**：把 v0.4.0 现有实现逐视图对齐规范：`Esc`=返回 / `q`=退出 / `Enter`=打开 / `<>`切上下文标签 / `Tab`切面板；五态（loading/empty/error/ready/success）强制于每一列表面板；页脚 2 行（状态行+提示行）；视觉 Token + CJK 2 格 + 省略号裁剪；覆盖 dashboard/repo/diff/history/branches/tags/config/Hub 及 PR/checks/issues/通知详情；吸收 des-10 暂持的 🔒 文案（g 提示 / 空态 / b/h/t 标签 / 详情占位 / diff 文件名）并更新对应测试。
- **端到端验收**：任意视图间 Esc/q/Enter/<>/Tab 语义无歧义；空/加载/错误态一致；80 列下无 CJK 错位/截断；提示行统一含 `⏎ 打开  Esc 返回  q 退出  …  < > 切换  Tab 面板`。
- **依赖**：M2（v0.4.0 现有实现为改造基线）、`docs/DESIGN.md`
- **状态**：✅ 完成
- **版本**：0.5.0

### M4 多平台与收尾

- **核心功能**：Gitea/GitLab 适配层、token 按 remote URL 匹配、项目级切换用户（user.name/email 按仓库查看/切换）、按需补 worktree/分组
- **端到端验收**：非 GitHub 仓库在同一界面看到它的 PR/CI
- **子任务预估**：

  | # | 子任务 | 预估 |
  |---|--------|------|
  | 1 | 平台适配层抽象（GitHub / Gitea / GitLab 统一接口） | M |
  | 2 | token 按 remote URL 自动匹配 | S |
  | 3 | 项目级切换用户：按仓库查看/切换提交身份（user.name/email） | M |
  | 4 | worktree（按需） | 待评估 |
  | 5 | 分组（按需） | 待评估 |

- **依赖**：M2（在 GitHub 实现之上抽出适配层）、M1（配置与 token 路径基础）
- **状态**：🟨 进行中
- **进展**（v0.6.0，详见 `.mdflow/milestones/v0.6.0-多平台适配.md` 子任务 1–9）：
  - ✅ 子任务 1 平台适配层抽象：`src/platform/` 统一 `PlatformClient` 接口（GitHub / Gitea / GitLab），回执视图（PR / CI / Issue / 通知）按 `remote` 渲染平台标签与凭据提示，不再硬编码 GitHub。
  - ✅ 子任务 2 token 按 remote URL 匹配：`src/platform/auth.rs` 依据 host 解析 `GITLAB_TOKEN` / `GITEA_TOKEN`（GitHub 在该模块被显式拒绝，由 `src/github/auth.rs` 解析 `GITHUB_TOKEN` 并有 `gh` 登录兜底，GitLab 有 `glab` 登录兜底）。
  - ✅ 自托管路由与 API 契约缺陷修复（审查项）：`build_client` 拒绝未适配的自托管 host 并返回可读原因，回执面板按平台渲染断连提示。
- **剩余项**：
  - 🟨 自托管实例（`SelfHosted` host）暂不支持——尚无已知 API family，需先补齐其契约与路由。
  - ⬜ 子任务 3 项目级切换用户（按仓库查看/切换提交身份）。
  - ⬜ 子任务 4 worktree、子任务 5 分组（均待评估）。
  - ⬜ UI 文案 i18n：文案层 key 化 + 语言参数预留（先只提供中文），见 `.mdflow/milestones/v0.6.0-多平台适配.md` 沉淀台账。
