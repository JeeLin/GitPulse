//! Push/pull feedback rendered on the single-repo status line.

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    widgets::{Gauge, Paragraph},
    Frame,
};

use crate::app::RemoteStatus;

/// Render push/pull feedback on the status line (convention 3): a yellow
/// gauge while running, green on success, and the error reason in red on
/// failure (persisting until the next operation).
pub fn render_remote_status(f: &mut Frame, area: Rect, status: &RemoteStatus, op: &str) {
    match status {
        RemoteStatus::Progress(progress) => {
            let gauge = Gauge::default()
                .gauge_style(Style::default().fg(Color::Yellow))
                .percent(progress.percent as u16)
                .label(format!("{} ({}%)", progress.message, progress.percent));
            f.render_widget(gauge, area);
        }
        RemoteStatus::Success => {
            let text = if op.is_empty() {
                "操作完成".to_string()
            } else {
                format!("{op} 成功")
            };
            f.render_widget(
                Paragraph::new(text).style(Style::default().fg(Color::Green)),
                area,
            );
        }
        RemoteStatus::Error(err) => {
            f.render_widget(
                Paragraph::new(err.clone()).style(Style::default().fg(Color::Red)),
                area,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Progress;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn render_to_text(status: &RemoteStatus, op: &str) -> String {
        let backend = TestBackend::new(80, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_remote_status(f, f.area(), status, op))
            .unwrap();
        // Wide CJK glyphs leave padding cells; compare whitespace-stripped.
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
    fn render_progress_shows_message_and_percent() {
        let status = RemoteStatus::Progress(Progress {
            percent: 0,
            message: "正在 push…".to_string(),
        });
        let text = render_to_text(&status, "push");
        assert!(text.contains("正在push…"), "missing message: {text}");
        assert!(text.contains("0%"), "missing percent: {text}");
    }

    #[test]
    fn render_success_names_the_operation() {
        let text = render_to_text(&RemoteStatus::Success, "push");
        assert!(text.contains("push成功"), "missing success: {text}");
    }

    #[test]
    fn render_error_shows_reason() {
        let status = RemoteStatus::Error("pull 失败: diverged".to_string());
        let text = render_to_text(&status, "pull");
        assert!(text.contains("diverged"), "missing reason: {text}");
    }
}
