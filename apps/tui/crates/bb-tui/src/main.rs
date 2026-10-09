mod app;

use std::time::Duration;

use anyhow::Result;
use bb_client::ServerEndpoint;
use clap::Parser;
use crossterm::event::{self, Event, KeyEventKind};

use crate::app::App;

#[derive(Debug, Parser)]
#[command(name = "bb-tui", version, about = "Terminal client for bb")]
struct Args {
    #[arg(long, env = "BB_SERVER_URL")]
    server_url: Option<String>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let endpoint = args
        .server_url
        .as_deref()
        .map(ServerEndpoint::parse)
        .transpose()?;
    let mut app = App::new(endpoint);
    ratatui::run(|terminal| -> Result<()> {
        while !app.should_quit() {
            terminal.draw(|frame| app.render(frame))?;
            if event::poll(Duration::from_millis(250))?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                app.handle_key(key);
            }
        }
        Ok(())
    })
}
