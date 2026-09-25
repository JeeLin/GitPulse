//! TUI rendering: view dispatch shared by all screens.

pub mod branches;
pub mod commit;
pub mod config;
pub mod dashboard;
pub mod diff;
pub mod remote;
pub mod repo;

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::app::{App, StatusLine, View};
use crate::ui::dashboard::DashboardState;

/// Shared selection style for every list: dark background + bold keeps the
/// cursor row readable on any block style.
pub(crate) fn list_highlight() -> Style {
    Style::default()
        .bg(Color::DarkGray)
        .add_modifier(Modifier::BOLD)
}

/// Marker rendered before the selected list row.
pub(crate) const HIGHLIGHT_SYMBOL: &str = "► ";

/// Draw the bottom status line shared by every view (convention 3): errors
/// render red and persist until the next operation, anything else keeps the
/// default style. `None` leaves the row blank.
pub fn render_status_line(f: &mut Frame, area: Rect, status: Option<&StatusLine>) {
    if let Some(status) = status {
        let style = if status.is_error {
            Style::default().fg(Color::Red)
        } else {
            Style::default()
        };
        f.render_widget(Paragraph::new(status.message.clone()).style(style), area);
    }
}

/// Shared y/n confirmation overlay (convention 8): a centered red-bordered
/// popup titled `title`, showing the caller's `body` lines above the standard
/// `y/n` hint row. `percent_x`/`height` size the popup; `wrap` soft-wraps
/// long target names (discard dialog) instead of clipping them.
pub fn render_confirm_overlay(
    f: &mut Frame,
    area: Rect,
    title: &str,
    percent_x: u16,
    height: u16,
    body: Vec<Line<'static>>,
    wrap: bool,
) {
    let popup = repo::centered_rect(percent_x, height, area);
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::Red));

    let mut lines = body;
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("y", Style::default().fg(Color::Cyan)),
        Span::raw(" 确认  "),
        Span::styled("n", Style::default().fg(Color::Cyan)),
        Span::raw(" 取消"),
    ]));

    let paragraph = Paragraph::new(lines).block(block);
    let paragraph = if wrap {
        paragraph.wrap(Wrap { trim: false })
    } else {
        paragraph
    };
    f.render_widget(Clear, popup);
    f.render_widget(paragraph, popup);
}

/// Shared single-line input overlay (convention 9): a centered popup titled
/// `title` showing `text` after a cyan `> ` prompt. `show_cursor` appends the
/// `▌` caret (config manual input); popups whose hints live in the title
/// (branch create, commit message) omit it.
pub fn render_input_overlay(f: &mut Frame, area: Rect, title: &str, text: &str, show_cursor: bool) {
    let popup = repo::centered_rect(60, 5, area);
    f.render_widget(Clear, popup);

    let mut spans = vec![
        Span::styled("> ", Style::default().fg(Color::Cyan)),
        Span::raw(text),
    ];
    if show_cursor {
        spans.push(Span::styled("▌", Style::default().fg(Color::Cyan)));
    }
    let block = Block::default().title(title).borders(Borders::ALL);
    f.render_widget(Paragraph::new(Line::from(spans)).block(block), popup);
}

/// Dispatch rendering to the active view.
///
/// The current [`App`] fields are borrowed directly into the view state
/// (checkpoint 2: no separate mock data source).
pub fn render(f: &mut Frame, app: &mut App) {
    match app.view {
        View::Dashboard => {
            let batch_running = app.batch_running();
            let mut state = DashboardState {
                repos: &app.repos,
                status: app.status.as_ref(),
                list_state: &mut app.dashboard_state,
                batch_log: &app.batch_log,
                batch_op: app.batch_op,
                batch_running,
            };
            dashboard::render_dashboard(f, &mut state);
        }
        View::Repo => {
            let sel = app.file_state.selected();
            let confirm_file = if app.show_discard_confirm {
                sel.and_then(|i| app.file_entries.get(i))
            } else {
                None
            };
            let state = repo::RepoState {
                entries: &app.file_entries,
                status: app.status.as_ref(),
                list_state: &mut app.file_state,
                show_discard_confirm: app.show_discard_confirm,
                confirm_file,
                remote_status: app.remote_status.as_ref(),
                remote_op: app.remote_op,
            };
            repo::render_repo(f, state);
        }
        View::Diff => {
            diff::render_diff(f, app);
        }
        View::Commit => {
            // Repo view as background, commit input popup on top.
            {
                let state = repo::RepoState {
                    entries: &app.file_entries,
                    status: app.status.as_ref(),
                    list_state: &mut app.file_state,
                    show_discard_confirm: false,
                    confirm_file: None,
                    remote_status: app.remote_status.as_ref(),
                    remote_op: app.remote_op,
                };
                repo::render_repo(f, state);
            }
            commit::render_commit_popup(f, &app.commit_message);
        }
        View::Branches => {
            let confirm = if app.show_branch_delete_confirm {
                app.branch_state
                    .selected()
                    .and_then(|i| app.branches.get(i))
            } else {
                None
            };
            let repo_name = app.branch_repo_name().to_string();
            let state = branches::BranchesState {
                branches: &app.branches,
                status: app.status.as_ref(),
                list_state: &mut app.branch_state,
                repo_name: &repo_name,
                show_input: app.show_branch_input,
                input: &app.branch_input,
                show_delete_confirm: app.show_branch_delete_confirm,
                confirm_branch: confirm,
            };
            branches::render_branches(f, state);
        }
        View::Config => {
            // Field-level borrow of `app.config` coexists with the
            // `&mut app.config_state` below (disjoint fields).
            let state = config::ConfigState {
                scan_dirs: &app.config.scan_dirs,
                status: app.status.as_ref(),
                list_state: &mut app.config_state,
                show_input: app.show_config_input,
                input: &app.config_input,
                show_delete_confirm: app.show_config_delete_confirm,
                show_picker: app.show_config_picker,
                picker_dir: &app.picker_dir,
                picker_entries: &app.picker_entries,
                picker_state: &mut app.picker_state,
            };
            config::render_config(f, state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::git::{BranchInfo, FileEntry, FileState};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn base_app() -> App {
        App::new(Config::default())
    }

    /// Render one frame through the view dispatcher and return the
    /// whitespace-stripped buffer text (wide CJK glyphs leave padding cells).
    fn render_compact(app: &mut App) -> String {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, app)).unwrap();
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
    fn dashboard_view_renders_title_for_empty_repo_list() {
        let mut app = base_app();
        let text = render_compact(&mut app);
        assert!(
            text.contains("GitPulse仓库状态(0)"),
            "missing dashboard title: {text}"
        );
    }

    #[test]
    fn repo_view_renders_file_list_and_hints() {
        let mut app = base_app();
        app.view = View::Repo;
        app.file_entries = vec![FileEntry {
            path: std::path::PathBuf::from("src/main.rs"),
            state: FileState::Modified,
        }];
        app.file_state.select(Some(0));
        let text = render_compact(&mut app);
        assert!(text.contains("文件状态(1文件)"), "missing title: {text}");
        assert!(text.contains("Msrc/main.rs"), "missing entry: {text}");
        assert!(text.contains("Esc返回"), "missing hints: {text}");
    }

    #[test]
    fn repo_view_shows_discard_confirm_overlay() {
        let mut app = base_app();
        app.view = View::Repo;
        app.file_entries = vec![FileEntry {
            path: std::path::PathBuf::from("src/main.rs"),
            state: FileState::Modified,
        }];
        app.file_state.select(Some(0));
        app.show_discard_confirm = true;
        let text = render_compact(&mut app);
        assert!(
            text.contains("确认丢弃更改"),
            "missing dialog title: {text}"
        );
        assert!(text.contains("y确认"), "missing confirm key: {text}");
    }

    #[test]
    fn diff_view_renders_content_and_title() {
        let mut app = base_app();
        app.view = View::Diff;
        app.diff_content = "+added_line\n-removed_line\n".to_string();
        let text = render_compact(&mut app);
        assert!(text.contains("文件差异"), "missing title: {text}");
        assert!(text.contains("+added_line"), "missing added line: {text}");
        assert!(
            text.contains("-removed_line"),
            "missing removed line: {text}"
        );
    }

    #[test]
    fn commit_view_renders_popup_over_repo_background() {
        let mut app = base_app();
        app.view = View::Commit;
        app.commit_message = "feat: hello".to_string();
        let text = render_compact(&mut app);
        assert!(text.contains("提交信息"), "missing popup title: {text}");
        assert!(text.contains("feat:hello"), "missing message: {text}");
    }

    #[test]
    fn branches_view_renders_empty_list_with_title_and_hints() {
        let mut app = base_app();
        app.view = View::Branches;
        let text = render_compact(&mut app);
        // No dedicated empty-state copy: the title carries the zero count.
        assert!(text.contains("分支[](0)"), "missing empty title: {text}");
        assert!(text.contains("n新建"), "missing create hint: {text}");
        assert!(text.contains("Esc返回"), "missing back hint: {text}");
    }

    #[test]
    fn branches_view_shows_delete_confirm_overlay() {
        let mut app = base_app();
        app.view = View::Branches;
        app.branches = vec![BranchInfo {
            name: "feature".to_string(),
            is_head: false,
            upstream: None,
        }];
        app.branch_state.select(Some(0));
        app.show_branch_delete_confirm = true;
        let text = render_compact(&mut app);
        assert!(
            text.contains("确认删除分支"),
            "missing dialog title: {text}"
        );
        assert!(text.contains("feature"), "missing branch name: {text}");
    }

    #[test]
    fn config_view_renders_title_and_hints_via_dispatch() {
        let mut app = base_app();
        app.view = View::Config;
        let text = render_compact(&mut app);
        // Default config has no scan dirs: the title carries the zero count.
        assert!(
            text.contains("扫描目录配置(0)"),
            "missing config title: {text}"
        );
        assert!(text.contains("j/k选择"), "missing nav hint: {text}");
        assert!(text.contains("a添加"), "missing add hint: {text}");
        assert!(text.contains("i手动"), "missing manual hint: {text}");
        assert!(text.contains("d删除"), "missing delete hint: {text}");
        assert!(text.contains("Esc返回"), "missing back hint: {text}");
    }

    #[test]
    fn config_view_input_overlay_renders_via_dispatch() {
        let mut app = base_app();
        app.view = View::Config;
        app.show_config_input = true;
        app.config_input = "/home/user/repos".to_string();
        let text = render_compact(&mut app);
        assert!(text.contains("添加扫描目录"), "missing input title: {text}");
        assert!(
            text.contains(">/home/user/repos"),
            "missing typed path: {text}"
        );
        assert!(text.contains("Enter确认"), "missing confirm hint: {text}");
        assert!(text.contains("Esc取消"), "missing cancel hint: {text}");
    }

    #[test]
    fn config_view_delete_confirm_renders_via_dispatch() {
        let mut app = base_app();
        app.view = View::Config;
        app.config_state.select(None);
        app.show_config_delete_confirm = true;
        let text = render_compact(&mut app);
        assert!(text.contains("移除该目录"), "missing prompt: {text}");
        assert!(text.contains("y确认"), "missing confirm key: {text}");
        assert!(text.contains("n取消"), "missing cancel key: {text}");
    }

    #[test]
    fn config_view_picker_overlay_renders_via_dispatch() {
        let mut app = base_app();
        app.view = View::Config;
        app.show_config_picker = true;
        app.picker_dir = std::path::PathBuf::from("/home/user");
        app.picker_entries = vec![std::path::PathBuf::from("/home/user/code")];
        app.picker_state.select(Some(1));
        let text = render_compact(&mut app);
        assert!(text.contains("选择扫描目录"), "missing title: {text}");
        assert!(text.contains("↳选择当前目录"), "missing select row: {text}");
        assert!(text.contains("code/"), "missing subdir row: {text}");
        assert!(text.contains("/home/user"), "missing path line: {text}");
        assert!(text.contains("Enter选择"), "missing select hint: {text}");
        assert!(text.contains("Esc取消"), "missing cancel hint: {text}");
    }
}
