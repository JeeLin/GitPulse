//! CI checks receipts panel: the head-commit check runs of the selected pull.

use ratatui::{layout::Rect, widgets::ListItem, Frame};

use crate::app::App;

use super::pulls::check_line;
use crate::ui::components::receipts::{render_receipts_list, ReceiptPanel};

/// Empty-state copy: no selected pull, or a pull without check runs. Zero
/// runs is a valid state — a failed load paints the panel's `error` state.
const EMPTY_CHECKS: &str = "—（暂无检查记录）";

/// Render the head-commit check runs for the selected pull request.
///
/// The panel draws its content only (§4.1): the status row and the hint row
/// are both painted once by the footer. The five-state dispatch is the
/// receipts spine's shared one, see [`crate::ui::components::receipts`].
pub fn render_checks(f: &mut Frame, content: Rect, app: &App) {
    let meta = ReceiptPanel {
        label: "CI 检查",
        load_sub: "正在拉取检查记录…",
        empty_copy: EMPTY_CHECKS,
        empty_description: "选中的 PR 没有检查记录",
    };
    let checks = &app.github_state.checks;
    render_receipts_list(f, content, meta, app, app.github_state.selected, |width| {
        checks
            .iter()
            .map(|c| ListItem::new(check_line(c, width)))
            .collect()
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::config::Config;
    use crate::platform::CheckSummary;
    use crate::ui::components::receipts;
    use crate::ui::components::theme::color;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// Connected app: the `octo/demo` remote resolved and the readiness gate
    /// built, so the panel renders the real title instead of the `—` fallback.
    fn connected_app() -> App {
        let mut app = App::new(Config::default());
        app.github_state.remote = Some(crate::ui::github_remote());
        app.github_state.client = Some(receipts::offline_client());
        app
    }

    fn check(name: &str, status: &str, conclusion: &str) -> CheckSummary {
        CheckSummary {
            name: name.to_string(),
            status: status.to_string(),
            conclusion: conclusion.to_string(),
            html_url: "https://example.invalid".to_string(),
        }
    }

    /// The panel plus its footer: the hint row the panel no longer draws
    /// reaches the buffer through [`crate::ui::components::footer`]
    /// (§2.3), so a test that skipped it would miss the row entirely.
    fn render_text(app: &App) -> String {
        receipts::compact(&receipt_buffer(app))
    }

    /// The rendered frame, so a test can address the single row an async
    /// result landed on (§4.1).
    fn receipt_buffer(app: &App) -> ratatui::buffer::Buffer {
        receipts::render_receipt_buffer(app, false, render_checks)
    }

    #[test]
    fn checks_list_shows_name_status_conclusion_and_title() {
        let mut app = connected_app();
        app.github_state.checks = vec![
            check("unit", "completed", "success"),
            check("e2e", "in_progress", ""),
            check("lint", "completed", "failure"),
        ];
        app.github_state.selected = 0;
        let text = render_text(&app);
        assert!(text.contains("CI检查·octo/demo"), "missing title: {text}");
        assert!(text.contains("unitcompletedsuccess"), "missing row: {text}");
        assert!(
            text.contains("e2ein_progress—"),
            "missing dash fill: {text}"
        );
        assert!(text.contains("lintcompletedfailure"), "missing row: {text}");
        assert!(text.contains("Esc返回"), "missing back hint: {text}");
    }

    #[test]
    fn checks_list_colors_verdicts_by_the_shared_palette() {
        let mut app = connected_app();
        app.github_state.checks = vec![check("unit", "completed", "failure")];
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render_checks(f, f.area(), &app)).unwrap();
        let buffer = terminal.backend().buffer();
        // Row 1 is the first item row (row 0 is the block's top border).
        let mut status_fg = None;
        let mut conclusion_fg = None;
        for x in 3..buffer.area.width {
            let cell = &buffer[(x, 1)];
            match cell.symbol() {
                "c" if status_fg.is_none() => status_fg = Some(cell.fg),
                "f" if conclusion_fg.is_none() => conclusion_fg = Some(cell.fg),
                _ => {}
            }
        }
        assert_eq!(
            status_fg,
            Some(color::CI_SKIP),
            "a settled run reads the skip token"
        );
        assert_eq!(conclusion_fg, Some(color::CI_FAIL), "failure reads danger");
    }

    #[test]
    fn checks_list_shows_empty_copy_for_zero_rows() {
        let app = connected_app();
        let text = render_text(&app);
        assert!(text.contains("—（暂无检查记录）"), "missing copy: {text}");
    }

    /// §4.1/§4.2: the readiness gate replaces the list, and the reason stays on
    /// the footer status row — the one landing spot for an async result.
    #[test]
    fn checks_disconnected_keeps_the_reason_on_the_footer_status_row() {
        let mut app = App::new(Config::default());
        app.set_error("解析远端失败: 未配置远端");
        app.github_state.checks = vec![check("unit", "completed", "success")];
        let buffer = receipt_buffer(&app);
        let text = receipts::compact(&buffer);
        // No resolved remote means `kind == None`, which falls back to the
        // GitHub gate copy, verbatim as in v0.5.0.
        assert!(text.contains("尚未连接GitHub"), "missing gate copy: {text}");
        assert!(!text.contains("unit"), "no rows without a client: {text}");
        assert_eq!(
            receipts::rows_with(&buffer, "解析远端失败:未配置远端"),
            vec![buffer.area.height - 2],
            "the reason belongs on the footer status row only: {text}"
        );
    }

    /// U4: the gate copy follows the remote's platform — a Gitea remote asks for
    /// `GITEA_TOKEN`, never the GitHub hint.
    #[test]
    fn checks_disconnected_uses_the_platform_of_the_resolved_remote() {
        let mut app = App::new(Config::default());
        app.github_state.remote =
            Some(crate::platform::parse_remote("https://gitea.com/octo/demo.git").unwrap());
        let text = render_text(&app);
        assert!(text.contains("尚未连接Gitea"), "missing gate copy: {text}");
        assert!(text.contains("GITEA_TOKEN"), "missing gitea hint: {text}");
        assert!(!text.contains("GITHUB_TOKEN"), "no github hint: {text}");
        assert!(
            text.contains("CI检查·octo/demo"),
            "title keeps the remote: {text}"
        );
    }
}
