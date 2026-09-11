# GitPulse

[English](#english) | [中文](#中文)

---

## English

Terminal dashboard for managing multiple Git repositories — see all your repos at a glance, batch operations in one click, cross-platform notifications.

### Features

#### Local Repository Management

| Feature | Description |
|---------|-------------|
| **Repo Discovery** | Auto-scan directories to find all Git repos, excluding `node_modules`/`target` etc. |
| **Status Dashboard** | All repos at a glance: branch, dirty files, last commit, ahead/behind status |
| **Batch Operations** | Pull / Fetch / Stash / Clean / Push across all repos with progress reporting |
| **Branch Management** | Smart branch marking (merged/stale/orphaned), batch cleanup of merged branches |
| **Worktree Management** | View all worktrees, create/delete worktrees |
| **Submodule Support** | Submodule status display, batch `update --init --recursive` |
| **Repo Groups** | Organize repos by project, batch operations per group |

#### Remote Platform Integration

| Feature | Description |
|---------|-------------|
| **Authentication** | Auto-detect tokens from `GITHUB_TOKEN` / `gh auth login` / config file |
| **Multi-Account** | Multiple GitHub/Gitea/GitLab accounts, per-repo account binding |
| **Commit User** | Per-repo `git user.name` / `user.email` management |
| **Notification Center** | Aggregated notifications: mentions, PR reviews, CI failures, releases |
| **Remote Repos** | Browse your GitHub repos, clone to local with one click |
| **Issues & PRs** | View Issues and PRs with status filtering and CI status |
| **CI/CD Monitoring** | View GitHub Actions workflow run status |
| **Trending** | Browse today's trending repos by language |
| **Stars** | Manage your starred repos with language/time filters |

#### Cross-Repo Search

| Feature | Description |
|---------|-------------|
| **Code Search** | Search across all local repos via `git grep` |
| **Commit Search** | Search commit messages by author/time/keyword |
| **File Search** | Search filenames across repos |

### Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `j` / `k` / `↑` / `↓` | Navigate |
| `Enter` | Repo details |
| `Space` | Select/deselect (batch) |
| `a` | Select all |
| `p` | Batch pull |
| `f` | Batch fetch |
| `s` | Batch stash |
| `r` | Refresh |
| `/` | Search |
| `?` | Help |
| `A` | Switch account |
| `g` | Switch group view |
| `q` | Quit / Back |
| `n` | Notifications |
| `b` | Branches |
| `R` | Remote repos |
| `I` | Issues |
| `P` | Pull requests |
| `C` | CI/CD |
| `W` | Worktrees |
| `T` | Trending |
| `S` | Stars |

### Tech Stack

| Component | Choice |
|-----------|--------|
| Language | Rust |
| TUI | ratatui |
| Git | git2-rs |
| GitHub API | octocrab |
| Async | tokio |
| Storage | SQLite (rusqlite) |

### Installation

```bash
# From source
git clone https://github.com/JeeLin/GitPulse.git
cd GitPulse
cargo build --release
cp target/release/gitpulse ~/.local/bin/
```

### Configuration

All data lives in `~/.gitpulse/`:

| File | Purpose |
|------|---------|
| `config.toml` | Scan paths, groups, accounts, branch rules, theme |
| `state.db` | Repo cache, notification state, stars |
| `gitpulse.log` | Runtime logs |

### License

MIT

---

## 中文

终端多仓库仪表盘 — 一眼看清所有仓库状态，一键批量操作，跨平台通知不遗漏。

### 功能特性

#### 本地仓库管理

| 功能 | 说明 |
|------|------|
| **仓库发现** | 自动扫描目录，找到所有 Git 仓库，排除 `node_modules`/`target` 等 |
| **状态仪表盘** | 所有仓库一览：分支、脏文件数、最近提交、远端同步状态 |
| **批量操作** | 一键 Pull / Fetch / Stash / Clean / Push，逐仓库报告进度 |
| **分支管理** | 智能标记分支状态（已合并/已过期/孤立），批量清理已合并分支 |
| **Worktree 管理** | 查看所有 worktree，创建/删除 worktree |
| **Submodule 支持** | Submodule 状态显示，批量 `update --init --recursive` |
| **仓库分组** | 按业务/项目分组管理，支持分组维度批量操作 |

#### 远端平台集成

| 功能 | 说明 |
|------|------|
| **认证** | 自动检测 `GITHUB_TOKEN` / `gh auth login` / 配置文件中的 token |
| **多账户管理** | 支持 GitHub/Gitea/GitLab 多账户，仓库绑定特定账户 |
| **提交用户维护** | 全局默认用户 + 按仓库覆盖 `git user.name` / `user.email` |
| **通知中心** | 聚合通知：Issue 提及、PR 审核、CI 失败、新版本发布 |
| **远端仓库浏览** | 浏览 GitHub 仓库列表，一键克隆到本地 |
| **Issues / PR 浏览** | 查看 Issue 和 PR 列表，支持状态过滤和 CI 状态显示 |
| **CI/CD 监控** | 查看 GitHub Actions 运行状态 |
| **Trending** | 浏览今日热门仓库，按编程语言过滤 |
| **Stars 管理** | 管理收藏列表，按语言/时间过滤 |

#### 跨仓库搜索

| 功能 | 说明 |
|------|------|
| **代码搜索** | 基于 `git grep` 在所有本地仓库中搜索代码 |
| **提交搜索** | 按作者/时间/关键词搜索提交信息 |
| **文件搜索** | 跨仓库搜索文件名 |

### 键盘快捷键

| 按键 | 功能 |
|------|------|
| `j` / `k` / `↑` / `↓` | 上下移动 |
| `Enter` | 进入仓库详情 |
| `Space` | 选中/取消（批量操作） |
| `a` | 全选 |
| `p` | 批量 Pull |
| `f` | 批量 Fetch |
| `s` | 批量 Stash |
| `r` | 刷新 |
| `/` | 搜索 |
| `?` | 帮助面板 |
| `A` | 切换账户 |
| `g` | 切换分组视图 |
| `q` | 退出 / 返回 |
| `n` | 通知中心 |
| `b` | 分支管理 |
| `R` | 远端仓库 |
| `I` | Issues |
| `P` | Pull Requests |
| `C` | CI/CD |
| `W` | Worktrees |
| `T` | Trending |
| `S` | Stars |

### 技术栈

| 组件 | 选型 |
|------|------|
| 语言 | Rust |
| TUI | ratatui |
| Git | git2-rs |
| GitHub API | octocrab |
| 异步 | tokio |
| 本地存储 | SQLite (rusqlite) |

### 安装

```bash
# 从源码构建
git clone https://github.com/JeeLin/GitPulse.git
cd GitPulse
cargo build --release
cp target/release/gitpulse ~/.local/bin/
```

### 配置

所有数据统一存放在 `~/.gitpulse/` 目录：

| 文件 | 用途 |
|------|------|
| `config.toml` | 配置文件（扫描路径、分组、账户、分支规则、主题等） |
| `state.db` | SQLite 数据库（仓库缓存、通知状态、Stars 列表） |
| `gitpulse.log` | 运行日志 |

### 许可证

MIT
