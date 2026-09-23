//! TUI rendering: view dispatch shared by all screens.

pub mod dashboard;

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
        // Repo/Diff/Commit/Branches land in M1 subtasks 5-7.
        View::Repo | View::Diff | View::Commit | View::Branches => {}
    }
}
