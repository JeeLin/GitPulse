//! TUI rendering: view dispatch shared by all screens.

pub mod branches;
pub mod commit;
pub mod config;
pub mod dashboard;
pub mod diff;
pub mod remote;
pub mod repo;

use ratatui::Frame;

use crate::app::{App, View};
use crate::ui::dashboard::DashboardState;

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
            // Owned copy: `scan_dirs()` borrows all of `app`, which would
            // conflict with the `&mut app.config_state` below.
            let scan_dirs = app.scan_dirs().to_vec();
            let state = config::ConfigState {
                scan_dirs: &scan_dirs,
                status: app.status.as_ref(),
                list_state: &mut app.config_state,
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
        assert!(text.contains("d删除"), "missing delete hint: {text}");
        assert!(text.contains("Esc返回"), "missing back hint: {text}");
    }
}
