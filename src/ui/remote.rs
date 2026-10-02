//! Push/pull feedback rendered on the single-repo status line.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::Line,
    widgets::Paragraph,
    Frame,
};

use crate::ui::status::{RemoteStatus, StatusLine};

/// Blank columns reserved between the remote feedback and the pending
/// notice: two cells keep a long feedback label from touching the notice text.
const NOTICE_GAP: u16 = 2;

/// Split the status row into the remote feedback cell and — when a notice is
/// pending — the cell that renders it beside the feedback.
///
/// The notice keeps its own columns up front so the feedback cannot be laid
/// out over them; a long notice is clipped at half the row rather than
/// squeezing the feedback out of view. With no notice pending the whole row
/// is handed back as-is.
fn split_for_notice(
    area: Rect,
    pending: Option<&StatusLine>,
) -> (Rect, Option<(Rect, &StatusLine)>) {
    let Some(note) = pending else {
        return (area, None);
    };
    // Measure the notice by display width without cloning its text.
    let cols = (Line::from(note.message.as_str()).width() as u16).min(area.width / 2);
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(1),             // remote feedback
            Constraint::Length(NOTICE_GAP), // separator
            Constraint::Length(cols),       // pending notice
        ])
        .split(area);
    let feedback_area = chunks[0];
    let notice_area = chunks[2];
    (feedback_area, Some((notice_area, note)))
}

/// The one line of a (possibly multi-line) git failure worth showing on the
/// single-line status row: the first line git marked as the failure
/// (`error:` / `fatal:`), falling back to the first line of the message —
/// a bare reason has no marker and stays verbatim (H4).
fn error_key_line(err: &str) -> &str {
    let first = err.lines().next().unwrap_or(err);
    err.lines()
        .map(str::trim_start)
        .find(|line| line.starts_with("error:") || line.starts_with("fatal:"))
        .unwrap_or(first)
}

/// Render push/pull feedback on the status line (convention 3): a yellow
/// running line while the operation works, green on success, and the error
/// reason in red on failure (persisting until the next operation).
///
/// `pending` is the plain status line written while the operation runs —
/// e.g. the `已有远端操作进行中` notice a rejected second push records. It used
/// to be swallowed whole by the feedback; all three states now split the row
/// through [`split_for_notice`] so the notice shares it in every terminal
/// state (F1, G2), and a multi-line git failure shows only its key line
/// (H4).
///
/// While running the row carries the operation's text instead of a gauge
/// (G1): the git layer stopped emitting progress percentages when
/// `on_progress` was removed (R5), so the gauge was frozen at 0% — a half
/// dead link. The pending-notice split is unchanged (G2/F1).
pub fn render_remote_status(
    f: &mut Frame,
    area: Rect,
    status: &RemoteStatus,
    op: &str,
    pending: Option<&StatusLine>,
) {
    let (feedback_area, notice_area) = split_for_notice(area, pending);
    match status {
        RemoteStatus::Running(message) => {
            f.render_widget(
                Paragraph::new(message.as_str()).style(Style::default().fg(Color::Yellow)),
                feedback_area,
            );
        }
        RemoteStatus::Success => {
            let text = if op.is_empty() {
                "操作完成".to_string()
            } else {
                format!("{op} 成功")
            };
            f.render_widget(
                Paragraph::new(text).style(Style::default().fg(Color::Green)),
                feedback_area,
            );
        }
        RemoteStatus::Error(err) => {
            f.render_widget(
                Paragraph::new(error_key_line(err)).style(Style::default().fg(Color::Red)),
                feedback_area,
            );
        }
    }
    // Painted after the feedback so a long label can never cover it.
    if let Some((rect, note)) = notice_area {
        super::render_status_line(f, rect, Some(note));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    /// G1: while running the row carries the operation's text — the
    /// percentage is gone with the gauge, because there is no progress
    /// source left to report (R5 removed the callback, so any percent was a
    /// frozen 0%).
    #[test]
    fn render_running_shows_message_without_dead_percent() {
        let status = RemoteStatus::Running("正在 push…".to_string());
        let text = render_to_text(&status, "push", None);
        assert!(text.contains("正在push…"), "missing message: {text}");
        assert!(
            !text.contains('%'),
            "frozen progress percent must be gone: {text}"
        );
    }

    /// F1/G1: a notice recorded while the operation runs must share the row
    /// instead of being swallowed by it — the running message stays verbatim
    /// and the notice renders beside it.
    #[test]
    fn render_running_shows_pending_notice_beside_it() {
        let status = RemoteStatus::Running("正在 push…".to_string());
        let pending = StatusLine {
            message: "已有远端操作进行中".to_string(),
            is_error: false,
        };
        let text = render_to_text(&status, "push", Some(&pending));
        assert!(text.contains("正在push…"), "missing message: {text}");
        assert!(
            text.contains("已有远端操作进行中"),
            "notice hidden by the running line: {text}"
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

    /// G2: a notice written after the operation settled must share the row
    /// with the terminal feedback instead of being swallowed by it — the
    /// Success/Error rows now split exactly like the running row (F1).
    #[test]
    fn render_success_shows_pending_notice_beside_it() {
        let pending = StatusLine {
            message: "已删除标签 v1.0".to_string(),
            is_error: false,
        };
        let text = render_to_text(&RemoteStatus::Success, "push", Some(&pending));
        assert!(text.contains("push成功"), "missing success: {text}");
        assert!(
            text.contains("已删除标签v1.0"),
            "notice swallowed by the success row: {text}"
        );
    }

    #[test]
    fn render_error_shows_pending_notice_beside_it() {
        let pending = StatusLine {
            message: "刷新失败".to_string(),
            is_error: true,
        };
        let status = RemoteStatus::Error("pull 失败: diverged".to_string());
        let text = render_to_text(&status, "pull", Some(&pending));
        assert!(text.contains("diverged"), "missing reason: {text}");
        assert!(
            text.contains("刷新失败"),
            "notice swallowed by the error row: {text}"
        );
    }

    /// G24: the notice keeps two blank columns between itself and the
    /// feedback, whatever the notice length — one column let a long feedback
    /// label touch the notice text.
    #[test]
    fn split_for_notice_keeps_two_column_gap() {
        let area = Rect::new(0, 0, 80, 1);
        for message in ["N", "提示", "已有远端操作进行中"] {
            let pending = StatusLine {
                message: message.to_string(),
                is_error: false,
            };
            let (feedback, notice) = split_for_notice(area, Some(&pending));
            let (notice_area, _) = notice.expect("notice cell must be reserved");
            assert_eq!(
                notice_area.x,
                feedback.right() + NOTICE_GAP,
                "gap must be {NOTICE_GAP} columns for {message:?}"
            );
            // The notice is clipped at half the row, never squeezed away.
            assert!(notice_area.width >= 1, "notice must stay visible");
        }
        // No pending notice: the whole row belongs to the feedback.
        let (feedback, notice) = split_for_notice(area, None);
        assert_eq!(feedback, area);
        assert!(notice.is_none(), "no cell is reserved without a notice");
    }

    /// H4: the row is one column high, so a multi-line git failure keeps only
    /// its key line — the first one git flagged as the failure.
    #[test]
    fn render_error_shows_key_line_of_multiline_stderr() {
        let status = RemoteStatus::Error(
            "push 失败: To origin\n ! [rejected] main -> main (non-fast-forward)\nerror: failed to push some refs to 'origin'\nhint: integrate the remote changes".to_string(),
        );
        let text = render_to_text(&status, "push", None);
        assert!(text.contains("failedtopush"), "key line missing: {text}");
        assert!(
            !text.contains("non-fast-forward"),
            "only the key line belongs on one row: {text}"
        );
        assert!(!text.contains("hint:"), "hint line must be dropped: {text}");
    }

    /// A `fatal:` failure has no `error:` line; it is the key line itself.
    #[test]
    fn render_error_falls_back_to_fatal_line() {
        let status =
            RemoteStatus::Error("fatal: refusing to merge unrelated histories\n".to_string());
        let text = render_to_text(&status, "pull", None);
        assert!(
            text.contains("refusingtomerge"),
            "fatal line missing: {text}"
        );
    }

    /// A single-line reason carries no marker: it renders verbatim (H4).
    #[test]
    fn render_error_keeps_bare_single_line_reason() {
        let status = RemoteStatus::Error("未配置远端".to_string());
        let text = render_to_text(&status, "push", None);
        assert!(text.contains("未配置远端"), "reason changed: {text}");
    }
}
