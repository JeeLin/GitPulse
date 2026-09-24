//! TUI rendering: view dispatch shared by all screens.

pub mod branches;
pub mod commit;
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
    }
}
