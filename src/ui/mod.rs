//! TUI rendering: view dispatch shared by all screens.

pub mod dashboard;
pub mod diff;
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
            let mut state = DashboardState {
                repos: &app.repos,
                status: app.status.as_ref(),
                list_state: &mut app.dashboard_state,
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
            };
            repo::render_repo(f, state);
        }
        View::Diff => {
            diff::render_diff(f, app);
        }
        View::Commit | View::Branches => {}
    }
}
