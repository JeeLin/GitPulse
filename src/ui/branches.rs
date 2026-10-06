//! Branch panel: local branch list with create / switch / delete overlays.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState},
    Frame,
};

use crate::git::BranchInfo;
use crate::ui::components::footer::{hints as join_hints, HintRow};

use super::components::state_panel::{self, PanelState};
use super::{hint, pulls};

/// Borrowed render state for the branch panel, referencing `App.branches`
/// directly so no mock data source exists (checkpoint 2).
pub struct BranchesState<'a> {
    /// Live branch snapshots from `App.branches`.
    pub branches: &'a [BranchInfo],
    /// Selection cursor shared across frames.
    pub list_state: &'a mut ListState,
    /// Name of the repository the panel edits (shown in the title).
    pub repo_name: &'a str,
    /// Whether the single-line create input overlay is active.
    pub show_input: bool,
    /// Branch name being typed in create mode (convention 9).
    pub input: &'a str,
    /// Whether the delete confirmation dialog is shown.
    pub show_delete_confirm: bool,
    /// Branch targeted by the delete confirmation, when shown.
    pub confirm_branch: Option<&'a BranchInfo>,
    /// Load failure of the branch list (§4.3): the panel shows its `error`
    /// state instead of the list.
    pub load_error: Option<&'a str>,
}

/// Render the branch list, plus the create/confirm overlays when active.
///
/// The panel draws its content only (§4.1): the status row and the hint row
/// are both drawn once by the footer — the hint row from
/// [`branches_hints`] — so nothing here reserves a row for either.
///
/// A failed load is not a footer result: it paints the panel's `error` state
/// over the list (§4.3), so an empty panel never masquerades as success
/// (checkpoint 2) and an operation failure leaves the list untouched.
pub fn render_branches(f: &mut Frame, content: Rect, mut state: BranchesState<'_>) {
    if let Some(message) = state.load_error {
        state_panel::render_state_panel(
            f,
            content,
            &branch_title(state.repo_name, 0),
            &PanelState::Error { message },
        );
        return;
    }

    render_branch_list(f, content, &mut state);

    if state.show_delete_confirm {
        if let Some(branch) = state.confirm_branch {
            render_delete_dialog(f, content, branch);
        }
    }
    if state.show_input {
        render_create_input(f, content, state.input);
    }
}

/// Bottom key-hint row: submit/cancel while the create input is open, y/n/Esc
/// while the delete confirmation is open, list navigation otherwise.
///
/// Both modals swallow every list key (`input_modal_key` / `confirm_modal_key`
/// consume the press), so the list row must not be advertised then; the wording
/// mirrors each overlay's own title / `y n` line.
fn key_hints(input_open: bool, confirm_open: bool) -> Line<'static> {
    if confirm_open {
        // Shared rows (G5): the y/n/Esc wording of the confirmation, and
        // `Enter 创建` matching the create-input title
        // `新建分支 (Enter 创建 / Esc 取消)`.
        super::confirm_hints()
    } else if input_open {
        super::submit_hints("创建")
    } else {
        join_hints([
            hint("j/k", "选择"),
            hint("Enter", "切换"),
            hint("n", "新建"),
            hint("d", "删除"),
            hint("r", "刷新"),
            hint("q", "退出"),
            hint("Esc", "返回"),
        ])
    }
}

/// Footer hint row for the branch panel (§2.3): the modal-aware wording from
/// [`key_hints`], joined through [`join_hints`] so every key/desc pair keeps
/// the two-cell gap the footer composes the row with.
pub(crate) fn branches_hints(show_input: bool, show_delete_confirm: bool) -> HintRow {
    HintRow::new(key_hints(show_input, show_delete_confirm))
}

/// Panel title: `分支 [repo] (count)` — shared by the list and the `error`
/// state so a failed load still names the panel it belongs to.
fn branch_title(repo_name: &str, count: usize) -> String {
    format!("分支 [{repo_name}] ({count})")
}

fn render_branch_list(f: &mut Frame, area: Rect, state: &mut BranchesState<'_>) {
    let width = pulls::row_width(area);
    let items: Vec<ListItem> = state
        .branches
        .iter()
        .map(|branch| {
            let marker = if branch.is_head { "* " } else { "  " };
            let name_style = if branch.is_head {
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let mut spans = vec![
                Span::raw(marker),
                Span::styled(pulls::ellipsize(&branch.name, width), name_style),
            ];
            if let Some(upstream) = &branch.upstream {
                spans.push(Span::styled(
                    format!("  → {}", pulls::ellipsize(upstream, width)),
                    Style::default().fg(Color::DarkGray),
                ));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    let block = Block::default()
        .title(branch_title(state.repo_name, state.branches.len()))
        .borders(Borders::ALL);

    let list = List::new(items)
        .block(block)
        .highlight_style(super::list_highlight())
        .highlight_symbol(super::HIGHLIGHT_SYMBOL);

    f.render_stateful_widget(list, area, state.list_state);
}

/// Single-line create input (convention 9): Enter creates, Esc cancels.
fn render_create_input(f: &mut Frame, area: Rect, input: &str) {
    super::render_input_overlay(f, area, "新建分支", input, true);
}

/// Delete confirmation dialog (discard-style, convention 8): y confirms.
fn render_delete_dialog(f: &mut Frame, area: Rect, branch: &BranchInfo) {
    let body = vec![Line::from(vec![
        Span::raw("确定删除分支 "),
        Span::styled(branch.name.clone(), Style::default().fg(Color::Yellow)),
        Span::raw("?"),
    ])];
    super::render_confirm_overlay(f, area, "确认删除分支", 50, 8, body, true);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::components::footer::StatusRow;
    use crate::ui::status::StatusLine;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn branch(name: &str, is_head: bool, upstream: Option<&str>) -> BranchInfo {
        BranchInfo {
            name: name.to_string(),
            is_head,
            upstream: upstream.map(str::to_string),
        }
    }

    fn base_state<'a>(
        branches: &'a [BranchInfo],
        list_state: &'a mut ListState,
    ) -> BranchesState<'a> {
        BranchesState {
            branches,
            list_state,
            repo_name: "alpha",
            show_input: false,
            input: "",
            show_delete_confirm: false,
            confirm_branch: None,
            load_error: None,
        }
    }

    fn render_to_terminal(state: BranchesState<'_>) -> Terminal<TestBackend> {
        render_to_terminal_sized(state, 80, 12)
    }

    /// Taller frame for modal tests: the delete dialog is 8 rows high and
    /// would otherwise cover the bottom hint row of a 12-row terminal.
    ///
    /// The panel draws content only (§2.3), so the frame goes through
    /// [`crate::ui::components::footer::draw_frame`] — the shape
    /// `ui::render` uses — and the hint row is the bottom row.
    fn render_to_terminal_sized(
        state: BranchesState<'_>,
        width: u16,
        height: u16,
    ) -> Terminal<TestBackend> {
        let hints = branches_hints(state.show_input, state.show_delete_confirm);
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                crate::ui::components::footer::draw_frame(
                    f,
                    StatusRow::plain(None),
                    &hints,
                    |f, body| render_branches(f, body, state),
                );
            })
            .unwrap();
        terminal
    }

    fn render_to_text(state: BranchesState<'_>) -> String {
        let terminal = render_to_terminal(state);
        let raw: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        // Wide CJK glyphs leave padding cells; compare whitespace-stripped.
        raw.chars().filter(|c| !c.is_whitespace()).collect()
    }

    /// Whitespace-stripped text of one row; `y` defaults to the last row,
    /// which is the hint row (§2.3).
    fn compact_row(terminal: &Terminal<TestBackend>, y: u16) -> String {
        let buffer = terminal.backend().buffer();
        let raw: String = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol().to_string())
            .collect();
        raw.chars().filter(|c| !c.is_whitespace()).collect()
    }

    #[test]
    fn render_list_shows_head_marker_and_upstream() {
        let branches = vec![
            branch("main", true, Some("origin/main")),
            branch("feature", false, None),
        ];
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        let text = render_to_text(base_state(&branches, &mut list_state));
        assert!(text.contains("*main"), "missing head marker: {text}");
        assert!(text.contains("→origin/main"), "missing upstream: {text}");
        assert!(text.contains("feature"), "missing branch: {text}");
        assert!(text.contains("alpha"), "missing repo name: {text}");
    }

    /// §4.1: the panel body draws no status row — a failed load lands on the
    /// footer's status row and nowhere else, with the empty list untouched.
    #[test]
    fn render_error_status_shows_reason_on_the_footer_row_only() {
        let status = StatusLine {
            message: "加载分支列表失败: boom".to_string(),
            is_error: true,
        };
        let mut list_state = ListState::default();
        let state = base_state(&[], &mut list_state);
        let backend = TestBackend::new(80, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                crate::ui::components::footer::draw_frame(
                    f,
                    StatusRow::plain(Some(&status)),
                    &branches_hints(false, false),
                    |f, body| render_branches(f, body, state),
                );
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let rows: Vec<u16> = (0..buffer.area.height)
            .filter(|y| {
                let raw: String = (0..buffer.area.width)
                    .map(|x| buffer[(x, *y)].symbol().to_string())
                    .collect();
                let row: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
                row.contains("boom")
            })
            .collect();
        assert_eq!(
            rows,
            vec![buffer.area.height - 2],
            "the error must land on the footer status row only"
        );
    }

    #[test]
    fn render_create_input_shows_title_and_typed_name() {
        let branches = vec![branch("main", true, None)];
        let mut list_state = ListState::default();
        let mut state = base_state(&branches, &mut list_state);
        state.show_input = true;
        state.input = "hotfix";
        let text = render_to_text(state);
        assert!(text.contains("新建分支"), "missing title: {text}");
        assert!(text.contains(">hotfix"), "missing input: {text}");
    }

    #[test]
    fn render_delete_confirm_shows_branch_and_keys() {
        let branches = vec![branch("feature", false, None)];
        let mut list_state = ListState::default();
        let mut state = base_state(&branches, &mut list_state);
        state.show_delete_confirm = true;
        state.confirm_branch = branches.first();
        let text = render_to_text(state);
        assert!(text.contains("确认删除分支"), "missing title: {text}");
        assert!(text.contains("feature"), "missing branch: {text}");
        assert!(text.contains("y确认"), "missing confirm key: {text}");
    }

    /// View segment of the hint row: the bottom row without the global
    /// segment (§2.3), which is fixed on every screen and would otherwise
    /// answer for words like `刷新` / `返回` in every assertion below.
    fn view_segment(terminal: &Terminal<TestBackend>) -> String {
        let row = compact_row(terminal, terminal.backend().buffer().area.height - 1);
        let global: String = crate::ui::components::footer::global_hints()
            .spans
            .iter()
            .map(|s| s.content.to_string())
            .collect();
        let global: String = global.chars().filter(|c| !c.is_whitespace()).collect();
        row.strip_prefix(&global)
            .unwrap_or(row.as_str())
            .to_string()
    }

    #[test]
    fn render_hints_line_shows_navigation_keys() {
        let branches = vec![branch("main", true, None)];
        let mut list_state = ListState::default();
        let text = render_to_text(base_state(&branches, &mut list_state));
        assert!(text.contains("Esc返回"), "missing hints: {text}");
        assert!(text.contains("n新建"), "missing create hint: {text}");
    }

    /// List keys the delete confirm swallows (`confirm_modal_key` takes only
    /// y/n/Esc) must disappear from the bottom hints row.
    #[test]
    fn render_delete_confirm_switches_bottom_key_hints() {
        let branches = vec![branch("feature", false, None)];
        let mut list_state = ListState::default();
        let mut state = base_state(&branches, &mut list_state);
        state.show_delete_confirm = true;
        state.confirm_branch = branches.first();
        let terminal = render_to_terminal_sized(state, 80, 24);
        let hints = view_segment(&terminal);
        for swallowed in ["选择", "切换", "新建", "删除", "刷新", "返回"] {
            assert!(
                !hints.contains(swallowed),
                "swallowed key {swallowed} still hinted: {hints}"
            );
        }
        // Only the keys the confirmation actually handles are shown.
        assert!(hints.contains("y确认"), "missing confirm hint: {hints}");
        assert!(hints.contains("n取消"), "missing cancel hint: {hints}");
        assert!(hints.contains("Esc取消对话框"), "missing Esc hint: {hints}");
    }

    /// Same for the create input: every press goes to `input_modal_key`
    /// (Enter submits, Esc cancels, the rest edits the buffer).
    #[test]
    fn render_create_input_switches_bottom_key_hints() {
        let branches = vec![branch("main", true, None)];
        let mut list_state = ListState::default();
        let mut state = base_state(&branches, &mut list_state);
        state.show_input = true;
        state.input = "hotfix";
        let terminal = render_to_terminal_sized(state, 80, 24);
        let hints = view_segment(&terminal);
        for swallowed in ["选择", "切换", "新建", "删除", "刷新", "返回"] {
            assert!(
                !hints.contains(swallowed),
                "swallowed key {swallowed} still hinted: {hints}"
            );
        }
        assert!(hints.contains("Enter创建"), "missing submit hint: {hints}");
        assert!(hints.contains("Esc取消"), "missing cancel hint: {hints}");
    }
}
