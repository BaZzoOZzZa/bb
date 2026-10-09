use std::io::Write;
use std::process::{Command, Stdio};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

use crate::drafts::{DraftDir, default_dir};

pub trait Platform: Send + Sync + 'static {
    fn copy(&self, text: &str) -> Result<(), String>;

    fn open(&self, target: &str) -> Result<(), String>;

    fn load_draft(&self, thread_id: &str) -> Option<String>;

    fn save_draft(&self, thread_id: &str, raw: Option<&str>) -> Result<(), String>;
}

#[derive(Debug, Clone, Default)]
pub struct SystemPlatform {
    drafts: Option<DraftDir>,
}

impl SystemPlatform {
    pub fn new() -> Self {
        Self {
            drafts: default_dir().map(DraftDir::new),
        }
    }

    pub fn with_drafts(drafts: Option<DraftDir>) -> Self {
        Self { drafts }
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc52_encodes_utf8_as_base64() {
        assert_eq!(osc52_sequence("hi ✓"), "\u{1b}]52;c;aGkg4pyT\u{7}");
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
