use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

use crate::config::DesktopMethod;
use crate::drafts::{DraftDir, default_dir};
use crate::notifications::Alert;

pub const NATIVE_NOTIFICATIONS: bool = cfg!(feature = "notifications");

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawEvent {
    Bytes(Vec<u8>),
    Resize { cols: u16, rows: u16 },
}

pub struct RawInput {
    events: tokio::sync::mpsc::UnboundedReceiver<RawEvent>,
    stop: Option<Box<dyn FnOnce() + Send>>,
}

impl RawInput {
    pub fn new(
        events: tokio::sync::mpsc::UnboundedReceiver<RawEvent>,
        stop: impl FnOnce() + Send + 'static,
    ) -> Self {
        Self {
            events,
            stop: Some(Box::new(stop)),
        }
    }

    pub async fn next(&mut self) -> Option<RawEvent> {
        self.events.recv().await
    }
}

impl Drop for RawInput {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop();
        }
    }
}

pub trait Platform: Send + Sync + 'static {
    fn copy(&self, text: &str) -> Result<(), String>;

    fn open(&self, target: &str) -> Result<(), String>;

    fn load_draft(&self, thread_id: &str) -> Option<String>;

    fn save_draft(&self, thread_id: &str, raw: Option<&str>) -> Result<(), String>;

    fn load_palette_recents(&self) -> Vec<String> {
        Vec::new()
    }

    fn save_palette_recents(&self, _recents: &[String]) -> Result<(), String> {
        Ok(())
    }

    fn edit_file(&self, _path: &str, _line: Option<u32>) -> Result<(), String> {
        Err("no editor available".into())
    }

    fn alert(&self, _alert: &Alert) -> Result<(), String> {
        Ok(())
    }

    fn set_title(&self, _title: &str) -> Result<(), String> {
        Ok(())
    }

    fn raw_input(&self) -> Option<RawInput> {
        None
    }

    fn suspend_screen(&self) -> Result<(), String> {
        Ok(())
    }

    fn resume_screen(&self) -> Result<(), String> {
        Ok(())
    }

    fn write_raw(&self, _bytes: &[u8]) -> Result<(), String> {
        Ok(())
    }

    fn screen_size(&self) -> Option<(u16, u16)> {
        None
    }
}

#[derive(Debug, Clone, Default)]
pub struct SystemPlatform {
    drafts: Option<DraftDir>,
    recents: Option<PathBuf>,
    keyboard_enhanced: bool,
    focus_reporting: bool,
    tmux: bool,
}

impl SystemPlatform {
    pub fn new() -> Self {
        Self {
            drafts: default_dir().map(DraftDir::new),
            recents: recents_path(),
            keyboard_enhanced: false,
            focus_reporting: false,
            tmux: std::env::var_os("TMUX").is_some(),
        }
    }

    pub fn with_drafts(drafts: Option<DraftDir>) -> Self {
        Self {
            drafts,
            recents: None,
            keyboard_enhanced: false,
            focus_reporting: false,
            tmux: false,
        }
    }

    pub fn with_focus_reporting(mut self, enabled: bool) -> Self {
        self.focus_reporting = enabled;
        self
    }

    pub fn with_keyboard_enhancement(mut self, enhanced: bool) -> Self {
        self.keyboard_enhanced = enhanced;
        self
    }

    pub fn with_recents(mut self, recents: Option<PathBuf>) -> Self {
        self.recents = recents;
        self
    }
}

pub fn recents_path() -> Option<PathBuf> {
    dirs::state_dir()
        .or_else(dirs::data_local_dir)
        .map(|base| base.join("bb-tui").join("palette-recents.json"))
}

pub fn parse_recents(raw: &str) -> Vec<String> {
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(serde_json::Value::Array(values)) => values
            .into_iter()
            .filter_map(|value| match value {
                serde_json::Value::String(id) => Some(id),
                _ => None,
            })
            .take(crate::palette::RECENTS_LIMIT)
            .collect(),
        _ => Vec::new(),
    }
}

pub fn osc52_sequence(text: &str) -> String {
    format!("\u{1b}]52;c;{}\u{7}", STANDARD.encode(text))
}

fn sanitize(text: &str) -> String {
    text.chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect::<String>()
        .trim()
        .to_owned()
}

pub fn desktop_sequence(method: DesktopMethod, title: &str, body: &str) -> Option<String> {
    let title = sanitize(title);
    let body = sanitize(body);
    match method {
        DesktopMethod::Osc9 => {
            let message = if body.is_empty() {
                title
            } else {
                format!("{title}: {body}")
            };
            Some(format!("\u{1b}]9;{message}\u{7}"))
        }
        DesktopMethod::Osc777 => Some(format!(
            "\u{1b}]777;notify;{};{body}\u{7}",
            title.replace(';', ",")
        )),
        DesktopMethod::Auto | DesktopMethod::Native | DesktopMethod::Off => None,
    }
}

pub fn tmux_passthrough(sequence: &str) -> String {
    format!(
        "\u{1b}Ptmux;{}\u{1b}\\",
        sequence.replace('\u{1b}', "\u{1b}\u{1b}")
    )
}

pub fn alert_output(alert: &Alert, tmux: bool) -> String {
    let mut out = String::new();
    if alert.bell {
        out.push('\u{7}');
    }
    if let Some(desktop) = &alert.desktop
        && let Some(sequence) = desktop_sequence(desktop.method, &desktop.title, &desktop.body)
    {
        if tmux {
            out.push_str(&tmux_passthrough(&sequence));
        } else {
            out.push_str(&sequence);
        }
    }
    out
}

pub fn title_sequence(title: &str) -> String {
    format!("\u{1b}]2;{}\u{7}", sanitize(title))
}

pub const TITLE_PUSH: &str = "\u{1b}[22;2t";
pub const TITLE_POP: &str = "\u{1b}[23;2t";

#[cfg(feature = "notifications")]
fn native_notify(title: &str, body: &str) -> Result<(), String> {
    let title = title.to_owned();
    let body = body.to_owned();
    std::thread::spawn(move || {
        if let Err(error) = notify_rust::Notification::new()
            .appname("bb-tui")
            .summary(&title)
            .body(&body)
            .show()
        {
            tracing::debug!(%error, "desktop notification failed");
        }
    });
    Ok(())
}

#[cfg(not(feature = "notifications"))]
fn native_notify(_title: &str, _body: &str) -> Result<(), String> {
    Err("built without the `notifications` feature".into())
}

fn write_stdout(text: &str) -> Result<(), String> {
    write_bytes(text.as_bytes())
}

fn write_bytes(bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() {
        return Ok(());
    }
    let mut stdout = std::io::stdout();
    stdout
        .write_all(bytes)
        .and_then(|()| stdout.flush())
        .map_err(|error| error.to_string())
}

#[cfg(unix)]
fn spawn_raw_input() -> Option<RawInput> {
    use std::io::IsTerminal;
    use std::os::fd::AsFd;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use rustix::event::{PollFd, PollFlags, Timespec};

    if !std::io::stdin().is_terminal() {
        return None;
    }
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let handle = std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let fd = stdin.as_fd();
        let mut size = crossterm::terminal::size().ok();
        let mut buffer = vec![0u8; 16 * 1024];
        let timeout = Timespec {
            tv_sec: 0,
            tv_nsec: 50_000_000,
        };
        while !flag.load(Ordering::SeqCst) {
            let mut fds = [PollFd::new(&fd, PollFlags::IN)];
            match rustix::event::poll(&mut fds, Some(&timeout)) {
                Ok(0) => {}
                Ok(_) => {
                    let revents = fds[0].revents();
                    if revents.intersects(PollFlags::IN) {
                        match rustix::io::read(fd, &mut buffer[..]) {
                            Ok(0) | Err(_) => break,
                            Ok(read) => {
                                if tx.send(RawEvent::Bytes(buffer[..read].to_vec())).is_err() {
                                    break;
                                }
                            }
                        }
                    } else if revents.intersects(PollFlags::HUP | PollFlags::ERR | PollFlags::NVAL)
                    {
                        break;
                    }
                }
                Err(rustix::io::Errno::INTR) => {}
                Err(_) => break,
            }
            let current = crossterm::terminal::size().ok();
            if current != size {
                size = current;
                if let Some((cols, rows)) = current
                    && tx.send(RawEvent::Resize { cols, rows }).is_err()
                {
                    break;
                }
            }
        }
    });
    Some(RawInput::new(rx, move || {
        stop.store(true, Ordering::SeqCst);
        let _ = handle.join();
    }))
}

#[cfg(not(unix))]
fn spawn_raw_input() -> Option<RawInput> {
    None
}

pub fn open_command(target: &str) -> (&'static str, Vec<String>) {
    if cfg!(target_os = "macos") {
        ("open", vec![target.to_owned()])
    } else if cfg!(target_os = "windows") {
        (
            "cmd",
            vec![
                "/C".into(),
                "start".into(),
                String::new(),
                target.to_owned(),
            ],
        )
    } else {
        ("xdg-open", vec![target.to_owned()])
    }
}

pub fn editor_command(
    editor: &str,
    path: &str,
    line: Option<u32>,
) -> Option<(String, Vec<String>)> {
    let mut words = crate::palette::split_argv(editor).ok()?.into_iter();
    let program = words.next().filter(|program| !program.is_empty())?;
    let mut args: Vec<String> = words.collect();
    let name = std::path::Path::new(&program)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    if let Some(line) = line.filter(|line| *line > 0) {
        match name.as_str() {
            "vi" | "vim" | "nvim" | "gvim" | "nano" | "emacs" | "emacsclient" | "kak" | "micro"
            | "helix" | "hx" | "ne" | "joe" | "mg" => args.push(format!("+{line}")),
            "code" | "code-insiders" | "codium" | "cursor" | "subl" | "zed" => {
                args.push("--goto".into());
                args.push(format!("{path}:{line}"));
                return Some((program, args));
            }
            _ => {}
        }
    }
    args.push(path.to_owned());
    Some((program, args))
}

pub fn expand_home(target: &str) -> String {
    match (target.strip_prefix("~/"), dirs::home_dir()) {
        (Some(rest), Some(home)) => home.join(rest).to_string_lossy().into_owned(),
        _ => target.to_owned(),
    }
}

impl Platform for SystemPlatform {
    fn copy(&self, text: &str) -> Result<(), String> {
        let mut stdout = std::io::stdout();
        stdout
            .write_all(osc52_sequence(text).as_bytes())
            .and_then(|()| stdout.flush())
            .map_err(|error| error.to_string())
    }

    fn open(&self, target: &str) -> Result<(), String> {
        let target = expand_home(target);
        let (program, args) = open_command(&target);
        let mut child = Command::new(program)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("{program}: {error}"))?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }

    fn load_draft(&self, thread_id: &str) -> Option<String> {
        self.drafts.as_ref()?.load(thread_id)
    }

    fn save_draft(&self, thread_id: &str, raw: Option<&str>) -> Result<(), String> {
        match &self.drafts {
            Some(drafts) => drafts
                .save(thread_id, raw)
                .map_err(|error| format!("{}: {error}", drafts.root().display())),
            None => Ok(()),
        }
    }

    fn edit_file(&self, path: &str, line: Option<u32>) -> Result<(), String> {
        use crossterm::event::{
            DisableBracketedPaste, EnableBracketedPaste, KeyboardEnhancementFlags,
            PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
        };
        use crossterm::terminal::{
            EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
        };
        if !std::path::Path::new(path).exists() {
            return Err(format!("{path} is not on this machine"));
        }
        let editor = std::env::var("VISUAL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                std::env::var("EDITOR")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
            })
            .unwrap_or_else(|| "vi".into());
        let (program, args) = editor_command(&editor, path, line)
            .ok_or_else(|| format!("invalid editor {editor}"))?;
        let mut stdout = std::io::stdout();
        if self.keyboard_enhanced {
            let _ = crossterm::execute!(stdout, PopKeyboardEnhancementFlags);
        }
        if self.focus_reporting {
            let _ = crossterm::execute!(stdout, crossterm::event::DisableFocusChange);
        }
        let _ = crossterm::execute!(
            stdout,
            DisableBracketedPaste,
            LeaveAlternateScreen,
            crossterm::cursor::Show
        );
        let _ = disable_raw_mode();
        let status = Command::new(&program).args(&args).status();
        let _ = enable_raw_mode();
        let _ = crossterm::execute!(stdout, EnterAlternateScreen, EnableBracketedPaste);
        if self.focus_reporting {
            let _ = crossterm::execute!(stdout, crossterm::event::EnableFocusChange);
        }
        if self.keyboard_enhanced {
            let _ = crossterm::execute!(
                stdout,
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
            );
        }
        match status {
            Ok(status) if status.success() => Ok(()),
            Ok(status) => Err(format!("{program} exited with {status}")),
            Err(error) => Err(format!("{program}: {error}")),
        }
    }

    fn alert(&self, alert: &Alert) -> Result<(), String> {
        write_stdout(&alert_output(alert, self.tmux))?;
        match &alert.desktop {
            Some(desktop) if desktop.method == DesktopMethod::Native => {
                native_notify(&desktop.title, &desktop.body)
            }
            _ => Ok(()),
        }
    }

    fn set_title(&self, title: &str) -> Result<(), String> {
        write_stdout(&title_sequence(title))
    }

    fn raw_input(&self) -> Option<RawInput> {
        spawn_raw_input()
    }

    fn suspend_screen(&self) -> Result<(), String> {
        use crossterm::event::{DisableBracketedPaste, PopKeyboardEnhancementFlags};
        use crossterm::terminal::LeaveAlternateScreen;
        let mut stdout = std::io::stdout();
        if self.keyboard_enhanced {
            let _ = crossterm::execute!(stdout, PopKeyboardEnhancementFlags);
        }
        if self.focus_reporting {
            let _ = crossterm::execute!(stdout, crossterm::event::DisableFocusChange);
        }
        crossterm::execute!(
            stdout,
            DisableBracketedPaste,
            LeaveAlternateScreen,
            crossterm::cursor::Show
        )
        .map_err(|error| error.to_string())?;
        crossterm::terminal::enable_raw_mode().map_err(|error| error.to_string())
    }

    fn resume_screen(&self) -> Result<(), String> {
        use crossterm::event::{
            EnableBracketedPaste, KeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
        };
        use crossterm::terminal::EnterAlternateScreen;
        let mut stdout = std::io::stdout();
        let _ = crossterm::terminal::enable_raw_mode();
        let result = crossterm::execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)
            .map_err(|error| error.to_string());
        if self.focus_reporting {
            let _ = crossterm::execute!(stdout, crossterm::event::EnableFocusChange);
        }
        if self.keyboard_enhanced {
            let _ = crossterm::execute!(
                stdout,
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
            );
        }
        result
    }

    fn write_raw(&self, bytes: &[u8]) -> Result<(), String> {
        write_bytes(bytes)
    }

    fn screen_size(&self) -> Option<(u16, u16)> {
        crossterm::terminal::size().ok()
    }

    fn load_palette_recents(&self) -> Vec<String> {
        self.recents
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|raw| parse_recents(&raw))
            .unwrap_or_default()
    }

    fn save_palette_recents(&self, recents: &[String]) -> Result<(), String> {
        let Some(path) = &self.recents else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let raw = serde_json::to_string(recents).map_err(|error| error.to_string())?;
        std::fs::write(path, raw).map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc52_encodes_utf8_as_base64() {
        assert_eq!(osc52_sequence("hi ✓"), "\u{1b}]52;c;aGkg4pyT\u{7}");
    }

    #[test]
    fn palette_recents_round_trip_and_ignore_garbage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("recents.json");
        let platform = SystemPlatform::with_drafts(None).with_recents(Some(path.clone()));
        assert!(platform.load_palette_recents().is_empty());
        platform
            .save_palette_recents(&["app:thread.new".into(), "tui:Quit".into()])
            .unwrap();
        assert_eq!(
            platform.load_palette_recents(),
            ["app:thread.new", "tui:Quit"]
        );
        assert_eq!(parse_recents("{\"a\":1}"), Vec::<String>::new());
        assert_eq!(parse_recents("[1, \"x\"]"), ["x"]);
    }

    #[test]
    fn editor_command_adds_line_for_known_editors() {
        assert_eq!(
            editor_command("nvim", "/w/a.rs", Some(12)),
            Some(("nvim".into(), vec!["+12".into(), "/w/a.rs".into()]))
        );
        assert_eq!(
            editor_command("code --wait", "/w/a b.rs", Some(3)),
            Some((
                "code".into(),
                vec!["--wait".into(), "--goto".into(), "/w/a b.rs:3".into()]
            ))
        );
        assert_eq!(
            editor_command("'/opt/my ed'", "/w/a.rs", Some(3)),
            Some(("/opt/my ed".into(), vec!["/w/a.rs".into()]))
        );
        assert_eq!(editor_command("  ", "/w/a.rs", None), None);
    }

    #[test]
    fn desktop_sequences_strip_control_characters() {
        use crate::notifications::DesktopNotice;
        assert_eq!(
            desktop_sequence(DesktopMethod::Osc9, "Fix\u{7} bug", "done\nnow").as_deref(),
            Some("\u{1b}]9;Fix  bug: done now\u{7}")
        );
        assert_eq!(
            desktop_sequence(DesktopMethod::Osc777, "a;b", "c;d").as_deref(),
            Some("\u{1b}]777;notify;a,b;c;d\u{7}")
        );
        assert_eq!(desktop_sequence(DesktopMethod::Native, "t", "b"), None);
        let alert = Alert {
            bell: true,
            desktop: Some(DesktopNotice {
                method: DesktopMethod::Osc9,
                title: "t".into(),
                body: String::new(),
            }),
        };
        assert_eq!(alert_output(&alert, false), "\u{7}\u{1b}]9;t\u{7}");
        assert_eq!(
            alert_output(&alert, true),
            "\u{7}\u{1b}Ptmux;\u{1b}\u{1b}]9;t\u{7}\u{1b}\\"
        );
        assert_eq!(title_sequence("(2) bb\u{1b}"), "\u{1b}]2;(2) bb\u{7}");
    }

    #[test]
    fn open_command_passes_target_verbatim() {
        let (_, args) = open_command("https://example.com/a b");
        assert_eq!(
            args.last().map(String::as_str),
            Some("https://example.com/a b")
        );
    }
}
