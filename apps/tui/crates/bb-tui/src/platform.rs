use std::io::Write;
use std::process::{Command, Stdio};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

pub trait Platform: Send + Sync + 'static {
    fn copy(&self, text: &str) -> Result<(), String>;

    fn open(&self, target: &str) -> Result<(), String>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemPlatform;

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
