# Changelog

All notable changes to this project are documented in this file.
Format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.9.1] - 2026-10-13

### Changed
- 清零全库 25 条 rustdoc 断链（`cargo doc --no-deps` 的 `unresolved link` 25 → 0）：按 A–D 四类根因逐条复核后修复——`#[cfg(test)]` 目标与私有 fn 降级为纯文本、随重构漂移的模块路径重写为真实路径（`ui::remote` → `ui::components::remote`、`crate::App` → `crate::app::App`）、枚举变体补 `Self::` 限定、跨模块符号补全从 crate 根出发的全路径限定。未放宽任何可见性，`src/` 可执行代码 diff 为 0 行。
- `src/main.rs` 新增 `#![deny(rustdoc::broken_intra_doc_links)]`，断链清零从一次性清理变为构建期门禁；`AGENTS.md` 「质量门禁」与「测试命令」同步登记该命令及其人工触发要求（本项目无 CI、无 `scripts/`，门禁仅在实际执行 `cargo doc` 时生效）。

### Added
- `AGENTS.md` 「代码审查维度」补位 6 条可执行验证动作（弹窗标题宽度 ≤ 80 cell、枚举/列集合注释的列数须等于数组 `const` 长度、新增 `pub(crate) fn` 须有生产调用方、影响断言真假的宿主环境条件分支须改为无条件断言、「承诺不改的文件」用 diff 空集验证、新增 rustdoc 内链须逐条验证目标存在且总数不高于基线）。

## [0.9.0] - 2026-10-12

### Added
- **仓库级提交身份回显**：仓库行新增 `作者` 列（§6.1 第 7 列、低优先，`min_width` 12 格，窄屏按 §5.4 整列消失），显示该仓库**生效的 `user.name`**——取值来自 git2 的 local→global→system 解析，与提交路径 `src/git/commit.rs` 同一套 fallback，故列里看到的就是提交会用的身份；解析不出时显示 `无`（同 `远端: 无`，非错误态）。
- **提交弹窗标题回显作者**：标题形如 `提交信息 · {名字} <{邮箱}>`，解析不出时为 `提交信息 · (无本地配置)`（§6.2.1）。输入行仍是提交信息本身，作者只进标题。
- **`A` 键切换仓库身份**：仓库视图按 `A` 打开单行输入（`名字 <邮箱>`，Enter 提交 / Esc 取消），写入该仓库 `.git/config` 的 **local** `user.name`/`user.email` 后即时生效——仓库行与提交弹窗下帧即更新，之后的 commit 真实由新身份签名；成功回状态行 `✓ 已切换为 {名字}`，解析/写入失败只落状态行瞬时提示、不污染面板 `error` 态。

### Fixed
- 作者读取不再二次打开仓库：`read_author` 改为接收 `query_status` 已打开的 `&Repository`，扫描 N 个仓库由 2N 次 `Repository::open` 降为 N 次（兑现「零额外打开」的设计承诺）。

## [0.8.0] - 2026-10-12

### Added
- **worktree 识别**：扫描目录**嵌套在父仓库目录树内**的 linked worktree 现在会被发现（`discover_in_dir` 越过仓库边界继续下探），仪表盘以 `worktree` 标记区分于主仓库，详情面板显示所属主仓库路径；每个主仓库的 linked worktree 硬上限 32。
- **扫描目录分组**：`scan_groups` 配置项（`[{ name, prefixes }]`，最长前缀优先、按完整路径段匹配）把仪表盘仓库列表渲染成可折叠分组。组头形如 `▾ 组名 (N)` / `▸ 组名 (N)`，`z` 折叠/展开光标所在分组；折叠态**仅存在于内存**（重启全展开，不写回配置）。未配置分组时列表与 v0.7.0 逐字相同，未归入任何分组的仓库排在末尾且不带组头。
- 仓库行新增 `远端` 列（读取本地 `.git/config` 的 remote URL，**不发网络请求**，复用既有 `PlatformKind::from_host`），无远端时显示 `远端: 无`。

### Changed
- **[Breaking — interaction]** 仪表盘仓库行按 DESIGN §6.1 补齐列集：新增 `远端` 与 `⚠`（冲突数）两列，移除规范未列的 `Time` 列；`⚠` 从原冲突列中拆出独立成列。
- 分组配置项非法（如前缀不是完整路径段）时按条目丢弃并落到面板 `error` 态 + `r 重试`（DESIGN §4.3），不再静默作废整份配置。

### Fixed
- 仪表盘提示行在**未配置分组**时不再广告 `z 折叠分组`——该键此时无派发，属 DESIGN §2.3 死键（已由 `every_advertised_key_on_every_screen_dispatches_an_action` 扫描器捕获）。
- 命令面板在仪表盘上的仓库/分支目标改由**光标所指的行**解析，而非 `current_repo`（分组引入组头后光标索引不再是仓库索引）。

## [0.5.0] - 2026-10-04
### Added
- `?` help overlay and `Ctrl-P` command-palette float modals (DESIGN §6.11 / §127), layered above the footer via the unified 2-line footer contract.

### Changed
- **[Breaking — interaction]** Unified global key grammar in `app.rs`: `Esc` = back/pop, `q` = exit current root screen (and close any modal/input), `Enter` = open, `<`/`>`/`Tab` = switch context tabs without pushing on the stack, `r` = refresh, `?`/`Ctrl-P` = float overlays (DESIGN §1.2 §2 §6.11).
- Unified 5-state list panels (`loading`/`empty`/`error`/`ready`/`success`) across every view via the shared `render_state_panel` (DESIGN §3.2).
- 2-line footer: status row + hint row; local git and GitHub feedback consistent; toast channel.
- Visual token system + CJK 2-cell width budget + ≤80-column hint wrapping (`components/cells` width budget, DESIGN §5).
- All views migrated to the footer-split `render(f, content, app)` contract with per-view `{view}_hints()` exports feeding the unified hint row.

### Fixed
- `src/ui/mod.rs` test-truncation (unclosed `mod tests`) repaired; delete-confirm assertions restored; `config_view_picker_overlay_renders_via_dispatch` covered by `config.rs` unit tests instead.
- Three component test failures repaired: footer `render_rows` CJK hint-row width over-count; `state_panel` `PanelState::Error` block titling (`{title} · 加载失败`) resolving both the "titled 加载失败" and "panel keeps its title" contracts.

## [0.4.0]
- Single-platform (GitHub) receipt views; pre-redesign interaction grammar.
