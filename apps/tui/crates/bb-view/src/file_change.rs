use bb_wire::timeline::{DiffStats, FileChange};

use crate::path_display::{PathDisplayMode, format_timeline_path};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileChangeAction {
    Created,
    Deleted,
    Renamed,
    Edited,
}

impl FileChangeAction {
    pub fn past_tense(self) -> &'static str {
        match self {
            Self::Created => "Created",
            Self::Deleted => "Deleted",
            Self::Renamed => "Renamed",
            Self::Edited => "Edited",
        }
    }

    pub fn present_tense(self) -> &'static str {
        match self {
            Self::Created => "Creating",
            Self::Deleted => "Deleting",
            Self::Renamed => "Renaming",
            Self::Edited => "Editing",
        }
    }

    pub fn infinitive(self) -> &'static str {
        match self {
            Self::Created => "create",
            Self::Deleted => "delete",
            Self::Renamed => "rename",
            Self::Edited => "edit",
        }
    }
}

pub(crate) fn non_empty(value: Option<&String>) -> Option<&str> {
    value.map(String::as_str).filter(|value| !value.is_empty())
}

fn normalize_file_change_kind(kind: Option<&str>) -> String {
    kind.unwrap_or_default()
        .to_lowercase()
        .chars()
        .filter(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit())
        .collect()
}

pub fn is_patch_metadata_line(line: &str) -> bool {
    let line = line.trim_end();
    line.starts_with("diff --git ")
        || line.starts_with("index ")
        || line.starts_with("new file mode ")
        || line.starts_with("deleted file mode ")
        || line.starts_with("similarity index ")
        || line.starts_with("rename from ")
        || line.starts_with("rename to ")
        || line.starts_with("--- ")
        || line.starts_with("+++ ")
        || line.starts_with("@@")
        || line == "\\ No newline at end of file"
}

fn has_substantive_diff(change: &FileChange) -> bool {
    let Some(diff) = non_empty(change.diff.as_ref()) else {
        return change.diff_stats.added + change.diff_stats.removed > 0;
    };
    diff.split('\n').any(|line| {
        !(line.starts_with("+++ ") || line.starts_with("--- "))
            && (line.starts_with('+') || line.starts_with('-'))
    })
}

pub fn file_change_action(change: &FileChange) -> FileChangeAction {
    if non_empty(change.move_path.as_ref()).is_some() {
        return if has_substantive_diff(change) {
            FileChangeAction::Edited
        } else {
            FileChangeAction::Renamed
        };
    }
    let kind = normalize_file_change_kind(change.kind.as_deref());
    if kind.contains("add") || kind.contains("create") {
        FileChangeAction::Created
    } else if kind.contains("delete") || kind.contains("remove") {
        FileChangeAction::Deleted
    } else {
        FileChangeAction::Edited
    }
}

pub fn file_change_identity(change: &FileChange) -> &str {
    change.move_path.as_deref().unwrap_or(&change.path)
}

pub fn format_file_change_path(change: &FileChange, mode: PathDisplayMode) -> String {
    let source = format_timeline_path(&change.path, mode);
    let Some(move_path) = non_empty(change.move_path.as_ref()) else {
        return source;
    };
    if file_change_action(change) == FileChangeAction::Edited {
        return format_timeline_path(move_path, mode);
    }
    format!("{source} -> {}", format_timeline_path(move_path, mode))
}

fn count_plain_content_lines(diff: &str) -> u64 {
    diff.split('\n')
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .filter(|line| !is_patch_metadata_line(line))
        .count() as u64
}

fn has_patch_metadata(diff: &str) -> bool {
    diff.split('\n').any(is_patch_metadata_line)
}

pub fn file_change_diff_stats(change: &FileChange) -> DiffStats {
    let Some(diff) = non_empty(change.diff.as_ref()) else {
        return DiffStats::default();
    };
    let action = file_change_action(change);
    let plain = count_plain_content_lines(diff);
    if !has_patch_metadata(diff) {
        match action {
            FileChangeAction::Created => {
                return DiffStats {
                    added: plain,
                    removed: 0,
                };
            }
            FileChangeAction::Deleted => {
                return DiffStats {
                    added: 0,
                    removed: plain,
                };
            }
            FileChangeAction::Renamed | FileChangeAction::Edited => {}
        }
    }
    let mut added = 0;
    let mut removed = 0;
    let mut saw_unified = false;
    for line in diff.split('\n') {
        if line.starts_with("+++ ") || line.starts_with("--- ") {
            continue;
        }
        if line.starts_with('+') {
            saw_unified = true;
            added += 1;
        } else if line.starts_with('-') {
            saw_unified = true;
            removed += 1;
        }
    }
    if saw_unified {
        return DiffStats { added, removed };
    }
    match action {
        FileChangeAction::Created => DiffStats {
            added: plain,
            removed: 0,
        },
        FileChangeAction::Deleted => DiffStats {
            added: 0,
            removed: plain,
        },
        FileChangeAction::Renamed | FileChangeAction::Edited => DiffStats::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(path: &str, kind: &str, diff: &str) -> FileChange {
        FileChange {
            path: path.into(),
            kind: Some(kind.into()),
            move_path: None,
            diff: Some(diff.into()),
            diff_stats: DiffStats::default(),
        }
    }

    #[test]
    fn recognizes_patch_metadata_lines() {
        assert!(is_patch_metadata_line(
            "diff --git a/src/app.ts b/src/app.ts"
        ));
        assert!(is_patch_metadata_line("index 1111111..2222222 100644"));
        assert!(is_patch_metadata_line("--- a/src/app.ts"));
        assert!(is_patch_metadata_line("+++ b/src/app.ts"));
        assert!(is_patch_metadata_line("@@ -1 +1 @@"));
        assert!(!is_patch_metadata_line("+actual content"));
        assert!(!is_patch_metadata_line("-actual content"));
        assert!(!is_patch_metadata_line(" context content"));
    }

    #[test]
    fn treats_crlf_metadata_and_no_newline_markers_as_metadata() {
        assert!(is_patch_metadata_line("@@ -1 +1 @@\r"));
        assert!(is_patch_metadata_line("\\ No newline at end of file\r"));
    }

    #[test]
    fn ignores_patch_metadata_when_counting_created_content() {
        let diff = [
            "diff --git a/src/app.ts b/src/app.ts\r",
            "new file mode 100644\r",
            "--- /dev/null\r",
            "+++ b/src/app.ts\r",
            "@@ -0,0 +1,2 @@\r",
            "first line\r",
            "second line\r",
            "\\ No newline at end of file\r",
            "",
        ]
        .join("\n");
        assert_eq!(
            file_change_diff_stats(&change("src/app.ts", "create", &diff)),
            DiffStats {
                added: 2,
                removed: 0
            }
        );
    }

    #[test]
    fn counts_plain_created_and_deleted_markdown() {
        let diff = [
            "# Fund Toolkit Rearchitecture Plan",
            "",
            "## Core Boundary",
            "",
            "- durable fund memory",
            "- work records",
            "- run records",
            "",
        ]
        .join("\n");
        assert_eq!(
            file_change_diff_stats(&change("plans/x.md", "add", &diff)),
            DiffStats {
                added: 5,
                removed: 0
            }
        );
        assert_eq!(
            file_change_diff_stats(&change("plans/x.md", "delete", &diff)),
            DiffStats {
                added: 0,
                removed: 5
            }
        );
    }

    #[test]
    fn classifies_actions() {
        assert_eq!(
            file_change_action(&change("a", "Add", "")),
            FileChangeAction::Created
        );
        assert_eq!(
            file_change_action(&change("a", "remove_file", "")),
            FileChangeAction::Deleted
        );
        assert_eq!(
            file_change_action(&change("a", "update", "")),
            FileChangeAction::Edited
        );
        let mut moved = change("a.ts", "update", "");
        moved.move_path = Some("b.ts".into());
        assert_eq!(file_change_action(&moved), FileChangeAction::Renamed);
        assert_eq!(
            format_file_change_path(&moved, PathDisplayMode::Full),
            "a.ts -> b.ts"
        );
        moved.diff = Some("-x\n+y".into());
        assert_eq!(file_change_action(&moved), FileChangeAction::Edited);
        assert_eq!(
            format_file_change_path(&moved, PathDisplayMode::Full),
            "b.ts"
        );
    }
}
