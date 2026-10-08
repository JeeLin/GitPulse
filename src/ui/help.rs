//! `?` help overlay and `Ctrl-P` command palette (DESIGN §6.11, §127).
//!
//! Both are float overlays drawn on top of the footer after every view. The
//! help modal shows the footer's own hint-row composition — the global segment
//! plus the current view's segment — so the overlay teaches exactly the keys the
//! footer shows, never a re-derived echo. The command palette lists every jump
//! target (each screen plus the current repo/branch) so `Enter` can reset the
//! view stack straight onto it. The dispatch in `app.rs` owns their keys; this
//! module only renders the bools it is handed read-only.

use ratatui::{
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{block::BorderType, Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

#[cfg(test)]
use crate::app::SCREENS;
use crate::app::{App, PaletteTarget, View};
use crate::ui::components::footer::{global_hints, hint, hints as join_hints};
use crate::ui::components::theme::{color, role};
use crate::ui::{repo, view_hints, HIGHLIGHT_SYMBOL};

/// §127 modal shell: `surface` tint, rounded border, centered accent title.
/// Mirrors `state_panel::block_for` — a single rounded `Block` with the surface
/// background — so every overlay shares one geometry contract.
fn modal_block(title: &str) -> Block<'static> {
    Block::default()
        .title(Span::styled(title.to_string(), role::title()))
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(Style::default().bg(color::SURFACE))
}

/// Center a popup spanning `percent_x`% of the frame, `height` rows tall.
fn centered(f: &mut Frame, percent_x: u16, height: u16) -> Rect {
    repo::centered_rect(percent_x, height, f.area())
}

/// Draw the `?` help overlay (§6.11): a centered modal titled `帮助` whose body
/// is the footer's own hint-row composition — the global segment plus the
/// current view's segment — plus a one-line close hint.
pub fn render_help(f: &mut Frame, app: &App) {
    let popup = centered(f, 76, 14);
    let block = modal_block("帮助");

    let row = view_hints(app);
    let close: Line = join_hints([hint("Esc", "关闭"), hint("q", "关闭")]);

    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled("全局", role::heading())),
        global_hints(),
        Line::from(""),
        Line::from(Span::styled("当前视图", role::heading())),
        row.view,
    ];
    if let Some(tail) = row.tail {
        lines.push(tail);
    }
    lines.push(Line::from(""));
    lines.push(close);

    let paragraph = Paragraph::new(lines).block(block);

    f.render_widget(Clear, popup);
    f.render_widget(paragraph, popup);
}

/// Draw the `Ctrl-P` command palette (§6.11): a centered modal titled
/// `命令面板` listing every jump target. The cursor is `App::palette_state`;
/// `j`/`k` move it and `Enter` resets the view stack onto the target.
pub fn render_palette(f: &mut Frame, app: &mut App) {
    let popup = centered(f, 72, 22);
    let block = modal_block("命令面板");

    let targets = app.palette_targets();
    let items = targets
        .iter()
        .map(|t| ListItem::new(target_label(t, app.github_state.kind())))
        .collect::<Vec<_>>();

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(color::SURFACE_ALT)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(HIGHLIGHT_SYMBOL);

    f.render_widget(Clear, popup);
    f.render_stateful_widget(list, popup, &mut app.palette_state);
}

/// One palette row: the screen's label, or the repo/branch name it targets.
/// `kind` is the remote's platform, so a platform-specific screen name matches
/// the panel the row jumps to.
fn target_label(
    target: &PaletteTarget,
    kind: Option<crate::platform::PlatformKind>,
) -> Line<'static> {
    let (text, style) = match target {
        PaletteTarget::View(view) => (screen_label(*view, kind).to_string(), role::body()),
        PaletteTarget::Repo(name) => (format!("仓库 {name}"), role::code()),
        PaletteTarget::Branch(name) => (format!("分支 {name}"), role::code()),
    };
    Line::from(Span::styled(text, style))
}

/// Chinese label of every screen, mirroring its title so the palette and the
/// panel it jumps to agree on the name.
///
/// `kind` matters for the one slot whose name the platform decides: the
/// notification receipt is GitHub's `通知` but GitLab's `To-Dos`, so the row
/// derives that one from [`crate::platform::notifications_label`] — the same
/// source as the panel title and the footer key hint. A palette row that kept
/// `通知` under a `To-Dos` panel would name two different things on one screen.
fn screen_label(view: View, kind: Option<crate::platform::PlatformKind>) -> &'static str {
    match view {
        View::Dashboard => "仪表盘",
        View::Repo => "仓库工作区",
        View::Diff => "文件差异",
        View::Commit => "提交信息",
        View::CommitDetail => "提交详情",
        View::Branches => "分支",
        View::Config => "扫描目录配置",
        View::History => "提交历史",
        View::Tags => "标签",
        View::Pulls => "PR 回执",
        View::PullDetail => "PR 详情",
        View::Checks => "CI 检查",
        View::Issues => "Issues",
        View::IssueDetail => "Issue 详情",
        View::Notifications => crate::platform::notifications_label(kind),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Palette targets include every screen; with no repo open there is no
    /// repo / branch target.
    #[test]
    fn palette_targets_cover_all_screens_without_repo() {
        let app = crate::app::App::new(crate::config::Config::default());
        let targets = app.palette_targets();
        assert_eq!(
            targets
                .iter()
                .filter(|t| matches!(t, PaletteTarget::View(_)))
                .count(),
            SCREENS.len(),
        );
        assert!(
            targets
                .iter()
                .all(|t| !matches!(t, PaletteTarget::Repo(_) | PaletteTarget::Branch(_))),
            "no repo should be open"
        );
    }

    /// The screen labels are stable and distinct (no two screens share a name)
    /// for every platform — including GitLab's `To-Dos` slot, whose name the
    /// platform decides but which must still stay distinct from the rest.
    #[test]
    fn screen_labels_are_distinct() {
        use crate::platform::PlatformKind;
        for kind in [
            Some(PlatformKind::GitHub),
            Some(PlatformKind::GitLab),
            Some(PlatformKind::Gitea),
            Some(PlatformKind::SelfHosted),
            None,
        ] {
            let mut seen = std::collections::HashSet::new();
            for view in SCREENS {
                assert!(
                    seen.insert(screen_label(view, kind)),
                    "duplicate label for {view:?} on {kind:?}"
                );
            }
        }
    }

    /// The palette row for the notification slot names the same thing as the
    /// panel the row jumps to — the invariant `screen_label` documents.
    #[test]
    fn palette_row_agrees_with_the_notification_panel_label() {
        use crate::platform::PlatformKind;
        assert_eq!(
            screen_label(View::Notifications, Some(PlatformKind::GitLab)),
            crate::platform::notifications_label(Some(PlatformKind::GitLab)),
        );
        assert_eq!(
            screen_label(View::Notifications, Some(PlatformKind::GitHub)),
            "通知",
            "GitHub keeps its v0.5.0 wording in the palette too"
        );
    }
}
