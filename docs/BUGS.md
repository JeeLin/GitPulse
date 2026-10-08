# 缺陷池

> 引入于 v0.7.0：round-3 审查中未能于本里程碑封印的重构/测试/文档类发现，逐条编号以便里程碑追踪。
> 均为**非功能缺陷**（不影响 §2.3 / §3 / §4 / §5 / §6 任何交互不变量），或受冻结约束。

| ID | 提出版本 | 优先级 | 标题 | 来源 | 描述 |
|----|----------|--------|------|------|------|
| B4-01 | v0.7.0 | 🟡 | `split_history` 文档块逐字重复 | round-2 S1 | 位置 `src/ui/history.rs:117-128`；证据：117-122 与 123-128 字节相同、均以「§5.4 split gate for the history view」开头并引 `[theme_width::LIST_MIN]`；断言：删第 2 份(123-128)后 `split_history:129` 行为不变 |
| B4-02 | v0.7.0 | 🟡 | 提交详情 body 组成规则三处复制 | round-2 #21 | 位置 `src/ui/commit_detail.rs · src/ui/history.rs · src/app.rs` `93-108 · 328-342 · 3014-3018`；证据：提交详情 body 组成规则三处复制，仅 `Line::from(Span::raw(..))` 包装差异；第三次在 `app.rs` 以 `d.message.lines().count() + 1 + 1 + d.files.len()` 复述同一行数规则 |
| B4-03 | v0.7.0 | 🟡 | 跨层反向依赖 `components/` → `crate::ui::*` | round-2 #5 | 位置 `src/ui/components/receipts.rs · src/ui/components/footer.rs` `189, 367 · 199`；证据：`components/` 反向引用 `crate::ui::with_cursor`/`view_status`/`render_status_or_remote`；断言：`grep -rn "crate::ui::" src/ui/components/` |
| B4-04 | v0.7.0 | 🟡 | 选中仓库+文件前导四处逐字重复 | round-2 #7 | 位置 `src/app.rs` `1042-1045 · 1061-1064 · 1079-1082 · 1103-1106`；证据：`if let (Some(repo_idx), Some(file_idx)) = (self.current_repo, self.file_state.selected())` + 边界校验 + `.path.clone()` 四处同构 |
| B4-05 | v0.7.0 | 🟡 | `render_repo_view`/`render_commit_view` 构造 `RepoState` 逐字相同 | round-2 #8 | 位置 `src/ui/mod.rs` `563-586 · 590-608`；证据：12 个字段逐字相同，仅 `show_discard_confirm`/`confirm_file` 不同；断言：`fn repo_state(app, allow_discard)` 收编后 `render_commit_view` 降到 3 行 |
| B4-06 | v0.7.0 | 🟡 | PR 详情 / Issue 详情同构副本三段重复 | round-2 #9 | 位置 `src/ui/issues.rs · src/ui/pulls.rs` `53-96 · 121-137 · 214-253 · 290-306`；证据：①门控前导 `render_load_error`/`render_disconnected` ②shell+placeholder 骨架 ③行构造，三段逐字同构（仅 `LABEL`/`kind` 不同） |
| B4-07 | v0.7.0 | 🟡 | `meta_budget` 后分预算算式三份 | round-2 #10 | 位置 `src/ui/issues.rs · src/ui/notifications.rs · src/ui/pulls.rs` `207-212 · 60-65 · 164-169`；证据：`let budget=meta_budget(&meta,width); let head_width=if budget>0 { width-budget-cells::width(META_GAP) } else { width };` 逐字重复 3 处；断言：`grep -rn "let budget=meta_budget" src/ui/` |
| B4-08 | v0.7.0 | 🟡 | ready-list-shell ×3 同构 | round-2 #11 | 位置 `src/ui/mod.rs · src/ui/dashboard.rs · src/ui/config.rs` `185-191 · 279-285 · 277-283`；证据：`Block::default().title(T).borders(Borders::ALL).border_style(role::border(focused))` + `with_cursor(List::new(items).block(block), focused)` + `render_stateful_widget` 逐行复制 |
| B4-09 | v0.7.0 | 🟡 | 仪表盘空态 Action 与提示行 `c` 互矛盾 | round-2 #12 | 位置 `src/ui/dashboard.rs` `449-460`；证据：空态 action `hint("c","管理扫描目录")` 与 `dashboard_hints()` 已固化为 `⏎ 进入仓库  n 添加目录`（不含 `c`）；**行为已一致**（`c` 不绑定），纯文案待清 |
| B4-10 | v0.7.0 | 🟡 | branches/tags §5.4 宽度算法两份 | round-2 #14 | 位置 `src/ui/branches.rs · src/ui/tags.rs` `147-162 · 154-170`；证据：同一「§5.4 高优先 name + 低优先 tail 整体消失」算法两份；另 `branches.rs:137-164` 把 28 行行构造写成匿名闭包，其余面板均有具名构造器 |
| B4-11 | v0.7.0 | 🟡 | `render_create_input`/`render_delete_dialog` 同构 | round-2 #15 | 位置 `src/ui/branches.rs · src/ui/tags.rs` `182-194 · 176-193`；证据：覆层 2 对（各 3 行 / 各 8 行）归一化后仅 2 个字符串字面量不同 |
| B4-12 | v0.7.0 | 🟢 | 日历换算算法第二份实现 | round-2 #16 | 位置 `src/ui/dashboard.rs` `481-508`；证据：`format_time`/`civil_from_days`（`div_euclid(146_097)` Hinnant）与 `src/git/history.rs:394-405` 同算法；**受冻结约束**：`src/git/` 本版本不动 |
| B4-13 | v0.7.0 | 🟡 | `components/mod.rs` 模块文档与声明不符 | round-2 #18 | 位置 `src/ui/components/mod.rs:1-2`；证据：首段仅提 4 个组件，`grep -n "^pub" src/ui/components/mod.rs` → 6（cells/footer/header/receipts/state_panel/theme） |
| B4-14 | v0.7.0 | 🟢 | `split_area` 重复求 `has_context_tags` | round-2 #19 | 位置 `src/ui/components/header.rs:42-68`；证据：`grep -c "if has_context_tags" src/ui/components/header.rs` → 2，两分支各构造一次 `Layout` |
| B4-15 | v0.7.0 | 🟢 | help 浮层 `bg(color::SURFACE)` 施加三次 | round-2 #22 | 位置 `src/ui/help.rs:35 · 68 · 89`；证据：`modal_block` 给 `Block` 已覆盖整块区域，后两次疑似冗余；断言：`grep -c "bg(color::SURFACE)" src/ui/help.rs` → 3。**未修原因**：不确定是否承重，删后须 `help`/`palette` 渲染断言全绿方可合入 |
| B5-01 | v0.7.0 | 🟡 | 测试名承诺「每个状态同壳」却对 `error` 跳过边框断言 | round-2 step5 #12 | 位置 `src/ui/components/receipts.rs:603-609`；证据：`every_receipts_state_wears_the_same_focused_shell` 的 `if copy != "加载失败"` 使 `error` 用例不校验 `buffer[(0,0)].fg`；实现正确（`state_panel.rs` 对 `Error` 用 `DANGER`），缺的是断言 |
| B5-02 | v0.7.0 | 🟡 | §2.3 跨屏不变量测试只跑 6/15 屏却命名「every screen」 | round-2 step5 #9 | 位置 `src/ui/mod.rs:797-821`；证据：`hint_row_screens()` 只返回 `[dashboard, repo, pulls, checks, issues, notifications]`；断言：三个 §2.3 测试仍覆盖 6 屏，`branches`/`history`/`tags`/`config`/`diff`/`commit`/`commit_detail` 未纳入 |
| B7-01 | v0.7.0 | 🟡 | 按屏键→动作断言缺失 | round-2 step7 #3 | 位置 `src/ui/mod.rs` `design_canonical_keybindings_match_design_2_1_and_2_2`；证据：逐键源码 token 断言只验 §2.2 通用键「存在」，C9 与 §6.2.1 键位两处假闭环正是穿过此测试；断言：需按屏断言键→动作映射 |
| B7-02 | v0.7.0 | 🟡 | `P`/`p` 大写规范冲突 | round-2 step7 #4 | 位置 `src/app.rs` `handle_repo_key`；证据：`P` 实现为 pull，DESIGN §6.2.1 写作大写 `P`、§2.2 canonical 在 `p`=push；实现自洽但两处表述冲突，**需 DESIGN.md 裁决** |
| B7-03 | v0.7.0 | 🟢 | `MIN_PATH_CELLS=12` 经验阈值无 DESIGN 推导 | round-2 step7 #5 | 位置 `src/ui/repo.rs:277 funded_counts_digits`；证据：DESIGN §5.4 只给「低优先列先消失」原则未给具体阈值；断言：需在 DESIGN.md 补推导或在注释说明取值理由 |