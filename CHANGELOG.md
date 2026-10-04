# Changelog

All notable changes to this project are documented in this file.
Format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.0] - 2026-10-04
### Added
- `?` help overlay and `Ctrl-P` command-palette float modals (DESIGN §6.11 / §127), layered above the footer via the unified 2-line footer contract.

### Changed
- **[Breaking — interaction]** Unified global key grammar in `app.rs`: `Esc` = back/pop, `q` = exit current root screen (and close any modal/input), `Enter` = open, `<`/`>`/`Tab` = switch context tabs without pushing on the stack, `r` = refresh, `?`/`Ctrl-P` = float overlays (DESIGN §1.2 §2 §6.11).
- Unified 5-state list panels (`loading`/`empty`/`error`/`ready`/`success`) across every view via the shared `render_state_panel` (DESIGN §3.2).
- 2-line footer: status row + hint row; local git and GitHub feedback consistent; toast channel.
- Visual token system + CJK 2-cell width budget + ≤80-column hint wrapping (`components/cells` width budget, DESIGN §5).
- All views migrated to the footer-split `render(f, content, app)` contract with per-view `{view}_hints()` exports feeding the unified hint row.

### Fixed
- `src/ui/mod.rs` test-truncation (unclosed `mod tests`) repaired; delete-confirm assertions restored; `config_view_picker_overlay_renders_via_dispatch` covered by `config.rs` unit tests instead.
- Three component test failures repaired: footer `render_rows` CJK hint-row width over-count; `state_panel` `PanelState::Error` block titling (`{title} · 加载失败`) resolving both the "titled 加载失败" and "panel keeps its title" contracts.

## [0.4.0]
- Single-platform (GitHub) receipt views; pre-redesign interaction grammar.
