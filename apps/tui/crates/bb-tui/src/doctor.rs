use std::fmt::Write as _;
use std::time::{Duration, Instant};

use bb_client::discovery::{
    APP_RUNTIME_FILE_NAME, DEFAULT_SERVER_URL, SERVER_URL_ENV, probe_server, process_is_running,
    read_app_runtime_file,
};
use bb_client::{DiscoveryInput, ProfileStore, probe_realtime};

use crate::startup::{Args, config_dir, config_file, resolve_connection};
use crate::theme::resolve;

pub const REALTIME_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    Info,
    Warn,
    Fail,
}

impl Status {
    fn tag(self) -> &'static str {
        match self {
            Self::Ok => "ok  ",
            Self::Info => "    ",
            Self::Warn => "warn",
            Self::Fail => "FAIL",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub status: Status,
    pub label: String,
    pub detail: String,
}

#[derive(Debug, Default)]
pub struct Report {
    pub sections: Vec<(String, Vec<Check>)>,
}

impl Report {
    fn section(&mut self, title: &str) -> &mut Vec<Check> {
        self.sections.push((title.to_owned(), Vec::new()));
        &mut self.sections.last_mut().expect("section").1
    }

    pub fn failed(&self) -> bool {
        self.checks().any(|check| check.status == Status::Fail)
    }

    pub fn checks(&self) -> impl Iterator<Item = &Check> {
        self.sections.iter().flat_map(|(_, checks)| checks.iter())
    }

    pub fn render(&self) -> String {
        let width = self
            .checks()
            .map(|check| check.label.chars().count())
            .max()
            .unwrap_or(0);
        let mut out = String::new();
        for (index, (title, checks)) in self.sections.iter().enumerate() {
            if index > 0 {
                out.push('\n');
            }
            let _ = writeln!(out, "{title}");
            for check in checks {
                let _ = writeln!(
                    out,
                    "  {} {:width$}  {}",
                    check.status.tag(),
                    check.label,
                    check.detail
                );
            }
        }
        out
    }
}

fn check(status: Status, label: &str, detail: impl Into<String>) -> Check {
    Check {
        status,
        label: label.to_owned(),
        detail: detail.into(),
    }
}

fn millis(elapsed: Duration) -> String {
    format!("{} ms", elapsed.as_millis())
}

fn discovery_checks(args: &Args, checks: &mut Vec<Check>) {
    if let Some(profile) = &args.profile {
        checks.push(check(Status::Info, "profile", profile.clone()));
        return;
    }
    let input = DiscoveryInput::from_env(args.server_url.clone());
    checks.push(check(
        Status::Info,
        "--server-url",
        input.flag_url.clone().unwrap_or_else(|| "not set".into()),
    ));
    checks.push(check(
        Status::Info,
        SERVER_URL_ENV,
        input
            .env_url
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "not set".into()),
    ));
    let runtime = match &input.data_dir {
        None => "no data directory".to_owned(),
        Some(dir) => {
            let path = dir.join(APP_RUNTIME_FILE_NAME);
            match read_app_runtime_file(dir) {
                None => format!("{} not found", path.display()),
                Some(file) if process_is_running(file.pid) => format!(
                    "{} → {} (pid {} running)",
                    path.display(),
                    file.server_url,
                    file.pid
                ),
                Some(file) => format!(
                    "{} → {} (pid {} not running, ignored)",
                    path.display(),
                    file.server_url,
                    file.pid
                ),
            }
        }
    };
    checks.push(check(Status::Info, "app runtime file", runtime));
    checks.push(check(Status::Info, "default", DEFAULT_SERVER_URL));
}

fn settings_checks(args: &Args, checks: &mut Vec<Check>) {
    let loaded = crate::config::load(config_file(args));
    let path = match &loaded.path {
        Some(path) if path.exists() => path.display().to_string(),
        Some(path) => format!("{} (not created, defaults)", path.display()),
        None => "no config directory (defaults)".to_owned(),
    };
    checks.push(check(Status::Info, "config file", path));
    for warning in &loaded.warnings {
        checks.push(check(Status::Warn, "config file", warning.clone()));
    }
    if let Ok(dir) = config_dir(args) {
        match ProfileStore::new(&dir).load() {
            Ok(list) => {
                let active = list.active().map_or_else(
                    || "none active".to_owned(),
                    |profile| format!("active: {}", profile.name()),
                );
                checks.push(check(
                    Status::Info,
                    "profiles",
                    format!("{} saved, {active}", list.profiles.len()),
                ));
            }
            Err(error) => checks.push(check(Status::Warn, "profiles", error.to_string())),
        }
    }
    let appearance = args.appearance.unwrap_or(loaded.config.appearance);
    let theme = resolve(appearance, |name| std::env::var(name).ok());
    let colors = if theme.no_color {
        "off (NO_COLOR)".to_owned()
    } else {
        format!(
            "{} ({})",
            if theme.light { "light" } else { "dark" },
            appearance.as_str()
        )
    };
    checks.push(check(Status::Info, "colors", colors));
    let mouse = loaded.config.mouse && !args.no_mouse;
    checks.push(check(
        Status::Info,
        "mouse",
        if mouse { "captured" } else { "off" },
    ));
    if args.read_only {
        checks.push(check(Status::Info, "read-only", "on"));
    }
}

pub async fn diagnose(args: &Args) -> Report {
    let mut report = Report::default();
    discovery_checks(args, report.section("Discovery"));
    let connection = match resolve_connection(args) {
        Ok(connection) => connection,
        Err(error) => {
            report
                .section("Server")
                .push(check(Status::Fail, "connection", format!("{error:#}")));
            settings_checks(args, report.section("Settings"));
            return report;
        }
    };
    report.sections[0].1.push(check(
        Status::Ok,
        "using",
        format!("{} (from {})", connection.info.url, connection.source),
    ));

    let server = report.section("Server");
    let started = Instant::now();
    let reachable = match probe_server(&connection.client).await {
        Ok(probe) => {
            server.push(check(
                Status::Ok,
                "GET /health",
                format!("ok in {}", millis(started.elapsed())),
            ));
            if let Some(version) = &probe.version {
                let mut detail = version.current_version.clone();
                if version.is_development {
                    detail.push_str(" (development)");
                }
                server.push(check(Status::Ok, "GET /system/version", detail));
                if version.update_available {
                    server.push(check(
                        Status::Info,
                        "update",
                        format!(
                            "{} available · {}",
                            version.latest_version.clone().unwrap_or_default(),
                            version.upgrade_command
                        ),
                    ));
                }
            }
            for warning in &probe.warnings {
                server.push(check(Status::Warn, "compatibility", warning.to_string()));
            }
            true
        }
        Err(error) => {
            server.push(check(Status::Fail, "GET /health", error.to_string()));
            false
        }
    };
    let realtime = report.section("Realtime");
    if reachable {
        let started = Instant::now();
        match probe_realtime(&connection.client, REALTIME_TIMEOUT).await {
            Ok(url) => realtime.push(check(
                Status::Ok,
                "websocket",
                format!("{url} upgraded in {}", millis(started.elapsed())),
            )),
            Err(error) => realtime.push(check(Status::Fail, "websocket", error)),
        }
    } else {
        realtime.push(check(
            Status::Info,
            "websocket",
            "skipped (server unreachable)",
        ));
    }
    settings_checks(args, report.section("Settings"));
    report
}
