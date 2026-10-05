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

## 设计审查配置
人工复核：可选
