mod app;
mod config;
mod db;
mod git;
mod github;
mod notifications;
mod ui;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let config = config::Config::load()?;
    let mut app = app::App::new(config)?;
    app.run().await
}
