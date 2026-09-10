# GitPulse — 产品文档

## 一句话定位

终端里的多仓库仪表盘 — 一眼看清所有仓库状态，一键批量操作，跨平台通知不遗漏。

## 解决什么问题

开发者日常面对的 Git 痛点：

1. **仓库太多管不过来** — 手下有 10-20 个仓库，每天要逐个 `git status`、`git pull`，效率低
2. **远端信息分散** — 通知、Issue、CI 状态要打开浏览器逐个仓库查看
3. **批量操作麻烦** — 想 pull 所有仓库，只能写 shell 脚本循环
4. **分支腐烂** — 过期分支堆积，没人清理，不知道哪些该删
5. **没有全局视角** — 看不到"我的所有仓库今天发生了什么"

## 目标用户

- 管理多个仓库的全栈/独立开发者
- 需要跨仓库批量操作的技术 leader
- 远程服务器上管理 Git 的运维/DevOps

## 竞品对比

| 竞品 | 做什么 | 不做什么 | GitPulse 的差异 |
|------|--------|---------|----------------|
| **lazygit** | 单仓库 Git 操作（add/commit/rebase） | 不管其他仓库，不管远端平台 | GitPulse 管多个仓库 + GitHub / Gitea / GitLab |
| **gitui** | 同上，Rust 版 | 同上 | 同上 |
| **gh CLI** | GitHub 命令行工具 | 非交互式，每次敲命令 | GitPulse 是交互式仪表盘 |
| **tig** | 单仓库历史浏览 | 只看 log/diff | GitPulse 是全局管理 |
| **gwm-cli** | Git Worktree 管理 | 只做 worktree | GitPulse 覆盖更多 |

## 核心功能

### 一、本地仓库管理

#### 1.1 仓库发现

用户告诉 GitPulse 扫描哪些目录，它自动找到所有 git 仓库。

- 配置扫描路径（如 `~/projects`、`~/work`）
- 自动排除 `node_modules`、`.target`、`target` 等
- 缓存仓库列表，避免重复扫描

#### 1.2 状态仪表盘

打开 GitPulse 第一眼看到的界面：所有仓库的状态一览。

每个仓库显示：
- 仓库名称
- 当前分支
- 脏文件数（modified / staged / untracked）
- 最后一次提交的信息和时间
- 与远程的同步状态（ahead / behind 几个提交）
- CI 状态（如果接了远端平台，v0.1.1+）

#### 1.3 批量操作

| 操作 | 说明 |
|------|------|
| 批量 pull | 一键拉取所有仓库最新代码 |
| 批量 fetch | 一键 fetch 所有仓库 |
| 批量 clean | 清理所有仓库的未跟踪文件 |
| 批量 push | 推送所有本地提交到远程 |
| 批量 stash | 暂存所有仓库的修改 |

操作时显示每个仓库的进度，失败的单独报告。

#### 1.4 分支管理（v0.2）

- 查看所有仓库的分支列表
- 智能标记需关注的分支（排除 main / master / develop / release/* / v* / stable 等长生命周期分支）：
  - 🔴 已合并但未删除 — 直接清理
  - 🟡 无关联 PR 且长期无活动 — 可能废弃
  - 🟡 落后主干 N 个提交 — 需要 rebase / merge
  - 🟢 有活跃 PR — 正常，不动
- 支持自定义排除规则（配置文件 `[branches]` 段）
- 一键删除已合并的分支
- 按仓库分组展示

#### 1.5 Worktree 管理（v0.3）

- 查看当前所有 worktree 及其状态（分支、脏文件、占用进程）
- 创建新 worktree
- 删除 worktree（自动检测进程占用，占用时提示确认）
- 切换当前聚焦的 worktree（仅切换视图焦点，不影响其他进程）



#### 1.6 Submodule 支持

- 仪表盘显示仓库的 submodule 列表及状态（当前 commit、是否有未提交变更）
- 批量 submodule update --init --recursive
- 自动检测 submodule 未初始化 / commit 落后于父仓库的情况
- 子仓库也纳入 GitPulse 状态监控（可选）

#### 1.7 仓库分组

按业务/项目对仓库分组，方便管理和批量操作。

- 在配置文件中定义分组（`[groups]` 段），每组指定名称 + 仓库路径列表
- 支持「未分组」兜底，新发现的仓库自动归入
- 仪表盘按分组折叠展示，可展开/收起
- 批量操作可选择「全部」或「仅某分组」
- 支持 `g` 键切换当前分组视图
- 分组信息持久化到 `config.toml`

### 二、远端平台集成

#### 2.1 认证

按优先级自动检测：

1. `GITHUB_TOKEN` 环境变量（CI / 服务器场景）
2. `gh auth login` 已有的 token（读取 `~/.config/gh/hosts.yml` 的 oauth_token）
3. 配置文件手动填写（`~/.gitpulse/config.toml` 中 `[github]` 段）

补充说明：
- token 过期或权限不足时，在状态仪表盘顶部显示警告条，不阻断本地操作
- 多 GitHub 实例（github.com + GitHub Enterprise）v0.2 支持，配置文件 `[github.instances]` 段
- v0.3+ 支持 Gitea / GitLab / Gitee 等平台（Gitea API 与 GitHub v3 兼容，改动量小）

#### 2.2 远端账户管理（v0.2）

管理多个 Git 远端账户（GitHub / Gitea / GitLab），解决个人 + 工作多账号场景。

- 配置文件 `[accounts]` 段定义多个账户（名称、平台、token、默认用户名/邮箱）
- 账户切换：快捷键 `A` 弹出账户列表，选中后切换当前活跃账户
- 仓库绑定：每个仓库可绑定特定账户（`[repo_bindings]` 段），拉取/推送/浏览时自动使用对应账户
- 未绑定的仓库使用默认账户
- 当前活跃账户在状态栏显示

#### 2.3 Git 提交用户维护

管理每仓库的 `git user.name` / `user.email`，支持：

- 全局默认用户（配置文件 `[user]` 段）
- 按仓库覆盖（仓库绑定的账户自带用户名/邮箱，自动设置）
- 批量设置：选中多个仓库统一修改提交用户
- 当前提交用户在仓库详情中显示

#### 2.4 通知中心（v0.1.1 起）

聚合你所有远端仓库的未读通知：

- Issue 提及（有人 @ 你）
- PR 审核请求
- PR 被合并
- CI 运行失败
- 新版本发布

支持按仓库过滤、标记已读、批量操作。

**推送方式（分阶段）：**

- v0.1.1：CLI 内通知中心 + 桌面通知（notify-rust，轻量弹窗）
- v0.2+：后台守护模式 + Bot 推送（飞书 / 钉钉 / Telegram / 自定义 Webhook）
- 未来方向：AI Agent 智能摘要推送（结合 LLM 分析，只推高优先级通知，避免刷屏）

#### 2.5 远端仓库浏览（v0.2）

浏览你在 GitHub / Gitea / GitLab 上的仓库列表：

- 按组织/个人分组展示
- 显示仓库基本信息（语言、Star 数、最后更新时间、是否 Fork）
- 一键克隆到本地指定目录
- 克隆后自动纳入 GitPulse 管理
- 支持搜索远端仓库

#### 2.6 Issues / PR 浏览（v0.2）

- 查看指定仓库的 Issue 列表
- 查看 PR 列表和 CI 状态
- 按状态/标签/指派人过滤
- 快速创建 Issue

#### 2.7 CI/CD 监控（v0.2）

- 查看 CI/CD 运行状态（GitHub Actions 等）
- 失败的 pipeline 显示详情
- 一键重试失败的运行

#### 2.8 Trending（v0.3）

- 浏览今日/本周热门仓库
- 按编程语言过滤
- 收藏感兴趣的仓库

#### 2.9 Stars 管理（v0.3）

- 查看我的 Stars / 收藏列表
- 按语言/时间过滤
- 搜索 Stars

### 三、跨仓库搜索（v0.2）

- 在所有本地仓库中搜索代码（基于 git grep / ripgrep）
- 搜索提交信息（按作者/时间/关键词）
- 搜索文件名

## 数据与配置

所有数据统一存放在 `~/.gitpulse/` 目录：

| 文件 | 用途 |
|------|------|
| `config.toml` | 配置文件（扫描路径、分组、账户、提交用户、分支规则、主题等） |
| `state.db` | SQLite 数据库（仓库缓存、通知状态、Stars 列表） |
| `gitpulse.log` | 运行日志 |

备份/迁移/清理只需操作这一个目录。

## 性能约束

- 支持 50+ 仓库无感操作
- 远端 API 走 octocrab 异步调用 + 本地 SQLite 缓存，减少 API 调用
- 仓库扫描支持增量更新，避免全量重扫
- 批量操作并发执行，逐仓库报告进度

## 不做什么

- ❌ 不替代 lazygit 的本地 Git 操作（add/commit/rebase 等还是用 lazygit）
- ❌ 不做代码编辑
- ❌ 不做代码审查（那是平台的事）
- ❌ 不做 CI/CD 配置（只监控状态）

## UI 风格

- 深色主题为主
- 左右分栏布局
- 状态用颜色区分：🟢 正常 / 🟡 注意 / 🔴 问题

### 键盘操作

| 按键 | 功能 |
|------|------|
| `j` / `k` 或 `↑` / `↓` | 上下移动光标 |
| `Enter` | 进入仓库详情 |
| `Space` | 选中/取消（批量操作） |
| `a` | 全选 |
| `p` | 批量 pull |
| `f` | 批量 fetch |
| `s` | 批量 stash |
| `r` | 刷新 |
| `/` | 搜索 |
| `?` | 帮助面板 |
| `A` | 切换远端账户 |
| `g` | 切换分组视图 |
| `q` | 退出 / 返回上级 |

支持鼠标点击操作。

## 技术选型

| 组件 | 选型 |
|------|------|
| 语言 | Rust |
| TUI | ratatui |
| Git | git2-rs |
| GitHub API | octocrab |
| 异步 | tokio |
| 本地存储 | SQLite（rusqlite） |

## 版本规划

### v0.1 — 能用

- 仓库发现 + 状态仪表盘
- 批量 pull / fetch / stash / clean
- GitHub 认证（复用 gh CLI）
- 配置持久化（`~/.gitpulse/`）
- Submodule 状态显示 + 批量 update

### v0.1.1 — 通知

- 通知中心（只读聚合，GitHub / Gitea / GitLab）

### v0.2 — 好用

- 远端账户管理 + 仓库绑定
- Git 提交用户维护
- 远端仓库浏览 + 一键克隆
- Issues / PR 浏览
- CI/CD 监控
- 跨仓库搜索
- Gitea / GitLab 多平台支持

### v0.3 — 完整

- 分支管理 + 清理
- Worktree 管理
- Trending + Stars

## 未来方向

- Web 版（通过 WASM 编译，团队共享仓库状态大屏）
- AI Agent Git 工具层：
  - Skill 模式：封装为可复用 skill，DSH 环境内 Agent 直接调用
  - API 模式：`gitpulse api` 启动 HTTP API，供外部 AI Agent（Cursor / Windsurf / 自建）调用
  - 能力：仓库状态查询、批量操作、跨仓库搜索、通知摘要
- 插件机制（自定义数据源、自定义操作）
