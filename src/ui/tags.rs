//! Tag panel: local tag list with create / delete overlays and push feedback.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::app::{RemoteStatus, StatusLine, TagsModal};
use crate::git::tag::TagInfo;

use super::hint;

/// Borrowed render state for the tag panel, referencing `App.tags_state`
/// directly so no mock data source exists (checkpoint 2).
pub struct TagsRender<'a> {
    /// Live tag snapshots from `App.tags_state.tags`.
    pub tags: &'a [TagInfo],
    /// Selection cursor shared across frames.
    pub list_state: &'a mut ListState,
    /// Open overlay, when any: the create input or the delete confirmation.
    pub modal: Option<&'a TagsModal>,
    /// Current status line, if any; errors render red and persist.
    pub status: Option<&'a StatusLine>,
    /// Push feedback; takes over the status line while present.
    pub remote_status: Option<&'a RemoteStatus>,
    /// Label of the running/last remote operation (`push`).
    pub remote_op: &'a str,
    /// Name of the repository the panel edits (shown in the title).
    pub repo_name: &'a str,
}

/// Render the tag list with the bottom status line, key hints, and the
/// create/confirm overlays when active.
///
/// Push feedback ([`RemoteStatus`]) outranks the plain status line, exactly
/// like the single-repo view, so a running push shows its gauge and a failed
/// one keeps its red reason until the next operation.
pub fn render_tags(f: &mut Frame, mut state: TagsRender<'_>) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),    // tag list
            Constraint::Length(1), // status line
            Constraint::Length(1), // key hints
        ])
        .split(area);

    render_tag_list(f, chunks[0], &mut state);

    match state.remote_status {
        Some(status) => super::remote::render_remote_status(f, chunks[1], status, state.remote_op),
        None => super::render_status_line(f, chunks[1], state.status),
    }

    f.render_widget(Paragraph::new(key_hints(state.modal)), chunks[2]);

    match state.modal {
        Some(TagsModal::Input { buffer }) => render_create_input(f, area, buffer),
        Some(TagsModal::Confirm { tag_to_delete }) => {
            render_delete_dialog(f, area, tag_to_delete);
        }
        None => {}
    }
}

/// Bottom key-hint row: submit/cancel while the create input is open, y/n/Esc
/// while the delete confirmation is open, list navigation otherwise.
///
/// Both overlays swallow every list key (`input_modal_key` /
/// `confirm_modal_key` consume the press), so the list row must not be
/// advertised then; the wording mirrors each overlay's own title / `y n`
/// line.
fn key_hints(modal: Option<&TagsModal>) -> Line<'static> {
    match modal {
        Some(TagsModal::Confirm { .. }) => [
            hint("y", "确认"),
            hint("n", "取消"),
            hint("Esc", "取消对话框"),
        ]
        .into_iter()
        .flatten()
        .collect(),
        Some(TagsModal::Input { .. }) => [hint("Enter", "创建"), hint("Esc", "取消")]
            .into_iter()
            .flatten()
            .collect(),
        None => [
            hint("j/k", "选择"),
            hint("c", "新建"),
            hint("d", "删除"),
            hint("p", "推送"),
            hint("a", "全部推送"),
            hint("Esc", "返回"),
        ]
        .into_iter()
        .flatten()
        .collect(),
    }
}

/// Tag list, or the empty-state copy for a repository without tags.
///
/// Zero tags is a valid state rather than a failure: the error status
/// (rendered by the caller) covers failed loads.
fn render_tag_list(f: &mut Frame, area: Rect, state: &mut TagsRender<'_>) {
    let block = Block::default()
        .title(format!("标签 [{}] ({})", state.repo_name, state.tags.len()))
        .borders(Borders::ALL);

    if state.tags.is_empty() {
        let paragraph = Paragraph::new(Span::styled(
            "暂无标签",
            Style::default().fg(Color::DarkGray),
        ))
        .block(block);
        f.render_widget(paragraph, area);
        return;
    }

    let items: Vec<ListItem> = state.tags.iter().map(tag_item).collect();
    let list = List::new(items)
        .block(block)
        .highlight_style(super::list_highlight())
        .highlight_symbol(super::HIGHLIGHT_SYMBOL);
    f.render_stateful_widget(list, area, state.list_state);
}

/// One tag row: name, short hash of the target commit, then the
/// lightweight / annotated marker.
fn tag_item(tag: &TagInfo) -> ListItem<'static> {
    let (kind, kind_style) = if tag.annotated {
        ("注释", Style::default().fg(Color::Cyan))
    } else {
        ("轻量", Style::default().fg(Color::DarkGray))
    };
    ListItem::new(Line::from(vec![
        Span::raw(tag.name.clone()),
        Span::styled(
            format!("  {}", tag.target),
            Style::default().fg(Color::Yellow),
        ),
        Span::styled(format!("  {kind}"), kind_style),
    ]))
}

/// Single-line create input (convention 9): Enter creates, Esc cancels.
fn render_create_input(f: &mut Frame, area: Rect, buffer: &str) {
    super::render_input_overlay(f, area, "新建标签 (Enter 创建 / Esc 取消)", buffer, false);
}

/// Delete confirmation dialog (discard-style, convention 8): y confirms.
fn render_delete_dialog(f: &mut Frame, area: Rect, tag_to_delete: &str) {
    let body = vec![Line::from(vec![
        Span::raw("确定删除标签 "),
        Span::styled(
            tag_to_delete.to_string(),
            Style::default().fg(Color::Yellow),
        ),
        Span::raw("?"),
    ])];
    super::render_confirm_overlay(f, area, "确认删除标签", 50, 8, body, false);
}
