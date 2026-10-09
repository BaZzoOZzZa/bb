use std::path::{Path, PathBuf};

use bb_wire::project::{UploadedAttachmentType, UploadedPromptAttachment};

use crate::app::{AppState, Effect, Mode, ToastLevel};
use crate::input::LineInput;
use crate::palette::split_argv;
use crate::platform::expand_home;
use crate::prompt::{AttachmentKind, AttachmentOwner, DraftAttachment};
use crate::typeahead::ComposerTarget;

pub const ATTACHMENT_MAX_BYTES: u64 = 35 * 1024 * 1024;
pub const ATTACH_COMMAND: &str = ":attach";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadRequest {
    pub target: ComposerTarget,
    pub project_id: String,
    pub path: PathBuf,
}

pub fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

pub fn mime_type_for(path: &Path) -> Option<String> {
    let extension = path.extension()?.to_string_lossy().to_ascii_lowercase();
    let mime = match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "heic" => "image/heic",
        "heif" => "image/heif",
        "pdf" => "application/pdf",
        "json" => "application/json",
        "zip" => "application/zip",
        "gz" => "application/gzip",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "md" | "markdown" => "text/markdown",
        "txt" | "log" => "text/plain",
        "xml" => "application/xml",
        "yaml" | "yml" => "application/yaml",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        _ => return None,
    };
    Some(mime.to_owned())
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = text.get(index + 1..index + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

pub fn normalize_path_word(word: &str) -> Option<PathBuf> {
    let word = word.trim();
    if word.is_empty() {
        return None;
    }
    let path = match word.strip_prefix("file://") {
        Some(rest) => {
            let rest = rest.strip_prefix("localhost").unwrap_or(rest);
            percent_decode(rest)?
        }
        None => expand_home(word),
    };
    let path = PathBuf::from(path);
    path.is_absolute().then_some(path)
}

pub fn parse_path_list(text: &str) -> Option<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let words = split_argv(line).ok()?;
        if words.is_empty() {
            return None;
        }
        for word in words {
            paths.push(normalize_path_word(&word)?);
        }
    }
    (!paths.is_empty()).then_some(paths)
}

pub fn pasted_file_paths(text: &str, is_file: impl Fn(&Path) -> bool) -> Option<Vec<PathBuf>> {
    let whole = text.trim();
    if whole.is_empty() {
        return None;
    }
    if let Some(path) = normalize_path_word(whole).filter(|path| is_file(path)) {
        return Some(vec![path]);
    }
    let paths = parse_path_list(whole)?;
    paths.iter().all(|path| is_file(path)).then_some(paths)
}

pub fn attach_command_argument(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    let rest = trimmed.strip_prefix(ATTACH_COMMAND)?;
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    Some(rest.trim())
}

pub fn draft_attachment(uploaded: &UploadedPromptAttachment) -> DraftAttachment {
    DraftAttachment {
        kind: match uploaded.attachment_type {
            UploadedAttachmentType::LocalImage => AttachmentKind::LocalImage,
            _ => AttachmentKind::LocalFile,
        },
        path: uploaded.path.clone(),
        name: uploaded.name.clone(),
        mime_type: uploaded.mime_type.clone().filter(|mime| !mime.is_empty()),
        size_bytes: Some(uploaded.size_bytes).filter(|size| *size > 0),
        owner: uploaded
            .source_project_id
            .clone()
            .filter(|project| !project.is_empty())
            .map_or(AttachmentOwner::None, AttachmentOwner::Project),
    }
}

pub fn complete_path(
    text: &str,
    list_dir: impl Fn(&Path) -> Vec<(String, bool)>,
) -> Option<String> {
    let expanded = expand_home(text);
    let index = expanded.rfind('/')?;
    let (dir, prefix) = (&expanded[..=index], &expanded[index + 1..]);
    let entries: Vec<(String, bool)> = list_dir(Path::new(dir))
        .into_iter()
        .filter(|(name, _)| {
            name.starts_with(prefix) && (!name.starts_with('.') || prefix.starts_with('.'))
        })
        .collect();
    let first = entries.first()?;
    let mut common = first.0.clone();
    for (name, _) in &entries[1..] {
        let shared = common
            .chars()
            .zip(name.chars())
            .take_while(|(left, right)| left == right)
            .map(|(ch, _)| ch.len_utf8())
            .sum();
        common.truncate(shared);
    }
    let suffix = if entries.len() == 1 && first.1 {
        "/"
    } else {
        ""
    };
    let completed = format!("{dir}{common}{suffix}");
    (completed != expanded).then_some(completed)
}

pub fn list_dir(path: &Path) -> Vec<(String, bool)> {
    let Ok(entries) = std::fs::read_dir(path) else {
        return Vec::new();
    };
    let mut names: Vec<(String, bool)> = entries
        .flatten()
        .map(|entry| {
            let is_dir = entry.path().is_dir();
            (entry.file_name().to_string_lossy().into_owned(), is_dir)
        })
        .collect();
    names.sort();
    names
}

impl AppState {
    pub fn upload_project_id(&self, target: &ComposerTarget) -> Option<String> {
        match target {
            ComposerTarget::Thread(thread_id) => self
                .sidebar_entry(thread_id)
                .map(|entry| entry.thread.project_id.clone())
                .or_else(|| {
                    self.snapshot
                        .thread(thread_id)
                        .map(|thread| thread.thread.project_id.clone())
                }),
            ComposerTarget::NewThread => {
                self.new_thread.as_ref().map(|form| form.project_id.clone())
            }
        }
        .filter(|project_id| !project_id.is_empty())
    }

    pub(crate) fn attach_paths(
        &mut self,
        target: &ComposerTarget,
        paths: Vec<PathBuf>,
    ) -> Vec<Effect> {
        if self.read_only {
            self.toast(
                "Read-only mode: cannot upload attachments",
                ToastLevel::Error,
            );
            return Vec::new();
        }
        let Some(project_id) = self.upload_project_id(target) else {
            self.toast("Attachments need a project", ToastLevel::Error);
            return Vec::new();
        };
        let Some(composer) = self.composer_for_mut(target) else {
            return Vec::new();
        };
        let mut effects = Vec::new();
        for path in paths {
            let key = path.to_string_lossy().into_owned();
            if composer.uploads.contains(&key) {
                continue;
            }
            composer.uploads.push(key);
            effects.push(Effect::Upload(UploadRequest {
                target: target.clone(),
                project_id: project_id.clone(),
                path,
            }));
        }
        effects
    }

    pub(crate) fn attach_from_text(&mut self, target: &ComposerTarget, text: &str) -> Vec<Effect> {
        if let Some(path) = normalize_path_word(text).filter(|path| path.is_file()) {
            return self.attach_paths(target, vec![path]);
        }
        match parse_path_list(text) {
            Some(paths) => self.attach_paths(target, paths),
            None => {
                self.toast(
                    "Give an absolute path (or ~/…) to attach",
                    ToastLevel::Error,
                );
                Vec::new()
            }
        }
    }

    pub(crate) fn open_attach_prompt(&mut self) -> Vec<Effect> {
        match self.attachment_target() {
            Some(_) if self.read_only => {
                self.toast(
                    "Read-only mode: cannot upload attachments",
                    ToastLevel::Error,
                );
            }
            Some(target) => {
                let start = dirs::home_dir()
                    .map(|home| format!("{}/", home.to_string_lossy()))
                    .unwrap_or_default();
                self.mode = Mode::AttachPrompt {
                    target,
                    input: LineInput::new(start),
                };
            }
            None => self.toast(
                "Open a thread or the new-thread form first",
                ToastLevel::Info,
            ),
        }
        Vec::new()
    }

    pub(crate) fn open_attachment_list(&mut self) -> Vec<Effect> {
        let Some(target) = self.attachment_target() else {
            self.toast(
                "Open a thread or the new-thread form first",
                ToastLevel::Info,
            );
            return Vec::new();
        };
        let count = self
            .composer_for(&target)
            .map_or(0, |composer| composer.draft().attachments.len());
        if count == 0 {
            self.toast("No attachments", ToastLevel::Info);
        } else {
            self.mode = Mode::Attachments {
                target,
                selected: count - 1,
            };
        }
        Vec::new()
    }

    pub(crate) fn attachment_uploaded(
        &mut self,
        target: ComposerTarget,
        path: PathBuf,
        result: Result<UploadedPromptAttachment, String>,
    ) {
        let now = self.clock;
        let key = path.to_string_lossy().into_owned();
        let name = file_name(&path);
        let Some(composer) = self.composer_for_mut(&target) else {
            return;
        };
        composer.uploads.retain(|pending| *pending != key);
        match result {
            Ok(uploaded) => {
                composer.add_attachment(draft_attachment(&uploaded), now);
            }
            Err(error) => self.toast(
                format!("Failed to attach {name}: {error}"),
                ToastLevel::Error,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pasted_paths_accept_uris_quotes_and_escapes() {
        let all = |_: &Path| true;
        assert_eq!(
            pasted_file_paths("/tmp/a b.png", all),
            Some(vec![PathBuf::from("/tmp/a b.png")])
        );
        assert_eq!(
            pasted_file_paths("'/tmp/x y.txt' /tmp/z\\ w.md", all),
            Some(vec![
                PathBuf::from("/tmp/x y.txt"),
                PathBuf::from("/tmp/z w.md")
            ])
        );
        assert_eq!(
            pasted_file_paths("file:///tmp/%D1%84%D0%B0%D0%B9%D0%BB.txt\n", all),
            Some(vec![PathBuf::from("/tmp/файл.txt")])
        );
        assert_eq!(pasted_file_paths("hello world", all), None);
        assert_eq!(pasted_file_paths("relative/path.rs", all), None);
        assert_eq!(pasted_file_paths("/tmp/missing", |_| false), None);
    }

    #[test]
    fn attach_command_needs_a_separator() {
        assert_eq!(attach_command_argument(":attach /a"), Some("/a"));
        assert_eq!(attach_command_argument("  :attach  "), Some(""));
        assert_eq!(attach_command_argument(":attachment"), None);
        assert_eq!(attach_command_argument("see :attach"), None);
    }

    #[test]
    fn completion_extends_the_common_prefix() {
        let list = |_: &Path| {
            vec![
                ("report-2024.pdf".to_owned(), false),
                ("report-2025.pdf".to_owned(), false),
                ("src".to_owned(), true),
            ]
        };
        assert_eq!(complete_path("/w/rep", list), Some("/w/report-202".into()));
        assert_eq!(complete_path("/w/s", list), Some("/w/src/".into()));
        assert_eq!(complete_path("/w/zz", list), None);
    }

    #[test]
    fn uploaded_attachment_keeps_its_project_owner() {
        let uploaded = UploadedPromptAttachment {
            source_project_id: Some("proj_1".into()),
            attachment_type: UploadedAttachmentType::LocalImage,
            path: "attachments/a.png".into(),
            name: "a.png".into(),
            mime_type: Some("image/png".into()),
            size_bytes: 12,
        };
        let draft = draft_attachment(&uploaded);
        assert_eq!(draft.kind, AttachmentKind::LocalImage);
        assert_eq!(draft.owner, AttachmentOwner::Project("proj_1".into()));
        assert_eq!(draft.size_bytes, Some(12));
    }
}
