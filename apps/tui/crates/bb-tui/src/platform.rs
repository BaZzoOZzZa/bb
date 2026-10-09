use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

use crate::drafts::{DraftDir, default_dir};

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
}

#[derive(Debug, Clone, Default)]
pub struct SystemPlatform {
    drafts: Option<DraftDir>,
    recents: Option<PathBuf>,
    keyboard_enhanced: bool,
}

impl SystemPlatform {
    pub fn new() -> Self {
        Self {
            drafts: default_dir().map(DraftDir::new),
            recents: recents_path(),
            keyboard_enhanced: false,
        }
    }

    pub fn with_drafts(drafts: Option<DraftDir>) -> Self {
        Self {
            drafts,
            recents: None,
            keyboard_enhanced: false,
        }
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
    fn open_command_passes_target_verbatim() {
        let (_, args) = open_command("https://example.com/a b");
        assert_eq!(
            args.last().map(String::as_str),
            Some("https://example.com/a b")
        );
    }
}
