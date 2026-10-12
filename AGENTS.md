# GitPulse — 项目约定

## 技术栈
- 语言：Rust
- TUI：ratatui
- Git：git2-rs
- GitHub：octocrab（typed API，primary）+ gh CLI（auth token 解析，零配置；无 gh 时 GITHUB_TOKEN 兜底）
- 异步：tokio
- 依赖管理：mise

## 项目结构
```
GitPulse/
├── docs/
│   ├── PRODUCT.md          # 产品文档
│   ├── DEVELOPMENT.md      # 开发设计文档
│   └── DESIGN.md           # 交互设计规范（设计层唯一真源，见下）
├── .mdflow/
│   └── milestones/         # 里程碑文档与流程报告（本地存储，已 gitignore，不入库）
├── src/
│   ├── main.rs
│   ├── app.rs              # 应用入口
│   ├── ui/                 # TUI 界面
│   ├── git/                # Git 操作（git2-rs）
│   ├── github/             # GitHub API（octocrab）
│   ├── config/             # 配置管理
│   └── db/                 # SQLite 存储（预留，暂未启用）
├── Cargo.toml
├── mise.toml
└── .gitignore
```

## 设计约定（docs/DESIGN.md 为设计层唯一真源）

**改动任何 UI 之前必须先读 `docs/DESIGN.md`**，它定义交互系统的全部不变量。与 `PRODUCT.md`（做什么）、`DEVELOPMENT.md`（怎么实现）并列，三者冲突时以 DESIGN.md 的交互语义为准。

不得自行发明或偏离以下已固化的规范（详见 DESIGN.md 对应章节）：

| 主题 | 规范位置 | 要点 |
|------|---------|------|
| 四条全局动词 | §0 | `Enter` 打开 / `Esc` 返回 / `q` 退出 / `Ctrl-C` 强制退出。`Esc` 永不退出应用 |
| 导航三维度 | §1.1 | 栈（纵向，`Enter`/`Esc`）· 上下文标签（横向，`<`/`>`，**不入栈**）· 面板焦点（`Tab`/`BackTab`，**不入栈**）。三者语义不重叠 |
| 提示行 | §2.3 | **全应用唯一一行**，固定最底行、永不换行。顺序固定：全局段（`⏎ 打开  Esc 返回  r 刷新  q 退出`，**永不截断**）→ 视图段 → 末段（`Tab 面板` / `< > 切换`）。键名 `key` 样式 + 动作 `fg_muted`，**段间两个空格** |
| 页脚 | §4.1 | **恒为 2 行**：状态行（N-1）+ 提示行（N）。永不随内容增减。状态行是异步结果/进度/错误的**唯一**着陆点 |
| 焦点 | §3.1 | 同一时刻只有一个焦点面板；焦点边框 `border_focus`(accent)、非焦点 `border`(muted)；光标统一「行首 `▌` + `surface_alt` 底色」 |
| 列表面板五态 | §3.2 | 每个列表面板都要实现 `loading`/`empty`/`error`/`ready`/`success-flash`；`error` 态必须给出 `r 重试`。不做特例 |
| 错误落点 | §4.3 | **加载失败 → 面板 error 态（持久）；操作结果 → 状态行（瞬时）**。本地 git 与 GitHub 回执一视同仁 |
| 视觉 token | §5 | 颜色一律用 token（`accent`/`fg_muted`/`success`/`danger`/`warning`/`info`/`surface_alt`…），**不写死色值** |
| 宽度预算 | §5.4 | 按显示单元格宽度累加（CJK=2，emoji=2）。单行永不换行；溢出末尾加 `…`；表格低优先列**先消失**而非缩小；80 列下仅当列表 ≥28 且详情 ≥32 才允许左右分栏 |
| 视图布局 | §6 | 各视图的面板构成、列定义、键位表、提示行视图段、五态文案逐节给出（§6.1 仪表盘 · §6.2 仓库工作区 · §6.3 远端中心 · §6.4–6.10 各子屏 · §6.11 浮层） |

**布局硬约束**：仓库列表固定在仪表盘左侧且为焦点面板；详情面板在窄屏隐藏或改为内联预览，不得挤压列表。

审查与验收时，「设计一致性」是与功能完整性同级的必查维度。

## 代码规范
- 遵循 Rust 官方风格（rustfmt）
- 公共 API 必须有文档注释
- 错误处理使用 anyhow + thiserror
- 异步代码使用 tokio

## 测试命令
- 编译检查：`cargo check`
- Lint：`cargo clippy --all-targets -- -D warnings`
- 测试：`cargo test`（**本 crate 只有一个 bin target，没有 lib target** —— `cargo test --lib <filter>` 会报 `no library targets found in package 'gitpulse'`。按模块过滤请用 `cargo test <filter>` 或 `cargo test --bin gitpulse <filter>`）
- 覆盖率：`cargo llvm-cov`

## 质量门禁
| 检查项 | 命令 |
|--------|------|
| 编译检查 | `cargo check` |
| Lint 检查 | `cargo clippy -- -D warnings` |
| 测试 | `cargo test` |
| 覆盖率 | `cargo llvm-cov`（目标 90%） |

## 审查维度
- 功能完整性
- 错误处理
- 性能
- 安全性

## 代码审查维度

代码审查（mdflow 步骤5）以 `## 审查维度` 为通用轮廓外，本项目还必须逐项核对以下维度。每项给出**可执行的验证动作**；命中即当轮修复。「设计一致性」与功能完整性同级，是审查与验收的必查维度。

> 提示：提示行 / 页脚 / 焦点 / 五态 / 错误落点 / 视觉 token / 宽度预算 诸约束的权威定义见 `docs/DESIGN.md` §0–§5，审查时以 DESIGN.md 语义为准。

| 维度 | 检查项 | 验证动作 |
|------|--------|----------|
| 设计一致性：提示行 + 死键 | 提示行全应用唯一一行、固定最底、永不换行；全局段永不截断；键名 `key` 样式 + 动作 `fg_muted`、段间两空格。广告的键在对应屏须有可观测派发（广告了却无处理器 = 死键，§2.3） | `grep -rn "fn .*_hints" src/ui/` → 每 `_hints()` 的 `HintRow` 段序与 §2.3 一致；新增键 `hint("<键>"` 必在 `dispatch_key`/`handle_*_key` 找到该屏的派发分支；`z`/`P` 系已踩坑，复查大小写 |
| 设计一致性：页脚 + 错误落点 | 页脚恒 2 行（状态行 + 提示行）；加载失败→面板 error 态（持久）；操作结果→状态行（瞬时）；本地 git 与 GitHub 回执一视同仁 | `set_load_error`（面板）与 `set_error`（状态行）用法不得混用；面板 error 态永给 `r 重试` |
| 设计一致性：焦点 + 光标 | 单一焦点面板；焦点边框 `border_focus`、非焦点 `border`；光标统一「行首 `▌` + `surface_alt` 底色」 | 复用 §3.1 三件套（`with_cursor`/`list_highlight`/`HIGHLIGHT_SYMBOL`），不得各面板自造光标形态 |
| 设计一致性：五态 | 每个列表面板都有 `loading/empty/error/ready/success-flash` | 逐面板核对 §3.2 |
| 设计一致性：视觉 token | 颜色一律用 §5 token，**不写死色值** | `grep -rnE "Color::|color::[a-z]+\(" src/ui/` → 不得出现裸 RGB/HSL/索引色 |
| 设计一致性：宽度预算 | 按显示单元格宽度累加（CJK=2）；单行不换行、溢出加 `…`、低优先列先消失而非缩小 | 字宽论断须用项目口径 `cells::width()` 实测 —— 不得凭 Unicode 类别推断（§5.4） |
| 注释与文档准确性 | 注释中的符号引用/行号引用/行为描述须仍成立（rustdoc 断链要 `cargo doc` 才暴露） | `grep -nE '///.*\[[A-Za-z_:]+::[a-z_]+\]' src/` 逐个验证目标存在；行号引用以施工时实读为准 |
| 注释位置 | 新增代码不得把 `///` 块插进相邻注释块 | `grep -nB1 '^\s*///' src/` → 每 `///` 块紧邻的下一个非注释行是它所描述的 `fn`/`struct`/字段 |
| 结构体新增必填字段 | 加必填字段前逐处穷举构造点，标注哪些在 `#[cfg(test)]` 内 | `grep -rn "<Struct> {" src/ \| grep -v "fn .* -> <Struct>"` → 文档所列数与实测一致；无 `..Default::default()` 的逐处补位 |
| 派生视图的索引稳定性 | 引入排序/分组后裸 `usize` 索引会静默错指 | 所有裸索引按稳定标识（`path`）重映射，且覆盖**全部**读点 —— 仅 `enter_repo` 不够，还含 `enter_branches`/clamp/`palette_targets` |
| 未决状态不泄漏/改动不脱钩 | 核对点未裁决前，设计正文只写「见核对点 N」；已裁决的落地要求须回灌子任务「文档改动」段 | 核对点未 ✅ 不可出现具体方案；收口时逐项反查落地名词在子任务「文档改动」出现 |
| 验收断言有效性 | 断言须可证伪；`grep -v` 不得自我抵消（路径含子串会把命中全滤空） | `wc -l` 断言先记录当前 **非 0** 实测值；`grep -v` 按完整 token 且排除注释/`#[cfg(test)]` 行；删除实现应变红 |
| 设计一致性：弹窗标题宽度 | 弹窗标题宽度 ≤ 80 cell（§5.4） | 弹窗标题快照断言 `cells::width(title) <= 80`，不靠人工目测 |
| 注释与文档准确性：列/变体计数 | 枚举/列集合注释的列数须等于数组 const 长度声明 | 注释所称列数 == 对应 `const` 的长度（如 `REPO_COLUMNS`），成员增删须同步注释 |
| 死代码：新增 `pub(crate) fn` 的生产调用方 | 新增 `pub(crate) fn` 必须有生产调用方 | `grep -rn "<fn>" src/ \| grep -v "fn <fn>"` → 除 `#[cfg(test)]` 外至少 1 处生产调用 |
| 验收断言有效性：宿主环境条件分支 | 影响断言真假的宿主环境条件分支须改为无条件断言 | `grep -rn "\.is_some()\|config.get_string" src/` → 不得用宿主环境包裹决定断言真假的条件 |
| 验收断言有效性：承诺不改的文件 | 「承诺不改的文件」用 diff 空集验证 | 对里程碑「不改」清单逐项 `git diff <start-ref> -- <file> \| wc -l` → 期望 0 行 |
| 注释与文档准确性：新增内链 | 新增 rustdoc 内链须逐条验证目标存在，总数不高于基线 | 每个新增链接 `grep` 目标须命中；`cargo doc --no-deps 2>&1 \| grep -c "unresolved link"` 不高于基线 |

## 设计审查配置
人工复核：可选


