use std::borrow::Cow;

use bb_wire::JsonValue;
use bb_wire::timeline::{
    ActivityIntent, CommandWorkRow, FileReadWorkRow, SearchMode, SearchWorkRow, TimelineRowStatus,
    TimelineWorkRow,
};

use crate::path_display::{PathDisplayMode, format_timeline_path};

const SKILL_FILE_NAME: &str = "SKILL.md";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IntentType {
    Read,
    ListFiles,
    Search,
}

#[derive(Debug, Clone, Copy)]
pub enum ExplorationRow<'a> {
    Command(&'a CommandWorkRow),
    FileRead(&'a FileReadWorkRow),
    Search(&'a SearchWorkRow),
}

impl<'a> ExplorationRow<'a> {
    pub fn from_work(row: &'a TimelineWorkRow) -> Option<Self> {
        match row {
            TimelineWorkRow::Command(row) => Some(Self::Command(row)),
            TimelineWorkRow::FileRead(row) => Some(Self::FileRead(row)),
            TimelineWorkRow::Search(row) => Some(Self::Search(row)),
            _ => None,
        }
    }

    pub fn id(&self) -> &'a str {
        match self {
            Self::Command(row) => &row.base.id,
            Self::FileRead(row) => &row.base.id,
            Self::Search(row) => &row.base.id,
        }
    }

    pub fn status(&self) -> TimelineRowStatus {
        match self {
            Self::Command(row) => row.status,
            Self::FileRead(row) => row.status,
            Self::Search(row) => row.status,
        }
    }
}

pub fn intent_type(intent: &ActivityIntent) -> Option<IntentType> {
    match intent {
        ActivityIntent::Read { .. } => Some(IntentType::Read),
        ActivityIntent::ListFiles { .. } => Some(IntentType::ListFiles),
        ActivityIntent::Search { .. } => Some(IntentType::Search),
        ActivityIntent::Generic { .. } | ActivityIntent::Unknown(_) => None,
    }
}

pub fn intent_command(intent: &ActivityIntent) -> &str {
    match intent {
        ActivityIntent::Read { command, .. }
        | ActivityIntent::ListFiles { command, .. }
        | ActivityIntent::Search { command, .. }
        | ActivityIntent::Generic { command } => command,
        ActivityIntent::Unknown(value) => value
            .get("command")
            .and_then(JsonValue::as_str)
            .unwrap_or_default(),
    }
}

pub fn timeline_row_activity_intents<'a>(row: ExplorationRow<'a>) -> Cow<'a, [ActivityIntent]> {
    match row {
        ExplorationRow::Command(row) => Cow::Borrowed(&row.activity_intents),
        ExplorationRow::FileRead(row) => Cow::Owned(vec![ActivityIntent::Read {
            command: row.cmd.clone().unwrap_or_else(|| row.path.clone()),
            name: "fileRead".to_owned(),
            path: Some(row.path.clone()),
        }]),
        ExplorationRow::Search(row) => {
            let command = row.cmd.clone().unwrap_or_else(|| row.query.clone());
            if row.mode == SearchMode::Content {
                Cow::Owned(vec![ActivityIntent::Search {
                    command,
                    query: Some(row.query.clone()),
                    path: row.path.clone(),
                }])
            } else {
                let path = row.path.clone().or_else(|| {
                    if row.query.is_empty() {
                        None
                    } else {
                        Some(row.query.clone())
                    }
                });
                Cow::Owned(vec![ActivityIntent::ListFiles { command, path }])
            }
        }
    }
}

pub fn primary_timeline_activity_intent(row: ExplorationRow<'_>) -> Option<ActivityIntent> {
    timeline_row_activity_intents(row)
        .iter()
        .find(|intent| intent_type(intent).is_some())
        .cloned()
}

pub fn has_timeline_exploration_intent(row: ExplorationRow<'_>) -> bool {
    timeline_row_activity_intents(row)
        .iter()
        .any(|intent| intent_type(intent).is_some())
}

pub fn work_row_has_exploration_intent(row: &TimelineWorkRow) -> bool {
    ExplorationRow::from_work(row).is_some_and(has_timeline_exploration_intent)
}

pub(crate) fn read_target<'a>(path: &'a Option<String>, name: &'a str) -> &'a str {
    path.as_deref().unwrap_or(name)
}

fn format_read_target(target: &str, mode: PathDisplayMode) -> String {
    if mode == PathDisplayMode::Compact
        && let Some(skill) = format_compact_skill_target(target)
    {
        return skill;
    }
    format_timeline_path(target, mode)
}

fn format_compact_skill_target(path: &str) -> Option<String> {
    let normalized = path.replace('\\', "/");
    let segments: Vec<&str> = normalized.split('/').collect();
    let file_index = segments.len() - 1;
    if segments[file_index] != SKILL_FILE_NAME {
        return None;
    }
    if let Some(name) = format_plugin_cache_root_skill_target(file_index, &segments) {
        return Some(name);
    }
    segments[..file_index]
        .iter()
        .rev()
        .find(|segment| !segment.is_empty())
        .map(|name| format!("{name}/{SKILL_FILE_NAME}"))
}

fn format_plugin_cache_root_skill_target(file_index: usize, segments: &[&str]) -> Option<String> {
    let at = |offset: usize| file_index.checked_sub(offset).map(|index| segments[index]);
    let plugin_name = at(2).filter(|name| !name.is_empty())?;
    if at(4) != Some("cache") || at(5) != Some("plugins") {
        return None;
    }
    Some(format!("{plugin_name}/{SKILL_FILE_NAME}"))
}

pub fn is_skill_read_intent(intent: &ActivityIntent) -> bool {
    match intent {
        ActivityIntent::Read { path, name, .. } => read_target(path, name)
            .replace('\\', "/")
            .rsplit('/')
            .next()
            .is_some_and(|file| file == SKILL_FILE_NAME),
        _ => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntentTextParts {
    pub prefix: Option<String>,
    pub content: String,
}

impl IntentTextParts {
    pub fn joined(&self) -> String {
        match &self.prefix {
            Some(prefix) if !prefix.is_empty() => format!("{prefix} {}", self.content),
            _ => self.content.clone(),
        }
    }
}

fn truthy(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|value| !value.is_empty())
}

pub fn format_timeline_activity_intent_detail_parts(
    intent: &ActivityIntent,
    mode: PathDisplayMode,
    pending: bool,
) -> IntentTextParts {
    let parts = |prefix: &str, content: String| IntentTextParts {
        prefix: Some(prefix.to_owned()),
        content,
    };
    match intent {
        ActivityIntent::Read { path, name, .. } => parts(
            if pending { "Reading" } else { "Read" },
            format_read_target(read_target(path, name), mode),
        ),
        ActivityIntent::ListFiles { path, .. } => parts(
            if pending { "Listing" } else { "Listed" },
            match truthy(path) {
                Some(path) => format!("files in {path}"),
                None => "files".to_owned(),
            },
        ),
        ActivityIntent::Search { query, path, .. } => {
            let verb = if pending { "Searching" } else { "Searched" };
            let content = match (truthy(query), truthy(path)) {
                (Some(query), Some(path)) => format!("for {query} in {path}"),
                (Some(query), None) => format!("for {query}"),
                (None, Some(path)) => format!("in {path}"),
                (None, None) => "files".to_owned(),
            };
            parts(verb, content)
        }
        ActivityIntent::Generic { .. } | ActivityIntent::Unknown(_) => IntentTextParts {
            prefix: None,
            content: intent_command(intent).to_owned(),
        },
    }
}

pub fn format_timeline_activity_intent_detail(
    intent: &ActivityIntent,
    mode: PathDisplayMode,
    pending: bool,
) -> String {
    format_timeline_activity_intent_detail_parts(intent, mode, pending).joined()
}

pub fn activity_intent_dedupe_key(intent: &ActivityIntent) -> Option<String> {
    match intent {
        ActivityIntent::Read { path, name, .. } => {
            Some(format!("read:{}", read_target(path, name)))
        }
        ActivityIntent::ListFiles { path, .. } => {
            Some(format!("list:{}", path.as_deref().unwrap_or_default()))
        }
        ActivityIntent::Search { query, path, .. } => Some(format!(
            "search:{}|{}",
            query.as_deref().unwrap_or_default(),
            path.as_deref().unwrap_or_default()
        )),
        ActivityIntent::Generic { .. } | ActivityIntent::Unknown(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(path: &str) -> ActivityIntent {
        ActivityIntent::Read {
            command: format!("cat {path}"),
            name: path.rsplit('/').next().unwrap_or(path).to_owned(),
            path: Some(path.to_owned()),
        }
    }

    #[test]
    fn compacts_skill_paths() {
        let nested = "/Users/a/.codex/plugins/cache/openai-bundled/browser/26.608.12217/skills/control-in-app-browser/SKILL.md";
        let root = "/Users/a/.codex/plugins/cache/openai-bundled/browser/26.608.12217/SKILL.md";
        let detail = |path: &str| {
            format_timeline_activity_intent_detail(&read(path), PathDisplayMode::Compact, false)
        };
        assert_eq!(detail(nested), "Read control-in-app-browser/SKILL.md");
        assert_eq!(detail(root), "Read browser/SKILL.md");
        assert_eq!(detail("SKILL.md"), "Read SKILL.md");
        assert_eq!(detail("/repo/src/app.ts"), "Read app.ts");
        assert!(is_skill_read_intent(&read(root)));
        assert!(!is_skill_read_intent(&read("/repo/a.md")));
    }

    #[test]
    fn formats_search_and_list_details() {
        let search = |query: Option<&str>, path: Option<&str>| ActivityIntent::Search {
            command: "rg".into(),
            query: query.map(Into::into),
            path: path.map(Into::into),
        };
        let fmt = |intent: &ActivityIntent, pending| {
            format_timeline_activity_intent_detail(intent, PathDisplayMode::Full, pending)
        };
        assert_eq!(
            fmt(&search(Some("TODO"), Some("src")), false),
            "Searched for TODO in src"
        );
        assert_eq!(fmt(&search(Some("TODO"), None), true), "Searching for TODO");
        assert_eq!(fmt(&search(None, Some("src")), false), "Searched in src");
        assert_eq!(fmt(&search(Some(""), None), false), "Searched files");
        let list = ActivityIntent::ListFiles {
            command: "ls".into(),
            path: Some("src".into()),
        };
        assert_eq!(fmt(&list, false), "Listed files in src");
        let generic = ActivityIntent::Generic {
            command: "make".into(),
        };
        assert_eq!(fmt(&generic, false), "make");
        assert_eq!(activity_intent_dedupe_key(&generic), None);
        assert_eq!(
            activity_intent_dedupe_key(&search(Some("a"), None)).as_deref(),
            Some("search:a|")
        );
    }

    #[test]
    fn synthesizes_intents_for_search_rows() {
        let row = SearchWorkRow {
            mode: SearchMode::Path,
            query: "*.rs".into(),
            ..SearchWorkRow::default()
        };
        let intents = timeline_row_activity_intents(ExplorationRow::Search(&row));
        assert_eq!(
            intents.as_ref(),
            &[ActivityIntent::ListFiles {
                command: "*.rs".into(),
                path: Some("*.rs".into())
            }]
        );
    }
}
