//! The receipts panels' shared body: the §3.2 five-state dispatch, the
//! platform-gate body, the panel titles and the row budget of the four
//! receipts lists (PR / CI checks / Issues / notifications).
//!
//! The four panels are the same screen four times over — one gate, one
//! `loading` sub-label, one empty copy each — so `loading` / `empty` / `error` /
//! `ready`, the §3.1 cursor and the credential guidance live here instead of
//! four near-copies. A panel hands over its own title and its own rows through
//! [`ReceiptPanel`] and [`render_receipts_list`]; the geometry stays shared.
//!
//! The two detail screens (`PR 详情` / `Issue 详情`) are not lists: they gate
//! through [`render_load_error`] / [`render_disconnected`] and then draw their
//! own panes, so they take the two primitives instead of the dispatcher.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{block::BorderType, Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::app::App;
use crate::platform::PlatformKind;

use super::footer::hint;
use super::footer::hints as join_hints;
use super::footer::HintRow;
use super::state_panel;
use super::theme::{color, role};

/// Columns a list row gives up to the block border (2) and the highlight
/// symbol (2): [`List`] indents every line of the selected item by the §3.1
/// cursor, so rows are fitted to what remains.
const ROW_GUTTER: u16 = 4;

/// Retry hint for an instance this app does not support. It is deliberately
/// *not* a token prompt: self-hosted hosts are rejected before any credential
/// lookup, so telling the user to configure a token would be a dead end.
const SELF_HOSTED_HINT: &str = "该实例暂不支持（当前支持 github.com / gitea.com / gitlab.com）";

/// One receipts panel's identity: the title label plus the copy of the two
/// states only this panel can word.
///
/// The four fields travel together so the dispatcher's argument list stays
/// small and a panel's copy is visible at a glance.
pub(crate) struct ReceiptPanel<'a> {
    /// Panel title prefix, e.g. `PR 列表` / `Issues`; the remote pair is
    /// appended by [`panel_title`].
    pub label: &'a str,
    /// Sub-label naming what the `loading` state reads (`正在拉取 PR…`).
    pub load_sub: &'a str,
    /// What the panel would otherwise list (`—（暂无 PR）`).
    pub empty_copy: &'a str,
    /// Why it is empty, one line.
    pub empty_description: &'a str,
}

/// Draw one receipts list in whichever of the five states §3.2 it is in.
///
/// The order is the same for all four panels: a failed load paints the panel's
/// `error` state (§4.3 — the content did not arrive, so the failure is
/// persistent and `r` retries it), a missing client paints the platform gate
/// instead of a list that could never load, a fetch in flight paints `loading`,
/// zero rows paint `empty`, and anything else paints `ready`.
///
/// The receipts screens are single-panel (§1.1), so the panel is always the
/// focused one: `loading` / `empty` / `error` / `ready` all wear the same
/// focused, rounded shell (§3.1 + §5.3) instead of four geometries that could
/// drift apart.
///
/// `items` builds the rows from the row budget of `area` — it is called only in
/// the `ready` state, so a gated panel never builds rows it cannot show.
pub(crate) fn render_receipts_list(
    f: &mut Frame,
    area: Rect,
    meta: ReceiptPanel<'_>,
    app: &App,
    selected: usize,
    items: impl FnOnce(usize) -> Vec<ListItem<'static>>,
) {
    let title = panel_title(
        meta.label,
        app.github_state.owner(),
        app.github_state.repo(),
    );

    if let Some(message) = app.load_error.as_deref() {
        render_load_error(f, area, &title, message);
        return;
    }

    if !app.github_state.connected() {
        render_disconnected(f, area, meta.label, app.github_state.kind(), app);
        return;
    }

    if app.github_loading() {
        render_state(
            f,
            area,
            &title,
            &state_panel::loading_state(Some(meta.load_sub)),
        );
        return;
    }

    let rows = items(row_width(area));
    if rows.is_empty() {
        render_state(
            f,
            area,
            &title,
            &state_panel::empty_state(
                meta.empty_copy,
                meta.empty_description,
                Some(hint("r", "重新拉取")),
            ),
        );
        return;
    }

    render_rows(f, area, &title, rows, selected);
}

/// The panel's `error` state for a failed load of its own content (§4.3):
/// `加载失败`, the reason and `按 r 重试`. The body is painted `danger` by
/// `error_lines`, while the shell keeps the §3.1 focus border (so a failed
/// panel still says which panel it is) — a failure is not a focus signal and a
/// focus border is not a failure (see [`state_panel::render_state_panel_with_focus`]).
///
/// A `focused` receipt panel takes that focus border the same as `ready`:
/// the §5.1 `danger` tone lives on the body text, not the shell.
pub(crate) fn render_load_error(f: &mut Frame, area: Rect, title: &str, message: &str) {
    render_state(f, area, title, &state_panel::PanelState::Error { message });
}

/// Panel body shown while the platform client is not ready: the platform's own
/// gate copy above the retry guidance, instead of a list that could never
/// load. The reason the client is missing is the gate copy itself — nothing is
/// repeated on the status row (§4.1).
pub(crate) fn render_disconnected(
    f: &mut Frame,
    area: Rect,
    label: &str,
    kind: Option<PlatformKind>,
    app: &App,
) {
    let copy = disconnected_copy(kind);
    let lines = vec![
        Line::from(Span::styled(
            copy.title,
            Style::default()
                .fg(color::WARNING)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            copy.hint,
            Style::default().fg(color::FG_MUTED),
        )),
    ];
    f.render_widget(
        Paragraph::new(lines).block(shell(panel_title(
            label,
            app.github_state.owner(),
            app.github_state.repo(),
        ))),
        area,
    );
}

/// `ready` body of a receipts list (§3.2 + §3.1): the rows inside the shared
/// shell, with the cursor on `selected`.
///
/// `rows` arrive already fitted to the row budget by each caller, so they need
/// no padding here.
fn render_rows(
    f: &mut Frame,
    area: Rect,
    title: &str,
    rows: Vec<ListItem<'static>>,
    selected: usize,
) {
    let inner = render_state(f, area, title, &state_panel::PanelState::Ready);
    let mut state = ListState::default();
    state.select(list_cursor(selected, rows.len()));
    // The shell is already drawn, so the list carries only the §3.1 cursor.
    f.render_stateful_widget(super::with_cursor(List::new(rows), true), inner, &mut state);
}

/// One `loading` / `empty` / `error` / `ready` panel body in the shared
/// focused shell.
fn render_state(
    f: &mut Frame,
    area: Rect,
    title: &str,
    state: &state_panel::PanelState<'_>,
) -> Rect {
    state_panel::render_state_panel_with_focus(f, area, title, state, true)
}

/// The rounded, token-painted shell of a receipts body (§5.3 + §3.1): the
/// focused single panel takes `border_focus`, the same border role
/// [`state_panel`] gives the five states.
fn shell(title: String) -> Block<'static> {
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(role::border(true))
}

/// Panel title: `label · owner/repo`, falling back to `—` while the remote
/// pair is unknown (a failed hub entry never fills it in).
pub(crate) fn panel_title(label: &str, owner: Option<&str>, repo: Option<&str>) -> String {
    match (owner, repo) {
        (Some(owner), Some(repo)) if !owner.is_empty() && !repo.is_empty() => {
            format!("{label} · {owner}/{repo}")
        }
        _ => format!("{label} · —"),
    }
}

/// Cursor for a `len`-row list: `None` for an empty list, otherwise the
/// requested index clamped to the last row. `App::github_list_len` is private
/// to the app module, so the panels clamp against the vec they display — the
/// same `saturating_sub(1)` the app-side clamp applies.
pub(crate) fn list_cursor(selected: usize, len: usize) -> Option<usize> {
    if len == 0 {
        None
    } else {
        Some(selected.min(len.saturating_sub(1)))
    }
}

/// Cells a list row may use inside `area`: minus the block borders and the
/// highlight symbol.
pub(crate) fn row_width(area: Rect) -> usize {
    area.width.saturating_sub(ROW_GUTTER) as usize
}

/// The two lines the disconnected body prints above the status line: the
/// gate headline plus the credential hint of the platform that owns the
/// resolved remote. Both are `&'static str` so a family only ever grows one
/// entry here instead of a copy per panel.
pub(crate) struct DisconnectedCopy {
    /// Headline: which platform the client is not connected to.
    pub title: &'static str,
    /// How to obtain a credential for that platform, and the retry key.
    /// Built from [`PlatformKind::token_var`] / [`PlatformKind::cli_hint`]
    /// so the variable names live in one place only.
    pub hint: String,
}

/// Credential guidance for `kind`. An unresolved remote (`None`) falls back to
/// the GitHub copy, not to neutral wording: v0.5.0 knew no platform but
/// GitHub, so this state rendered exactly that text and the GitHub path keeps
/// its v0.5.0 wording verbatim.
///
/// Self-hosted is a dead-end credential path (`build_client` always rejects
/// such remotes), so its hint states the limitation instead of asking for a
/// token the app will never read.
pub(crate) fn disconnected_copy(kind: Option<PlatformKind>) -> DisconnectedCopy {
    match kind {
        Some(PlatformKind::GitHub) => DisconnectedCopy {
            title: "尚未连接 GitHub",
            hint: credential_hint(PlatformKind::GitHub),
        },
        Some(PlatformKind::GitLab) => DisconnectedCopy {
            title: "尚未连接 GitLab",
            hint: credential_hint(PlatformKind::GitLab),
        },
        Some(PlatformKind::Gitea) => DisconnectedCopy {
            title: "尚未连接 Gitea",
            hint: credential_hint(PlatformKind::Gitea),
        },
        Some(PlatformKind::SelfHosted) => DisconnectedCopy {
            title: "尚未连接自托管平台",
            hint: credential_hint(PlatformKind::SelfHosted),
        },
        None => DisconnectedCopy {
            title: "尚未连接 GitHub",
            hint: credential_hint(PlatformKind::GitHub),
        },
    }
}

/// Compose the retry hint from the platform's own token variable and CLI name.
///
/// An empty [`token_var`](PlatformKind::token_var) means the platform registers
/// no variable, which only self-hosted does: it is out of scope rather than
/// short of a credential, so it gets the unsupported-instance wording. An
/// empty [`cli_hint`](PlatformKind::cli_hint) means the platform has no auth
/// CLI (Gitea), so only the variable is named.
fn credential_hint(kind: PlatformKind) -> String {
    let var = kind.token_var();
    if var.is_empty() {
        return SELF_HOSTED_HINT.to_string();
    }
    let cli = kind.cli_hint();
    if cli.is_empty() {
        format!("配置 {var} 后按 r 重试")
    } else {
        format!("配置 {var} 或 {cli} 登录后按 r 重试")
    }
}

/// Bottom hint row of every receipt panel: hub entry, the four tabs, the
/// refresh and the two ways out.
///
/// `show_enter` adds `Enter 详情`, which is only useful on the Pulls and
/// Issues *lists* (where it opens the selected item's detail); pass `false` for
/// the Checks / Notifications / detail views, where Enter is a no-op.
///
/// `kind` is the remote's platform, so the fourth tab's name matches the
/// notification panel's own title: it comes from
/// [`crate::platform::notifications_label`], the single source for that slot.
/// GitHub and Gitea call it `通知`, GitLab `To-Dos` — a footer still saying
/// `通知` under a `To-Dos` title would name two different things on one screen.
pub(crate) fn receipt_hints(show_enter: bool, kind: Option<PlatformKind>) -> HintRow {
    let mut hints: Vec<Line<'static>> = vec![
        hint("g", "进入"),
        hint("p", "PR"),
        hint("c", "checks"),
        hint("i", "issues"),
        hint("n", crate::platform::notifications_label(kind)),
        hint("r", "刷新"),
    ];
    if show_enter {
        hints.push(hint("Enter", "详情"));
    }
    HintRow::new(join_hints(hints))
}

/// Footer hint row for a detail panel (§2.3): the receipt hub keys, minus
/// the `Enter 详情` drill-in, which a detail panel has nowhere to push. Pull
/// and Issue details share this row verbatim, so it is one function instead of
/// a pair of byte-for-byte copies. `kind` is the open platform, see
/// [`receipt_hints`].
pub(crate) fn detail_hints(kind: Option<PlatformKind>) -> HintRow {
    receipt_hints(false, kind)
}

#[cfg(test)]
/// Frame a receipt panel the way `ui::render` does: the panel draws content
/// only (§2.3) and [`super::footer::draw_frame`] adds the footer, so a panel
/// test that skips it would assert against a missing status/hint row.
///
/// `show_enter` mirrors what the panel passes to [`receipt_hints`] in
/// production.
pub(crate) fn draw_receipt_frame(
    f: &mut Frame,
    app: &App,
    show_enter: bool,
    content: impl FnOnce(&mut Frame, Rect),
) {
    let status = super::view_status(app);
    let hints = receipt_hints(show_enter, app.github_state.kind());
    super::footer::draw_frame(f, status, &hints, content);
}

/// The frame a receipts panel plus its footer produced, so a test can address
/// a single row of it (§4.1).
#[cfg(test)]
pub(crate) fn render_receipt_buffer(
    app: &App,
    show_enter: bool,
    draw: fn(&mut Frame, Rect, &App),
) -> ratatui::buffer::Buffer {
    let backend = ratatui::backend::TestBackend::new(80, 24);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal
        .draw(|f| draw_receipt_frame(f, app, show_enter, |f, body| draw(f, body, app)))
        .unwrap();
    terminal.backend().buffer().clone()
}

/// Readiness-gate client for the panel tests.
///
/// `GithubClient::new` only resolves a token (the environment wins) and
/// builds an octocrab instance — but octocrab spawns its tower buffer task,
/// which needs a Tokio context, so the build runs inside a throwaway
/// runtime. That task is never driven: these tests only render a frame, they
/// never issue a request.
#[cfg(test)]
pub(crate) fn offline_client() -> Box<dyn crate::platform::PlatformClient> {
    if std::env::var("GITHUB_TOKEN").is_err() {
        std::env::set_var("GITHUB_TOKEN", "test-token");
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let _guard = runtime.enter();
    Box::new(crate::github::GithubClient::new("octo", "demo").expect("offline client"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::ui::components::cells::compact;
    use crate::ui::github_remote;

    /// Panel title keeps the remote pair once it is known, and reads `—` while
    /// either half is missing or empty — a failed hub entry never fills it in.
    #[test]
    fn panel_title_falls_back_while_the_remote_pair_is_unknown() {
        assert_eq!(
            panel_title("PR 列表", Some("octo"), Some("demo")),
            "PR 列表 · octo/demo"
        );
        assert_eq!(panel_title("PR 列表", None, None), "PR 列表 · —");
        assert_eq!(panel_title("通知", Some("octo"), None), "通知 · —");
        assert_eq!(
            panel_title("通知", Some("octo"), Some("")),
            "通知 · —",
            "an empty segment still counts as unknown"
        );
    }

    /// U4: the disconnected body names the platform that owns the remote, so
    /// the credential hint always matches the token the client wants.
    #[test]
    fn disconnected_copy_names_the_credential_of_each_platform() {
        assert_eq!(
            disconnected_copy(Some(PlatformKind::GitHub)).title,
            "尚未连接 GitHub"
        );
        assert_eq!(
            disconnected_copy(Some(PlatformKind::GitHub)).hint,
            "配置 GITHUB_TOKEN 或 gh 登录后按 r 重试"
        );
        assert_eq!(
            disconnected_copy(Some(PlatformKind::GitLab)).title,
            "尚未连接 GitLab"
        );
        assert_eq!(
            disconnected_copy(Some(PlatformKind::GitLab)).hint,
            "配置 GITLAB_TOKEN 或 glab 登录后按 r 重试"
        );
        assert_eq!(
            disconnected_copy(Some(PlatformKind::Gitea)).title,
            "尚未连接 Gitea"
        );
        assert_eq!(
            disconnected_copy(Some(PlatformKind::Gitea)).hint,
            "配置 GITEA_TOKEN 后按 r 重试"
        );
        assert_eq!(
            disconnected_copy(Some(PlatformKind::SelfHosted)).title,
            "尚未连接自托管平台"
        );
        // Self-hosted remotes are rejected by `build_client`, so asking for a
        // token would send the user down a path that cannot succeed.
        let self_hosted = disconnected_copy(Some(PlatformKind::SelfHosted));
        assert!(
            !self_hosted.hint.contains("令牌"),
            "no credential is asked for: {}",
            self_hosted.hint
        );
        assert!(
            self_hosted.hint.contains("暂不支持"),
            "the limitation is stated: {}",
            self_hosted.hint
        );
        // No remote resolved: the gate falls back to the GitHub copy, because
        // v0.5.0 (which had no platform concept yet) always printed it.
        let unknown = disconnected_copy(None);
        assert_eq!(unknown.title, "尚未连接 GitHub");
        assert_eq!(unknown.hint, "配置 GITHUB_TOKEN 或 gh 登录后按 r 重试");
        let github = disconnected_copy(Some(PlatformKind::GitHub));
        assert_eq!(
            (unknown.title, unknown.hint),
            (github.title, github.hint),
            "the fallback is exactly the GitHub copy"
        );
    }

    /// §3.1: the cursor is clamped to the last row, and an empty list has none.
    #[test]
    fn list_cursor_clamps_a_stale_selection_and_has_none_when_empty() {
        assert_eq!(list_cursor(99, 2), Some(1));
        assert_eq!(list_cursor(0, 0), None, "empty list has no cursor");
    }

    /// §5.4: a row gives up two columns to the block borders and two to the
    /// cursor, so the builder gets the cells that are actually drawable.
    #[test]
    fn row_width_reserves_the_borders_and_the_cursor() {
        assert_eq!(row_width(Rect::new(0, 0, 80, 24)), 76);
        assert_eq!(row_width(Rect::new(0, 0, 2, 24)), 0);
    }

    /// §3.1 + §5.3: the gate body wears the same focused rounded shell the
    /// `ready` rows do — a panel must not lose its shape because it has no
    /// client yet (C1).
    #[test]
    fn disconnected_body_wears_the_focused_rounded_shell() {
        let mut app = App::new(Config::default());
        app.github_state.remote = Some(github_remote());
        let backend = ratatui::backend::TestBackend::new(40, 10);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_disconnected(f, f.area(), "PR 列表", app.github_state.kind(), &app))
            .unwrap();
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(0, 0)].symbol(), "╭", "rounded corners per §5.3");
        assert_eq!(
            buffer[(0, 0)].fg,
            color::BORDER_FOCUS,
            "the gate body takes the focus border of a single-panel screen"
        );
        let text = compact(buffer);
        assert!(text.contains("PR列表·octo/demo"), "missing title: {text}");
        assert!(text.contains("尚未连接GitHub"), "missing gate copy: {text}");
    }

    /// One `(copy, key, draw)` case of the shell parity check: the copy and the
    /// key hint the state must show, and the closure that paints it.
    type ShellCase = (&'static str, &'static str, fn(&mut Frame, Rect));

    /// §3.1 + §5.3: `loading`, `empty` and `error` wear the very shell the
    /// rows do, so one screen cannot change its panel shape between states
    /// (C2).
    #[test]
    fn every_receipts_state_wears_the_same_focused_shell() {
        let cases: [ShellCase; 3] = [
            ("正在拉取PR…", "加载中…", |f, area| {
                render_state(
                    f,
                    area,
                    "PR 列表 · octo/demo",
                    &state_panel::loading_state(Some("正在拉取 PR…")),
                );
            }),
            (
                "—（暂无PR）",
                "这个仓库没有开放的PR",
                |f, area| {
                    render_state(
                        f,
                        area,
                        "PR 列表 · octo/demo",
                        &state_panel::empty_state(
                            "—（暂无 PR）",
                            "这个仓库没有开放的 PR",
                            Some(hint("r", "重新拉取")),
                        ),
                    );
                },
            ),
            ("加载失败", "boom", |f, area| {
                render_load_error(f, area, "PR 列表 · octo/demo", "boom");
            }),
        ];
        for (copy, key, draw) in cases {
            let backend = ratatui::backend::TestBackend::new(40, 10);
            let mut terminal = ratatui::Terminal::new(backend).unwrap();
            terminal.draw(|f| draw(f, f.area())).unwrap();
            let buffer = terminal.backend().buffer();
            assert_eq!(buffer[(0, 0)].symbol(), "╭", "rounded corners per §5.3");
            let text = compact(buffer);
            assert!(text.contains("PR列表·octo/demo"), "missing title: {text}");
            assert!(text.contains(copy), "missing copy {copy:?}: {text}");
            assert!(text.contains(key), "missing key {key:?}: {text}");
            // §3.1: every state — error included — wears the focus border.
            assert_eq!(
                buffer[(0, 0)].fg,
                color::BORDER_FOCUS,
                "{copy:?} keeps the focus border"
            );
            // §3.2: the error body keeps its `danger` tone while the shell
            // keeps the focus border — the two never repaint each other.
            if copy == "加载失败" {
                let mut body_fg = None;
                for y in 0..buffer.area.height {
                    for x in 0..buffer.area.width {
                        if buffer[(x, y)].symbol() == "b" {
                            body_fg = Some(buffer[(x, y)].fg);
                        }
                    }
                }
                assert_eq!(
                    body_fg,
                    Some(color::DANGER),
                    "the error body keeps danger while the border does not"
                );
            }
        }
    }

    /// §3.1: `ready` draws the cursor marker on the selected row, because the
    /// receipts screens are single-panel and always focused.
    #[test]
    fn ready_rows_carry_the_section_cursor() {
        let items = vec![ListItem::new("alpha"), ListItem::new("beta")];
        let backend = ratatui::backend::TestBackend::new(40, 10);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_rows(f, f.area(), "PR 列表 · octo/demo", items, 1))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text = compact(buffer);
        assert!(text.contains("beta"), "selected row missing: {text}");
        assert!(
            buffer.content().iter().any(|c| c.symbol() == "▌"),
            "the cursor marker is missing: {text}"
        );
    }
}
