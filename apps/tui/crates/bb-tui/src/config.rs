use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Item, TableLike};

use crate::composer::SubmitKey;
use crate::theme::Appearance;

pub const CONFIG_FILE: &str = "config.toml";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DesktopMethod {
    #[default]
    Auto,
    Osc9,
    Osc777,
    Native,
    Off,
}

impl DesktopMethod {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Self::Auto,
            "osc9" | "osc-9" => Self::Osc9,
            "osc777" | "osc-777" => Self::Osc777,
            "native" | "system" => Self::Native,
            "off" | "none" | "false" => Self::Off,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Osc9 => "osc9",
            Self::Osc777 => "osc777",
            Self::Native => "native",
            Self::Off => "off",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DesktopWhen {
    #[default]
    Unfocused,
    Always,
}

impl DesktopWhen {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "unfocused" => Some(Self::Unfocused),
            "always" => Some(Self::Always),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unfocused => "unfocused",
            Self::Always => "always",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationPrefs {
    pub enabled: bool,
    pub turn_finished: bool,
    pub needs_input: bool,
    pub errors: bool,
    pub toast: bool,
    pub bell: bool,
    pub desktop: DesktopMethod,
    pub desktop_when: DesktopWhen,
    pub plugin_signals: bool,
    pub terminal_title: bool,
}

impl Default for NotificationPrefs {
    fn default() -> Self {
        Self {
            enabled: true,
            turn_finished: true,
            needs_input: true,
            errors: true,
            toast: true,
            bell: true,
            desktop: DesktopMethod::Auto,
            desktop_when: DesktopWhen::Unfocused,
            plugin_signals: true,
            terminal_title: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvilPrefs {
    pub which_key_delay_ms: u64,
    pub clipboard: bool,
}

impl Default for EvilPrefs {
    fn default() -> Self {
        Self {
            which_key_delay_ms: crate::evil::WHICH_KEY_DELAY.as_millis() as u64,
            clipboard: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub submit_key: Option<SubmitKey>,
    pub theme: Option<String>,
    pub appearance: Appearance,
    pub mouse: bool,
    pub notifications: NotificationPrefs,
    pub evil: EvilPrefs,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            submit_key: None,
            theme: None,
            appearance: Appearance::Auto,
            mouse: true,
            notifications: NotificationPrefs::default(),
            evil: EvilPrefs::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LoadedConfig {
    pub path: Option<PathBuf>,
    pub config: Config,
    pub warnings: Vec<String>,
}

pub fn default_path(config_dir: Option<&Path>) -> Option<PathBuf> {
    config_dir
        .map(Path::to_path_buf)
        .or_else(bb_client::profiles::default_config_dir)
        .map(|dir| dir.join(CONFIG_FILE))
}

pub fn load(path: Option<PathBuf>) -> LoadedConfig {
    let Some(path) = path else {
        return LoadedConfig::default();
    };
    match std::fs::read_to_string(&path) {
        Ok(raw) => {
            let (config, warnings) = parse(&raw);
            LoadedConfig {
                warnings: warnings
                    .into_iter()
                    .map(|warning| format!("{}: {warning}", path.display()))
                    .collect(),
                path: Some(path),
                config,
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => LoadedConfig {
            path: Some(path),
            ..LoadedConfig::default()
        },
        Err(error) => LoadedConfig {
            warnings: vec![format!("{}: {error}", path.display())],
            path: Some(path),
            config: Config::default(),
        },
    }
}

struct Reader<'a> {
    section: &'static str,
    warnings: &'a mut Vec<String>,
}

impl Reader<'_> {
    fn key(&self, key: &str) -> String {
        if self.section.is_empty() {
            key.to_owned()
        } else {
            format!("{}.{key}", self.section)
        }
    }

    fn bool(&mut self, key: &str, item: &Item, target: &mut bool) {
        match item.as_bool() {
            Some(value) => *target = value,
            None => {
                let key = self.key(key);
                self.warnings
                    .push(format!("`{key}` must be true or false; using {target}"));
            }
        }
    }

    fn choice<T: Copy>(
        &mut self,
        key: &str,
        item: &Item,
        parse: fn(&str) -> Option<T>,
        expected: &str,
        target: &mut T,
    ) {
        match item.as_str().and_then(parse) {
            Some(value) => *target = value,
            None => {
                let key = self.key(key);
                self.warnings
                    .push(format!("`{key}` must be one of {expected}"));
            }
        }
    }

    fn unknown(&mut self, key: &str) {
        let key = self.key(key);
        self.warnings.push(format!("unknown setting `{key}`"));
    }
}

pub fn parse(raw: &str) -> (Config, Vec<String>) {
    let mut config = Config::default();
    let mut warnings = Vec::new();
    let document = match raw.parse::<DocumentMut>() {
        Ok(document) => document,
        Err(error) => {
            let message = error.to_string();
            let first = message.lines().next().unwrap_or("invalid TOML").trim();
            warnings.push(format!("{first}; using the default settings"));
            return (config, warnings);
        }
    };
    let mut reader = Reader {
        section: "",
        warnings: &mut warnings,
    };
    let mut notifications: Option<&dyn TableLike> = None;
    let mut evil: Option<&dyn TableLike> = None;
    for (key, item) in document.as_table().iter() {
        match key {
            "submit_key" => match item.as_str().and_then(SubmitKey::parse) {
                Some(value) => config.submit_key = Some(value),
                None => reader
                    .warnings
                    .push("`submit_key` must be \"enter\" or \"ctrl-enter\"".into()),
            },
            "theme" => match item.as_str().map(str::trim).filter(|name| !name.is_empty()) {
                Some(name) => config.theme = Some(name.to_owned()),
                None => reader.warnings.push("`theme` must be a theme name".into()),
            },
            "appearance" => reader.choice(
                key,
                item,
                Appearance::parse,
                "\"auto\", \"dark\", \"light\"",
                &mut config.appearance,
            ),
            "mouse" => reader.bool(key, item, &mut config.mouse),
            "notifications" => match item.as_table_like() {
                Some(table) => notifications = Some(table),
                None => reader
                    .warnings
                    .push("`notifications` must be a table".into()),
            },
            "evil" => match item.as_table_like() {
                Some(table) => evil = Some(table),
                None => reader.warnings.push("`evil` must be a table".into()),
            },
            other => reader.unknown(other),
        }
    }
    if let Some(table) = notifications {
        reader.section = "notifications";
        let prefs = &mut config.notifications;
        for (key, item) in table.iter() {
            match key {
                "enabled" => reader.bool(key, item, &mut prefs.enabled),
                "turn_finished" => reader.bool(key, item, &mut prefs.turn_finished),
                "needs_input" => reader.bool(key, item, &mut prefs.needs_input),
                "errors" => reader.bool(key, item, &mut prefs.errors),
                "toast" => reader.bool(key, item, &mut prefs.toast),
                "bell" => reader.bool(key, item, &mut prefs.bell),
                "plugin_signals" => reader.bool(key, item, &mut prefs.plugin_signals),
                "terminal_title" => reader.bool(key, item, &mut prefs.terminal_title),
                "desktop" => reader.choice(
                    key,
                    item,
                    DesktopMethod::parse,
                    "\"auto\", \"osc9\", \"osc777\", \"native\", \"off\"",
                    &mut prefs.desktop,
                ),
                "desktop_when" => reader.choice(
                    key,
                    item,
                    DesktopWhen::parse,
                    "\"unfocused\", \"always\"",
                    &mut prefs.desktop_when,
                ),
                other => reader.unknown(other),
            }
        }
    }
    if let Some(table) = evil {
        reader.section = "evil";
        let prefs = &mut config.evil;
        for (key, item) in table.iter() {
            match key {
                "which_key_delay_ms" => {
                    match item.as_integer().and_then(|ms| u64::try_from(ms).ok()) {
                        Some(ms) => prefs.which_key_delay_ms = ms,
                        None => {
                            let key = reader.key(key);
                            reader.warnings.push(format!(
                                "`{key}` must be a number of milliseconds; using {}",
                                prefs.which_key_delay_ms
                            ));
                        }
                    }
                }
                "clipboard" => match item.as_str() {
                    Some("unnamedplus") => prefs.clipboard = true,
                    Some("") => prefs.clipboard = false,
                    _ => {
                        let key = reader.key(key);
                        reader
                            .warnings
                            .push(format!("`{key}` must be \"unnamedplus\" or \"\""));
                    }
                },
                other => reader.unknown(other),
            }
        }
    }
    (config, warnings)
}

pub fn render(config: &Config) -> String {
    let mut out = String::new();
    let submit = match config.submit_key.unwrap_or_default() {
        SubmitKey::Enter => "enter",
        SubmitKey::ModifierEnter => "ctrl-enter",
    };
    let _ = writeln!(out, "submit_key = \"{submit}\"");
    let theme = config
        .theme
        .clone()
        .unwrap_or_else(|| crate::highlight::DEFAULT_THEME.to_owned());
    let _ = writeln!(
        out,
        "theme = \"{}\"",
        theme.replace('\\', "\\\\").replace('"', "\\\"")
    );
    let _ = writeln!(out, "appearance = \"{}\"", config.appearance.as_str());
    let _ = writeln!(out, "mouse = {}", config.mouse);
    let prefs = &config.notifications;
    let _ = writeln!(out, "\n[notifications]");
    for (key, value) in [
        ("enabled", prefs.enabled),
        ("turn_finished", prefs.turn_finished),
        ("needs_input", prefs.needs_input),
        ("errors", prefs.errors),
        ("toast", prefs.toast),
        ("bell", prefs.bell),
    ] {
        let _ = writeln!(out, "{key} = {value}");
    }
    let _ = writeln!(out, "desktop = \"{}\"", prefs.desktop.as_str());
    let _ = writeln!(out, "desktop_when = \"{}\"", prefs.desktop_when.as_str());
    let _ = writeln!(out, "plugin_signals = {}", prefs.plugin_signals);
    let _ = writeln!(out, "terminal_title = {}", prefs.terminal_title);
    let _ = writeln!(out, "\n[evil]");
    let _ = writeln!(
        out,
        "which_key_delay_ms = {}",
        config.evil.which_key_delay_ms
    );
    let _ = writeln!(
        out,
        "clipboard = \"{}\"",
        if config.evil.clipboard {
            "unnamedplus"
        } else {
            ""
        }
    );
    out
}

pub fn resolve_desktop(
    method: DesktopMethod,
    native_available: bool,
    env: impl Fn(&str) -> Option<String>,
) -> DesktopMethod {
    match method {
        DesktopMethod::Native if !native_available => detect_terminal(&env),
        DesktopMethod::Auto if native_available => DesktopMethod::Native,
        DesktopMethod::Auto => detect_terminal(&env),
        other => other,
    }
}

fn detect_terminal(env: &impl Fn(&str) -> Option<String>) -> DesktopMethod {
    let var = |name: &str| env(name).unwrap_or_default();
    let program = var("TERM_PROGRAM").to_ascii_lowercase();
    let lc_terminal = var("LC_TERMINAL").to_ascii_lowercase();
    let term = var("TERM").to_ascii_lowercase();
    if matches!(program.as_str(), "iterm.app" | "wezterm" | "ghostty")
        || lc_terminal.starts_with("iterm")
        || lc_terminal == "wezterm"
        || env("KITTY_WINDOW_ID").is_some()
        || env("WEZTERM_PANE").is_some()
        || term.contains("kitty")
        || term.contains("ghostty")
    {
        return DesktopMethod::Osc9;
    }
    if term.starts_with("foot") || term.starts_with("rxvt") || term.contains("urxvt") {
        return DesktopMethod::Osc777;
    }
    DesktopMethod::Off
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_keys_fall_back_to_defaults() {
        let (config, warnings) = parse("");
        assert_eq!(config, Config::default());
        assert!(warnings.is_empty());
    }

    #[test]
    fn reads_every_setting() {
        let (config, warnings) = parse(
            r#"
submit_key = "ctrl-enter"
theme = "Solarized (dark)"
appearance = "light"
mouse = false

[notifications]
enabled = true
turn_finished = false
needs_input = true
errors = false
toast = false
bell = false
desktop = "osc777"
desktop_when = "always"
plugin_signals = false
terminal_title = true

[evil]
which_key_delay_ms = 150
clipboard = ""
"#,
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(config.submit_key, Some(SubmitKey::ModifierEnter));
        assert_eq!(config.theme.as_deref(), Some("Solarized (dark)"));
        assert_eq!(config.appearance, Appearance::Light);
        assert!(!config.mouse);
        assert_eq!(
            config.evil,
            EvilPrefs {
                which_key_delay_ms: 150,
                clipboard: false,
            }
        );
        assert_eq!(
            config.notifications,
            NotificationPrefs {
                enabled: true,
                turn_finished: false,
                needs_input: true,
                errors: false,
                toast: false,
                bell: false,
                desktop: DesktopMethod::Osc777,
                desktop_when: DesktopWhen::Always,
                plugin_signals: false,
                terminal_title: true,
            }
        );
        let (round_trip, warnings) = parse(&render(&config));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(round_trip, config);
    }

    #[test]
    fn invalid_values_warn_and_keep_defaults() {
        let (config, warnings) = parse(
            "submit_key = \"space\"\ncolor = 1\nappearance = \"sepia\"\nnotifications = { bell = \"loud\", desktop = \"pigeon\", sound = true }\n",
        );
        assert_eq!(config, Config::default());
        assert_eq!(
            warnings,
            [
                "`submit_key` must be \"enter\" or \"ctrl-enter\"",
                "unknown setting `color`",
                "`appearance` must be one of \"auto\", \"dark\", \"light\"",
                "`notifications.bell` must be true or false; using true",
                "`notifications.desktop` must be one of \"auto\", \"osc9\", \"osc777\", \"native\", \"off\"",
                "unknown setting `notifications.sound`",
            ]
        );
    }

    #[test]
    fn broken_toml_is_reported_once() {
        let (config, warnings) = parse("[notifications\nbell = false");
        assert_eq!(config, Config::default());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].ends_with("using the default settings"));
    }

    #[test]
    fn load_reports_missing_file_silently() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CONFIG_FILE);
        let loaded = load(Some(path.clone()));
        assert!(loaded.warnings.is_empty());
        std::fs::write(&path, "[notifications]\nbell = false\n").unwrap();
        let loaded = load(Some(path));
        assert!(!loaded.config.notifications.bell);
    }

    #[test]
    fn auto_desktop_picks_native_then_terminal_protocol() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| (*value).to_owned())
            }
        };
        assert_eq!(
            resolve_desktop(DesktopMethod::Auto, true, env(&[])),
            DesktopMethod::Native
        );
        assert_eq!(
            resolve_desktop(
                DesktopMethod::Auto,
                false,
                env(&[("TERM_PROGRAM", "WezTerm")])
            ),
            DesktopMethod::Osc9
        );
        assert_eq!(
            resolve_desktop(
                DesktopMethod::Auto,
                false,
                env(&[("TERM_PROGRAM", "tmux"), ("LC_TERMINAL", "iTerm2")])
            ),
            DesktopMethod::Osc9
        );
        assert_eq!(
            resolve_desktop(DesktopMethod::Auto, false, env(&[("TERM", "foot")])),
            DesktopMethod::Osc777
        );
        assert_eq!(
            resolve_desktop(
                DesktopMethod::Native,
                false,
                env(&[("TERM", "xterm-256color")])
            ),
            DesktopMethod::Off
        );
        assert_eq!(
            resolve_desktop(DesktopMethod::Osc9, true, env(&[])),
            DesktopMethod::Osc9
        );
    }
}
