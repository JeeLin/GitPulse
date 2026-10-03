//! Tag panel: local tag list with create / delete overlays and push feedback.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{ListItem, ListState, Paragraph},
    Frame,
};

use crate::app::TagsModal;
use crate::git::TagInfo;
use crate::ui::status::{RemoteStatus, StatusLine};

use super::{hint, pulls};

/// Empty-state copy of a repository that carries no tag (G7): zero tags is a
/// valid state rather than a failure, so the panel says so instead of
/// showing an empty list.
const EMPTY_TAGS: &str = "暂无标签";

/// Borrowed render state for the tag panel, referencing `App.tags_state`
/// directly so no mock data source exists (checkpoint 2).
pub struct TagsRender<'a> {
    /// Live tag snapshots from `App.tags_state.tags`.
    pub tags: &'a [TagInfo],
    /// Selection cursor shared across frames.
    pub list_state: &'a mut ListState,
    /// Open overlay, when any: the create input or the delete confirmation.
    pub modal: Option<&'a TagsModal>,
    /// Current status line, if any; errors render red and persist. While a
    /// remote operation runs it renders beside that feedback instead of
    /// being hidden.
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
/// Push feedback ([`RemoteStatus`]) takes over the status row, exactly like
/// the single-repo view, so a running push shows its running line and a
/// failed one keeps its red reason until the next operation. A notice
/// recorded while that operation runs — `已有远端操作进行中` on a rejected
/// second push — is not dropped: it shares the row on the feedback's right,
/// see [`super::remote::render_remote_status`].
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

    let tag_list_area = chunks[0];
    let status_area = chunks[1];
    let hints_area = chunks[2];

    render_tag_list(f, tag_list_area, &mut state);

    // Remote feedback takes over the status row; a notice written while it
    // runs shares it (F1/G2). Shared branch with the repo panel (G6).
    super::render_status_or_remote(
        f,
        status_area,
        state.remote_status,
        state.remote_op,
        state.status,
    );

    f.render_widget(Paragraph::new(key_hints(state.modal)), hints_area);

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
        // Both overlay rows are the shared ones (G5): the y/n/Esc wording of
        // the confirmation, and `Enter 创建` matching the input's own title.
        Some(TagsModal::Confirm { .. }) => super::confirm_hints(),
        Some(TagsModal::Input { .. }) => super::submit_hints("创建"),
        None => [
            hint("j/k", "选择"),
            hint("c", "新建"),
            hint("d", "删除"),
            hint("p", "推送"),
            hint("a", "全部推送"),
            hint("q/Esc", "返回"),
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
    let width = pulls::row_width(area);
    let items: Vec<ListItem> = state.tags.iter().map(|tag| tag_item(tag, width)).collect();
    super::render_named_list(
        f,
        area,
        "标签",
        state.repo_name,
        EMPTY_TAGS,
        items,
        state.list_state,
    );
}

/// One tag row: name, short hash of the target commit, then the
/// lightweight / annotated marker.
fn tag_item(tag: &TagInfo, width: usize) -> ListItem<'static> {
    let (kind, kind_style) = if tag.annotated {
        ("注释", Style::default().fg(Color::Cyan))
    } else {
        ("轻量", Style::default().fg(Color::DarkGray))
    };
    ListItem::new(Line::from(vec![
        Span::raw(pulls::ellipsize(&tag.name, width)),
        Span::styled(
            format!("  {}", pulls::ellipsize(&tag.target, width)),
            Style::default().fg(Color::Yellow),
        ),
        Span::styled(format!("  {kind}"), kind_style),
    ]))
}

/// Single-line create input (convention 9): Enter creates, Esc cancels.
fn render_create_input(f: &mut Frame, area: Rect, buffer: &str) {
    super::render_input_overlay(f, area, "新建标签", buffer, true);
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
    // `wrap` (H6): a long tag name would otherwise be clipped at the right
    // border — losing the trailing `?` — while `y` still deleted it.
    super::render_confirm_overlay(f, area, "确认删除标签", 50, 8, body, true);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn tag(name: &str, target: &str, annotated: bool) -> TagInfo {
        TagInfo {
            name: name.to_string(),
            // Every fixture name is valid UTF-8, so the raw bytes match it.
            name_bytes: name.as_bytes().to_vec(),
            target: target.to_string(),
            annotated,
        }
    }

    /// Render one frame and return the whitespace-stripped buffer text
    /// (wide CJK glyphs leave padding cells).
    fn render_to_text(state: TagsRender<'_>) -> String {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render_tags(f, state)).unwrap();
        let raw: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        raw.chars().filter(|c| !c.is_whitespace()).collect()
    }

    fn base_state<'a>(
        tags: &'a [TagInfo],
        list_state: &'a mut ListState,
        status: Option<&'a StatusLine>,
    ) -> TagsRender<'a> {
        TagsRender {
            tags,
            list_state,
            modal: None,
            status,
            remote_status: None,
            remote_op: "",
            repo_name: "demo",
        }
    }

    #[test]
    fn render_list_shows_name_short_hash_kind_and_hints() {
        let tags = vec![
            tag("v-light", "abc1234", false),
            tag("v-annotated", "def5678", true),
        ];
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        let text = render_to_text(base_state(&tags, &mut list_state, None));
        assert!(text.contains("v-light"), "missing tag name: {text}");
        assert!(text.contains("v-annotated"), "missing tag name: {text}");
        assert!(text.contains("abc1234"), "missing short hash: {text}");
        assert!(text.contains("def5678"), "missing short hash: {text}");
        assert!(text.contains("轻量"), "missing lightweight marker: {text}");
        assert!(text.contains("注释"), "missing annotated marker: {text}");
        assert!(text.contains("demo"), "missing repo name: {text}");
        assert!(text.contains("c新建"), "missing create hint: {text}");
        assert!(text.contains("d删除"), "missing delete hint: {text}");
        assert!(text.contains("p推送"), "missing push hint: {text}");
    }

    /// Zero tags is a valid state rather than a failure (checkpoint 2): the
    /// panel says so instead of showing an empty list.
    #[test]
    fn render_empty_tag_list_shows_empty_state() {
        let mut list_state = ListState::default();
        let text = render_to_text(base_state(&[], &mut list_state, None));
        assert!(text.contains("标签[demo](0)"), "missing title: {text}");
        assert!(text.contains("暂无标签"), "missing empty state: {text}");
        assert!(text.contains("q/Esc返回"), "missing back hint: {text}");
    }

    #[test]
    fn render_error_status_shows_reason_not_just_empty_list() {
        let status = StatusLine {
            message: "加载标签列表失败: boom".to_string(),
            is_error: true,
        };
        let mut list_state = ListState::default();
        let text = render_to_text(base_state(&[], &mut list_state, Some(&status)));
        assert!(text.contains("boom"), "missing error: {text}");
    }

    #[test]
    fn render_create_input_shows_title_and_typed_name() {
        let tags = vec![tag("v1.0", "abc1234", false)];
        let mut list_state = ListState::default();
        let mut state = base_state(&tags, &mut list_state, None);
        let modal = TagsModal::Input {
            buffer: "v3".to_string(),
        };
        state.modal = Some(&modal);
        let text = render_to_text(state);
        assert!(text.contains("新建标签"), "missing input title: {text}");
        assert!(text.contains(">v3"), "missing typed name: {text}");
        assert!(text.contains("Enter创建"), "missing submit hint: {text}");
        assert!(text.contains("Esc取消"), "missing cancel hint: {text}");
    }

    #[test]
    fn render_delete_confirm_shows_tag_name_and_keys() {
        let tags = vec![tag("v2.0", "abc1234", false)];
        let mut list_state = ListState::default();
        let mut state = base_state(&tags, &mut list_state, None);
        let modal = TagsModal::Confirm {
            tag_to_delete: "v2.0".to_string(),
        };
        state.modal = Some(&modal);
        let text = render_to_text(state);
        assert!(
            text.contains("确认删除标签"),
            "missing dialog title: {text}"
        );
        assert!(text.contains("确定删除标签"), "missing dialog body: {text}");
        assert!(text.contains("v2.0"), "missing tag name: {text}");
        assert!(text.contains("y确认"), "missing confirm key: {text}");
        assert!(text.contains("n取消"), "missing cancel key: {text}");
    }

    /// H6: a long tag name must not be clipped at the right border while
    /// `y` still deletes it — the dialog wraps instead, so the full name
    /// (and with it the trailing `?`) stays visible on any terminal.
    #[test]
    fn render_delete_confirm_wraps_long_tag_name() {
        let tags = vec![tag("v1.2.3-rc.1+build.20261001", "abc1234", false)];
        let mut list_state = ListState::default();
        let mut state = base_state(&tags, &mut list_state, None);
        let modal = TagsModal::Confirm {
            tag_to_delete: "v1.2.3-rc.1+build.20261001".to_string(),
        };
        state.modal = Some(&modal);
        let text = render_to_text(state);
        assert!(
            text.contains("v1.2.3-rc.1+build.20261001?"),
            "long tag name clipped while still deletable: {text}"
        );
    }
}
