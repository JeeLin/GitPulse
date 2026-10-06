//! Commit history view: DAG column, commit list and commit detail pane.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::git::{CommitDetail, CommitRow, RefKind};

use super::components::state_panel::{self, PanelState};
use super::{hint, named_title, pulls};
use crate::ui::components::footer::{hints as join_hints, HintRow};
use crate::ui::components::theme::role;

/// Inner width of the DAG column: one cell per lane, capped at the lanes
/// `git::layout_graph` ever produces (`git::MAX_LANES`).
const GRAPH_WIDTH: usize = crate::git::MAX_LANES;

/// Columns the DAG column's own [`Block`] takes beyond [`GRAPH_WIDTH`]: the
/// vertical border on each side costs one cell (G23).
const GRAPH_BORDER_COLUMNS: u16 = 2;

/// Empty-state copy of a repository without history (G7): an unborn HEAD
/// loads zero commits, which is a valid state rather than a failure, so the
/// panel says so instead of showing an empty list.
const EMPTY_HISTORY: &str = "暂无提交记录";

/// Borrowed render state for the history view, referencing `App.history`
/// directly so no mock data source exists (checkpoint 2).
pub struct HistoryRender<'a> {
    /// Live commit snapshots from `App.history.commits`.
    pub commits: &'a [CommitRow],
    /// Detail payload of the highlighted commit, from `App.history.detail`.
    pub detail: Option<&'a CommitDetail>,
    /// One DAG row per entry of `commits`, from `git::layout_graph`: exactly
    /// [`GRAPH_WIDTH`] characters, each column already holding its glyph.
    pub graph: &'a [Vec<char>],
    /// Selection cursor shared across frames.
    pub list_state: &'a mut ListState,
    /// Index of the highlighted commit, from `App.history.selected` — the
    /// single source of "which row is selected" the renderer reads (G9), so
    /// the view no longer re-derives it from `list_state`.
    pub selected: usize,
    /// Name of the repository being read (shown in the title).
    pub repo_name: &'a str,
    /// Load failure of the commit list or of the detail payload (§4.3): the
    /// commit-list panel shows its `error` state instead of the list.
    pub load_error: Option<&'a str>,
    /// Whether the commit list holds the screen's panel focus (§3.1): it
    /// picks the border (`border_focus` against `border`) and whether the
    /// list draws the `▌` / `surface_alt` cursor.
    pub focused: bool,
}

/// Render the history view: DAG column, commit list and detail pane. The
/// status row and the hint row belong to the footer (§4.1).
///
/// The list is drawn first so the DAG column can scroll with the offset the
/// list actually settled on, keeping graph rows and commit rows aligned.
///
/// A failed load lands on the panel whose content is missing (§4.3): with no
/// commits the error state replaces the list, while a commit list that did
/// load keeps its rows and the detail pane reports the failure — the content
/// that arrived stays readable either way.
pub fn render_history(f: &mut Frame, content: Rect, mut state: HistoryRender<'_>) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(GRAPH_WIDTH as u16 + GRAPH_BORDER_COLUMNS), // DAG + borders
            Constraint::Min(24),                                           // commit list
            Constraint::Percentage(40),                                    // detail pane
        ])
        .split(content);

    let graph_area = columns[0];
    let list_area = columns[1];
    let detail_area = columns[2];

    if let Some(message) = state.load_error {
        // Which pane lost its content decides where the error state goes: a
        // failed list has no rows at all, a failed detail keeps them.
        if state.commits.is_empty() {
            let title = named_title("提交历史", state.repo_name, 0);
            state_panel::render_state_panel(f, list_area, &title, &PanelState::Error { message });
            return;
        }
        let title = detail_pane_title(&state);
        state_panel::render_state_panel(f, detail_area, &title, &PanelState::Error { message });
        render_commit_list(f, list_area, &mut state);
        render_graph(f, graph_area, &state);
        return;
    }

    render_commit_list(f, list_area, &mut state);
    render_graph(f, graph_area, &state);
    render_detail(f, detail_area, &state);
}

/// Footer hint row for the history view (§2.3): list navigation, then the
/// two keys that return to the repo view.
pub(crate) fn history_hints() -> HintRow {
    HintRow::new(join_hints([hint("j/k", "选择"), hint("q/Esc", "返回")]))
}

/// Commit list, or the empty-state copy for a repository without history.
///
/// An unborn HEAD loads zero commits, which is a valid state rather than a
/// failure: the error status (rendered by the caller) covers failed loads.
fn render_commit_list(f: &mut Frame, area: Rect, state: &mut HistoryRender<'_>) {
    let width = pulls::row_width(area);
    let items: Vec<ListItem> = state
        .commits
        .iter()
        .map(|commit| commit_item(commit, width))
        .collect();
    super::render_named_list(
        f,
        area,
        super::NamedPanel {
            label: "提交历史",
            repo_name: state.repo_name,
            empty: EMPTY_HISTORY,
            focused: state.focused,
        },
        items,
        state.list_state,
    );
}

/// One commit row: short hash, summary, ref badges, then author and time.
fn commit_item(commit: &CommitRow, width: usize) -> ListItem<'static> {
    let mut spans = vec![
        Span::styled(commit.id.clone(), Style::default().fg(Color::Yellow)),
        Span::raw(" "),
        Span::raw(pulls::ellipsize(&commit.summary, width)),
    ];
    for mark in &commit.refs {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            format!("[{}]", mark.name),
            ref_style(mark.kind),
        ));
    }
    spans.push(Span::styled(
        format!("  {} {}", commit.author, commit.time),
        Style::default().fg(Color::DarkGray),
    ));
    ListItem::new(Line::from(spans))
}

/// Badge style per reference kind: HEAD is highlighted, branches read green
/// and tags cyan, matching the colors the other panels use for refs.
fn ref_style(kind: RefKind) -> Style {
    match kind {
        RefKind::Head => Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        RefKind::Branch => Style::default().fg(Color::Green),
        RefKind::Tag => Style::default().fg(Color::Cyan),
    }
}

/// DAG column: one row per commit, laid out in lockstep with the commit
/// list. The git layer already positions every glyph at its lane, so a row
/// only has to be joined into a line (G8).
fn render_graph(f: &mut Frame, area: Rect, state: &HistoryRender<'_>) {
    let lines: Vec<Line> = state
        .graph
        .iter()
        .map(|row| Line::from(row.iter().collect::<String>()))
        .collect();

    // The DAG column is a context pane, never the focus target, so it wears
    // the muted border unconditionally (§3.1).
    let paragraph = Paragraph::new(lines)
        .block(
            Block::default()
                .title("DAG")
                .borders(Borders::ALL)
                .border_style(role::border(false)),
        )
        .scroll((state.list_state.offset() as u16, 0));
    f.render_widget(paragraph, area);
}

/// Detail pane: the complete commit message soft-wrapped, followed by the
/// list of paths the commit changed.
fn render_detail(f: &mut Frame, area: Rect, state: &HistoryRender<'_>) {
    // The detail pane is the screen's preview panel, so it wears
    // `border_focus` exactly while it holds the focus (§3.1).
    let block = Block::default()
        .title(detail_pane_title(state))
        .borders(Borders::ALL)
        .border_style(role::border(state.focused));

    let paragraph = match state.detail {
        None => Paragraph::new(Span::styled(
            "选择一个提交查看详情",
            Style::default().fg(Color::DarkGray),
        ))
        .block(block),
        Some(detail) => {
            let mut lines: Vec<Line> = detail
                .message
                .lines()
                .map(|line| Line::from(line.to_string()))
                .collect();
            if !lines.is_empty() {
                lines.push(Line::from(""));
            }
            lines.push(Line::from(Span::styled(
                format!("变更文件 ({})", detail.files.len()),
                Style::default().fg(Color::Cyan),
            )));
            for file in &detail.files {
                lines.push(Line::from(format!("  {file}")));
            }
            Paragraph::new(lines)
                .block(block)
                .wrap(Wrap { trim: false })
        }
    };
    f.render_widget(paragraph, area);
}

/// Title of the detail pane: `详情 [id]` while a commit is highlighted,
/// `详情` otherwise. The `error` state of a failed detail load reuses it, so
/// the pane it replaces keeps its name.
fn detail_pane_title(state: &HistoryRender<'_>) -> String {
    match selected_commit(state) {
        Some(commit) => format!("详情 [{}]", commit.id),
        None => "详情".to_string(),
    }
}

/// The highlighted commit: the index `App` passed in, when it still points
/// inside the loaded list (a stale index yields the same `None` as an empty
/// view). `HistoryState::select` keeps the index and the render cursor in
/// sync, so the renderer reads the one source instead of re-deriving it from
/// `list_state` (G9).
fn selected_commit<'a>(state: &HistoryRender<'a>) -> Option<&'a CommitRow> {
    state.commits.get(state.selected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::HistoryState;
    use crate::git::history::RefMark;
    use crate::ui::components::footer::StatusRow;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// Single-commit state carrying every element the layout renders: a
    /// two-lane graph row, all three reference kinds and a detail payload
    /// with a body plus changed files.
    fn loaded_state() -> HistoryState {
        let mut state = HistoryState::default();
        // A positioned row exactly as `git::layout_graph` emits it: the dot
        // in lane 0, the vertical in lane 1, every other lane a blank.
        let mut graph_row = vec![' '; GRAPH_WIDTH];
        graph_row[0] = '●';
        graph_row[1] = '│';
        state.set_commits(
            vec![CommitRow {
                full_id: "0123456789abcdef0123456789abcdef01234567".to_string(),
                id: "abc1234".to_string(),
                summary: "merge things".to_string(),
                author: "GitPulse Test".to_string(),
                time: "2023-11-14 22:18".to_string(),
                refs: vec![
                    RefMark {
                        kind: RefKind::Head,
                        name: "HEAD".to_string(),
                    },
                    RefMark {
                        kind: RefKind::Branch,
                        name: "main".to_string(),
                    },
                    RefMark {
                        kind: RefKind::Tag,
                        name: "v1.0.0".to_string(),
                    },
                ],
                parents: vec![],
            }],
            vec![graph_row],
        );
        state.detail = Some(CommitDetail {
            message: "merge things\n\nBody of the commit.".to_string(),
            files: vec!["src/main.rs".to_string(), "src/lib.rs".to_string()],
        });
        state
    }

    /// Render one frame at 120 columns — wide enough for the badges and the
    /// author/time tail to fit on the selected row — and return the
    /// whitespace-stripped buffer text (wide CJK glyphs leave padding cells).
    fn render_to_text(state: HistoryState) -> String {
        render_to_text_with(state, None)
    }

    /// Same frame with a load failure in play, so a test can place the
    /// panel's `error` state (§4.3).
    fn render_to_text_with(state: HistoryState, load_error: Option<&str>) -> String {
        let hints = history_hints();
        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let mut state = state;
                crate::ui::components::footer::draw_frame(
                    f,
                    StatusRow::plain(None),
                    &hints,
                    |f, body| {
                        let render = HistoryRender {
                            commits: &state.commits,
                            detail: state.detail.as_ref(),
                            graph: &state.graph,
                            list_state: &mut state.list_state,
                            selected: state.selected,
                            repo_name: "demo",
                            load_error,
                            focused: true,
                        };
                        render_history(f, body, render)
                    },
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

    #[test]
    fn render_shows_dag_glyph_ref_badges_and_detail() {
        let text = render_to_text(loaded_state());
        assert!(text.contains("abc1234"), "missing short hash: {text}");
        assert!(text.contains('●'), "missing DAG glyph: {text}");
        assert!(text.contains("[HEAD]"), "missing HEAD badge: {text}");
        assert!(text.contains("[main]"), "missing branch badge: {text}");
        assert!(text.contains("[v1.0.0]"), "missing tag badge: {text}");
        assert!(
            text.contains("Bodyofthecommit."),
            "missing wrapped message body: {text}"
        );
        assert!(text.contains("变更文件(2)"), "missing file header: {text}");
        assert!(text.contains("src/main.rs"), "missing changed path: {text}");
        assert!(text.contains("j/k选择"), "missing nav hint: {text}");
    }

    #[test]
    fn render_empty_history_shows_empty_state_in_every_pane() {
        let text = render_to_text(HistoryState::default());
        assert!(text.contains("提交历史[demo](0)"), "missing title: {text}");
        assert!(text.contains("暂无提交记录"), "missing empty state: {text}");
        assert!(
            text.contains("选择一个提交查看详情"),
            "missing detail placeholder: {text}"
        );
        assert!(text.contains("q/Esc返回"), "missing back hint: {text}");
    }

    /// A loaded list whose detail could not be attached (failed reload)
    /// keeps the rows and falls back to the placeholder pane.
    #[test]
    fn render_list_without_detail_shows_placeholder() {
        let mut state = loaded_state();
        state.detail = None;
        let text = render_to_text(state);
        assert!(text.contains("abc1234"), "missing commit row: {text}");
        assert!(
            text.contains("选择一个提交查看详情"),
            "missing detail placeholder: {text}"
        );
        assert!(!text.contains("变更文件"), "stale detail: {text}");
    }
}
