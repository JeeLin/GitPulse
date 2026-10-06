//! Tag panel: local tag list with create / delete overlays and push feedback.

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{ListItem, ListState},
    Frame,
};

use crate::app::TagsModal;
use crate::git::TagInfo;
use crate::ui::components::footer::{hints as join_hints, HintRow};

use super::components::state_panel::{self, PanelState};
use super::{hint, named_title, pulls};

/// Copy the tag panel's `empty` state leads with (G7): zero tags is a valid
/// state rather than a failure, so the panel says so instead of showing an
/// empty list.
const EMPTY_TAGS: &str = "暂无标签";

/// The tag panel's §3.2 `empty` state: the copy above, one line of why the
/// panel is empty, and the panel's own create key — so a repository without
/// tags reads as an invitation rather than a bare sentence.
fn empty_tags_state() -> PanelState<'static> {
    state_panel::empty_state(
        EMPTY_TAGS,
        "这个仓库还没有标签",
        Some(hint("c", "新建第一个标签")),
    )
}

/// Borrowed render state for the tag panel, referencing `App.tags_state`
/// directly so no mock data source exists (checkpoint 2).
pub struct TagsRender<'a> {
    /// Live tag snapshots from `App.tags_state.tags`.
    pub tags: &'a [TagInfo],
    /// Selection cursor shared across frames.
    pub list_state: &'a mut ListState,
    /// Open overlay, when any: the create input or the delete confirmation.
    pub modal: Option<&'a TagsModal>,
    /// Name of the repository the panel edits (shown in the title).
    pub repo_name: &'a str,
    /// Load failure of the tag list (§4.3): the panel shows its `error` state
    /// instead of the list.
    pub load_error: Option<&'a str>,
    /// Whether the tag list holds the screen's panel focus (§3.1): it picks
    /// the border (`border_focus` against `border`) and whether the list
    /// draws the `▌` / `surface_alt` cursor.
    pub focused: bool,
}

/// Render the tag list, plus the create/confirm overlays when active.
///
/// The panel draws its content only (§4.1): the status row and the hint row
/// are both painted once by the footer — the hint row from [`tags_hints`] —
/// so no async *result* is read here. Push feedback ([`RemoteStatus`]) takes
/// over that row, and a notice recorded while it runs shares it, see
/// [`super::remote::render_remote_status`].
///
/// A failed load is not a footer result: it paints the panel's `error` state
/// over the list (§4.3), while a failed delete or push keeps the list and
/// reports on the status row.
pub fn render_tags(f: &mut Frame, content: Rect, mut state: TagsRender<'_>) {
    if let Some(message) = state.load_error {
        state_panel::render_state_panel(
            f,
            content,
            &named_title("标签", state.repo_name, 0),
            &PanelState::Error { message },
        );
        return;
    }

    render_tag_list(f, content, &mut state);

    match state.modal {
        Some(TagsModal::Input { buffer }) => render_create_input(f, content, buffer),
        Some(TagsModal::Confirm { tag_to_delete }) => {
            render_delete_dialog(f, content, tag_to_delete);
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
        None => join_hints([
            hint("j/k", "选择"),
            hint("c", "新建"),
            hint("d", "删除"),
            hint("p", "推送"),
            hint("a", "全部推送"),
            hint("q/Esc", "返回"),
        ]),
    }
}

/// Footer hint row for the tag panel (§2.3): the panel's own verbs, with the
/// input/confirm wording selected by the modal state.
pub(crate) fn tags_hints(modal: Option<&crate::app::TagsModal>) -> HintRow {
    HintRow::new(key_hints(modal))
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
        super::NamedPanel {
            label: "标签",
            repo_name: state.repo_name,
            empty: &empty_tags_state(),
            focused: state.focused,
        },
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
    use crate::ui::components::footer::StatusRow;
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
    /// Global segment of the hint row, whitespace-stripped (§2.3).
    const GLOBAL_PREFIX: &str = "⏎打开Esc返回r刷新q退出";

    /// Render the panel through the footer into an 80×24 frame.
    fn render_to_terminal(state: TagsRender<'_>) -> Terminal<TestBackend> {
        let hints = tags_hints(state.modal);
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                crate::ui::components::footer::draw_frame(
                    f,
                    StatusRow::plain(None),
                    &hints,
                    |f, body| render_tags(f, body, state),
                );
            })
            .unwrap();
        terminal
    }

    /// Whitespace-stripped text of one row, `y` from the top.
    fn compact_row(terminal: &Terminal<TestBackend>, y: u16) -> String {
        let buffer = terminal.backend().buffer();
        let raw: String = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol().to_string())
            .collect();
        raw.chars().filter(|c| !c.is_whitespace()).collect()
    }

    /// The panel draws content only (§4.1), so the frame goes through
    /// [`crate::ui::components::footer::draw_frame`] — the shape
    /// `ui::render` uses — and the footer takes the bottom two rows.
    fn render_to_text(state: TagsRender<'_>) -> String {
        let hints = tags_hints(state.modal);
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                crate::ui::components::footer::draw_frame(
                    f,
                    StatusRow::plain(None),
                    &hints,
                    |f, body| render_tags(f, body, state),
                );
            })
            .unwrap();
        let raw: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        raw.chars().filter(|c| !c.is_whitespace()).collect()
    }

    fn base_state<'a>(tags: &'a [TagInfo], list_state: &'a mut ListState) -> TagsRender<'a> {
        TagsRender {
            tags,
            list_state,
            modal: None,
            repo_name: "demo",
            load_error: None,
            focused: true,
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
        let text = render_to_text(base_state(&tags, &mut list_state));
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
        let text = render_to_text(base_state(&[], &mut list_state));
        assert!(text.contains("标签[demo](0)"), "missing title: {text}");
        assert!(text.contains("暂无标签"), "missing empty state: {text}");
        // The view segment is right-truncated at 80 columns (§2.3), so the
        // empty state asserts the global segment still leads the row.
        let terminal = render_to_terminal(base_state(&[], &mut list_state));
        let hints = compact_row(&terminal, terminal.backend().buffer().area.height - 1);
        assert!(
            hints.starts_with(GLOBAL_PREFIX),
            "hint row must lead with the global segment: {hints}"
        );
    }

    /// §4.3: a failed tag load paints the panel's `error` state (`加载失败` +
    /// reason + `按 r 重试`) instead of the empty state, and the reason never
    /// reaches the footer status row.
    #[test]
    fn render_load_error_shows_the_panel_error_state_not_the_status_row() {
        let mut list_state = ListState::default();
        let mut state = base_state(&[], &mut list_state);
        state.load_error = Some("加载标签列表失败: boom");
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                crate::ui::components::footer::draw_frame(
                    f,
                    StatusRow::plain(None),
                    &tags_hints(None),
                    |f, body| render_tags(f, body, state),
                );
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let rows_with = |needle: &str| -> Vec<u16> {
            let buffer = buffer.clone();
            (0..buffer.area.height)
                .filter(|y| {
                    let raw: String = (0..buffer.area.width)
                        .map(|x| buffer[(x, *y)].symbol().to_string())
                        .collect();
                    let row: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
                    row.contains(needle)
                })
                .collect()
        };
        assert!(
            rows_with("加载标签列表失败:boom")
                .iter()
                .all(|y| *y != buffer.area.height - 2),
            "a load failure must not land on the status row"
        );
        assert!(
            !rows_with("加载标签列表失败:boom").is_empty(),
            "the reason must reach the panel"
        );
        let all: String = (0..buffer.area.height)
            .map(|y| {
                let raw: String = (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect();
                let row: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
                row
            })
            .collect();
        assert!(all.contains("加载失败"), "missing error title: {all}");
        assert!(all.contains("按r重试"), "missing retry hint: {all}");
        assert!(
            all.contains("标签[demo]"),
            "the panel keeps its title: {all}"
        );
        // The empty state must not read as "this repository has no tags".
        assert!(
            !all.contains("暂无标签"),
            "empty state masked the error: {all}"
        );
    }

    #[test]
    fn render_create_input_shows_title_and_typed_name() {
        let tags = vec![tag("v1.0", "abc1234", false)];
        let mut list_state = ListState::default();
        let mut state = base_state(&tags, &mut list_state);
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
        let mut state = base_state(&tags, &mut list_state);
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
        let mut state = base_state(&tags, &mut list_state);
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
