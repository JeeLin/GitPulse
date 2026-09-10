#[allow(dead_code)]
pub mod dashboard;
#[allow(dead_code)]
pub mod groups;
#[allow(dead_code)]
pub mod widgets;
#[allow(dead_code)]
pub mod notifications;

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame, Terminal,
};
use std::io;
use crate::app::App;

pub async fn run(app: &mut App) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let result = run_app(&mut terminal, app).await;
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    result
}

async fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> Result<()> {
    let mut list_state = ListState::default();
    list_state.select(Some(0));

    loop {
        terminal.draw(|f| ui(f, app, &mut list_state))?;
        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press {
                if app.notification_state.is_visible {
                    if notifications::handle_notification_input(key.code, &mut app.notification_state, &app.db) {
                        continue;
                    }
                }
                match key.code {
                    KeyCode::Char('q') => { app.should_quit = true; break; }
                    KeyCode::Char('n') => { app.notification_state.toggle_visibility(); }
                    KeyCode::Up | KeyCode::Char('j') => {
                        let i = list_state.selected().map_or(0, |i| if i == 0 { app.repos.len().saturating_sub(1) } else { i - 1 });
                        list_state.select(Some(i));
                    }
                    KeyCode::Down | KeyCode::Char('k') => {
                        let i = list_state.selected().map_or(0, |i| if i >= app.repos.len().saturating_sub(1) { 0 } else { i + 1 });
                        list_state.select(Some(i));
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

fn ui(f: &mut Frame, app: &App, list_state: &mut ListState) {
    if app.notification_state.is_visible {
        notifications::render_notifications(f, &app.notification_state, &app.db, f.area());
        return;
    }
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(f.area());

    let items: Vec<ListItem> = app.repos.iter().map(|repo| {
        let status = if repo.dirty.is_clean() { "✓".to_string() } else { format!("~{}", repo.dirty.total()) };
        let style = if repo.dirty.is_clean() { Style::default().fg(Color::Green) } else { Style::default().fg(Color::Yellow) };
        ListItem::new(Line::from(vec![
            Span::styled(format!("{} ", status), style),
            Span::raw(&repo.name),
            Span::raw(" "),
            Span::styled(&repo.branch, Style::default().fg(Color::Cyan)),
        ]))
    }).collect();

    let list = List::new(items)
        .block(Block::default().title("Repos").borders(Borders::ALL))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol(">> ");
    f.render_stateful_widget(list, chunks[0], list_state);

    if let Some(idx) = list_state.selected() {
        if let Some(repo) = app.repos.get(idx) {
            let detail = format!(
                "Name: {}\nBranch: {}\nModified: {}\nStaged: {}\nUntracked: {}\nAhead: {}\nBehind: {}\nLast: {}",
                repo.name, repo.branch, repo.dirty.modified, repo.dirty.staged, repo.dirty.untracked, repo.ahead, repo.behind, repo.last_commit.message
            );
            let paragraph = Paragraph::new(detail).block(Block::default().title("Detail").borders(Borders::ALL));
            f.render_widget(paragraph, chunks[1]);
        }
    }
}
