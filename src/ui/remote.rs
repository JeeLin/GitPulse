//! Push/pull feedback rendered on the single-repo status line.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::Line,
    widgets::{Gauge, Paragraph},
    Frame,
};

use crate::app::{RemoteStatus, StatusLine};

/// Render push/pull feedback on the status line (convention 3): a yellow
/// gauge while running, green on success, and the error reason in red on
/// failure (persisting until the next operation).
///
/// `pending` is the plain status line written while the operation runs —
/// e.g. the `已有远端操作进行中` notice a rejected second push records. It used
/// to be swallowed whole by the gauge; it now shares the running row: the
/// gauge keeps its label on the left, and the notice renders on the right in
/// the same style the plain status row would use. Success and failure rows
/// are untouched, and so is every row where no notice is pending.
pub fn render_remote_status(
    f: &mut Frame,
    area: Rect,
    status: &RemoteStatus,
    op: &str,
    pending: Option<&StatusLine>,
) {
    match status {
        RemoteStatus::Progress(progress) => {
            // Reserve the notice cells up front so the gauge cannot be laid
            // out over them; the notice is painted after the gauge so a long
            // progress label can never cover it. A long notice is clipped at
            // half the row rather than squeezing the gauge out of view.
            let (gauge_area, notice_area) = match pending {
                None => (area, None),
                Some(note) => {
                    let cols =
                        (Line::from(note.message.clone()).width() as u16).min(area.width / 2);
                    let chunks = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([
                            Constraint::Min(1),       // gauge
                            Constraint::Length(1),    // separator
                            Constraint::Length(cols), // pending notice
                        ])
                        .split(area);
                    (chunks[0], Some((chunks[2], note)))
                }
            };
            let gauge = Gauge::default()
                .gauge_style(Style::default().fg(Color::Yellow))
                .percent(progress.percent as u16)
                .label(format!("{} ({}%)", progress.message, progress.percent));
            f.render_widget(gauge, gauge_area);
            if let Some((rect, note)) = notice_area {
                super::render_status_line(f, rect, Some(note));
            }
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

    fn render_to_text(status: &RemoteStatus, op: &str, pending: Option<&StatusLine>) -> String {
        let backend = TestBackend::new(80, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_remote_status(f, f.area(), status, op, pending))
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
        let text = render_to_text(&status, "push", None);
        assert!(text.contains("正在push…"), "missing message: {text}");
        assert!(text.contains("0%"), "missing percent: {text}");
    }

    /// F1: a notice recorded while the gauge runs must share the row instead
    /// of being swallowed by it — the gauge keeps its own label verbatim.
    #[test]
    fn render_progress_shows_pending_notice_beside_gauge() {
        let status = RemoteStatus::Progress(Progress {
            percent: 42,
            message: "正在 push…".to_string(),
        });
        let pending = StatusLine {
            message: "已有远端操作进行中".to_string(),
            is_error: false,
        };
        let text = render_to_text(&status, "push", Some(&pending));
        assert!(text.contains("正在push…"), "missing message: {text}");
        assert!(text.contains("42%"), "missing percent: {text}");
        assert!(
            text.contains("已有远端操作进行中"),
            "notice hidden by gauge: {text}"
        );
    }

    #[test]
    fn render_success_names_the_operation() {
        let text = render_to_text(&RemoteStatus::Success, "push", None);
        assert!(text.contains("push成功"), "missing success: {text}");
    }

    #[test]
    fn render_error_shows_reason() {
        let status = RemoteStatus::Error("pull 失败: diverged".to_string());
        let text = render_to_text(&status, "pull", None);
        assert!(text.contains("diverged"), "missing reason: {text}");
    }
}
