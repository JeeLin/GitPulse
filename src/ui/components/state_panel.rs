//! The five-state body of every list panel (§3.2).
//!
//! Loading, empty and error are rendered here so no panel invents its own
//! wording or geometry for them; `ready` and `success-flash` hand the caller
//! the inner rect to fill with rows, because only the view knows its item
//! type. The panel keeps its border and title in every state — a loading or
//! failed panel still says which panel it is.

#![allow(dead_code)]

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{block::BorderType, Block, Borders, Paragraph},
    Frame,
};

use super::cells;
use super::theme::{color, role};

/// Static "spinning" glyph of the loading state. The terminal only repaints
/// on input (§4.4), so a rotating braille frame would sit still anyway —
/// one frame plus the label carries "in progress" honestly.
const SPINNER: &str = "⠋";

/// Lines of a message the error state shows before it gives up: three read
/// as one glance, the rest would turn the panel into a log (§3.2).
const ERROR_MAX_LINES: usize = 3;

/// Body of a list panel in one of the five states (§3.2).
///
/// A view picks the variant it is in and hands it to
/// [`render_state_panel`]; the panel text is caller-supplied so existing
/// empty-state copy (`暂无标签`, `暂无提交记录`, …) stays verbatim.
#[derive(Debug, Clone)]
pub enum PanelState<'a> {
    /// Centered spinner glyph + label, with an optional sub-label naming
    /// what is being loaded (`正在读取远端 PR…`).
    Loading {
        /// Short label: `加载中…`.
        label: &'a str,
        /// Optional second line describing the specific load.
        sub: Option<&'a str>,
    },
    /// Centered title + one-line description + the primary action that
    /// resolves the emptiness (`按 n 添加扫描目录`).
    Empty {
        /// What the panel would otherwise list: `还没有仓库`.
        title: &'a str,
        /// Why it is empty, one line.
        description: &'a str,
        /// Action prompt; styled by the caller so its key cap stands out.
        action: Line<'a>,
    },
    /// `danger` block titled `加载失败`: up to [`ERROR_MAX_LINES`] message
    /// lines (each ellipsized to the panel width) over `按 r 重试`.
    Error {
        /// Failure reason as the loader reported it.
        message: &'a str,
    },
    /// Content with a cursor: the caller fills the returned inner rect.
    Ready,
    /// `ready` plus the transient success flash — the same green line the
    /// status row shows, kept on the panel while the flash lasts (§3.2).
    Success {
        /// Result of the operation that just completed.
        message: &'a str,
    },
}

/// Draw `area` as a bordered panel titled `title` in state `state` and
/// return its inner rect.
///
/// The inner rect is what `ready` / `success-flash` callers fill with their
/// rows; the other three states are complete once this returns.
pub fn render_state_panel(f: &mut Frame, area: Rect, title: &str, state: &PanelState<'_>) -> Rect {
    let block = block_for(title, state);
    let inner = block.inner(area);

    match state {
        PanelState::Loading { label, sub } => {
            f.render_widget(block, area);
            render_centered(f, inner, loading_lines(label, *sub));
        }
        PanelState::Empty {
            title,
            description,
            action,
        } => {
            f.render_widget(block, area);
            render_centered(
                f,
                inner,
                vec![
                    Line::styled(title.to_string(), role::heading()),
                    Line::styled(description.to_string(), role::caption()),
                    Line::from(""),
                    action.clone(),
                ],
            );
        }
        PanelState::Error { message } => {
            f.render_widget(block, area);
            render_centered(f, inner, error_lines(message, inner.width as usize));
        }
        PanelState::Ready => {
            f.render_widget(block, area);
        }
        PanelState::Success { message } => {
            f.render_widget(block, area);
            let flash = vec![
                Line::styled(format!("✓ {message}"), Style::default().fg(color::SUCCESS)),
                Line::from(""),
            ];
            render_centered(f, inner, flash);
        }
    }
    inner
}

/// Bordered shell shared by every state: rounded corners and the title at
/// the top-left (§5.3). The error state repaints the whole shell `danger` and
/// is additionally labelled `加载失败` (PanelState::Error contract), while still
/// keeping the caller's panel title so the panel says which panel it is.
fn block_for(title: &str, state: &PanelState<'_>) -> Block<'static> {
    let is_error = matches!(state, PanelState::Error { .. });
    let title = if is_error {
        format!("{title} · 加载失败")
    } else {
        title.to_string()
    };
    let mut block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded);
    if is_error {
        block = block.style(Style::default().fg(color::DANGER));
    }
    block
}

/// Loading body: `⠋ 加载中…` with the optional sub-label below it.
fn loading_lines(label: &str, sub: Option<&str>) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(vec![
        Span::styled(SPINNER.to_string(), Style::default().fg(color::INFO)),
        Span::raw(" "),
        Span::styled(label.to_string(), role::caption()),
    ])];
    if let Some(sub) = sub {
        lines.push(Line::styled(sub.to_string(), role::caption()));
    }
    lines
}

/// Error body: the message cut to [`ERROR_MAX_LINES`] lines, each ellipsized
/// to the panel width, over the retry prompt (§3.2).
fn error_lines(message: &str, width: usize) -> Vec<Line<'static>> {
    let mut lines: Vec<Line> = message
        .lines()
        .take(ERROR_MAX_LINES)
        .map(|line| {
            Line::styled(
                cells::ellipsize(line, width),
                Style::default().fg(color::DANGER),
            )
        })
        .collect();
    if lines.is_empty() {
        lines.push(Line::styled(
            "加载失败".to_string(),
            Style::default().fg(color::DANGER),
        ));
    }
    lines.push(Line::from(""));
    lines.push(Line::styled("按 r 重试", role::caption()));
    lines
}

/// Render `lines` vertically centered inside `area`, each line clipped to
/// the area width — a panel too short for the block shows what fits.
fn render_centered(f: &mut Frame, area: Rect, lines: Vec<Line<'_>>) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let height = (lines.len() as u16).min(area.height);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(50),
            Constraint::Length(height),
            Constraint::Percentage(50),
        ])
        .split(area);
    let visible: Vec<Line> = lines.into_iter().take(height as usize).collect();
    f.render_widget(
        Paragraph::new(visible).alignment(ratatui::layout::Alignment::Center),
        rows[1],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// Render one state into an 80×12 frame and return the
    /// whitespace-stripped buffer text (wide CJK glyphs leave padding cells).
    fn render_text(title: &str, state: &PanelState<'_>) -> String {
        let backend = TestBackend::new(80, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render_state_panel(f, f.area(), title, state);
            })
            .unwrap();
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
    fn loading_state_shows_spinner_label_and_title() {
        let text = render_text(
            "PR 列表",
            &PanelState::Loading {
                label: "加载中…",
                sub: Some("正在读取远端 PR…"),
            },
        );
        assert!(text.contains("PR列表"), "missing title: {text}");
        assert!(text.contains('⠋'), "missing spinner glyph: {text}");
        assert!(text.contains("加载中…"), "missing label: {text}");
        assert!(text.contains("正在读取远端PR…"), "missing sub: {text}");
    }

    #[test]
    fn loading_state_without_a_sub_label_still_reads() {
        let text = render_text(
            "分支",
            &PanelState::Loading {
                label: "加载中…",
                sub: None,
            },
        );
        assert!(text.contains("加载中…"), "missing label: {text}");
    }

    /// The empty state reuses the panel's own copy — the same wording the
    /// views already print — plus the action that resolves it.
    #[test]
    fn empty_state_shows_copy_description_and_action() {
        let text = render_text(
            "标签 [demo] (0)",
            &PanelState::Empty {
                title: "暂无标签",
                description: "这个仓库还没有标签",
                action: Line::from("按 n 新建第一个标签"),
            },
        );
        assert!(text.contains("标签[demo](0)"), "missing title: {text}");
        assert!(text.contains("暂无标签"), "missing empty copy: {text}");
        assert!(
            text.contains("这个仓库还没有标签"),
            "missing description: {text}"
        );
        assert!(text.contains("按n新建第一个标签"), "missing action: {text}");
    }

    #[test]
    fn error_state_shows_reason_retry_hint_and_danger_shell() {
        let text = render_text(
            "PR 列表",
            &PanelState::Error {
                message: "GitHub 请求失败: 401",
            },
        );
        assert!(text.contains("加载失败"), "missing error title: {text}");
        assert!(
            text.contains("GitHub请求失败:401"),
            "missing reason: {text}"
        );
        assert!(text.contains("按r重试"), "missing retry hint: {text}");

        let backend = TestBackend::new(80, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render_state_panel(
                    f,
                    f.area(),
                    "PR 列表",
                    &PanelState::Error {
                        message: "GitHub 请求失败: 401",
                    },
                );
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        assert_eq!(
            buffer[(0, 0)].fg,
            color::DANGER,
            "the error shell is painted danger"
        );
    }

    /// §3.2: at most three message lines, the rest dropped entirely.
    #[test]
    fn error_state_caps_the_message_at_three_lines() {
        let message = "line one\nline two\nline three\nline four\nline five";
        let text = render_text("列表", &PanelState::Error { message });
        assert!(text.contains("lineone"), "missing first line: {text}");
        assert!(text.contains("linethree"), "missing third line: {text}");
        assert!(!text.contains("linefour"), "fourth line must be cut");
        assert!(!text.contains("linefive"), "fifth line must be cut");
        assert!(text.contains("按r重试"), "retry hint must survive: {text}");
    }

    #[test]
    fn ready_state_keeps_the_shell_and_returns_the_inner_rect() {
        let backend = TestBackend::new(80, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut inner = Rect::default();
        terminal
            .draw(|f| {
                inner = render_state_panel(f, f.area(), "分支 [demo] (2)", &PanelState::Ready);
            })
            .unwrap();
        assert_eq!(inner.x, 1, "inner starts after the border");
        assert_eq!(inner.y, 1, "inner starts below the border");
        assert_eq!(inner.width, 78, "borders cost one cell each side");
        assert_eq!(inner.height, 10, "borders cost one row each side");

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(0, 0)].symbol(), "╭", "rounded corners per §5.3");
    }

    /// The success flash is the ready panel plus its green result line.
    #[test]
    fn success_state_shows_the_flash_and_keeps_the_shell() {
        let text = render_text(
            "文件状态",
            &PanelState::Success {
                message: "已暂存 src/main.rs",
            },
        );
        assert!(text.contains("✓"), "missing flash glyph: {text}");
        assert!(
            text.contains("已暂存src/main.rs"),
            "missing flash message: {text}"
        );

        let backend = TestBackend::new(80, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render_state_panel(
                    f,
                    f.area(),
                    "文件状态",
                    &PanelState::Success {
                        message: "已暂存 src/main.rs",
                    },
                );
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let mut flash_fg = None;
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                if buffer[(x, y)].symbol() == "✓" {
                    flash_fg = Some(buffer[(x, y)].fg);
                }
            }
        }
        assert_eq!(flash_fg, Some(color::SUCCESS), "flash reads success");
    }

    /// Every state keeps the panel title: a loading panel still says which
    /// panel it is (§3.2 "边框与标题不变").
    #[test]
    fn every_state_keeps_the_panel_title() {
        let states = [
            PanelState::Loading {
                label: "加载中…",
                sub: None,
            },
            PanelState::Empty {
                title: "暂无",
                description: "空",
                action: Line::from("按 r 刷新"),
            },
            PanelState::Error { message: "boom" },
            PanelState::Ready,
            PanelState::Success { message: "完成" },
        ];
        for state in &states {
            let text = render_text("提交历史 [demo] (0)", state);
            assert!(
                text.contains("提交历史[demo](0)"),
                "title lost in state {state:?}: {text}"
            );
        }
    }
}
