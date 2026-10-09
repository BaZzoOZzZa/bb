use bb_wire::timeline::FileChange;

use crate::file_change::{FileChangeAction, file_change_action, is_patch_metadata_line};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderablePatchText {
    pub disable_line_numbers: bool,
    pub patch: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SyntheticAction {
    Created,
    Deleted,
}

fn split_patch_lines(diff: &str) -> Vec<String> {
    let normalized = diff.replace("\r\n", "\n");
    if normalized.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<String> = normalized.split('\n').map(str::to_owned).collect();
    if lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}

fn patch_body_lines(diff: Option<&str>) -> Vec<String> {
    diff.filter(|diff| !diff.is_empty())
        .map(split_patch_lines)
        .unwrap_or_default()
        .into_iter()
        .filter(|line| !is_patch_metadata_line(line))
        .collect()
}

fn normalize_patch_path(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches('/').to_owned()
}

fn synthetic_body_lines(lines: &[String], action: SyntheticAction) -> Vec<String> {
    let (content, opposite) = match action {
        SyntheticAction::Created => ('+', '-'),
        SyntheticAction::Deleted => ('-', '+'),
    };
    lines
        .iter()
        .filter_map(|line| {
            if line.starts_with(content) {
                Some(line.clone())
            } else if line.starts_with(opposite) || line.starts_with(' ') {
                None
            } else {
                Some(format!("{content}{line}"))
            }
        })
        .collect()
}

fn to_synthetic_patch(change: &FileChange, action: SyntheticAction) -> Option<String> {
    let lines = patch_body_lines(change.diff.as_deref());
    if lines.is_empty() {
        return None;
    }
    let path = normalize_patch_path(&change.path);
    let (from, to) = match action {
        SyntheticAction::Created => ("/dev/null".to_owned(), format!("b/{path}")),
        SyntheticAction::Deleted => (format!("a/{path}"), "/dev/null".to_owned()),
    };
    let body = synthetic_body_lines(&lines, action);
    if body.is_empty() {
        return None;
    }
    let (old_count, new_count) = match action {
        SyntheticAction::Created => (0, body.len()),
        SyntheticAction::Deleted => (body.len(), 0),
    };
    Some(format!(
        "diff --git a/{path} b/{path}\n--- {from}\n+++ {to}\n@@ -1,{old_count} +1,{new_count} @@\n{}\n",
        body.join("\n")
    ))
}

fn to_synthetic_update_patch(change: &FileChange) -> Option<String> {
    let body = patch_body_lines(change.diff.as_deref());
    if !body
        .iter()
        .any(|line| line.starts_with('+') || line.starts_with('-'))
    {
        return None;
    }
    let path = normalize_patch_path(change.move_path.as_deref().unwrap_or(&change.path));
    let removed = body.iter().filter(|line| line.starts_with('-')).count();
    let added = body.iter().filter(|line| line.starts_with('+')).count();
    Some(format!(
        "diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -1,{} +1,{} @@\n{}\n",
        removed.max(1),
        added.max(1),
        body.join("\n")
    ))
}

pub fn renderable_patch_text(change: &FileChange) -> Option<RenderablePatchText> {
    if let Some(patch) = change
        .diff
        .as_deref()
        .filter(|patch| !patch.trim().is_empty())
    {
        let trimmed = patch.trim_end();
        if trimmed.starts_with("diff --git")
            || (trimmed.contains("--- ") && trimmed.contains("+++ ") && trimmed.contains("@@"))
        {
            return Some(RenderablePatchText {
                patch: patch.to_owned(),
                disable_line_numbers: false,
            });
        }
        if patch.contains("@@") {
            let path = normalize_patch_path(change.move_path.as_deref().unwrap_or(&change.path));
            return Some(RenderablePatchText {
                patch: format!(
                    "diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n{}\n",
                    patch.trim_end()
                ),
                disable_line_numbers: false,
            });
        }
    }
    let synthetic = match file_change_action(change) {
        FileChangeAction::Created => to_synthetic_patch(change, SyntheticAction::Created),
        FileChangeAction::Deleted => to_synthetic_patch(change, SyntheticAction::Deleted),
        FileChangeAction::Edited | FileChangeAction::Renamed => None,
    }
    .or_else(|| to_synthetic_update_patch(change))?;
    Some(RenderablePatchText {
        patch: synthetic,
        disable_line_numbers: true,
    })
}

pub fn plain_diff_fallback(change: &FileChange, has_renderable_patch: bool) -> Option<String> {
    if has_renderable_patch {
        return None;
    }
    change
        .diff
        .as_deref()
        .map(str::trim_end)
        .filter(|diff| !diff.is_empty())
        .map(str::to_owned)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchLineKind {
    Context,
    Added,
    Removed,
    NoNewline,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchLine {
    pub kind: PatchLineKind,
    pub text: String,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchHunk {
    pub header: String,
    pub section: String,
    pub old_start: u32,
    pub old_count: u32,
    pub new_start: u32,
    pub new_count: u32,
    pub lines: Vec<PatchLine>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PatchFile {
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub is_new: bool,
    pub is_deleted: bool,
    pub is_binary: bool,
    pub hunks: Vec<PatchHunk>,
}

impl PatchFile {
    pub fn display_path(&self) -> Option<&str> {
        self.new_path.as_deref().or(self.old_path.as_deref())
    }

    pub fn stats(&self) -> (usize, usize) {
        self.hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .fold((0, 0), |(added, removed), line| match line.kind {
                PatchLineKind::Added => (added + 1, removed),
                PatchLineKind::Removed => (added, removed + 1),
                _ => (added, removed),
            })
    }
}

fn parse_range(value: &str) -> Option<(u32, u32)> {
    match value.split_once(',') {
        Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
        None => Some((value.parse().ok()?, 1)),
    }
}

pub fn parse_hunk_header(line: &str) -> Option<(u32, u32, u32, u32, String)> {
    let rest = line.strip_prefix("@@ -")?;
    let (ranges, section) = rest.split_once(" @@")?;
    let (old, new) = ranges.split_once(" +")?;
    let (old_start, old_count) = parse_range(old)?;
    let (new_start, new_count) = parse_range(new)?;
    Some((
        old_start,
        old_count,
        new_start,
        new_count,
        section.trim().to_owned(),
    ))
}

fn strip_path_prefix(path: &str) -> Option<String> {
    let path = path.split('\t').next().unwrap_or(path).trim_end();
    if path == "/dev/null" {
        return None;
    }
    Some(
        path.strip_prefix("a/")
            .or_else(|| path.strip_prefix("b/"))
            .unwrap_or(path)
            .to_owned(),
    )
}

fn parse_git_header_paths(rest: &str) -> (Option<String>, Option<String>) {
    match rest.split_once(" b/") {
        Some((old, new)) => (
            Some(old.strip_prefix("a/").unwrap_or(old).to_owned()),
            Some(new.to_owned()),
        ),
        None => (None, None),
    }
}

pub fn parse_unified_patch(patch: &str) -> Vec<PatchFile> {
    let normalized = patch.replace("\r\n", "\n");
    let mut files: Vec<PatchFile> = Vec::new();
    let mut current: Option<PatchFile> = None;
    let mut old_line = 0u32;
    let mut new_line = 0u32;
    let mut old_remaining = 0u32;
    let mut new_remaining = 0u32;
    let mut in_hunk = false;
    let mut lines: Vec<&str> = normalized.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    for line in lines {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            if let Some(file) = current.take() {
                files.push(file);
            }
            let (old_path, new_path) = parse_git_header_paths(rest);
            current = Some(PatchFile {
                old_path,
                new_path,
                ..PatchFile::default()
            });
            in_hunk = false;
            continue;
        }
        if let Some((old_start, old_count, new_start, new_count, section)) = parse_hunk_header(line)
        {
            let file = current.get_or_insert_with(PatchFile::default);
            file.hunks.push(PatchHunk {
                header: line.to_owned(),
                section,
                old_start,
                old_count,
                new_start,
                new_count,
                lines: Vec::new(),
            });
            old_line = old_start;
            new_line = new_start;
            old_remaining = old_count;
            new_remaining = new_count;
            in_hunk = old_count > 0 || new_count > 0;
            continue;
        }
        if !in_hunk && line.starts_with('\\') {
            if let Some(hunk) = current.as_mut().and_then(|file| file.hunks.last_mut()) {
                hunk.lines.push(PatchLine {
                    kind: PatchLineKind::NoNewline,
                    text: line.to_owned(),
                    old_line: None,
                    new_line: None,
                });
            }
            continue;
        }
        if !in_hunk {
            if line.starts_with("--- ")
                && current.as_ref().is_some_and(|file| !file.hunks.is_empty())
                && let Some(file) = current.take()
            {
                files.push(file);
            }
            let file = current.get_or_insert_with(PatchFile::default);
            if let Some(path) = line.strip_prefix("--- ") {
                file.old_path = strip_path_prefix(path);
                if file.old_path.is_none() {
                    file.is_new = true;
                }
            } else if let Some(path) = line.strip_prefix("+++ ") {
                file.new_path = strip_path_prefix(path);
                if file.new_path.is_none() {
                    file.is_deleted = true;
                }
            } else if line.starts_with("new file mode ") {
                file.is_new = true;
            } else if line.starts_with("deleted file mode ") {
                file.is_deleted = true;
            } else if let Some(path) = line.strip_prefix("rename from ") {
                file.old_path = Some(path.to_owned());
            } else if let Some(path) = line.strip_prefix("rename to ") {
                file.new_path = Some(path.to_owned());
            } else if line.starts_with("Binary files ") || line == "GIT binary patch" {
                file.is_binary = true;
            }
            continue;
        }
        let Some(hunk) = current.as_mut().and_then(|file| file.hunks.last_mut()) else {
            continue;
        };
        let (kind, text, old, new) = if let Some(text) = line.strip_prefix('+') {
            let entry = (PatchLineKind::Added, text, None, Some(new_line));
            new_line += 1;
            new_remaining = new_remaining.saturating_sub(1);
            entry
        } else if let Some(text) = line.strip_prefix('-') {
            let entry = (PatchLineKind::Removed, text, Some(old_line), None);
            old_line += 1;
            old_remaining = old_remaining.saturating_sub(1);
            entry
        } else if line.starts_with('\\') {
            (PatchLineKind::NoNewline, line, None, None)
        } else {
            let text = line.strip_prefix(' ').unwrap_or(line);
            let entry = (PatchLineKind::Context, text, Some(old_line), Some(new_line));
            old_line += 1;
            new_line += 1;
            old_remaining = old_remaining.saturating_sub(1);
            new_remaining = new_remaining.saturating_sub(1);
            entry
        };
        if old_remaining == 0 && new_remaining == 0 {
            in_hunk = false;
        }
        hunk.lines.push(PatchLine {
            kind,
            text: text.to_owned(),
            old_line: old,
            new_line: new,
        });
    }
    if let Some(file) = current {
        files.push(file);
    }
    files
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderableDiff {
    pub disable_line_numbers: bool,
    pub files: Vec<PatchFile>,
}

pub fn renderable_diff(change: &FileChange) -> Option<RenderableDiff> {
    let text = renderable_patch_text(change)?;
    let files = parse_unified_patch(&text.patch);
    let files = if text.disable_line_numbers {
        files
            .into_iter()
            .map(|mut file| {
                for line in file.hunks.iter_mut().flat_map(|hunk| hunk.lines.iter_mut()) {
                    line.old_line = None;
                    line.new_line = None;
                }
                file
            })
            .collect()
    } else {
        files
    };
    Some(RenderableDiff {
        disable_line_numbers: text.disable_line_numbers,
        files,
    })
}

#[cfg(test)]
mod tests {
    use bb_wire::timeline::DiffStats;

    use super::*;

    fn change(path: &str, kind: &str, diff: Option<&str>) -> FileChange {
        FileChange {
            path: path.into(),
            kind: Some(kind.into()),
            move_path: None,
            diff: diff.map(Into::into),
            diff_stats: DiffStats::default(),
        }
    }

    #[test]
    fn keeps_full_git_patches_verbatim() {
        let patch = "diff --git a/a.ts b/a.ts\n--- a/a.ts\n+++ b/a.ts\n@@ -1 +1 @@\n-x\n+y\n";
        let text = renderable_patch_text(&change("a.ts", "update", Some(patch))).unwrap();
        assert_eq!(text.patch, patch);
        assert!(!text.disable_line_numbers);
    }

    #[test]
    fn wraps_bare_hunks_with_headers() {
        let text = renderable_patch_text(&change(
            "\\src\\a.ts",
            "update",
            Some("@@ -3,2 +3,2 @@\n a\n-b\n+c\n"),
        ))
        .unwrap();
        assert_eq!(
            text.patch,
            "diff --git a/src/a.ts b/src/a.ts\n--- a/src/a.ts\n+++ b/src/a.ts\n@@ -3,2 +3,2 @@\n a\n-b\n+c\n"
        );
        let files = parse_unified_patch(&text.patch);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].display_path(), Some("src/a.ts"));
        let lines = &files[0].hunks[0].lines;
        assert_eq!(
            lines
                .iter()
                .map(|line| (line.kind, line.old_line, line.new_line))
                .collect::<Vec<_>>(),
            vec![
                (PatchLineKind::Context, Some(3), Some(3)),
                (PatchLineKind::Removed, Some(4), None),
                (PatchLineKind::Added, None, Some(4)),
            ]
        );
    }

    #[test]
    fn synthesizes_created_and_deleted_patches() {
        let created = renderable_patch_text(&change("/new.md", "add", Some("one\ntwo\n"))).unwrap();
        assert_eq!(
            created.patch,
            "diff --git a/new.md b/new.md\n--- /dev/null\n+++ b/new.md\n@@ -1,0 +1,2 @@\n+one\n+two\n"
        );
        assert!(created.disable_line_numbers);
        let deleted =
            renderable_patch_text(&change("old.md", "delete", Some("-gone\n kept\n"))).unwrap();
        assert_eq!(
            deleted.patch,
            "diff --git a/old.md b/old.md\n--- a/old.md\n+++ /dev/null\n@@ -1,1 +1,0 @@\n-gone\n"
        );
        let parsed = renderable_diff(&change("/new.md", "add", Some("one\ntwo\n"))).unwrap();
        assert!(parsed.files[0].is_new);
        assert_eq!(parsed.files[0].stats(), (2, 0));
        assert!(
            parsed.files[0].hunks[0]
                .lines
                .iter()
                .all(|line| line.new_line.is_none())
        );
    }

    #[test]
    fn synthesizes_update_patches_and_falls_back_to_plain_text() {
        let update =
            renderable_patch_text(&change("a.ts", "update", Some("-before\n+after"))).unwrap();
        assert_eq!(
            update.patch,
            "diff --git a/a.ts b/a.ts\n--- a/a.ts\n+++ b/a.ts\n@@ -1,1 +1,1 @@\n-before\n+after\n"
        );
        let plain = change("a.ts", "update", Some("just text\n"));
        assert_eq!(renderable_patch_text(&plain), None);
        assert_eq!(
            plain_diff_fallback(&plain, false).as_deref(),
            Some("just text")
        );
        assert_eq!(plain_diff_fallback(&plain, true), None);
        assert_eq!(renderable_patch_text(&change("a.ts", "update", None)), None);
    }

    #[test]
    fn ends_hunks_by_line_counts() {
        let patch = "--- a/one.txt\n+++ b/one.txt\n@@ -1 +1 @@\n-a\n+b\n--- a/two.txt\n+++ b/two.txt\n@@ -1,2 +1 @@\n-x\n y\n";
        let files = parse_unified_patch(patch);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].hunks[0].lines.len(), 2);
        assert_eq!(files[0].display_path(), Some("one.txt"));
        assert_eq!(files[1].display_path(), Some("two.txt"));
        assert_eq!(files[1].stats(), (0, 1));
    }

    #[test]
    fn parses_multi_file_patches_with_metadata() {
        let patch = "diff --git a/old.rs b/new.rs\nsimilarity index 90%\nrename from old.rs\nrename to new.rs\n--- a/old.rs\n+++ b/new.rs\n@@ -10,3 +10,3 @@ fn main() {\n ctx\n-old\n+new\n\\ No newline at end of file\ndiff --git a/gone.rs b/gone.rs\ndeleted file mode 100644\n--- a/gone.rs\n+++ /dev/null\n@@ -1 +0,0 @@\n-bye\n";
        let files = parse_unified_patch(patch);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].old_path.as_deref(), Some("old.rs"));
        assert_eq!(files[0].new_path.as_deref(), Some("new.rs"));
        assert_eq!(files[0].hunks[0].section, "fn main() {");
        assert_eq!(files[0].hunks[0].lines[3].kind, PatchLineKind::NoNewline);
        assert!(files[1].is_deleted);
        assert_eq!(files[1].display_path(), Some("gone.rs"));
        assert_eq!(files[1].hunks[0].old_count, 1);
        assert_eq!(files[1].stats(), (0, 1));
    }
}
