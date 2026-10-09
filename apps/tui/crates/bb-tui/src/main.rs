use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use bb_client::RealtimeClient;
use bb_client::discovery::probe_server;
use bb_store::{Store, StoreConfig};
use bb_tui::app::{AppState, ToastLevel};
use bb_tui::config::{DesktopMethod, resolve_desktop};
use bb_tui::notifications::Notifications;
use bb_tui::platform::{NATIVE_NOTIFICATIONS, SystemPlatform, TITLE_POP, TITLE_PUSH};
use bb_tui::runtime::{realtime_actions, run, terminal_events};
use bb_tui::startup::{
    Args, TopCommand, config_file, resolve_connection, run_config_command, run_connect_command,
};
use bb_tui::time::now_ms;
use clap::Parser;
use crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, EnableBracketedPaste, EnableFocusChange,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::terminal::supports_keyboard_enhancement;
use std::io::Write;

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

fn enable_terminal_extras(title: bool) -> bool {
    let mut stdout = std::io::stdout();
    let _ = crossterm::execute!(stdout, EnableBracketedPaste, EnableFocusChange);
    if title {
        let _ = stdout.write_all(TITLE_PUSH.as_bytes());
        let _ = stdout.flush();
    }
    let enhanced = matches!(supports_keyboard_enhancement(), Ok(true));
    if enhanced {
        let _ = crossterm::execute!(
            stdout,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
    }
    enhanced
}

fn disable_terminal_extras(enhanced: bool, title: bool) {
    let mut stdout = std::io::stdout();
    if enhanced {
        let _ = crossterm::execute!(stdout, PopKeyboardEnhancementFlags);
    }
    let _ = crossterm::execute!(stdout, DisableBracketedPaste, DisableFocusChange);
    if title {
        let _ = stdout.write_all(TITLE_POP.as_bytes());
        let _ = stdout.flush();
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    init_logging();
    let mut args = Args::parse();
    match args.command.take() {
        Some(TopCommand::Connect(command)) => return run_connect_command(&args, command).await,
        Some(TopCommand::Config) => return run_config_command(&args),
        None => {}
    }
    let loaded = bb_tui::config::load(config_file(&args));
    let connection = resolve_connection(&args)?;
    let mut state = AppState::new(connection.info.clone(), Instant::now(), now_ms());
    state.read_only = args.read_only;
    state.submit_key = args
        .submit_key
        .or(loaded.config.submit_key)
        .unwrap_or_default();
    for warning in &loaded.warnings {
        state.toast(warning.clone(), ToastLevel::Error);
    }
    if let Some(theme) = &loaded.config.theme
        && let Err(error) = bb_tui::highlight::set_theme(theme)
    {
        state.toast(format!("config.toml: {error}"), ToastLevel::Error);
    }
    let mut prefs = loaded.config.notifications.clone();
    if prefs.desktop == DesktopMethod::Native && !NATIVE_NOTIFICATIONS {
        state.toast(
            "config.toml: desktop = \"native\" needs a build with --features notifications; using the terminal",
            ToastLevel::Error,
        );
    }
    prefs.desktop = resolve_desktop(prefs.desktop, NATIVE_NOTIFICATIONS, |name| {
        std::env::var(name).ok()
    });
    let title = prefs.terminal_title;
    state.notifications = Notifications::new(prefs);
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
    let enhanced = enable_terminal_extras(title);
    let events = futures::stream::select(terminal_events(), realtime_actions(realtime.events()));
    let result = run(
        &mut terminal,
        &mut state,
        store,
        api,
        Arc::new(
            SystemPlatform::new()
                .with_keyboard_enhancement(enhanced)
                .with_focus_reporting(true),
        ),
        events,
    )
    .await;
    disable_terminal_extras(enhanced, title);
    ratatui::restore();
    realtime.shutdown();
    result
}
