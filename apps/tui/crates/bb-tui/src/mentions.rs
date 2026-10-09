use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use bb_wire::plugin::{PluginMentionProviderContribution, PluginMentionSearchGroup};
use bb_wire::project::{
    PERSONAL_PROJECT_ID, ProviderCommand, ProviderCommandOrigin, ProviderCommandSource,
    WorkspacePathEntry,
};
use bb_wire::prompt::{
    PathEntryKind, PromptMentionCommandOrigin, PromptMentionCommandSource,
    PromptMentionCommandTrigger, PromptMentionPathSource, PromptMentionResource,
};
use bb_wire::system::ProviderComposerAction;
use bb_wire::thread::{Thread, ThreadVisibility};

use crate::fuzzy::match_text;

pub const DEFAULT_MENTION_TRIGGER: char = '@';
pub const PLUGIN_MENTION_TRIGGERS: [char; 5] = ['@', '#', '$', '!', '~'];
pub const TRIGGER_SCAN_WINDOW: usize = 256;
pub const MENTION_SOURCE_LIMIT: usize = 8;
pub const PATH_OVERSAMPLE: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TriggerKind {
    Mention,
    Command,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypeaheadTrigger {
    pub ch: char,
    pub kind: TriggerKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveTrigger {
    pub ch: char,
    pub kind: TriggerKind,
    pub query: String,
    pub from: usize,
    pub to: usize,
}

pub fn mention_triggers(providers: &[PluginMentionProviderContribution]) -> Vec<char> {
    let mut enabled: HashSet<char> = HashSet::from([DEFAULT_MENTION_TRIGGER]);
    for provider in providers {
        for trigger in &provider.triggers {
            let mut chars = trigger.chars();
            if let (Some(ch), None) = (chars.next(), chars.next())
                && PLUGIN_MENTION_TRIGGERS.contains(&ch)
            {
                enabled.insert(ch);
            }
        }
    }
    PLUGIN_MENTION_TRIGGERS
        .into_iter()
        .filter(|ch| enabled.contains(ch))
        .collect()
}

pub fn command_trigger_char(trigger: PromptMentionCommandTrigger) -> Option<char> {
    match trigger {
        PromptMentionCommandTrigger::Slash => Some('/'),
        PromptMentionCommandTrigger::Dollar => Some('$'),
        PromptMentionCommandTrigger::Unknown => None,
    }
}

pub fn command_trigger_of(ch: char) -> Option<PromptMentionCommandTrigger> {
    match ch {
        '/' => Some(PromptMentionCommandTrigger::Slash),
        '$' => Some(PromptMentionCommandTrigger::Dollar),
        _ => None,
    }
}

pub fn skills_triggers(actions: &[ProviderComposerAction]) -> Vec<char> {
    let mut triggers = Vec::new();
    for action in actions {
        if let ProviderComposerAction::Skills { trigger } = action
            && let Some(ch) = command_trigger_char(*trigger)
        {
            triggers.push(ch);
        }
    }
    triggers
}

pub fn prompt_action_commands(actions: &[ProviderComposerAction]) -> Vec<(char, String)> {
    actions
        .iter()
        .filter_map(|action| match action {
            ProviderComposerAction::Plan { command } | ProviderComposerAction::Goal { command } => {
                command_trigger_char(command.trigger).map(|ch| (ch, command.name.clone()))
            }
            _ => None,
        })
        .collect()
}

pub fn typeahead_triggers(mention: &[char], command: &[char]) -> Vec<TypeaheadTrigger> {
    mention
        .iter()
        .map(|ch| TypeaheadTrigger {
            ch: *ch,
            kind: TriggerKind::Mention,
        })
        .chain(command.iter().map(|ch| TypeaheadTrigger {
            ch: *ch,
            kind: TriggerKind::Command,
        }))
        .collect()
}

fn is_boundary(ch: char) -> bool {
    ch.is_whitespace() || matches!(ch, '(' | '[' | '{')
}

fn query_char_allowed(kind: TriggerKind, trigger: char, ch: char) -> bool {
    match kind {
        TriggerKind::Mention => ch == ' ' || (!ch.is_whitespace() && ch != trigger),
        TriggerKind::Command => !ch.is_whitespace(),
    }
}

pub fn find_active_trigger(
    scan: &[char],
    cursor: usize,
    triggers: &[TypeaheadTrigger],
) -> Option<ActiveTrigger> {
    let cursor = cursor.min(scan.len());
    let scan_start = cursor.saturating_sub(TRIGGER_SCAN_WINDOW);
    let windowed = scan_start > 0;
    let window = &scan[scan_start..cursor];
    for trigger in triggers {
        let mut tail_ok = vec![true; window.len() + 1];
        for index in (0..window.len()).rev() {
            tail_ok[index] =
                tail_ok[index + 1] && query_char_allowed(trigger.kind, trigger.ch, window[index]);
        }
        let found = (0..window.len()).find(|index| {
            window[*index] == trigger.ch
                && tail_ok[index + 1]
                && if *index == 0 {
                    !windowed
                } else {
                    is_boundary(window[index - 1])
                }
        });
        let Some(index) = found else {
            continue;
        };
        let query: String = window[index + 1..].iter().collect();
        return Some(ActiveTrigger {
            ch: trigger.ch,
            kind: trigger.kind,
            query,
            from: scan_start + index,
            to: cursor,
        });
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadRelation {
    Parent,
    Child,
    SameParent,
    SameEnvironment,
}

impl ThreadRelation {
    pub fn label(self) -> &'static str {
        match self {
            Self::Parent => "parent",
            Self::Child => "child",
            Self::SameParent => "same parent",
            Self::SameEnvironment => "same environment",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum MentionSuggestion {
    Path {
        source: PromptMentionPathSource,
        entry_kind: PathEntryKind,
        path: String,
        name: String,
        replacement: String,
    },
    Thread {
        replacement: String,
        project_id: String,
        project_name: Option<String>,
        thread_id: String,
        title: Option<String>,
        relation: Option<ThreadRelation>,
    },
    Project {
        replacement: String,
        project_id: String,
        name: String,
    },
    Section {
        replacement: String,
        section_id: String,
        name: String,
    },
    Plugin {
        plugin_id: String,
        provider_id: String,
        item_id: String,
        provider_label: String,
        title: String,
        subtitle: Option<String>,
        icon: Option<String>,
        replacement: String,
    },
}

fn path_section_label(source: PromptMentionPathSource) -> &'static str {
    if source == PromptMentionPathSource::ThreadStorage {
        "Thread storage"
    } else {
        "Workspace"
    }
}

impl MentionSuggestion {
    pub fn replacement(&self) -> &str {
        match self {
            Self::Path { replacement, .. }
            | Self::Thread { replacement, .. }
            | Self::Project { replacement, .. }
            | Self::Section { replacement, .. }
            | Self::Plugin { replacement, .. } => replacement,
        }
    }

    pub fn key(&self) -> String {
        match self {
            Self::Path {
                source,
                entry_kind,
                path,
                ..
            } => format!("path:{source:?}:{entry_kind:?}:{path}"),
            Self::Thread { thread_id, .. } => format!("thread:{thread_id}"),
            Self::Project { project_id, .. } => format!("project:{project_id}"),
            Self::Section { section_id, .. } => format!("section:{section_id}"),
            Self::Plugin {
                plugin_id,
                provider_id,
                item_id,
                ..
            } => format!("plugin:{plugin_id}:{provider_id}:{item_id}"),
        }
    }

    pub fn title(&self) -> String {
        match self {
            Self::Thread {
                title,
                thread_id,
                project_name,
                relation,
                ..
            } => {
                let mut parts = vec![
                    title
                        .clone()
                        .unwrap_or_else(|| format!("thread:{thread_id}")),
                ];
                parts.extend(project_name.clone());
                parts.extend(relation.map(|relation| relation.label().to_owned()));
                parts.join(" · ")
            }
            Self::Project { name, .. } => format!("Project: {name}"),
            Self::Section { name, .. } => format!("Section: {name}"),
            Self::Plugin {
                provider_label,
                title,
                ..
            } => format!("{provider_label}: {title}"),
            Self::Path {
                source,
                entry_kind,
                path,
                ..
            } => {
                let suffix = if *entry_kind == PathEntryKind::Directory && !path.ends_with('/') {
                    "/"
                } else {
                    ""
                };
                format!("{}: {path}{suffix}", path_section_label(*source))
            }
        }
    }

    pub fn detail(&self) -> Option<String> {
        match self {
            Self::Plugin { subtitle, .. } => subtitle.clone(),
            _ => None,
        }
    }

    pub fn resource(&self) -> PromptMentionResource {
        match self {
            Self::Thread {
                thread_id,
                project_id,
                title,
                ..
            } => PromptMentionResource::Thread {
                thread_id: thread_id.clone(),
                project_id: Some(project_id.clone()),
                label: title
                    .as_deref()
                    .map(str::trim)
                    .filter(|title| !title.is_empty())
                    .unwrap_or(thread_id)
                    .to_owned(),
            },
            Self::Project {
                project_id, name, ..
            } => PromptMentionResource::Project {
                project_id: project_id.clone(),
                label: non_empty_or(name, project_id),
            },
            Self::Section {
                section_id, name, ..
            } => PromptMentionResource::Section {
                section_id: section_id.clone(),
                label: non_empty_or(name, section_id),
            },
            Self::Plugin {
                plugin_id,
                icon,
                item_id,
                title,
                ..
            } => PromptMentionResource::Plugin {
                plugin_id: plugin_id.clone(),
                icon: icon.clone(),
                item_id: item_id.clone(),
                label: non_empty_or(title, item_id),
            },
            Self::Path {
                source,
                entry_kind,
                path,
                name,
                ..
            } => PromptMentionResource::Path {
                source: *source,
                entry_kind: *entry_kind,
                path: path.clone(),
                label: name.clone(),
            },
        }
    }

    pub fn serialized_text(&self, trigger: char) -> String {
        let replacement = self.replacement().trim();
        if replacement.starts_with(trigger) {
            replacement.to_owned()
        } else {
            format!("{trigger}{replacement}")
        }
    }
}

fn non_empty_or(value: &str, fallback: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        fallback.to_owned()
    } else {
        trimmed.to_owned()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MentionCandidate {
    pub suggestion: MentionSuggestion,
    pub visible_title: String,
    pub identity_terms: Vec<String>,
    pub supporting_terms: Vec<String>,
    pub group_key: String,
    pub group_label: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SuggestionGroup<T> {
    pub key: String,
    pub label: String,
    pub start: usize,
    pub items: Vec<T>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OrderedSuggestions<T> {
    pub groups: Vec<SuggestionGroup<T>>,
}

impl<T> Default for OrderedSuggestions<T> {
    fn default() -> Self {
        Self { groups: Vec::new() }
    }
}

impl<T> OrderedSuggestions<T> {
    pub fn len(&self) -> usize {
        self.groups.iter().map(|group| group.items.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get(&self, index: usize) -> Option<&T> {
        self.groups
            .iter()
            .flat_map(|group| group.items.iter())
            .nth(index)
    }

    pub fn flat(&self) -> Vec<&T> {
        self.groups
            .iter()
            .flat_map(|group| group.items.iter())
            .collect()
    }
}

const RANK_EXACT: u8 = 0;
const RANK_PREFIX: u8 = 1;
const RANK_SUBSTRING: u8 = 2;
const RANK_SUPPORTING: u8 = 3;
const RANK_NONE: u8 = 4;

fn normalize_term(term: &str) -> String {
    term.trim().to_lowercase()
}

pub fn mention_identity_match_rank(identities: &[&str], query: &str) -> u8 {
    let query = normalize_term(query);
    if query.is_empty() {
        return RANK_EXACT;
    }
    let normalized: Vec<String> = identities.iter().map(|term| normalize_term(term)).collect();
    if normalized.contains(&query) {
        RANK_EXACT
    } else if normalized
        .iter()
        .any(|identity| identity.starts_with(&query))
    {
        RANK_PREFIX
    } else if normalized.iter().any(|identity| identity.contains(&query)) {
        RANK_SUBSTRING
    } else {
        RANK_NONE
    }
}

fn candidate_rank(candidate: &MentionCandidate, query: &str) -> u8 {
    let mut identities: Vec<&str> = vec![candidate.visible_title.as_str()];
    identities.extend(candidate.identity_terms.iter().map(String::as_str));
    let rank = mention_identity_match_rank(&identities, query);
    if rank != RANK_NONE {
        return rank;
    }
    let query = normalize_term(query);
    if candidate
        .supporting_terms
        .iter()
        .any(|term| normalize_term(term).contains(&query))
    {
        RANK_SUPPORTING
    } else {
        RANK_NONE
    }
}

pub fn order_mention_candidates(
    candidates: Vec<MentionCandidate>,
    query: &str,
) -> OrderedSuggestions<MentionSuggestion> {
    struct Group {
        key: String,
        label: String,
        input_index: usize,
        best: u8,
        rows: Vec<(u8, usize, MentionSuggestion)>,
    }
    let mut groups: Vec<Group> = Vec::new();
    for (input_index, candidate) in candidates.into_iter().enumerate() {
        let rank = candidate_rank(&candidate, query);
        match groups
            .iter_mut()
            .find(|group| group.key == candidate.group_key)
        {
            Some(group) => {
                group.best = group.best.min(rank);
                group.rows.push((rank, input_index, candidate.suggestion));
            }
            None => groups.push(Group {
                key: candidate.group_key,
                label: candidate.group_label,
                input_index,
                best: rank,
                rows: vec![(rank, input_index, candidate.suggestion)],
            }),
        }
    }
    groups.sort_by(|left, right| {
        left.best
            .cmp(&right.best)
            .then(left.input_index.cmp(&right.input_index))
    });
    let mut start = 0;
    let groups = groups
        .into_iter()
        .map(|mut group| {
            group
                .rows
                .sort_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(&right.1)));
            let items: Vec<MentionSuggestion> =
                group.rows.into_iter().map(|(_, _, item)| item).collect();
            let ordered = SuggestionGroup {
                key: group.key,
                label: group.label,
                start,
                items,
            };
            start += ordered.items.len();
            ordered
        })
        .collect();
    OrderedSuggestions { groups }
}

pub fn mention_candidate(suggestion: MentionSuggestion) -> MentionCandidate {
    match &suggestion {
        MentionSuggestion::Thread {
            title,
            thread_id,
            project_name,
            ..
        } => MentionCandidate {
            visible_title: title
                .as_deref()
                .map(str::trim)
                .filter(|title| !title.is_empty())
                .unwrap_or(thread_id)
                .to_owned(),
            identity_terms: vec![thread_id.clone()],
            supporting_terms: project_name.iter().cloned().collect(),
            group_key: "threads".into(),
            group_label: "Threads".into(),
            suggestion,
        },
        MentionSuggestion::Project {
            name, project_id, ..
        } => MentionCandidate {
            visible_title: name.clone(),
            identity_terms: vec![project_id.clone()],
            supporting_terms: Vec::new(),
            group_key: "projects".into(),
            group_label: "Projects".into(),
            suggestion,
        },
        MentionSuggestion::Section {
            name, section_id, ..
        } => MentionCandidate {
            visible_title: name.clone(),
            identity_terms: vec![section_id.clone()],
            supporting_terms: Vec::new(),
            group_key: "sections".into(),
            group_label: "Sections".into(),
            suggestion,
        },
        MentionSuggestion::Path {
            source,
            path,
            name,
            replacement,
            ..
        } => MentionCandidate {
            visible_title: name.clone(),
            identity_terms: vec![path.clone(), replacement.clone()],
            supporting_terms: Vec::new(),
            group_key: format!("path:{source:?}"),
            group_label: path_section_label(*source).into(),
            suggestion,
        },
        MentionSuggestion::Plugin {
            title,
            subtitle,
            plugin_id,
            provider_id,
            provider_label,
            ..
        } => MentionCandidate {
            visible_title: title.clone(),
            identity_terms: Vec::new(),
            supporting_terms: subtitle.iter().cloned().collect(),
            group_key: format!("plugin:{plugin_id}:{provider_id}"),
            group_label: provider_label.clone(),
            suggestion,
        },
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MentionSources {
    pub paths: Vec<MentionSuggestion>,
    pub threads: Vec<MentionSuggestion>,
    pub projects: Vec<MentionSuggestion>,
    pub sections: Vec<MentionSuggestion>,
    pub plugins: Vec<MentionSuggestion>,
}

pub fn build_prompt_mention_results(
    query: &str,
    sources: MentionSources,
) -> OrderedSuggestions<MentionSuggestion> {
    let MentionSources {
        paths,
        threads,
        projects,
        sections,
        plugins,
    } = sources;
    let ordered: Vec<MentionSuggestion> = if query.trim().contains('/') {
        [paths, threads, projects, sections, plugins].concat()
    } else {
        [threads, projects, sections, paths, plugins].concat()
    };
    order_mention_candidates(ordered.into_iter().map(mention_candidate).collect(), query)
}

fn directory_path(path: &str) -> String {
    if path.ends_with('/') {
        path.to_owned()
    } else {
        format!("{path}/")
    }
}

pub fn path_replacement(
    source: PromptMentionPathSource,
    kind: PathEntryKind,
    path: &str,
) -> String {
    let mention_path = if kind == PathEntryKind::Directory {
        directory_path(path)
    } else {
        path.to_owned()
    };
    if source == PromptMentionPathSource::ThreadStorage {
        format!("thread-storage:{mention_path}")
    } else {
        mention_path
    }
}

pub fn build_path_suggestions(
    workspace: &[WorkspacePathEntry],
    storage: &[WorkspacePathEntry],
    limit: usize,
) -> Vec<MentionSuggestion> {
    let mut ranked: Vec<(u8, usize, &WorkspacePathEntry, PromptMentionPathSource)> = workspace
        .iter()
        .enumerate()
        .map(|(order, entry)| (0, order, entry, PromptMentionPathSource::Workspace))
        .chain(
            storage
                .iter()
                .enumerate()
                .map(|(order, entry)| (1, order, entry, PromptMentionPathSource::ThreadStorage)),
        )
        .collect();
    ranked.sort_by(|left, right| {
        right
            .2
            .score
            .partial_cmp(&left.2.score)
            .unwrap_or(Ordering::Equal)
            .then(left.0.cmp(&right.0))
            .then_with(|| {
                let left_dir = left.2.kind == PathEntryKind::Directory;
                let right_dir = right.2.kind == PathEntryKind::Directory;
                right_dir.cmp(&left_dir)
            })
            .then(left.1.cmp(&right.1))
            .then_with(|| left.2.path.cmp(&right.2.path))
    });
    ranked
        .into_iter()
        .take(limit)
        .map(|(_, _, entry, source)| MentionSuggestion::Path {
            source,
            entry_kind: entry.kind,
            path: entry.path.clone(),
            name: entry.name.clone(),
            replacement: path_replacement(source, entry.kind, &entry.path),
        })
        .collect()
}

#[derive(Debug, Clone, Default)]
pub struct ThreadMentionArgs<'a> {
    pub threads: Vec<&'a Thread>,
    pub query: &'a str,
    pub current_project_id: Option<&'a str>,
    pub current_thread_id: Option<&'a str>,
    pub current_environment_id: Option<&'a str>,
    pub project_names: HashMap<String, String>,
    pub limit: usize,
}

fn thread_display_title(thread: &Thread) -> Option<String> {
    thread
        .title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .or_else(|| {
            thread
                .title_fallback
                .as_deref()
                .map(str::trim)
                .filter(|title| !title.is_empty())
        })
        .map(str::to_owned)
}

pub fn build_thread_mention_suggestions(args: &ThreadMentionArgs<'_>) -> Vec<MentionSuggestion> {
    let query = args.query.trim();
    if query.is_empty() || args.limit == 0 {
        return Vec::new();
    }
    let candidates: Vec<&Thread> = args
        .threads
        .iter()
        .copied()
        .filter(|thread| {
            Some(thread.id.as_str()) != args.current_thread_id
                && thread.visibility != ThreadVisibility::Hidden
        })
        .collect();
    let current = args
        .current_thread_id
        .and_then(|id| args.threads.iter().find(|thread| thread.id == id));
    let current_parent = current.and_then(|thread| thread.parent_thread_id.as_deref());
    let current_project = args
        .current_project_id
        .or_else(|| current.map(|thread| thread.project_id.as_str()));
    let current_environment = args
        .current_environment_id
        .or_else(|| current.and_then(|thread| thread.environment_id.as_deref()));
    let items: Vec<(String, &Thread)> = candidates
        .iter()
        .map(|thread| {
            (
                thread_display_title(thread).unwrap_or_else(|| thread.id.clone()),
                *thread,
            )
        })
        .collect();
    let matches = match_text(
        &items,
        query,
        |item| item.0.as_str(),
        |item| vec![item.1.id.as_str()],
        items.len(),
    );
    let mut ranked: Vec<(u8, u8, i64, i64, MentionSuggestion)> = matches
        .into_iter()
        .map(|found| {
            let thread = items[found.index].1;
            let relation = if current_parent.is_some_and(|parent| thread.id == parent) {
                Some(ThreadRelation::Parent)
            } else if args
                .current_thread_id
                .is_some_and(|id| thread.parent_thread_id.as_deref() == Some(id))
            {
                Some(ThreadRelation::Child)
            } else if current_parent.is_some()
                && thread.parent_thread_id.as_deref() == current_parent
            {
                Some(ThreadRelation::SameParent)
            } else if current_environment.is_some()
                && thread.environment_id.as_deref() == current_environment
            {
                Some(ThreadRelation::SameEnvironment)
            } else {
                None
            };
            let relation_rank = match relation {
                Some(ThreadRelation::Parent | ThreadRelation::Child) => 0,
                Some(ThreadRelation::SameParent) => 1,
                Some(ThreadRelation::SameEnvironment) => 2,
                None if current_project == Some(thread.project_id.as_str()) => 3,
                None => 4,
            };
            let show_project = thread.project_id != PERSONAL_PROJECT_ID
                && current_project != Some(thread.project_id.as_str());
            let title = thread_display_title(thread);
            let match_rank = mention_identity_match_rank(
                &[title.as_deref().unwrap_or(&thread.id), thread.id.as_str()],
                query,
            );
            (
                match_rank,
                relation_rank,
                found.score,
                thread.updated_at,
                MentionSuggestion::Thread {
                    replacement: format!("thread:{}", thread.id),
                    project_id: thread.project_id.clone(),
                    project_name: show_project
                        .then(|| args.project_names.get(&thread.project_id).cloned())
                        .flatten(),
                    thread_id: thread.id.clone(),
                    title,
                    relation,
                },
            )
        })
        .collect();
    ranked.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then(left.1.cmp(&right.1))
            .then(right.2.cmp(&left.2))
            .then(right.3.cmp(&left.3))
            .then_with(|| match (&left.4, &right.4) {
                (
                    MentionSuggestion::Thread { thread_id: a, .. },
                    MentionSuggestion::Thread { thread_id: b, .. },
                ) => a.cmp(b),
                _ => Ordering::Equal,
            })
    });
    ranked
        .into_iter()
        .take(args.limit)
        .map(|(_, _, _, _, suggestion)| suggestion)
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedCandidate {
    pub id: String,
    pub name: String,
}

fn named_text(candidate: &NamedCandidate) -> &str {
    let trimmed = candidate.name.trim();
    if trimmed.is_empty() {
        &candidate.id
    } else {
        trimmed
    }
}

fn rank_named<'a>(
    candidates: &'a [NamedCandidate],
    query: &str,
    limit: usize,
) -> Vec<&'a NamedCandidate> {
    let query = query.trim();
    if query.is_empty() || limit == 0 {
        return Vec::new();
    }
    let mut matches: Vec<(i64, &NamedCandidate)> = match_text(
        candidates,
        query,
        named_text,
        |candidate| vec![candidate.id.as_str()],
        candidates.len(),
    )
    .into_iter()
    .map(|found| (found.score, &candidates[found.index]))
    .collect();
    matches.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.name.to_lowercase().cmp(&right.1.name.to_lowercase()))
            .then_with(|| left.1.id.cmp(&right.1.id))
    });
    matches
        .into_iter()
        .take(limit)
        .map(|(_, candidate)| candidate)
        .collect()
}

pub fn build_project_mention_suggestions(
    projects: &[NamedCandidate],
    query: &str,
    limit: usize,
) -> Vec<MentionSuggestion> {
    rank_named(projects, query, limit)
        .into_iter()
        .map(|project| MentionSuggestion::Project {
            replacement: format!("project:{}", project.id),
            project_id: project.id.clone(),
            name: named_text(project).to_owned(),
        })
        .collect()
}

pub fn build_section_mention_suggestions(
    sections: &[NamedCandidate],
    query: &str,
    limit: usize,
) -> Vec<MentionSuggestion> {
    rank_named(sections, query, limit)
        .into_iter()
        .map(|section| MentionSuggestion::Section {
            replacement: format!("section:{}", section.id),
            section_id: section.id.clone(),
            name: named_text(section).to_owned(),
        })
        .collect()
}

pub fn build_plugin_mention_suggestions(
    groups: &[PluginMentionSearchGroup],
) -> Vec<MentionSuggestion> {
    let mut suggestions = Vec::new();
    for group in groups {
        for item in &group.items {
            let title = item.title.trim();
            if title.is_empty() {
                continue;
            }
            suggestions.push(MentionSuggestion::Plugin {
                plugin_id: group.plugin_id.clone(),
                provider_id: group.provider_id.clone(),
                item_id: item.item_id.clone(),
                provider_label: group.label.clone(),
                title: title.to_owned(),
                subtitle: item.subtitle.clone(),
                icon: item.icon.clone(),
                replacement: title.to_owned(),
            });
        }
    }
    suggestions
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSuggestion {
    pub name: String,
    pub source: ProviderCommandSource,
    pub origin: ProviderCommandOrigin,
    pub description: Option<String>,
    pub argument_hint: Option<String>,
    pub plugin_id: Option<String>,
}

impl CommandSuggestion {
    pub fn from_command(command: &ProviderCommand) -> Self {
        Self {
            name: command.name.clone(),
            source: command.source,
            origin: command.origin,
            description: command.description.clone(),
            argument_hint: command.argument_hint.clone(),
            plugin_id: command.plugin_id.clone(),
        }
    }

    pub fn prompt_action(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            source: ProviderCommandSource::Command,
            origin: ProviderCommandOrigin::User,
            description: None,
            argument_hint: None,
            plugin_id: None,
        }
    }

    pub fn key(&self) -> String {
        format!(
            "command:{:?}:{:?}:{}:{}",
            self.source,
            self.origin,
            self.plugin_id.as_deref().unwrap_or(""),
            self.name
        )
    }

    pub fn resource(&self, trigger: PromptMentionCommandTrigger) -> PromptMentionResource {
        PromptMentionResource::Command {
            trigger,
            name: self.name.clone(),
            source: match self.source {
                ProviderCommandSource::Skill => PromptMentionCommandSource::Skill,
                ProviderCommandSource::Command => PromptMentionCommandSource::Command,
                ProviderCommandSource::Unknown => PromptMentionCommandSource::Unknown,
            },
            origin: match self.origin {
                ProviderCommandOrigin::Builtin => PromptMentionCommandOrigin::Builtin,
                ProviderCommandOrigin::Project => PromptMentionCommandOrigin::Project,
                ProviderCommandOrigin::User => PromptMentionCommandOrigin::User,
                ProviderCommandOrigin::Unknown => PromptMentionCommandOrigin::Unknown,
            },
            label: self.name.clone(),
            argument_hint: self.argument_hint.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CommandSection {
    AgentCommand,
    Skill,
    ProjectCommand,
    UserCommand,
}

impl CommandSection {
    pub fn label(self) -> &'static str {
        match self {
            Self::AgentCommand => "Commands",
            Self::Skill => "Skills",
            Self::ProjectCommand => "Project commands",
            Self::UserCommand => "User commands",
        }
    }
}

pub fn command_section(
    source: ProviderCommandSource,
    origin: ProviderCommandOrigin,
) -> CommandSection {
    if origin == ProviderCommandOrigin::Builtin {
        CommandSection::AgentCommand
    } else if source == ProviderCommandSource::Skill {
        CommandSection::Skill
    } else if origin == ProviderCommandOrigin::Project {
        CommandSection::ProjectCommand
    } else {
        CommandSection::UserCommand
    }
}

fn command_name_matches(suggestions: &[CommandSuggestion], query: &str) -> HashSet<usize> {
    match_text(
        suggestions,
        query,
        |suggestion| suggestion.name.as_str(),
        |_| Vec::new(),
        suggestions.len(),
    )
    .into_iter()
    .map(|found| found.index)
    .collect()
}

pub fn filter_command_suggestions(
    suggestions: Vec<CommandSuggestion>,
    query: &str,
) -> Vec<CommandSuggestion> {
    let query = query.trim().to_lowercase();
    let matches = command_name_matches(&suggestions, &query);
    suggestions
        .into_iter()
        .enumerate()
        .filter(|(index, suggestion)| {
            matches.contains(index)
                || [
                    suggestion.description.as_deref().unwrap_or(""),
                    suggestion.argument_hint.as_deref().unwrap_or(""),
                ]
                .iter()
                .any(|text| text.to_lowercase().contains(&query))
        })
        .map(|(_, suggestion)| suggestion)
        .collect()
}

fn command_search_names(suggestion: &CommandSuggestion) -> Vec<String> {
    let name = suggestion.name.to_lowercase();
    if suggestion.source != ProviderCommandSource::Skill {
        return vec![name];
    }
    match name.rfind(':') {
        Some(index) => {
            let short = name[index + 1..].to_owned();
            vec![name, short]
        }
        None => vec![name],
    }
}

fn command_match_rank(suggestion: &CommandSuggestion, query: &str, name_match: bool) -> u8 {
    if suggestion.name.to_lowercase() == query {
        return 0;
    }
    let names = command_search_names(suggestion);
    if names.iter().any(|name| name == query) {
        1
    } else if names.iter().any(|name| name.starts_with(query)) {
        2
    } else if names.iter().any(|name| name.contains(query)) {
        3
    } else if name_match {
        4
    } else {
        5
    }
}

pub fn order_command_suggestions(
    suggestions: Vec<CommandSuggestion>,
    query: &str,
) -> OrderedSuggestions<CommandSuggestion> {
    let query = query.trim().to_lowercase();
    let matches = command_name_matches(&suggestions, &query);
    let mut ranked: Vec<(u8, CommandSection, usize, CommandSuggestion)> = suggestions
        .into_iter()
        .enumerate()
        .map(|(index, suggestion)| {
            (
                command_match_rank(&suggestion, &query, matches.contains(&index)),
                command_section(suggestion.source, suggestion.origin),
                index,
                suggestion,
            )
        })
        .collect();
    ranked.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then(left.1.cmp(&right.1))
            .then(left.2.cmp(&right.2))
    });
    let mut groups: Vec<SuggestionGroup<CommandSuggestion>> = Vec::new();
    for (_, section, _, suggestion) in ranked {
        let key = format!("{section:?}");
        match groups.iter_mut().find(|group| group.key == key) {
            Some(group) => group.items.push(suggestion),
            None => groups.push(SuggestionGroup {
                key,
                label: section.label().into(),
                start: 0,
                items: vec![suggestion],
            }),
        }
    }
    let mut start = 0;
    for group in &mut groups {
        group.start = start;
        start += group.items.len();
    }
    OrderedSuggestions { groups }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandScope {
    Thread,
    NewThread,
}

pub fn build_command_suggestions(
    commands: &[ProviderCommand],
    prompt_actions: &[(char, String)],
    trigger: char,
    scope: CommandScope,
    query: &str,
) -> Vec<CommandSuggestion> {
    let trimmed = query.trim();
    let actions = filter_command_suggestions(
        prompt_actions
            .iter()
            .filter(|(ch, _)| *ch == trigger)
            .map(|(_, name)| CommandSuggestion::prompt_action(name))
            .collect(),
        &trimmed.to_lowercase(),
    );
    let discovered = filter_command_suggestions(
        commands
            .iter()
            .map(CommandSuggestion::from_command)
            .filter(|suggestion| {
                (trigger != '$' || suggestion.source == ProviderCommandSource::Skill)
                    && (scope == CommandScope::Thread
                        || suggestion.source != ProviderCommandSource::Command
                        || suggestion.origin != ProviderCommandOrigin::Builtin
                        || suggestion.name != "compact")
            })
            .collect(),
        trimmed,
    );
    let mut seen = HashSet::new();
    actions
        .into_iter()
        .chain(discovered)
        .filter(|suggestion| seen.insert(format!("{:?}:{}", suggestion.source, suggestion.name)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(text: &str) -> Vec<char> {
        text.chars().collect()
    }

    fn triggers() -> Vec<TypeaheadTrigger> {
        typeahead_triggers(&['@', '#'], &['/', '$'])
    }

    fn active(text: &str) -> Option<(char, String, usize)> {
        let scan = chars(text);
        find_active_trigger(&scan, scan.len(), &triggers())
            .map(|found| (found.ch, found.query, found.from))
    }

    #[test]
    fn mention_trigger_needs_a_boundary_and_allows_spaces() {
        assert_eq!(active("@"), Some(('@', String::new(), 0)));
        assert_eq!(active("hi @src ma"), Some(('@', "src ma".into(), 3)));
        assert_eq!(active("mail@host"), None);
        assert_eq!(active("(@x"), Some(('@', "x".into(), 1)));
        assert_eq!(active("@a\nb"), None);
        assert_eq!(active("Привет @файл"), Some(('@', "файл".into(), 7)));
        assert_eq!(active("😀 @日本"), Some(('@', "日本".into(), 2)));
    }

    #[test]
    fn command_trigger_takes_the_leftmost_slash_and_stops_at_spaces() {
        assert_eq!(active("/rev"), Some(('/', "rev".into(), 0)));
        assert_eq!(active("/a/b"), Some(('/', "a/b".into(), 0)));
        assert_eq!(active("/review now"), None);
        assert_eq!(active("x $sk"), Some(('$', "sk".into(), 2)));
    }

    #[test]
    fn scan_window_requires_a_boundary_inside_it() {
        let long = format!("{}@q", "x".repeat(300));
        assert_eq!(active(&long), None);
        let spaced = format!("{} @q", "x".repeat(300));
        assert_eq!(active(&spaced).map(|found| found.1), Some("q".into()));
    }

    #[test]
    fn identity_ranks_and_groups_follow_client_core() {
        assert_eq!(mention_identity_match_rank(&["Alpha"], ""), RANK_EXACT);
        assert_eq!(mention_identity_match_rank(&["Alpha"], "alpha"), RANK_EXACT);
        assert_eq!(mention_identity_match_rank(&["Alpha"], "al"), RANK_PREFIX);
        assert_eq!(
            mention_identity_match_rank(&["Alpha"], "ph"),
            RANK_SUBSTRING
        );
        assert_eq!(mention_identity_match_rank(&["Alpha"], "zz"), RANK_NONE);
        let project = MentionSuggestion::Project {
            replacement: "project:p1".into(),
            project_id: "p1".into(),
            name: "readme".into(),
        };
        let path = MentionSuggestion::Path {
            source: PromptMentionPathSource::Workspace,
            entry_kind: PathEntryKind::File,
            path: "README.md".into(),
            name: "README.md".into(),
            replacement: "README.md".into(),
        };
        let ordered = build_prompt_mention_results(
            "readme",
            MentionSources {
                paths: vec![path.clone()],
                projects: vec![project.clone()],
                ..MentionSources::default()
            },
        );
        assert_eq!(ordered.groups[0].label, "Projects");
        assert_eq!(ordered.groups[1].label, "Workspace");
        assert_eq!(ordered.groups[1].start, 1);
        let with_slash = build_prompt_mention_results(
            "src/",
            MentionSources {
                paths: vec![path],
                projects: vec![project],
                ..MentionSources::default()
            },
        );
        assert_eq!(with_slash.groups[0].label, "Workspace");
    }

    #[test]
    fn path_replacements_and_ordering() {
        let entry = |path: &str, kind: PathEntryKind, score: f64| WorkspacePathEntry {
            kind,
            path: path.into(),
            name: path.rsplit('/').next().unwrap_or(path).into(),
            score,
            positions: Vec::new(),
        };
        let suggestions = build_path_suggestions(
            &[
                entry("src/a.rs", PathEntryKind::File, 5.0),
                entry("src", PathEntryKind::Directory, 5.0),
            ],
            &[entry("notes.md", PathEntryKind::File, 9.0)],
            8,
        );
        let replacements: Vec<&str> = suggestions
            .iter()
            .map(MentionSuggestion::replacement)
            .collect();
        assert_eq!(
            replacements,
            ["thread-storage:notes.md", "src/", "src/a.rs"]
        );
        assert_eq!(suggestions[1].serialized_text('@'), "@src/");
    }

    #[test]
    fn thread_suggestions_rank_relations_and_skip_current() {
        let thread = |id: &str, title: &str, parent: Option<&str>, env: Option<&str>| Thread {
            id: id.into(),
            project_id: "p1".into(),
            title: Some(title.into()),
            parent_thread_id: parent.map(str::to_owned),
            environment_id: env.map(str::to_owned),
            ..Thread::default()
        };
        let current = thread(
            "thr_cur",
            "Deploy current",
            Some("thr_parent"),
            Some("env1"),
        );
        let parent = thread("thr_parent", "Deploy parent", None, None);
        let sibling = thread("thr_sib", "Deploy sibling", Some("thr_parent"), None);
        let roommate = thread("thr_room", "Deploy roommate", None, Some("env1"));
        let other = thread("thr_other", "Deploy other", None, None);
        let suggestions = build_thread_mention_suggestions(&ThreadMentionArgs {
            threads: vec![&other, &roommate, &sibling, &parent, &current],
            query: "deploy",
            current_thread_id: Some("thr_cur"),
            limit: 8,
            ..ThreadMentionArgs::default()
        });
        let ids: Vec<String> = suggestions
            .iter()
            .map(|suggestion| match suggestion {
                MentionSuggestion::Thread { thread_id, .. } => thread_id.clone(),
                _ => String::new(),
            })
            .collect();
        assert_eq!(ids, ["thr_parent", "thr_sib", "thr_room", "thr_other"]);
        assert_eq!(suggestions[0].serialized_text('@'), "@thread:thr_parent");
        assert_eq!(suggestions[0].title(), "Deploy parent · parent".to_owned());
    }

    #[test]
    fn command_ordering_prefers_exact_then_sections() {
        let command = |name: &str, source, origin| ProviderCommand {
            name: name.into(),
            source,
            origin,
            description: None,
            argument_hint: None,
            plugin_id: None,
        };
        let commands = vec![
            command(
                "review-pr",
                ProviderCommandSource::Command,
                ProviderCommandOrigin::Project,
            ),
            command(
                "review",
                ProviderCommandSource::Command,
                ProviderCommandOrigin::Builtin,
            ),
            command(
                "pdf:review",
                ProviderCommandSource::Skill,
                ProviderCommandOrigin::User,
            ),
            command(
                "compact",
                ProviderCommandSource::Command,
                ProviderCommandOrigin::Builtin,
            ),
        ];
        let built = build_command_suggestions(&commands, &[], '/', CommandScope::NewThread, "rev");
        assert!(built.iter().all(|suggestion| suggestion.name != "compact"));
        let ordered = order_command_suggestions(built, "review");
        let labels: Vec<&str> = ordered
            .groups
            .iter()
            .map(|group| group.label.as_str())
            .collect();
        assert_eq!(labels, ["Commands", "Skills", "Project commands"]);
        let skills = build_command_suggestions(&commands, &[], '$', CommandScope::Thread, "");
        assert_eq!(skills.len(), 1);
        let resource = skills[0].resource(PromptMentionCommandTrigger::Dollar);
        assert_eq!(
            serde_json::to_value(&resource).unwrap()["source"],
            serde_json::json!("skill")
        );
        let with_action = build_command_suggestions(
            &commands,
            &[('/', "plan".into())],
            '/',
            CommandScope::Thread,
            "pla",
        );
        assert_eq!(with_action[0].name, "plan");
    }

    #[test]
    fn plugin_triggers_are_ordered_and_deduplicated() {
        let provider = |triggers: &[&str]| PluginMentionProviderContribution {
            plugin_id: "p".into(),
            id: "x".into(),
            label: "X".into(),
            triggers: triggers.iter().map(|value| (*value).to_owned()).collect(),
        };
        assert_eq!(mention_triggers(&[]), ['@']);
        assert_eq!(
            mention_triggers(&[provider(&["~", "#"]), provider(&["#", "??"])]),
            ['@', '#', '~']
        );
    }
}
