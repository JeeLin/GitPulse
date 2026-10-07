//! CI checks receipts panel: the head-commit check runs of the selected pull.

use ratatui::{layout::Rect, widgets::ListItem, Frame};

use crate::app::App;

use super::pulls::{
    check_line, render_disconnected, render_load_error, render_receipts_empty,
    render_receipts_loading, render_receipts_rows, row_width,
};

/// Empty-state copy: no selected pull, or a pull without check runs. Zero
/// runs is a valid state — a failed load paints the panel's `error` state.
const EMPTY_CHECKS: &str = "—（暂无检查记录）";

/// Render the head-commit check runs for the selected pull request.
///
/// The panel draws its content only (§4.1): the status row and the hint row
/// are both painted once by the footer.
pub fn render_checks(f: &mut Frame, content: Rect, app: &App) {
    if let Some(message) = app.load_error.as_deref() {
        render_load_error(
            f,
            content,
            "CI 检查",
            app.github_state.owner(),
            app.github_state.repo(),
            message,
        );
        return;
    }

    if !app.github_state.connected() {
        render_disconnected(f, content, "CI 检查", app.github_state.kind(), app);
        return;
    }

    if app.github_loading() {
        render_receipts_loading(f, content, "CI 检查", app, "正在拉取检查记录…");
        return;
    }

    let checks = &app.github_state.checks;
    if checks.is_empty() {
        render_receipts_empty(
            f,
            content,
            "CI 检查",
            app,
            EMPTY_CHECKS,
            "选中的 PR 没有检查记录",
        );
    } else {
        let width = row_width(content);
        let items = checks
            .iter()
            .map(|c| ListItem::new(check_line(c, width)))
            .collect();
        render_receipts_rows(f, content, "CI 检查", app, items, app.github_state.selected);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::config::Config;
    use crate::platform::CheckSummary;
    use crate::ui::components::theme::color;
    use crate::ui::pulls::{draw_receipt_frame, offline_client};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// Connected app: the `octo/demo` remote resolved and the readiness gate
    /// built, so the panel renders the real title instead of the `—` fallback.
    fn connected_app() -> App {
        let mut app = App::new(Config::default());
        app.github_state.remote = Some(crate::ui::github_remote());
        app.github_state.client = Some(offline_client());
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
        render_text_sized(app, 80)
    }

    fn render_text_sized(app: &App, width: u16) -> String {
        compact_text(&render_buffer_sized(app, width))
    }

    /// Whitespace-stripped text of the whole frame (wide CJK glyphs leave
    /// padding cells).
    fn compact_text(buffer: &ratatui::buffer::Buffer) -> String {
        let mut raw = String::new();
        for cell in buffer.content() {
            raw.push_str(cell.symbol());
        }
        raw.chars().filter(|c| !c.is_whitespace()).collect()
    }

    /// The rendered frame, so a test can address the single row an async
    /// result landed on (§4.1).
    fn render_buffer_sized(app: &App, width: u16) -> ratatui::buffer::Buffer {
        let backend = TestBackend::new(width, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| draw_receipt_frame(f, app, false, |f, body| render_checks(f, body, app)))
            .unwrap();
        terminal.backend().buffer().clone()
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
        let buffer = render_buffer_sized(&app, 80);
        let text = compact_text(&buffer);
        // No resolved remote means `kind == None`, which falls back to the
        // GitHub gate copy, verbatim as in v0.5.0.
        assert!(text.contains("尚未连接GitHub"), "missing gate copy: {text}");
        assert!(!text.contains("unit"), "no rows without a client: {text}");
        assert_eq!(
            crate::ui::pulls::rows_with(&buffer, "解析远端失败:未配置远端"),
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
