//! GitPulse entry point: CLI parsing, tokio runtime, TUI startup.

mod app;
mod config;
mod git;
mod ui;

use anyhow::Result;
use clap::Parser;

/// GitPulse — TUI dashboard for local git repositories.
#[derive(Debug, Parser)]
#[command(version)]
struct Cli;

fn main() -> Result<()> {
    let _cli = Cli::parse();
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(app::run())
}
