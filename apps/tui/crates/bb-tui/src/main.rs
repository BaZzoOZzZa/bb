use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use bb_client::RealtimeClient;
use bb_client::discovery::probe_server;
use bb_store::{Store, StoreConfig};
use bb_tui::app::{AppState, ToastLevel};
use bb_tui::runtime::{run, terminal_events};
use bb_tui::startup::{Args, TopCommand, resolve_connection, run_connect_command};
use bb_tui::time::now_ms;
use clap::Parser;

fn init_logging() {
    let Some(path) = std::env::var_os("BB_TUI_LOG") else {
        return;
    };
    let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return;
    };
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_ansi(false)
        .with_writer(std::sync::Mutex::new(file))
        .try_init();
}

#[tokio::main]
async fn main() -> Result<()> {
    init_logging();
    let mut args = Args::parse();
    if let Some(TopCommand::Connect(command)) = args.command.take() {
        return run_connect_command(&args, command).await;
    }
    let connection = resolve_connection(&args)?;
    let mut state = AppState::new(connection.info.clone(), Instant::now(), now_ms());
    match probe_server(&connection.client).await {
        Ok(report) => {
            for warning in report.warnings {
                state.toast(warning.to_string(), ToastLevel::Error);
            }
        }
        Err(error) => state.toast(
            format!("{} ({}): {error}", connection.info.url, connection.source),
            ToastLevel::Error,
        ),
    }
    let realtime = RealtimeClient::spawn(connection.client.clone());
    let store = Store::connect(
        connection.client.clone(),
        realtime.clone(),
        StoreConfig::default(),
    );
    let api = Arc::new(connection.client.clone());

    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &mut state, store, api, terminal_events()).await;
    ratatui::restore();
    realtime.shutdown();
    result
}
