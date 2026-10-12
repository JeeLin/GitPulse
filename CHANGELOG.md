# Changelog

All notable changes to this project are documented in this file.
Format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
