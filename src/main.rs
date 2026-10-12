//! GitPulse entry point: CLI parsing, tokio runtime, TUI startup.

#![deny(rustdoc::broken_intra_doc_links)]

mod app;
mod config;
mod datetime;
mod git;
mod github;
mod platform;
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
