//! Branch panel: local branch list with create / switch / delete overlays.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::app::StatusLine;
use crate::git::BranchInfo;

/// Borrowed render state for the branch panel, referencing `App.branches`
/// directly so no mock data source exists (checkpoint 2).
pub struct BranchesState<'a> {
    /// Live branch snapshots from `App.branches`.
    pub branches: &'a [BranchInfo],
    /// Current status line, if any; errors render red and persist.
    pub status: Option<&'a StatusLine>,
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
}

/// Render the branch list with the bottom status line, key hints, and the
/// create/confirm overlays when active.
///
/// A failed load arrives as an error [`StatusLine`] with an empty list, so
/// an empty panel never masquerades as success (checkpoint 2).
pub fn render_branches(f: &mut Frame, mut state: BranchesState<'_>) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),    // branch list
            Constraint::Length(1), // status line
            Constraint::Length(1), // key hints
        ])
        .split(area);

    render_branch_list(f, chunks[0], &mut state);

    super::render_status_line(f, chunks[1], state.status);

    let hints = Line::from(vec![
        Span::styled("j/k", Style::default().fg(Color::Cyan)),
        Span::raw(" 选择 "),
        Span::styled("Enter", Style::default().fg(Color::Cyan)),
        Span::raw(" 切换 "),
        Span::styled("n", Style::default().fg(Color::Cyan)),
        Span::raw(" 新建 "),
        Span::styled("d", Style::default().fg(Color::Cyan)),
        Span::raw(" 删除 "),
        Span::styled("r", Style::default().fg(Color::Cyan)),
        Span::raw(" 刷新 "),
        Span::styled("Esc", Style::default().fg(Color::Cyan)),
        Span::raw(" 返回"),
    ]);
    f.render_widget(Paragraph::new(hints), chunks[2]);

    if state.show_delete_confirm {
        if let Some(branch) = state.confirm_branch {
            render_delete_dialog(f, area, branch);
        }
    }
    if state.show_input {
        render_create_input(f, area, state.input);
    }
}

fn render_branch_list(f: &mut Frame, area: Rect, state: &mut BranchesState<'_>) {
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
                Span::styled(branch.name.clone(), name_style),
            ];
            if let Some(upstream) = &branch.upstream {
                spans.push(Span::styled(
                    format!("  → {upstream}"),
                    Style::default().fg(Color::DarkGray),
                ));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    let block = Block::default()
        .title(format!(
            "分支 [{}] ({})",
            state.repo_name,
            state.branches.len()
        ))
        .borders(Borders::ALL);

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("► ");

    f.render_stateful_widget(list, area, state.list_state);
}

/// Single-line create input (convention 9): Enter creates, Esc cancels.
fn render_create_input(f: &mut Frame, area: Rect, input: &str) {
    super::render_input_overlay(f, area, "新建分支 (Enter 创建 / Esc 取消)", input, false);
}

/// Delete confirmation dialog (discard-style, convention 8): y confirms.
fn render_delete_dialog(f: &mut Frame, area: Rect, branch: &BranchInfo) {
    let body = vec![Line::from(vec![
        Span::raw("确定删除分支 "),
        Span::styled(branch.name.clone(), Style::default().fg(Color::Yellow)),
        Span::raw("?"),
    ])];
    super::render_confirm_overlay(f, area, "确认删除分支", 50, 8, body, false);
}

#[cfg(test)]
mod tests {
    use super::*;
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
        status: Option<&'a StatusLine>,
    ) -> BranchesState<'a> {
        BranchesState {
            branches,
            status,
            list_state,
            repo_name: "alpha",
            show_input: false,
            input: "",
            show_delete_confirm: false,
            confirm_branch: None,
        }
    }

    fn render_to_text(state: BranchesState<'_>) -> String {
        let backend = TestBackend::new(80, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render_branches(f, state)).unwrap();
        // Wide CJK glyphs leave padding cells; compare whitespace-stripped.
        let raw: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
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
        let text = render_to_text(base_state(&branches, &mut list_state, None));
        assert!(text.contains("*main"), "missing head marker: {text}");
        assert!(text.contains("→origin/main"), "missing upstream: {text}");
        assert!(text.contains("feature"), "missing branch: {text}");
        assert!(text.contains("alpha"), "missing repo name: {text}");
    }

    #[test]
    fn render_error_status_shows_reason_not_just_empty_list() {
        let status = StatusLine {
            message: "加载分支列表失败: boom".to_string(),
            is_error: true,
        };
        let mut list_state = ListState::default();
        let text = render_to_text(base_state(&[], &mut list_state, Some(&status)));
        assert!(text.contains("boom"), "missing error: {text}");
    }

    #[test]
    fn render_create_input_shows_title_and_typed_name() {
        let branches = vec![branch("main", true, None)];
        let mut list_state = ListState::default();
        let mut state = base_state(&branches, &mut list_state, None);
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
        let mut state = base_state(&branches, &mut list_state, None);
        state.show_delete_confirm = true;
        state.confirm_branch = branches.first();
        let text = render_to_text(state);
        assert!(text.contains("确认删除分支"), "missing title: {text}");
        assert!(text.contains("feature"), "missing branch: {text}");
        assert!(text.contains("y确认"), "missing confirm key: {text}");
    }

    #[test]
    fn render_hints_line_shows_navigation_keys() {
        let branches = vec![branch("main", true, None)];
        let mut list_state = ListState::default();
        let text = render_to_text(base_state(&branches, &mut list_state, None));
        assert!(text.contains("Esc返回"), "missing hints: {text}");
        assert!(text.contains("n新建"), "missing create hint: {text}");
    }
}
