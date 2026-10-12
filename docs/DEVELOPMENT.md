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
- **状态**：✅ 已完成

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
- **状态**：✅ 已完成
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
- **状态**：✅ 已完成
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
- **状态**：✅ 已完成

### M3 交互改版落地（Adopt Interaction Design）

- **前提**：完成从头的交互设计规范 `docs/DESIGN.md`（统一 q/Esc/Enter 文法、五态面板、2 行页脚、视觉 Token、≤80 列 CJK 裁剪）。
- **核心**：把 v0.4.0 现有实现逐视图对齐规范：`Esc`=返回 / `q`=退出 / `Enter`=打开 / `<>`切上下文标签 / `Tab`切面板；五态（loading/empty/error/ready/success）强制于每一列表面板；页脚 2 行（状态行+提示行）；视觉 Token + CJK 2 格 + 省略号裁剪；覆盖 dashboard/repo/diff/history/branches/tags/config/Hub 及 PR/checks/issues/通知详情；吸收 des-10 暂持的 🔒 文案（g 提示 / 空态 / b/h/t 标签 / 详情占位 / diff 文件名）并更新对应测试。
- **端到端验收**：任意视图间 Esc/q/Enter/<>/Tab 语义无歧义；空/加载/错误态一致；80 列下无 CJK 错位/截断；提示行统一含 `⏎ 打开  Esc 返回  q 退出  …  < > 切换  Tab 面板`。
- **依赖**：M2（v0.4.0 现有实现为改造基线）、`docs/DESIGN.md`
- **状态**：✅ 已完成
- **版本**：0.5.0

### M4 多平台与收尾

- **核心功能**：Gitea/GitLab 适配层、token 按 remote URL 匹配、项目级切换用户（user.name/email 按仓库查看/切换）、按需补 worktree/分组
- **端到端验收**：非 GitHub 仓库在同一界面看到它的 PR/CI
- **子任务预估**：

  | # | 子任务 | 预估 |
  |---|--------|------|
  | 1 | 平台适配层抽象（GitHub / Gitea / GitLab 统一接口） | M |
  | 2 | token 按 remote URL 自动匹配 | S |
  | 3 | ~~项目级切换用户~~（按仓库查看/切换提交身份）→ **已裁决移出 M4**，见下方「剩余项」 | — |
  | 4 | worktree（按需） | 待评估 |
  | 5 | 分组（按需） | 待评估 |

- **依赖**：M2（在 GitHub 实现之上抽出适配层）、M1（配置与 token 路径基础）
- **状态**：✅ 已完成（收尾提交 `dde6b6e`，详见 `.mdflow/milestones/v0.6.0-多平台适配.md`；下列「剩余项」已各自改判去向）
- **进展**（v0.6.0，详见 `.mdflow/milestones/v0.6.0-多平台适配.md` 子任务 1–9）：
  - ✅ 子任务 1 平台适配层抽象：`src/platform/` 统一 `PlatformClient` 接口（GitHub / Gitea / GitLab），回执视图（PR / CI / Issue / 通知）按 `remote` 渲染平台标签与凭据提示，不再硬编码 GitHub。
  - ✅ 子任务 2 token 按 remote URL 匹配：`src/platform/auth.rs` 依据 host 解析 `GITLAB_TOKEN` / `GITEA_TOKEN`（GitHub 在该模块被显式拒绝，由 `src/github/auth.rs` 解析 `GITHUB_TOKEN` 并有 `gh` 登录兜底，GitLab 有 `glab` 登录兜底）。
  - ✅ 自托管路由与 API 契约缺陷修复（审查项）：`build_client` 拒绝未适配的自托管 host 并返回可读原因，回执面板按平台渲染断连提示。
- **剩余项**（去向已裁决）：
  - ⬜ 自托管实例（`SelfHosted` host）暂不支持——尚无已知 API family，需先补齐其契约与路由；`PRODUCT.md` 未列入功能列表。
  - ⬜ 项目级切换用户（按仓库查看/切换提交身份）→ 仍在 `PRODUCT.md` 候选池（`[待规划]`），待后续里程碑选入。
  - ⬜ worktree、分组 → 已纳入 **M6 / v0.8.0**。
  - ⬜ UI 文案 i18n：文案层 key 化 + 语言参数预留（先只提供中文），见 `.mdflow/milestones/v0.6.0-多平台适配.md` 沉淀台账。

### M5 交互规范收敛（对齐 DESIGN.md） ✅ 已完成

- **核心功能**：把全应用逐视图**严格对齐** `docs/DESIGN.md` 的交互不变量——提示行（§2.3 唯一行 + 全局段永不截断 + key/动作分段两空格）、页脚恒 2 行与状态行唯一落点（§4.1/§4.2）、错误落点（§4.3）、焦点与五态（§3）、视觉 Token 与宽度预算（§5）、各视图布局（§6，仓库列表固定左侧且为焦点面板）
- **背景**：`docs/DESIGN.md` 在 v0.6.0 收尾时才被写入 `AGENTS.md`（前 6 轮开发与审查从未要求对齐它），用户复核发现三处确定性偏离——① 提示行 `j/k 选择g 进入s/u 暂存切换…` 段间无分隔且键名/动作粘连；② 同一异步结果在**面板体**（`render_disconnected` 提升 `app.status.message`）与**全局页脚**（`ui/remote.rs` 渲染 `RemoteStatus`）各渲染一次，出现「两个正在 pull」；③ 仓库列表未按 §6.1 固定于仪表盘左侧
- **端到端验收**：提示行全应用唯一一行、固定最底行、`⏎ 打开  Esc 返回  r 刷新  q 退出` 恒定在前且永不截断；任意视图**仅一处**状态行渲染同一异步结果；焦点面板 `border_focus`、光标 `▌` + `surface_alt`；列表面板五态齐备且 error 态给 `r 重试`；仓库列表恒在仪表盘左侧且为焦点面板；80 列下 CJK 无错位、单行不换行、溢出以 `…` 结尾
- **子任务预估**：

  | # | 子任务 | 预估 |
  |---|--------|------|
  | 1 | 提示行 §2.3 收敛：全应用唯一一行 + 全局段优先永不截断 + `key`/`fg_muted` 分段两空格样式 | M |
  | 2 | 页脚 §4.1/§4.2 收敛：恒 2 行（状态行 + 提示行），状态行为异步结果唯一落点，消除面板体/页脚双重渲染 | M |
  | 3 | 错误落点 §4.3：加载失败 → 面板 error 态（持久）；操作结果 → 状态行（瞬时） | S |
  | 4 | 焦点与五态 §3：焦点 `border_focus`、光标 `▌`+`surface_alt`、每列表面板五态齐备 + error 态 `r 重试` | M |
  | 5 | 视图布局 §6：仓库列表固定仪表盘左侧且为焦点面板；详情面板窄屏隐藏或内联预览，不挤压列表 | M |
  | 6 | 视觉 Token 与宽度预算 §5：颜色一律 token 不写死、CJK 2 格、单行不换行、溢出 `…`、80 列分栏门槛 | M |
  | 7 | 设计一致性审查维度落地：审查 checklist + 关键不变量测试钉住 | S |

- **依赖**：M3（`docs/DESIGN.md` 规范与 v0.5.0 交互改版基线）、M4（v0.6.0 多平台代码基）
- **状态**：✅ 已完成（收尾提交 `b99c221`，544 tests / cov 95.38%；详见 `.mdflow/milestones/v0.7.0-交互规范收敛.md`）
- **版本类型**：minor
- **版本号**：0.7.0
- **边界说明**（均为用户已裁决）：
  - **解除** v0.6.0 的「GitHub 路径用户可见文案与 v0.5.0 逐字一致」硬约束——该约束仅作用于 v0.6.0；本里程碑按 DESIGN.md 调整 UI 布局/提示行/页脚必然产生新的可见文案，属预期产出。**后端行为**仍以 v0.6.0 为回归基线，测试不放松
  - **i18n 不纳入**：文案 key 化 + 语言参数另开独立里程碑（先只提供中文）
  - **分组展示不纳入**：推迟到分组功能落地时一并实现（分组功能本身仍 `[待规划]`；已核实 `App::repos` 为扁平列表、无分组字段）→ **已在 v0.8.0 落地**
  - **项目级切换用户、worktree 不纳入**：分别待文档口径裁决 / 待评估 → **worktree 已在 v0.8.0 落地**；项目级切换用户仍在候选池

### M6 worktree 与分组 ✅ 已完成

- **核心功能**：补齐本地仓库视图的三项能力——`worktree` 识别与呈现、`scan_dirs` 分组展示、以及按步骤2 裁决并入的 DESIGN §6.1 仓库行列集补齐（新增 `远端`/`⚠` 列、移除规范未列的 `Time` 列；该偏差为 v0.7.0 遗留，非本里程碑引入）；并清空 v0.7.0 递交的重构/测试债（规划期实测剔除 5 条后余 15 条）
- **背景**：M4 把 worktree / 分组列为「待评估」收尾项，M5 裁决推迟；二者均不触碰远端层，属纯本地层增量。现状核实（v0.8.0 起始 ref）：① `src/git/discover.rs:79-81` `is_git_repo` 已把 `.git` **文件**（worktree 标志）视为仓库，故平铺在扫描目录下的 worktree **已能被发现**；② 但 `discover_in_dir:53-55` 遇仓库边界即 `return`，**嵌套在父仓库目录树内的 worktree 扫不到**；③ `RepoInfo`（`src/git/status.rs:38-53`）无任何 worktree 元数据，worktree 与主仓库在 UI 上无法区分；④ `App::repos`（`src/app.rs:552`）为扁平 `Vec<RepoInfo>`、`Config`（`src/config/types.rs:10-19`）只有 `scan_dirs`，无分组字段
- **端到端验收**：扫描目录下嵌套的 worktree 出现在仪表盘并带 worktree 标记；在配置里给 `scan_dirs` 配分组后，仪表盘按组折叠展示且组内计数正确；两个既有 `config.toml` 无 `groups` 字段时加载不报错、行为与 v0.7.0 逐字一致；仪表盘仓库行与 `docs/DESIGN.md:262-264` 样例逐列一致（`远端` 列显平台名或 `远端: 无`、`⚠` 列显冲突数、无时间戳列），无远端的仓库照常渲染而不报错
- **子任务预估**：

  | # | 子任务 | 预估 |
  |---|--------|------|
  | 1 | 缺陷池重构债：B4-02..B4-13（12 条纯重构，含跨层依赖下沉、重复外壳收编、宽度算法归一） | M |
  | 2 | 缺陷池测试与文档债：B5-02 / B7-01 / B7-03（3 条，含按屏键→动作断言、§2.3 测试扩至 15 屏、`MIN_PATH_CELLS` 推导） | S |
  | 3 | worktree + 远端数据层：`RepoInfo` 增 worktree 元数据与 `remote` 字段 + 嵌套 worktree 发现不漏扫 | M |
  | 4 | worktree + 远端列呈现：补 §6.1 `远端`/`⚠` 列、移除 `Time` 列 + 标注与排序（§5 token，禁裸色） | M |
  | 5 | 分组配置：`Config` 增分组定义（路径前缀 → 组名），向后兼容既有 `scan_dirs` | M |
  | 6 | 分组呈现：仪表盘组头 + 折叠/展开（遵守 §6.1 布局、§5.4 宽度预算） | M |

- **依赖**：M1（配置与本地 git 层）、M5（v0.7.0 交互规范基线，子任务 1/2 是其欠债）
- **状态**：✅ 已完成（收尾提交 `ec250c6`+`6bb5446`+`02b1d88`，585 tests / 行覆盖 95.98%；详见 `.mdflow/milestones/v0.8.0-worktree与分组.md`，审查轮 2 的步骤4/5/6/7 报告在 `v0.8.0-reports/round2/`）
- **版本类型**：minor
- **版本号**：0.8.0（已交付，`Cargo.toml`/`Cargo.lock` 同步）
- **缺陷池 bug**：15 个（B4-02..B4-13 重构债 / B5-02 测试有效性 / B7-01·B7-03 键位断言与阈值）——已由 `mdflow-planner` 逐条写入 `.mdflow/milestones/v0.8.0-worktree与分组.md` 的 `## Bugs` 表并在规划时从缺陷池删行。另有 5 条在规划期经实测证伪或就地修复（B4-01 / B5-01 / B4-14 / B4-15 伪条目——round3 建池未实测 grep 锚点；B7-02 为 v0.7.0 的 `P` 死键回归，已在 `src/app.rs:3430` 修复并补回归测试），以 `[x]` 证据行留存。另有 1 条步骤1 新登记、步骤2 裁决后**并入本里程碑范围**的 🔴（DESIGN §6.1 仓库行列集与实现不符，`src/ui/dashboard.rs:223` 注释断言为假）——用户裁定「本里程碑补齐远端列」，数据层归子任务 3、呈现层 6 列列集归子任务 4，`src/platform/` 维持冻结
- **边界说明**（均为用户已裁决）：
  - **不做 worktree 新建/删除**：只识别与呈现既有 worktree，创建/删除需 libgit2 worktree 写入 API，另开里程碑
  - **不做项目级切换用户**：`PRODUCT.md` 仍 `[待规划]`，不在本里程碑范围
  - **不做 i18n**、**不做自托管实例**：同 M5/M4 裁决，仍在候选池

### M7 项目级切换用户 ✅ 已完成

- **核心功能**：在 GitPulse 仪表盘/单仓库视图按**仓库维度查看与切换提交身份**（user.name / user.email）；提交时应用该仓库绑定的身份；未显式配置时回退为该仓库/全局的默认身份，行为与 v0.8.0 逐字一致
- **背景**：M4 将其列为子任务 3，v0.6.0 收尾时 `~~项目级切换用户~~` 裁决**已裁决移出 M4** → `PRODUCT.md` 候选池 `[待规划]`。M6 收尾明确「**不做项目级切换用户**：PRODUCT.md 仍 `[待规划]`，不在本里程碑范围」。v0.8.0 的 `RepoInfo` 结构体扩展与 `scan_dirs` 分组收窄了 per-repo 数据面，这是切换身份的直接前提。
- **端到端验收**：在 TUI 中为两个不同仓库分别设定提交身份，分别 commit 后 `git log` 显示不同作者；从未设置本地身份的仓库沿用全局 user.*，提交作者与 v0.8.0 一致；仪表盘 repo 行能回显当前生效的提交身份
- **子任务预估**：

  | # | 子任务 | 预估 |
  |---|--------|------|
  | 1 | 数据层：`RepoInfo` 扩展 `author_name`/`author_email`（读 `.git/config` local，回退全局 `user.*`） | M |
  | 2 | 应用层：进入单仓库视图时绑定该 repo 身份到 `App`；`commit` 操作使用绑定身份而非默认 | M |
  | 3 | 切换入口：仪表盘/详情回显当前身份，提供切换（修改 `.git/config` local）+ 即时生效 | M |
  | 4 | 回归测试：切换后本地提交 author 真实生效；向后兼容无 user.* 配置的仓库 | S |
  | 5 | 文档：`docs/DESIGN.md` §6 回显位置/文案补位、提示行键位补位 | S |

- **依赖**：M1（配置与本地 git 层）、M6（v0.8.0 的 `RepoInfo` 扩展是数据面的直接前提）
- **版本类型**：minor
- **版本号**：0.9.0
- **缺陷池 bug**：0 个（`docs/BUGS.md` 缺陷池已清空）
- **交付结果**（v0.9.0，2026-10-12）：子任务 1-5 全部 ✅；仓库行新增低优先 `作者` 列、提交弹窗标题回显 `提交信息 · {名字} <{邮箱}>`、仓库视图 `A` 键打开单行输入并写入 `.git/config` local 后即时生效（端到端验证：切换后 commit 真实由新身份签名）。提交路径 `src/git/commit.rs` 与 `src/platform/`、`src/github/`、`src/config/` 均未改动，`Config` schema 不变，向后兼容未配置 `user.*` 的仓库。605 tests / 行覆盖 95.93% / clippy 零警告。审查轮 1 产出 4 条发现（🟡×3 + 🟢×1，均已修复并登记 Bugs 表）→ 统一打回步骤3 → 第2轮 4✓ 5✓ 6✓ 7✓ 全通过；步骤4/5/6/7 报告在 `.mdflow/milestones/v0.9.0-reports/round2/`。
- **后续**：表内已无无标记条目，下次里程碑由 `mdflow-planner` 从 `PRODUCT.md` 候选池规划。

### M8 文档链接与技术债清理 🔄 当前

- **核心功能**：清零全库 `cargo doc` rustdoc 断链（25 条存量，本里程碑**零新功能**），并把 `AGENTS.md`「代码审查维度」表中已裁决落地的可执行验证动作补齐，使同类问题此后每轮审查自动拦截。**纯 patch，不改任何运行时行为**——`src/` 的可执行代码 diff 应为 0 行（仅 doc comment 行变动）。
- **背景**：`PRODUCT.md` 25 项功能全部 `[x]`、`docs/BUGS.md` 缺陷池为空，规划器无输入；由维护者裁决「做质量/债务清理里程碑」而立项。债的两条来源：(1) v0.9.0 收尾台账第 13 条登记的**存量 25 条 rustdoc 断链**（基线 25 / 当前 25，本里程碑零新增，存量未清）；(2) 台账中 6 条落地目标为 `scripts/check.sh` 的候选——该路径在本项目**不存在**，改投 `AGENTS.md` 审查维度。
- **端到端验收**：`cargo doc --no-deps 2>&1 | grep -c "unresolved link"` 输出 **0**（当前 25）；`cargo test` 仍 605 全绿、覆盖率 ≥90%、clippy 零警告、fmt 干净；25 条内链每条逐条验证目标真实存在且可见性可达（非仅靠 `cargo doc` 绿灯）。
- **25 条断链的实测根因分类**（`cargo doc --no-deps` 实测，非估算）：

  | 根因 | 条数 | 证据 | 处置方向 |
  |------|------|------|---------|
  | **A. `#[cfg(test)]` 目标在非测试构建下不存在** | 2 | `set_config_path_override`（`src/config/path.rs:21` 带 `#[cfg(test)]`，被 `src/config/path.rs:39` 引用）、`compose_hint_row`（`src/ui/components/footer.rs:266` 私有 fn，被 `src/ui/repo.rs:149` 引用） | 改为纯文本（`` `fn 名` ``）或改指公开替代；不为此放宽可见性 |
  | **B. 路径已随重构漂移** | 4 | `crate::ui::remote::*`→实际在 `ui::components::remote`（`src/app.rs:2575`、`src/ui/tags.rs:62`）；`crate::ui::render_status_line`→实际在 `ui::components::footer`（`src/ui/components/state_panel.rs:8`）；`crate::App`/`crate::ui::components::footer::compose_hint_row`（`src/ui/components/status.rs:21`、`src/ui/repo.rs:149`） | 重写为当前真实路径 |
  | **C. 同名兄弟模块需限定** | 9 | `PlatformError`/`GithubError` 枚举变体 `Http`/`Network`/`Other` 在自身枚举定义内写 `[`Http`]`（`src/platform/error.rs:10`、`src/github/error.rs:9`）；`render_status_line`/`render_status_or_remote`/`render_footer` 在 `components/mod.rs` 内写裸名但实际定义于 `footer` 子模块 | 用 `Self::` 或全路径限定 |
  | **D. 目标需 `use` 引入才在作用域内** | 8 | `PlatformKind::as_str`/`SelfHosted`（`src/app.rs:313-314`，`use` 在函数体内 `src/app.rs:301`）、`PlatformRemote`（`src/app.rs:1671`）、`build_client`（`src/app.rs:1912`）、`App::dashboard_rows`/`toggle_selected_group`（`src/ui/dashboard.rs:29,603`）、`theme::width`（`src/ui/dashboard.rs:67`，实际为 `theme::width` 模块，`width` fn 在 `cells`）、`App::group_of`（`src/config/types.rs:98`） | 补全路径限定 |
  | **E. 目标名已不存在（陈旧注释）** | 2 | `theme::width` 的「`width` 函数」实为 `cells::width`（`src/ui/components/cells.rs:13`）；`RemoteStatus`（`src/ui/tags.rs:60`，定义在 `src/ui/components/status.rs:32`，tags 未 `use`） | 指向真实符号或改纯文本 |

  > 分类按**主因**归类，B/E 存在交叉；总数以 `cargo doc` 实测 25 为准，步骤 1 须逐条复核后再定稿。
- **子任务预估**：

  | # | 子任务 | 预估 |
  |---|--------|------|
  | 1 | 逐条复核 25 条断链的真实目标与可见性，按 A–E 根因定稿修复方案（禁止仅凭 `cargo doc` 绿灯判定修复完成） | S |
  | 2 | 修复 A/B/E 三类共 8 条：`#[cfg(test)]` 目标降级为纯文本、漂移路径重写、陈旧符号指向真实符号；**不改任何可见性** | M |
  | 3 | 修复 C/D 两类共 17 条：同模块内枚举变体与兄弟模块符号用 `Self::`／全路径限定，补全 `use` 作用域外目标的路径 | M |
  | 4 | 回归门禁：新增测试断言 `cargo doc --no-deps` 的 `unresolved link` 计数为 0（或以构建期断言/`#![deny(rustdoc::broken_intra_doc_links)]` 钉住），使断链清零成为可持续契约而非一次性清理 | M |
  | 5 | `AGENTS.md`「代码审查维度」补位：把台账中裁决落地为该表的 6 条可执行验证动作（弹窗标题宽度 ≤80 cell、枚举列注释计数、`pub(crate) fn` 生产调用方、宿主条件分支改无条件断言、「承诺不改的文件」diff 空集验证、新增内链总数不高于基线） | M |
  | 6 | 文档：记录「新增 rustdoc 内链须逐条验证目标存在且总数不高于基线」到 `AGENTS.md`；`CHANGELOG.md` 记 patch 条目；`PRODUCT.md`/`DESIGN.md` 无交互变更故不动 | S |

- **依赖**：M7 / v0.9.0（基线 ref `4076f15`；断链基线数 25 由该 ref 确立）
- **边界说明**（均不属本里程碑，越界即打回）：
  - **不改运行时行为**：可执行代码 diff 必须为 0 行，仅 doc comment 可动；`cargo test` 须保持 605 全绿
  - **不放宽可见性**：不得为「让 `cargo doc` 变绿」把 `#[cfg(test)]` 私有项或私有 fn 提升为 `pub`/`pub(crate)`
  - **不删断链以求绿**：把 `[`X`]` 改成纯文本须是「目标确不可链接」时才可，且须在子任务 1 复核中说明理由
  - **不做**新功能、**不做** v0.9.0 台账中 4 条 `❌` 候选（已被实测证伪，登记即关闭）
- **状态**：🔄 当前
- **版本类型**：patch
- **版本号**：0.9.1
- **缺陷池 bug**：0 个（本里程碑由维护者直接裁决立项，不经缺陷池；25 条断链为 v0.9.0 台账第 13 条登记的存量债，非 `docs/BUGS.md` 条目）
- **后续**：本里程碑为 patch 清理，不产出新功能；此后规划器仍需 `PRODUCT.md` 新增 `[待规划]` 功能或 `docs/BUGS.md` 新增缺陷方有输入。产品走向的进一步讨论见项目 issue 讨论任务。
