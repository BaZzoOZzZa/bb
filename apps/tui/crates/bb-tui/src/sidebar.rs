use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use bb_wire::environment::Host;
use bb_wire::project::{ProjectKind, ProjectWithThreadsResponse, SidebarBootstrapResponse};
use bb_wire::thread::{ThreadListEntry, ThreadStatus, ThreadVisibility};
use bb_wire::thread_list::{
    ChronologicalSort, OrganizationMode, SortDirection, ThreadListPreferences,
};
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use crate::indicator::{Indicator, is_unread_done};

pub const PINNED_GROUP: &str = "@pinned";
pub const THREADS_GROUP: &str = "@threads";
pub const MORE_GROUP: &str = "@more";
pub const ARCHIVED_MORE: &str = "@archived-more";
pub const ARCHIVED_NOTICE: &str = "@archived-status";
pub const NO_MACHINE_GROUP_KEY: &str = "no-machine";
pub const CHRONOLOGICAL_CONTAINER_ID: &str = "chronological";
const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RowKey {
    Group(String),
    Thread(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Collapse {
    BuiltIn(&'static str),
    Project(String),
    ThreadSection(String),
    Machine(String),
    Environment(String),
    Thread(String),
    More,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupAction {
    Collapse(Collapse),
    ShowMoreArchived,
    Notice,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupRow {
    pub key: String,
    pub label: String,
    pub action: GroupAction,
    pub depth: usize,
    pub project_id: Option<String>,
    pub thread_count: usize,
    pub collapsed: bool,
    pub attention: usize,
    pub ancestors: Vec<Collapse>,
}

impl GroupRow {
    pub fn collapse(&self) -> Option<&Collapse> {
        match &self.action {
            GroupAction::Collapse(collapse) => Some(collapse),
            GroupAction::ShowMoreArchived | GroupAction::Notice => None,
        }
    }

    pub fn is_foldable(&self) -> bool {
        self.collapse().is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadRow {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub indicator: Indicator,
    pub unread: bool,
    pub depth: usize,
    pub pinned: bool,
    pub archived: bool,
    pub attention_at: i64,
    pub child_count: usize,
    pub expanded: bool,
    pub ancestors: Vec<Collapse>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidebarRow {
    Group(GroupRow),
    Thread(ThreadRow),
}

impl SidebarRow {
    pub fn key(&self) -> RowKey {
        match self {
            Self::Group(group) => RowKey::Group(group.key.clone()),
            Self::Thread(thread) => RowKey::Thread(thread.id.clone()),
        }
    }

    pub fn thread(&self) -> Option<&ThreadRow> {
        match self {
            Self::Thread(thread) => Some(thread),
            Self::Group(_) => None,
        }
    }

    pub fn depth(&self) -> usize {
        match self {
            Self::Group(group) => group.depth,
            Self::Thread(thread) => thread.depth,
        }
    }

    pub fn own_collapse(&self) -> Option<Collapse> {
        match self {
            Self::Group(group) => group.collapse().cloned(),
            Self::Thread(thread) => Some(Collapse::Thread(thread.id.clone())),
        }
    }

    pub fn ancestors(&self) -> &[Collapse] {
        match self {
            Self::Group(group) => &group.ancestors,
            Self::Thread(thread) => &thread.ancestors,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArchivedStatus {
    #[default]
    Off,
    Loading,
    Error,
    Ready {
        has_more: bool,
        loading_more: bool,
        more_failed: bool,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct SidebarSource<'a> {
    pub sidebar: &'a SidebarBootstrapResponse,
    pub archived: &'a [ThreadListEntry],
    pub archived_status: ArchivedStatus,
    pub hosts: &'a [Host],
    pub prefs: &'a ThreadListPreferences,
}

impl<'a> SidebarSource<'a> {
    pub fn new(sidebar: &'a SidebarBootstrapResponse, prefs: &'a ThreadListPreferences) -> Self {
        Self {
            sidebar,
            archived: &[],
            archived_status: ArchivedStatus::Off,
            hosts: &[],
            prefs,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SidebarView<'a> {
    pub filter: &'a str,
    pub more_open: bool,
    pub force_open: bool,
    pub expanded_while_grouped: &'a HashSet<String>,
    pub held_read: Option<(&'a str, bool)>,
}

impl<'a> SidebarView<'a> {
    pub fn plain(expanded_while_grouped: &'a HashSet<String>) -> Self {
        Self {
            filter: "",
            more_open: false,
            force_open: false,
            expanded_while_grouped,
            held_read: None,
        }
    }
}

pub fn thread_title(entry: &ThreadListEntry) -> String {
    entry
        .thread
        .title_or_fallback()
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or("Untitled thread")
        .to_owned()
}

pub fn project_label(project: &ProjectWithThreadsResponse) -> String {
    if project.project.kind == ProjectKind::Personal && project.project.name.trim().is_empty() {
        return "Personal".into();
    }
    if project.project.name.trim().is_empty() {
        return project.project.id.clone();
    }
    project.project.name.clone()
}

fn all_projects(
    sidebar: &SidebarBootstrapResponse,
) -> impl Iterator<Item = &ProjectWithThreadsResponse> {
    sidebar
        .projects
        .iter()
        .chain(std::iter::once(&sidebar.personal_project))
        .filter(|project| !project.project.id.is_empty())
}

fn remote_tail(url: &str) -> Option<String> {
    let trimmed = url.trim().trim_end_matches('/');
    let trimmed = trimmed.strip_suffix(".git").unwrap_or(trimmed);
    let path = match trimmed.split_once("://") {
        Some((_, rest)) => rest.split_once('/').map(|(_, path)| path)?,
        None => trimmed.rsplit_once(':').map(|(_, path)| path)?,
    };
    let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    let tail = parts[parts.len().saturating_sub(2)..].join("/");
    (!tail.is_empty()).then_some(tail)
}

fn source_path(project: &ProjectWithThreadsResponse) -> Option<&str> {
    project
        .sources
        .iter()
        .find(|source| source.is_default)
        .or_else(|| project.sources.first())
        .map(|source| source.path.trim_end_matches(['/', '\\']))
        .filter(|path| !path.is_empty())
}

fn path_tail(path: &str, depth: usize) -> String {
    let parts: Vec<&str> = path
        .split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .collect();
    parts[parts.len().saturating_sub(depth)..].join("/")
}

fn distinct_path_tails(paths: &[Option<&str>]) -> Vec<Option<String>> {
    let deepest = paths
        .iter()
        .flatten()
        .map(|path| {
            path.split(['/', '\\'])
                .filter(|part| !part.is_empty())
                .count()
        })
        .max()
        .unwrap_or(0);
    let mut depths = vec![1usize; paths.len()];
    for _ in 0..deepest {
        let tails: Vec<Option<String>> = paths
            .iter()
            .zip(&depths)
            .map(|(path, depth)| path.map(|path| path_tail(path, *depth)))
            .collect();
        let mut changed = false;
        for (index, tail) in tails.iter().enumerate() {
            let Some(tail) = tail else { continue };
            if tails
                .iter()
                .enumerate()
                .any(|(other, value)| other != index && value.as_ref() == Some(tail))
                && depths[index] < deepest
            {
                depths[index] += 1;
                changed = true;
            }
        }
        if !changed {
            return tails;
        }
    }
    paths
        .iter()
        .zip(&depths)
        .map(|(path, depth)| path.map(|path| path_tail(path, *depth)))
        .collect()
}

fn unique_among(values: &[Option<String>], index: usize) -> Option<String> {
    let value = values[index].as_ref()?;
    let duplicated = values
        .iter()
        .enumerate()
        .any(|(other, candidate)| other != index && candidate.as_ref() == Some(value));
    (!duplicated).then(|| value.clone())
}

pub fn project_labels(sidebar: &SidebarBootstrapResponse) -> HashMap<String, String> {
    let projects: Vec<&ProjectWithThreadsResponse> = all_projects(sidebar).collect();
    let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, project) in projects.iter().enumerate() {
        by_name
            .entry(project_label(project).trim().to_lowercase())
            .or_default()
            .push(index);
    }
    let mut labels = HashMap::new();
    for indexes in by_name.values() {
        if indexes.len() == 1 {
            let project = projects[indexes[0]];
            labels.insert(project.project.id.clone(), project_label(project));
            continue;
        }
        let remotes: Vec<Option<String>> = indexes
            .iter()
            .map(|index| {
                projects[*index]
                    .project
                    .git_remote_url
                    .as_deref()
                    .and_then(remote_tail)
            })
            .collect();
        let paths: Vec<Option<&str>> = indexes
            .iter()
            .map(|index| source_path(projects[*index]))
            .collect();
        let tails = distinct_path_tails(&paths);
        for (position, index) in indexes.iter().enumerate() {
            let project = projects[*index];
            let qualifier = unique_among(&remotes, position)
                .or_else(|| unique_among(&tails, position))
                .unwrap_or_else(|| {
                    let id = &project.project.id;
                    id.strip_prefix("proj_")
                        .unwrap_or(id)
                        .chars()
                        .take(8)
                        .collect()
                });
            labels.insert(
                project.project.id.clone(),
                format!("{} ({qualifier})", project_label(project)),
            );
        }
    }
    labels
}

pub fn project_display_label(
    sidebar: &SidebarBootstrapResponse,
    project_id: &str,
) -> Option<String> {
    project_labels(sidebar).remove(project_id)
}

fn compare_codepoint(left: &str, right: &str) -> Ordering {
    left.cmp(right)
}

fn locale_cmp(left: &str, right: &str) -> Ordering {
    left.to_lowercase()
        .cmp(&right.to_lowercase())
        .then_with(|| left.cmp(right).reverse())
}

fn compare_created_desc(left: &ThreadListEntry, right: &ThreadListEntry) -> Ordering {
    right
        .thread
        .created_at
        .cmp(&left.thread.created_at)
        .then_with(|| compare_codepoint(&left.thread.id, &right.thread.id))
}

pub fn compare_standard(left: &ThreadListEntry, right: &ThreadListEntry) -> Ordering {
    let left_active = left.thread.status == ThreadStatus::Active;
    let right_active = right.thread.status == ThreadStatus::Active;
    if left_active != right_active {
        return if left_active {
            Ordering::Less
        } else {
            Ordering::Greater
        };
    }
    if left_active {
        return compare_created_desc(left, right);
    }
    right
        .thread
        .latest_attention_at
        .cmp(&left.thread.latest_attention_at)
        .then_with(|| compare_created_desc(left, right))
}

pub fn compare_pinned(left: &ThreadListEntry, right: &ThreadListEntry) -> Ordering {
    if let (Some(a), Some(b)) = (&left.pin_sort_key, &right.pin_sort_key) {
        let by_key = compare_codepoint(a, b);
        if by_key != Ordering::Equal {
            return by_key;
        }
    }
    right
        .thread
        .pinned_at
        .unwrap_or(0)
        .cmp(&left.thread.pinned_at.unwrap_or(0))
        .then_with(|| compare_created_desc(left, right))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortField {
    Updated,
    Created,
    Alpha,
}

#[derive(Debug, Clone, Copy)]
struct ReadGrouping<'h> {
    held: Option<(&'h str, bool)>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Comparator<'h> {
    field: SortField,
    reversed: bool,
    read: Option<ReadGrouping<'h>>,
}

impl<'h> Comparator<'h> {
    fn standard() -> Self {
        Self {
            field: SortField::Updated,
            reversed: false,
            read: None,
        }
    }

    pub(crate) fn new(
        sort: ChronologicalSort,
        direction: SortDirection,
        group_by_read: bool,
        held: Option<(&'h str, bool)>,
    ) -> Self {
        let field = match sort {
            ChronologicalSort::Alpha => SortField::Alpha,
            ChronologicalSort::Created => SortField::Created,
            ChronologicalSort::Updated | ChronologicalSort::None => SortField::Updated,
        };
        let natural = if field == SortField::Alpha {
            SortDirection::Ascending
        } else {
            SortDirection::Descending
        };
        Self {
            field,
            reversed: direction != SortDirection::Default && direction != natural,
            read: group_by_read.then_some(ReadGrouping { held }),
        }
    }

    fn apply(&self, ordering: Ordering) -> Ordering {
        if self.reversed {
            ordering.reverse()
        } else {
            ordering
        }
    }

    fn is_unread(&self, entry: &ThreadListEntry) -> bool {
        let Some(grouping) = self.read else {
            return false;
        };
        match grouping.held {
            Some((id, unread)) if id == entry.thread.id => {
                entry.thread.parent_thread_id.is_none()
                    && matches!(
                        entry.thread.status,
                        ThreadStatus::Idle | ThreadStatus::Error
                    )
                    && unread
            }
            _ => is_unread_done(&entry.thread),
        }
    }

    fn read_order(&self, left: &ThreadListEntry, right: &ThreadListEntry) -> Ordering {
        if self.read.is_none() {
            return Ordering::Equal;
        }
        self.is_unread(right).cmp(&self.is_unread(left))
    }

    fn threads(&self, left: &ThreadListEntry, right: &ThreadListEntry) -> Ordering {
        self.read_order(left, right)
            .then_with(|| self.base(left, right))
    }

    fn base(&self, left: &ThreadListEntry, right: &ThreadListEntry) -> Ordering {
        match self.field {
            SortField::Alpha => self.apply(
                locale_cmp(&thread_title(left), &thread_title(right))
                    .then_with(|| locale_cmp(&left.thread.id, &right.thread.id)),
            ),
            SortField::Created => self.apply(compare_created_desc(left, right)),
            SortField::Updated => {
                let comparison = compare_standard(left, right);
                if (left.thread.status == ThreadStatus::Active)
                    != (right.thread.status == ThreadStatus::Active)
                {
                    comparison
                } else {
                    self.apply(comparison)
                }
            }
        }
    }

    fn has_item_order(&self) -> bool {
        self.field == SortField::Alpha
    }

    fn items(&self, left: &Item<'_>, right: &Item<'_>) -> Ordering {
        let read = match (left, right) {
            (Item::Thread(a), Item::Thread(b)) => self.read_order(a.thread, b.thread),
            _ => Ordering::Equal,
        };
        read.then_with(|| self.apply(compare_items_by_title(left, right)))
    }
}

fn compare_items_by_title(left: &Item<'_>, right: &Item<'_>) -> Ordering {
    let by_label = locale_cmp(&left.alpha_label(), &right.alpha_label());
    if by_label != Ordering::Equal {
        return by_label;
    }
    if let (Some(a), Some(b)) = (left.lead_thread(), right.lead_thread()) {
        let by_id = locale_cmp(&a.thread.id, &b.thread.id);
        if by_id != Ordering::Equal {
            return by_id;
        }
    }
    let by_kind = left.kind_name().cmp(right.kind_name());
    if by_kind != Ordering::Equal {
        return by_kind;
    }
    match (left, right) {
        (Item::Section { key: a, .. }, Item::Section { key: b, .. }) => locale_cmp(a, b),
        _ => Ordering::Equal,
    }
}

type Entry<'a> = &'a ThreadListEntry;

#[derive(Debug, Clone)]
struct Node<'a> {
    thread: Entry<'a>,
    children: Vec<Item<'a>>,
}

#[derive(Debug, Clone)]
enum Item<'a> {
    Thread(Node<'a>),
    Environment {
        id: String,
        nodes: Vec<Node<'a>>,
    },
    Section {
        id: String,
        key: String,
        name: String,
        items: Vec<Item<'a>>,
    },
}

impl<'a> Item<'a> {
    fn kind_name(&self) -> &'static str {
        match self {
            Self::Thread(_) => "thread",
            Self::Environment { .. } => "environment",
            Self::Section { .. } => "section",
        }
    }

    fn lead_thread(&self) -> Option<Entry<'a>> {
        match self {
            Self::Thread(node) => Some(node.thread),
            Self::Environment { nodes, .. } => nodes.first().map(|node| node.thread),
            Self::Section { .. } => None,
        }
    }

    fn representative(&self) -> Option<Entry<'a>> {
        match self {
            Self::Section { items, .. } => items.first().and_then(Item::representative),
            _ => self.lead_thread(),
        }
    }

    fn alpha_label(&self) -> String {
        match self {
            Self::Section { name, .. } => name.clone(),
            _ => self.lead_thread().map(thread_title).unwrap_or_default(),
        }
    }

    fn fallback_label(&self) -> String {
        match self {
            Self::Thread(node) => node.thread.thread.id.clone(),
            Self::Environment { id, .. } => id.clone(),
            Self::Section { name, .. } => name.clone(),
        }
    }

    fn ordering_thread(&self, compare: &Comparator<'_>) -> Option<Entry<'a>> {
        match self {
            Self::Section { items, .. } => {
                let mut descendants = Vec::new();
                collect_items(items, &mut descendants);
                descendants.into_iter().reduce(|first, thread| {
                    if compare.threads(thread, first) == Ordering::Less {
                        thread
                    } else {
                        first
                    }
                })
            }
            _ => self.lead_thread(),
        }
    }
}

fn collect_node<'a>(node: &Node<'a>, out: &mut Vec<Entry<'a>>) {
    out.push(node.thread);
    collect_items(&node.children, out);
}

fn collect_items<'a>(items: &[Item<'a>], out: &mut Vec<Entry<'a>>) {
    for item in items {
        match item {
            Item::Thread(node) => collect_node(node, out),
            Item::Environment { nodes, .. } => {
                for node in nodes {
                    collect_node(node, out);
                }
            }
            Item::Section { items, .. } => collect_items(items, out),
        }
    }
}

fn is_hidden(entry: &ThreadListEntry) -> bool {
    entry.thread.visibility == ThreadVisibility::Hidden
}

struct TreeBuilder<'a, 'c, 'h> {
    children: HashMap<&'a str, Vec<Entry<'a>>>,
    visited: HashSet<&'a str>,
    compare: &'c Comparator<'h>,
    group_environments: bool,
}

impl<'a> TreeBuilder<'a, '_, '_> {
    fn node(&mut self, thread: Entry<'a>, ancestors: &[&'a str]) -> Node<'a> {
        let id = thread.thread.id.as_str();
        self.visited.insert(id);
        let mut next = ancestors.to_vec();
        next.push(id);
        let mut nodes = Vec::new();
        if next.len() < MAX_DEPTH {
            let candidates = self.children.get(id).cloned().unwrap_or_default();
            for child in candidates {
                let child_id = child.thread.id.as_str();
                if next.contains(&child_id) || self.visited.contains(child_id) {
                    continue;
                }
                nodes.push(self.node(child, &next));
            }
        }
        let children = sorted_items(nodes, self.compare, self.group_environments, false);
        Node { thread, children }
    }
}

fn tree_items<'a>(
    all: &[Entry<'a>],
    compare: &Comparator<'_>,
    group_environments: bool,
    respect_sections: bool,
) -> Vec<Item<'a>> {
    let threads: Vec<Entry<'a>> = all.iter().copied().filter(|t| !is_hidden(t)).collect();
    let ids: HashSet<&str> = threads.iter().map(|t| t.thread.id.as_str()).collect();
    let mut children: HashMap<&'a str, Vec<Entry<'a>>> = HashMap::new();
    for thread in &threads {
        if let Some(parent) = thread.thread.parent_thread_id.as_deref()
            && let Some(parent) = ids.get(parent)
        {
            children.entry(parent).or_default().push(thread);
        }
    }
    let mut builder = TreeBuilder {
        children,
        visited: HashSet::new(),
        compare,
        group_environments,
    };
    let mut roots = Vec::new();
    for thread in &threads {
        let is_root = thread
            .thread
            .parent_thread_id
            .as_deref()
            .is_none_or(|parent| !ids.contains(parent));
        if is_root && !builder.visited.contains(thread.thread.id.as_str()) {
            roots.push(builder.node(thread, &[]));
        }
    }
    for thread in &threads {
        if !builder.visited.contains(thread.thread.id.as_str()) {
            roots.push(builder.node(thread, &[]));
        }
    }
    sorted_items(roots, compare, group_environments, respect_sections)
}

fn sorted_items<'a>(
    mut nodes: Vec<Node<'a>>,
    compare: &Comparator<'_>,
    group_environments: bool,
    respect_sections: bool,
) -> Vec<Item<'a>> {
    if group_environments && respect_sections {
        let mut buckets: Vec<(Option<String>, Vec<Node<'a>>)> = Vec::new();
        for node in nodes {
            let section = node.thread.thread.section_id.clone();
            match buckets.iter_mut().find(|(id, _)| *id == section) {
                Some((_, bucket)) => bucket.push(node),
                None => buckets.push((section, vec![node])),
            }
        }
        return buckets
            .into_iter()
            .flat_map(|(_, bucket)| sorted_items(bucket, compare, true, false))
            .collect();
    }
    if !group_environments {
        nodes.sort_by(|a, b| compare.threads(a.thread, b.thread));
        return nodes.into_iter().map(Item::Thread).collect();
    }
    let mut buckets: Vec<(String, Vec<usize>)> = Vec::new();
    for (index, node) in nodes.iter().enumerate() {
        let Some(environment) = node.thread.thread.environment_id.as_deref() else {
            continue;
        };
        if node.thread.environment_is_worktree != Some(true) {
            continue;
        }
        match buckets.iter_mut().find(|(id, _)| id == environment) {
            Some((_, bucket)) => bucket.push(index),
            None => buckets.push((environment.to_owned(), vec![index])),
        }
    }
    let grouped: HashSet<String> = buckets
        .iter()
        .filter(|(_, bucket)| bucket.len() >= 2)
        .map(|(id, _)| id.clone())
        .collect();
    let mut slots: Vec<Option<Node<'a>>> = nodes.into_iter().map(Some).collect();
    let mut environments = Vec::new();
    for (id, bucket) in buckets {
        if !grouped.contains(&id) {
            continue;
        }
        let mut members: Vec<Node<'a>> = bucket
            .into_iter()
            .filter_map(|index| slots[index].take())
            .collect();
        members.sort_by(|a, b| compare.threads(a.thread, b.thread));
        environments.push(Item::Environment { id, nodes: members });
    }
    let mut loose: Vec<Node<'a>> = slots.into_iter().flatten().collect();
    loose.sort_by(|a, b| compare.threads(a.thread, b.thread));
    let mut items: Vec<Item<'a>> = loose.into_iter().map(Item::Thread).collect();
    items.extend(environments);
    items.sort_by(|a, b| match (a.representative(), b.representative()) {
        (Some(a), Some(b)) => compare.threads(a, b),
        _ => Ordering::Equal,
    });
    items
}

fn order_siblings<'a>(mut items: Vec<Item<'a>>, compare: &Comparator<'_>) -> Vec<Item<'a>> {
    items.sort_by(|a, b| {
        let a_section = matches!(a, Item::Section { .. });
        let b_section = matches!(b, Item::Section { .. });
        if a_section != b_section {
            return if a_section {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }
        if compare.has_item_order() {
            return compare.items(a, b);
        }
        match (a.ordering_thread(compare), b.ordering_thread(compare)) {
            (Some(x), Some(y)) => compare.threads(x, y),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => compare_codepoint(&a.fallback_label(), &b.fallback_label()),
        }
    });
    items
}

fn section_key(container: &str, section_id: &str) -> String {
    format!("{container}::{section_id}")
}

fn section_thread_list<'a>(
    threads: &[Entry<'a>],
    compare: &Comparator<'_>,
    sections: &[(String, String)],
    group_environments: bool,
) -> Vec<Item<'a>> {
    let items = tree_items(threads, compare, group_environments, true);
    let mut ordered: Vec<(String, String)> = Vec::new();
    for (id, name) in sections {
        if !ordered.iter().any(|(known, _)| known == id) {
            ordered.push((id.clone(), name.clone()));
        }
    }
    let mut by_section: HashMap<String, Vec<Item<'a>>> = ordered
        .iter()
        .map(|(id, _)| (id.clone(), Vec::new()))
        .collect();
    let mut loose = Vec::new();
    for item in items {
        let section = item
            .ordering_thread(compare)
            .and_then(|thread| thread.thread.section_id.clone())
            .filter(|id| !id.is_empty());
        let Some(section) = section else {
            loose.push(item);
            continue;
        };
        if !by_section.contains_key(&section) {
            ordered.push((section.clone(), "Section".into()));
            by_section.insert(section.clone(), Vec::new());
        }
        if let Some(bucket) = by_section.get_mut(&section) {
            bucket.push(item);
        }
    }
    let section_items: Vec<Item<'a>> = ordered
        .into_iter()
        .map(|(id, name)| Item::Section {
            key: section_key(CHRONOLOGICAL_CONTAINER_ID, &id),
            items: order_siblings(by_section.remove(&id).unwrap_or_default(), compare),
            id,
            name,
        })
        .collect();
    let mut result = if compare.has_item_order() {
        order_siblings(section_items, compare)
    } else {
        section_items
    };
    result.extend(order_siblings(loose, compare));
    result
}

struct PinnedState<'a> {
    effective: HashSet<&'a str>,
    items: Vec<Item<'a>>,
}

fn pinned_state<'a>(threads: &[Entry<'a>], group_environments: bool) -> PinnedState<'a> {
    let mut children: HashMap<&str, Vec<Entry<'a>>> = HashMap::new();
    for thread in threads {
        if let Some(parent) = thread.thread.parent_thread_id.as_deref() {
            children.entry(parent).or_default().push(thread);
        }
    }
    let mut effective: HashSet<&'a str> = HashSet::new();
    for thread in threads.iter().filter(|t| t.thread.pinned_at.is_some()) {
        effective.insert(thread.thread.id.as_str());
        let mut visited: HashSet<&str> = HashSet::new();
        let mut stack = vec![thread.thread.id.as_str()];
        while let Some(parent) = stack.pop() {
            if !visited.insert(parent) {
                continue;
            }
            for child in children.get(parent).map(Vec::as_slice).unwrap_or(&[]) {
                effective.insert(child.thread.id.as_str());
                stack.push(child.thread.id.as_str());
            }
        }
    }
    let pinned: Vec<Entry<'a>> = threads
        .iter()
        .copied()
        .filter(|t| effective.contains(t.thread.id.as_str()))
        .collect();
    let mut items = tree_items(&pinned, &Comparator::standard(), group_environments, false);
    for item in &mut items {
        if let Item::Environment { nodes, .. } = item {
            nodes.sort_by(|a, b| compare_pinned(a.thread, b.thread));
        }
    }
    items.sort_by(|a, b| match (a.representative(), b.representative()) {
        (Some(a), Some(b)) => compare_pinned(a, b),
        _ => Ordering::Equal,
    });
    PinnedState { effective, items }
}

fn machine_groups<'a>(
    threads: &[Entry<'a>],
    hosts: &[(String, String)],
) -> Vec<(String, String, Vec<Entry<'a>>)> {
    let mut by_key: Vec<(String, Vec<Entry<'a>>)> = Vec::new();
    for thread in threads {
        let key = thread
            .environment_host_id
            .clone()
            .unwrap_or_else(|| NO_MACHINE_GROUP_KEY.into());
        match by_key.iter_mut().find(|(known, _)| *known == key) {
            Some((_, bucket)) => bucket.push(thread),
            None => by_key.push((key, vec![thread])),
        }
    }
    let mut take = |key: &str| {
        by_key
            .iter()
            .position(|(known, _)| known == key)
            .map(|index| by_key.remove(index).1)
    };
    let mut groups = Vec::new();
    for (id, name) in hosts {
        let threads = take(id).unwrap_or_default();
        groups.push((id.clone(), name.clone(), threads));
    }
    let no_machine = take(NO_MACHINE_GROUP_KEY);
    by_key.sort_by(|a, b| locale_cmp(&a.0, &b.0));
    for (key, threads) in by_key {
        groups.push((key, "Unknown machine".into(), threads));
    }
    if let Some(threads) = no_machine {
        groups.push((NO_MACHINE_GROUP_KEY.into(), "No machine".into(), threads));
    }
    groups
}

fn is_section_id(value: &str) -> bool {
    value == "pinned"
        || value == "threads"
        || value.starts_with("project:")
        || value.starts_with("section:")
        || value.starts_with("machine:")
}

pub fn normalize_section_order(
    stored: &[String],
    entity_ids: &[String],
    legacy_anchor: &str,
    has_pinned: bool,
    has_threads: bool,
) -> Vec<String> {
    let mut available: HashSet<&str> = entity_ids.iter().map(String::as_str).collect();
    if has_pinned {
        available.insert("pinned");
    }
    if has_threads {
        available.insert("threads");
    }
    let entity_set: HashSet<&str> = entity_ids.iter().map(String::as_str).collect();
    let mut seen: HashSet<String> = HashSet::new();
    let mut normalized: Vec<String> = Vec::new();
    let mut expanded_legacy = false;
    let append = |id: &str, seen: &mut HashSet<String>, normalized: &mut Vec<String>| {
        if available.contains(id) && seen.insert(id.to_owned()) {
            normalized.push(id.to_owned());
        }
    };
    for stored_id in stored {
        if stored_id == legacy_anchor {
            expanded_legacy = true;
            for entity in entity_ids {
                append(entity, &mut seen, &mut normalized);
            }
            continue;
        }
        if is_section_id(stored_id) {
            append(stored_id, &mut seen, &mut normalized);
        }
    }
    if has_pinned && !seen.contains("pinned") {
        normalized.insert(0, "pinned".into());
        seen.insert("pinned".into());
    }
    let missing: Vec<String> = entity_ids
        .iter()
        .filter(|id| !seen.contains(id.as_str()))
        .cloned()
        .collect();
    if !missing.is_empty() {
        let last_entity = normalized
            .iter()
            .rposition(|id| entity_set.contains(id.as_str()));
        let threads_index = normalized.iter().position(|id| id == "threads");
        let insertion = match last_entity {
            Some(index) => index + 1,
            None if threads_index.is_some() || expanded_legacy => threads_index.unwrap_or(0),
            None => normalized.len(),
        };
        for (offset, id) in missing.iter().enumerate() {
            normalized.insert(insertion + offset, id.clone());
            seen.insert(id.clone());
        }
    }
    if has_threads && !seen.contains("threads") {
        normalized.push("threads".into());
    }
    normalized
}

struct FuzzyFilter {
    pattern: Option<Pattern>,
    matcher: Matcher,
    buffer: Vec<char>,
}

impl FuzzyFilter {
    fn new(filter: &str) -> Self {
        let trimmed = filter.trim();
        Self {
            pattern: (!trimmed.is_empty())
                .then(|| Pattern::parse(trimmed, CaseMatching::Ignore, Normalization::Smart)),
            matcher: Matcher::new(Config::DEFAULT),
            buffer: Vec::new(),
        }
    }

    fn active(&self) -> bool {
        self.pattern.is_some()
    }

    fn matches(&mut self, haystack: &str) -> bool {
        let Some(pattern) = &self.pattern else {
            return true;
        };
        pattern
            .score(Utf32Str::new(haystack, &mut self.buffer), &mut self.matcher)
            .is_some()
    }
}

struct TopGroup<'a> {
    id: String,
    key: String,
    label: String,
    collapse: Collapse,
    project_id: Option<String>,
    items: Vec<Item<'a>>,
}

fn environment_label(entry: &ThreadListEntry) -> String {
    let folder = || {
        entry.environment_path.as_deref().and_then(|path| {
            path.split(['/', '\\'])
                .rfind(|part| !part.is_empty())
                .map(str::to_owned)
        })
    };
    entry
        .environment_name
        .clone()
        .filter(|name| !name.is_empty())
        .or_else(|| entry.environment_branch_name.clone())
        .or_else(|| match &entry.environment_provider_id {
            None => folder(),
            Some(provider) => Some(provider.clone()),
        })
        .unwrap_or_else(|| "Environment".into())
}

struct Emitter<'v, 'p> {
    view: SidebarView<'v>,
    prefs: &'p ThreadListPreferences,
    filter: FuzzyFilter,
    filtering: bool,
    visible: HashMap<String, bool>,
    stack: Vec<Collapse>,
    rows: Vec<SidebarRow>,
}

impl Emitter<'_, '_> {
    fn open_all(&self) -> bool {
        self.filtering || self.view.force_open
    }

    fn is_collapsed(&self, collapse: &Collapse) -> bool {
        if self.open_all() {
            return false;
        }
        let prefs = self.prefs;
        match collapse {
            Collapse::BuiltIn(id) => prefs.collapsed_sections.iter().any(|key| key == id),
            Collapse::Project(id) => prefs.collapsed_projects.contains(id),
            Collapse::ThreadSection(key) => prefs.collapsed_thread_sections.contains(key),
            Collapse::Machine(key) => prefs.collapsed_machines.contains(key),
            Collapse::Environment(id) => prefs.collapsed_environments.contains(id),
            Collapse::Thread(id) => {
                if prefs.group_by_read_status {
                    !self.view.expanded_while_grouped.contains(id)
                } else {
                    prefs.collapsed_threads.contains(id)
                }
            }
            Collapse::More => !self.view.more_open,
        }
    }

    fn node_visible(&mut self, node: &Node<'_>) -> bool {
        if !self.filtering {
            return true;
        }
        if let Some(known) = self.visible.get(&node.thread.thread.id) {
            return *known;
        }
        let mut visible = self.filter.matches(&thread_title(node.thread));
        for child in &node.children {
            visible |= self.item_visible(child);
        }
        self.visible.insert(node.thread.thread.id.clone(), visible);
        visible
    }

    fn item_visible(&mut self, item: &Item<'_>) -> bool {
        match item {
            Item::Thread(node) => self.node_visible(node),
            Item::Environment { nodes, .. } => {
                let mut visible = false;
                for node in nodes {
                    visible |= self.node_visible(node);
                }
                visible
            }
            Item::Section { items, .. } => {
                let mut visible = false;
                for item in items {
                    visible |= self.item_visible(item);
                }
                visible
            }
        }
    }

    fn visible_threads<'a>(&mut self, items: &[Item<'a>]) -> Vec<Entry<'a>> {
        let mut all = Vec::new();
        collect_items(items, &mut all);
        if !self.filtering {
            return all;
        }
        all.into_iter()
            .filter(|thread| self.visible.get(&thread.thread.id) == Some(&true))
            .collect()
    }

    fn push_group(
        &mut self,
        key: String,
        label: String,
        collapse: Collapse,
        depth: usize,
        project_id: Option<String>,
        items: &[Item<'_>],
    ) -> bool {
        let threads = self.visible_threads(items);
        let collapsed = self.is_collapsed(&collapse);
        self.rows.push(SidebarRow::Group(GroupRow {
            key,
            label,
            action: GroupAction::Collapse(collapse),
            depth,
            project_id,
            thread_count: threads.len(),
            collapsed,
            attention: threads
                .iter()
                .filter(|thread| Indicator::for_entry(thread).needs_attention())
                .count(),
            ancestors: self.stack.clone(),
        }));
        !collapsed
    }

    fn emit_top(&mut self, group: &TopGroup<'_>, depth: usize) {
        let mut any = false;
        for item in &group.items {
            any |= self.item_visible(item);
        }
        if self.filtering && !any {
            return;
        }
        let open = self.push_group(
            group.key.clone(),
            group.label.clone(),
            group.collapse.clone(),
            depth,
            group.project_id.clone(),
            &group.items,
        );
        if open {
            self.stack.push(group.collapse.clone());
            self.emit_items(&group.items, depth, group.id == "pinned");
            self.stack.pop();
        }
    }

    fn emit_items(&mut self, items: &[Item<'_>], depth: usize, pinned: bool) {
        for item in items {
            if !self.item_visible(item) {
                continue;
            }
            match item {
                Item::Thread(node) => self.emit_node(node, depth, pinned),
                Item::Environment { id, nodes } => {
                    let Some(first) = nodes.first() else { continue };
                    let collapse = Collapse::Environment(id.clone());
                    let open = self.push_group(
                        format!("env:{}", first.thread.thread.id),
                        environment_label(first.thread),
                        collapse.clone(),
                        depth,
                        None,
                        std::slice::from_ref(item),
                    );
                    if open {
                        self.stack.push(collapse);
                        for node in nodes {
                            if self.node_visible(node) {
                                self.emit_node(node, depth + 1, pinned);
                            }
                        }
                        self.stack.pop();
                    }
                }
                Item::Section {
                    id,
                    key,
                    name,
                    items,
                } => {
                    let collapse = Collapse::ThreadSection(key.clone());
                    let open = self.push_group(
                        format!("section:{id}"),
                        name.clone(),
                        collapse.clone(),
                        depth,
                        None,
                        items,
                    );
                    if open {
                        self.stack.push(collapse);
                        self.emit_items(items, depth, pinned);
                        self.stack.pop();
                    }
                }
            }
        }
    }

    fn emit_node(&mut self, node: &Node<'_>, depth: usize, pinned: bool) {
        let entry = node.thread;
        let descendants = self.visible_threads(&node.children).len();
        let collapse = Collapse::Thread(entry.thread.id.clone());
        let expanded = descendants > 0 && !self.is_collapsed(&collapse);
        self.rows.push(SidebarRow::Thread(ThreadRow {
            id: entry.thread.id.clone(),
            project_id: entry.thread.project_id.clone(),
            title: thread_title(entry),
            indicator: Indicator::for_entry(entry),
            unread: is_unread_done(&entry.thread),
            depth,
            pinned,
            archived: entry.thread.archived_at.is_some(),
            attention_at: entry.thread.latest_attention_at,
            child_count: descendants,
            expanded,
            ancestors: self.stack.clone(),
        }));
        if expanded && depth < MAX_DEPTH {
            self.stack.push(collapse);
            self.emit_items(&node.children, depth + 1, pinned);
            self.stack.pop();
        }
    }

    fn push_plain(&mut self, key: &str, label: String, action: GroupAction) {
        self.rows.push(SidebarRow::Group(GroupRow {
            key: key.into(),
            label,
            action,
            depth: 0,
            project_id: None,
            thread_count: 0,
            collapsed: false,
            attention: 0,
            ancestors: Vec::new(),
        }));
    }
}

fn personal_ids(sidebar: &SidebarBootstrapResponse) -> HashSet<&str> {
    let mut ids: HashSet<&str> = sidebar
        .projects
        .iter()
        .filter(|project| project.project.kind == ProjectKind::Personal)
        .map(|project| project.project.id.as_str())
        .collect();
    if !sidebar.personal_project.project.id.is_empty() {
        ids.insert(sidebar.personal_project.project.id.as_str());
    }
    ids
}

fn selected_threads<'a>(source: &SidebarSource<'a>) -> Vec<Entry<'a>> {
    let sidebar = source.sidebar;
    let mut order: Vec<Entry<'a>> = Vec::new();
    let mut index: HashMap<&'a str, usize> = HashMap::new();
    let mut insert = |entry: Entry<'a>, order: &mut Vec<Entry<'a>>| {
        if entry.thread.deleted_at.is_some() {
            return;
        }
        match index.get(entry.thread.id.as_str()) {
            Some(position) => order[*position] = entry,
            None => {
                index.insert(entry.thread.id.as_str(), order.len());
                order.push(entry);
            }
        }
    };
    if source.prefs.shows_archived() {
        for entry in source.archived {
            if entry.thread.archived_at.is_some() {
                insert(entry, &mut order);
            }
        }
    }
    let projects: Vec<&'a ProjectWithThreadsResponse> = all_projects(sidebar).collect();
    if source.prefs.shows_active() {
        for project in &projects {
            for entry in &project.threads {
                if entry.thread.archived_at.is_none() {
                    insert(entry, &mut order);
                }
            }
        }
    }
    let mut buckets: HashMap<&str, Vec<Entry<'a>>> = HashMap::new();
    for entry in order {
        buckets
            .entry(entry.thread.project_id.as_str())
            .or_default()
            .push(entry);
    }
    let mut seen = HashSet::new();
    projects
        .iter()
        .filter(|project| seen.insert(project.project.id.as_str()))
        .flat_map(|project| {
            buckets
                .remove(project.project.id.as_str())
                .unwrap_or_default()
        })
        .collect()
}

fn root_project_resolver<'a>(threads: &[Entry<'a>]) -> impl Fn(Entry<'a>) -> &'a str {
    let by_id: HashMap<&'a str, Entry<'a>> = threads
        .iter()
        .map(|thread| (thread.thread.id.as_str(), *thread))
        .collect();
    move |thread: Entry<'a>| {
        let mut current = thread;
        let mut visited: HashSet<&str> = HashSet::from([thread.thread.id.as_str()]);
        while let Some(parent) = current.thread.parent_thread_id.as_deref() {
            let Some(parent) = by_id.get(parent) else {
                break;
            };
            if !visited.insert(parent.thread.id.as_str()) {
                break;
            }
            current = parent;
        }
        current.thread.project_id.as_str()
    }
}

fn host_list(source: &SidebarSource<'_>, threads: &[Entry<'_>]) -> Vec<(String, String)> {
    let mut hosts: Vec<(String, String)> = source
        .hosts
        .iter()
        .map(|host| (host.id.clone(), host.name.clone()))
        .collect();
    for thread in threads {
        if let Some(id) = &thread.environment_host_id
            && !hosts.iter().any(|(known, _)| known == id)
        {
            hosts.push((id.clone(), id.clone()));
        }
    }
    hosts
}

struct Layout<'a> {
    stored: &'a [String],
    legacy: &'static str,
    has_threads: bool,
    groups: Vec<TopGroup<'a>>,
}

fn project_layout<'a>(
    source: &SidebarSource<'a>,
    threads: &[Entry<'a>],
    pinned: &HashSet<&'a str>,
    compare: &Comparator<'_>,
    group_environments: bool,
) -> Layout<'a> {
    let sidebar = source.sidebar;
    let personal = personal_ids(sidebar);
    let resolve = root_project_resolver(threads);
    let mut by_project: HashMap<&str, Vec<Entry<'a>>> = HashMap::new();
    for thread in threads {
        if pinned.contains(thread.thread.id.as_str()) {
            continue;
        }
        by_project.entry(resolve(thread)).or_default().push(thread);
    }
    let labels = project_labels(sidebar);
    let rows: Vec<&ProjectWithThreadsResponse> = sidebar
        .projects
        .iter()
        .filter(|project| {
            !project.project.id.is_empty() && !personal.contains(project.project.id.as_str())
        })
        .collect();
    let personal_id = (!sidebar.personal_project.project.id.is_empty())
        .then(|| sidebar.personal_project.project.id.clone());
    let personal_threads: Vec<Entry<'a>> = personal
        .iter()
        .flat_map(|id| by_project.get(id).cloned().unwrap_or_default())
        .filter(|thread| !is_hidden(thread))
        .collect();
    let has_threads = !personal_threads.is_empty() || rows.is_empty();
    let mut groups = vec![TopGroup {
        id: "threads".into(),
        key: THREADS_GROUP.into(),
        label: "Threads".into(),
        collapse: Collapse::BuiltIn("threads"),
        project_id: personal_id,
        items: tree_items(&personal_threads, compare, group_environments, false),
    }];
    for project in rows {
        let id = project.project.id.clone();
        let threads = by_project.get(id.as_str()).cloned().unwrap_or_default();
        groups.push(TopGroup {
            id: format!("project:{id}"),
            key: id.clone(),
            label: labels
                .get(&id)
                .cloned()
                .unwrap_or_else(|| project_label(project)),
            collapse: Collapse::Project(id.clone()),
            project_id: Some(id),
            items: tree_items(&threads, compare, group_environments, false),
        });
    }
    Layout {
        stored: &source.prefs.section_order,
        legacy: "projects",
        has_threads,
        groups,
    }
}

fn chronological_layout<'a>(
    source: &SidebarSource<'a>,
    threads: &[Entry<'a>],
    pinned: &HashSet<&'a str>,
    compare: &Comparator<'_>,
    group_environments: bool,
) -> Layout<'a> {
    let rest: Vec<Entry<'a>> = threads
        .iter()
        .copied()
        .filter(|thread| !pinned.contains(thread.thread.id.as_str()))
        .collect();
    let sections: Vec<(String, String)> = source
        .sidebar
        .sections
        .iter()
        .map(|section| (section.id.clone(), section.name.clone()))
        .collect();
    let items = section_thread_list(&rest, compare, &sections, group_environments);
    let mut loose = Vec::new();
    let mut by_section: HashMap<String, (String, Vec<Item<'a>>)> = HashMap::new();
    for item in items {
        match item {
            Item::Section {
                id, name, items, ..
            } => {
                by_section.insert(id, (name, items));
            }
            other => loose.push(other),
        }
    }
    let mut groups = vec![TopGroup {
        id: "threads".into(),
        key: THREADS_GROUP.into(),
        label: "Threads".into(),
        collapse: Collapse::BuiltIn("threads"),
        project_id: None,
        items: loose,
    }];
    for (id, _) in &sections {
        let Some((name, items)) = by_section.remove(id) else {
            continue;
        };
        groups.push(TopGroup {
            id: format!("section:{id}"),
            key: format!("section:{id}"),
            label: name,
            collapse: Collapse::ThreadSection(section_key(CHRONOLOGICAL_CONTAINER_ID, id)),
            project_id: None,
            items,
        });
    }
    Layout {
        stored: &source.prefs.manual_section_order,
        legacy: "sections",
        has_threads: true,
        groups,
    }
}

fn machine_layout<'a>(
    source: &SidebarSource<'a>,
    threads: &[Entry<'a>],
    pinned: &HashSet<&'a str>,
    compare: &Comparator<'_>,
    group_environments: bool,
) -> Layout<'a> {
    let rest: Vec<Entry<'a>> = threads
        .iter()
        .copied()
        .filter(|thread| !pinned.contains(thread.thread.id.as_str()) && !is_hidden(thread))
        .collect();
    let hosts = host_list(source, threads);
    let machines = machine_groups(&rest, &hosts);
    let has_threads = machines.is_empty();
    let mut groups = vec![TopGroup {
        id: "threads".into(),
        key: THREADS_GROUP.into(),
        label: "Threads".into(),
        collapse: Collapse::BuiltIn("threads"),
        project_id: None,
        items: if has_threads {
            tree_items(&rest, compare, group_environments, false)
        } else {
            Vec::new()
        },
    }];
    for (key, label, threads) in machines {
        groups.push(TopGroup {
            id: format!("machine:{key}"),
            key: format!("machine:{key}"),
            label,
            collapse: Collapse::Machine(key.clone()),
            project_id: None,
            items: tree_items(&threads, compare, group_environments, false),
        });
    }
    Layout {
        stored: &source.prefs.machine_section_order,
        legacy: "machines",
        has_threads,
        groups,
    }
}

pub fn build_rows(source: SidebarSource<'_>, view: SidebarView<'_>) -> Vec<SidebarRow> {
    let prefs = source.prefs;
    let group_environments = prefs.groups_by_environment();
    let compare = Comparator::new(
        prefs.chronological_sort,
        prefs.sort_direction,
        prefs.group_by_read_status,
        view.held_read,
    );
    let threads = selected_threads(&source);
    let pinned = pinned_state(&threads, group_environments);
    let layout = match prefs.organization_mode {
        OrganizationMode::Project => project_layout(
            &source,
            &threads,
            &pinned.effective,
            &compare,
            group_environments,
        ),
        OrganizationMode::Chronological => chronological_layout(
            &source,
            &threads,
            &pinned.effective,
            &compare,
            group_environments,
        ),
        OrganizationMode::Machine => machine_layout(
            &source,
            &threads,
            &pinned.effective,
            &compare,
            group_environments,
        ),
    };
    let show_pinned = !pinned.items.is_empty();
    let entity_ids: Vec<String> = layout
        .groups
        .iter()
        .filter(|group| group.id != "threads")
        .map(|group| group.id.clone())
        .collect();
    let persisted = normalize_section_order(
        layout.stored,
        &entity_ids,
        layout.legacy,
        true,
        layout.has_threads,
    );
    let hidden: HashSet<&str> = prefs.hidden_groups.iter().map(String::as_str).collect();
    let pinned_group = TopGroup {
        id: "pinned".into(),
        key: PINNED_GROUP.into(),
        label: "Pinned".into(),
        collapse: Collapse::BuiltIn("pinned"),
        project_id: None,
        items: pinned.items,
    };
    let filter = FuzzyFilter::new(view.filter);
    let filtering = filter.active();
    let mut emitter = Emitter {
        view,
        prefs,
        filter,
        filtering,
        visible: HashMap::new(),
        stack: Vec::new(),
        rows: Vec::new(),
    };
    let by_id: HashMap<&str, &TopGroup<'_>> = layout
        .groups
        .iter()
        .map(|group| (group.id.as_str(), group))
        .chain(std::iter::once(("pinned", &pinned_group)))
        .collect();
    let mut hidden_groups = Vec::new();
    for id in &persisted {
        let Some(group) = by_id.get(id.as_str()) else {
            continue;
        };
        if id == "pinned" {
            if show_pinned {
                emitter.emit_top(group, 0);
            }
        } else if hidden.contains(id.as_str()) {
            hidden_groups.push(*group);
        } else {
            emitter.emit_top(group, 0);
        }
    }
    if !hidden_groups.is_empty() {
        let mut any = !filtering;
        for group in &hidden_groups {
            for item in &group.items {
                any |= emitter.item_visible(item);
            }
        }
        if any {
            let collapsed = emitter.is_collapsed(&Collapse::More);
            let count = hidden_groups.len();
            emitter.rows.push(SidebarRow::Group(GroupRow {
                key: MORE_GROUP.into(),
                label: "More".into(),
                action: GroupAction::Collapse(Collapse::More),
                depth: 0,
                project_id: None,
                thread_count: count,
                collapsed,
                attention: 0,
                ancestors: Vec::new(),
            }));
            if !collapsed {
                emitter.stack.push(Collapse::More);
                for group in hidden_groups {
                    emitter.emit_top(group, 1);
                }
                emitter.stack.pop();
            }
        }
    }
    if !filtering {
        match source.archived_status {
            ArchivedStatus::Off => {}
            ArchivedStatus::Loading => emitter.push_plain(
                ARCHIVED_NOTICE,
                "Loading archived threads…".into(),
                GroupAction::Notice,
            ),
            ArchivedStatus::Error => emitter.push_plain(
                ARCHIVED_NOTICE,
                "Archived threads unavailable".into(),
                GroupAction::Notice,
            ),
            ArchivedStatus::Ready {
                has_more,
                loading_more,
                more_failed,
            } => {
                if has_more {
                    let label = if loading_more {
                        "Loading…"
                    } else if more_failed {
                        "Retry loading"
                    } else {
                        "Show more"
                    };
                    emitter.push_plain(ARCHIVED_MORE, label.into(), GroupAction::ShowMoreArchived);
                }
            }
        }
    }
    emitter.rows
}

pub fn attention_count(sidebar: &SidebarBootstrapResponse) -> usize {
    sidebar
        .projects
        .iter()
        .chain(std::iter::once(&sidebar.personal_project))
        .flat_map(|project| project.threads.iter())
        .filter(|entry| {
            entry.thread.visibility != ThreadVisibility::Hidden
                && entry.thread.deleted_at.is_none()
                && entry.thread.archived_at.is_none()
                && Indicator::for_entry(entry).needs_attention()
        })
        .count()
}

#[cfg(test)]
pub(crate) mod fixtures {
    use bb_wire::project::{Project, ProjectKind, ProjectWithThreadsResponse};
    use bb_wire::thread::{ThreadListEntry, ThreadStatus};

    pub fn thread(id: &str, project: &str, title: &str, attention_at: i64) -> ThreadListEntry {
        let mut entry = ThreadListEntry::default();
        entry.thread.id = id.into();
        entry.thread.project_id = project.into();
        entry.thread.title = Some(title.into());
        entry.thread.status = ThreadStatus::Idle;
        entry.thread.latest_attention_at = attention_at;
        entry.thread.last_read_at = Some(attention_at);
        entry.thread.created_at = attention_at;
        entry.thread.updated_at = attention_at;
        entry
    }

    pub fn project(
        id: &str,
        name: &str,
        threads: Vec<ThreadListEntry>,
    ) -> ProjectWithThreadsResponse {
        ProjectWithThreadsResponse {
            project: Project {
                id: id.into(),
                kind: ProjectKind::Standard,
                name: name.into(),
                ..Project::default()
            },
            threads,
            ..ProjectWithThreadsResponse::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;
    use bb_wire::project::ThreadSection;
    use bb_wire::thread_list::{EnvironmentGrouping, ThreadLifecycle};

    fn labels(rows: &[SidebarRow]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                SidebarRow::Group(group) => {
                    format!("{}# {}", "  ".repeat(group.depth), group.label)
                }
                SidebarRow::Thread(thread) => {
                    format!("{}{}", "  ".repeat(thread.depth), thread.title)
                }
            })
            .collect()
    }

    fn prefs(mode: OrganizationMode) -> ThreadListPreferences {
        ThreadListPreferences {
            organization_mode: mode,
            ..ThreadListPreferences::default()
        }
    }

    fn rows_with(
        sidebar: &SidebarBootstrapResponse,
        prefs: &ThreadListPreferences,
        filter: &str,
    ) -> Vec<String> {
        let expanded = HashSet::new();
        let view = SidebarView {
            filter,
            ..SidebarView::plain(&expanded)
        };
        labels(&build_rows(SidebarSource::new(sidebar, prefs), view))
    }

    fn rows(sidebar: &SidebarBootstrapResponse, prefs: &ThreadListPreferences) -> Vec<String> {
        rows_with(sidebar, prefs, "")
    }

    fn sample() -> SidebarBootstrapResponse {
        let mut pinned = thread("t_pin", "p1", "Pinned one", 1);
        pinned.thread.pinned_at = Some(5);
        let mut active = thread("t_active", "p1", "Running", 2);
        active.thread.status = ThreadStatus::Active;
        let recent = thread("t_recent", "p1", "Recent", 50);
        let mut child = thread("t_child", "p1", "Child agent", 60);
        child.thread.parent_thread_id = Some("t_recent".into());
        let mut hidden = thread("t_hidden", "p1", "Hidden", 70);
        hidden.thread.visibility = ThreadVisibility::Hidden;
        let mut orphan = thread("t_orphan", "p2", "Orphan child", 10);
        orphan.thread.parent_thread_id = Some("gone".into());
        let mut unread = thread("t_unread", "p2", "Fix the bug", 20);
        unread.thread.last_read_at = Some(1);
        SidebarBootstrapResponse {
            sections: Vec::new(),
            projects: vec![
                project("p1", "alpha", vec![recent, pinned, active, child, hidden]),
                project("p2", "beta", vec![orphan, unread]),
            ],
            personal_project: project("proj_personal", "Personal", vec![]),
        }
    }

    #[test]
    fn project_mode_pins_first_runs_active_first_and_nests_children_open() {
        let sidebar = sample();
        let prefs = prefs(OrganizationMode::Project);
        assert_eq!(
            rows(&sidebar, &prefs),
            vec![
                "# Pinned",
                "Pinned one",
                "# alpha",
                "Running",
                "Recent",
                "  Child agent",
                "# beta",
                "Fix the bug",
                "Orphan child",
            ]
        );
        let built = build_rows(
            SidebarSource::new(&sidebar, &prefs),
            SidebarView::plain(&HashSet::new()),
        );
        let SidebarRow::Group(beta) = &built[6] else {
            panic!("group")
        };
        assert_eq!((beta.attention, beta.thread_count), (1, 2));
        let SidebarRow::Thread(child) = &built[5] else {
            panic!("thread")
        };
        assert_eq!(
            child.ancestors,
            vec![
                Collapse::Project("p1".into()),
                Collapse::Thread("t_recent".into())
            ]
        );
    }

    #[test]
    fn collapsed_preferences_fold_projects_built_ins_and_children() {
        let sidebar = sample();
        let mut prefs = prefs(OrganizationMode::Project);
        prefs.collapsed_projects = vec!["p2".into()];
        prefs.collapsed_threads = vec!["t_recent".into()];
        prefs.collapsed_sections = vec!["pinned".into()];
        assert_eq!(
            rows(&sidebar, &prefs),
            vec!["# Pinned", "# alpha", "Running", "Recent", "# beta"]
        );
    }

    #[test]
    fn filter_keeps_ancestors_opens_folds_and_drops_empty_groups() {
        let sidebar = sample();
        let mut prefs = prefs(OrganizationMode::Project);
        prefs.collapsed_projects = vec!["p1".into()];
        assert_eq!(
            rows_with(&sidebar, &prefs, "chld"),
            vec![
                "# alpha",
                "Recent",
                "  Child agent",
                "# beta",
                "Orphan child"
            ]
        );
    }

    #[test]
    fn personal_threads_form_the_threads_section_only_when_needed() {
        let prefs = prefs(OrganizationMode::Project);
        let mut sidebar = SidebarBootstrapResponse {
            projects: vec![project("p1", "alpha", vec![])],
            personal_project: project("proj_personal", "Personal", vec![]),
            ..SidebarBootstrapResponse::default()
        };
        assert_eq!(rows(&sidebar, &prefs), vec!["# alpha"]);
        sidebar.personal_project.threads = vec![thread("t1", "proj_personal", "Note", 1)];
        assert_eq!(rows(&sidebar, &prefs), vec!["# alpha", "# Threads", "Note"]);
        sidebar.projects.clear();
        sidebar.personal_project.threads.clear();
        assert_eq!(rows(&sidebar, &prefs), vec!["# Threads"]);
    }

    #[test]
    fn project_mode_files_cross_project_children_under_the_root_project() {
        let parent = thread("t_parent", "p1", "Parent", 10);
        let mut child = thread("t_child", "p2", "Child elsewhere", 20);
        child.thread.parent_thread_id = Some("t_parent".into());
        let sidebar = SidebarBootstrapResponse {
            projects: vec![
                project("p1", "alpha", vec![parent]),
                project("p2", "beta", vec![child]),
            ],
            ..SidebarBootstrapResponse::default()
        };
        assert_eq!(
            rows(&sidebar, &prefs(OrganizationMode::Project)),
            vec!["# alpha", "Parent", "  Child elsewhere", "# beta"]
        );
    }

    #[test]
    fn section_order_preference_reorders_and_hidden_groups_move_into_more() {
        let sidebar = sample();
        let mut prefs = prefs(OrganizationMode::Project);
        prefs.section_order = vec!["project:p2".into(), "pinned".into(), "projects".into()];
        assert_eq!(
            rows(&sidebar, &prefs)
                .into_iter()
                .filter(|row| row.starts_with('#'))
                .collect::<Vec<_>>(),
            vec!["# beta", "# Pinned", "# alpha"]
        );
        prefs.hidden_groups = vec!["project:p1".into()];
        assert_eq!(
            rows(&sidebar, &prefs),
            vec![
                "# beta",
                "Fix the bug",
                "Orphan child",
                "# Pinned",
                "Pinned one",
                "# More"
            ]
        );
        let expanded = HashSet::new();
        let view = SidebarView {
            more_open: true,
            ..SidebarView::plain(&expanded)
        };
        let built = build_rows(SidebarSource::new(&sidebar, &prefs), view);
        assert_eq!(
            labels(&built)[5..],
            [
                "# More",
                "  # alpha",
                "  Running",
                "  Recent",
                "    Child agent"
            ]
        );
        let SidebarRow::Thread(running) = &built[7] else {
            panic!("thread")
        };
        assert_eq!(
            running.ancestors,
            vec![Collapse::More, Collapse::Project("p1".into())]
        );
    }

    fn sectioned() -> SidebarBootstrapResponse {
        let mut work = thread("t_work", "p1", "Work item", 30);
        work.thread.section_id = Some("s_work".into());
        let mut later = thread("t_later", "p2", "Later item", 40);
        later.thread.section_id = Some("s_work".into());
        let mut ghost = thread("t_ghost", "p1", "Ghost section", 50);
        ghost.thread.section_id = Some("s_gone".into());
        let loose = thread("t_loose", "p2", "Loose", 20);
        let mut pinned = thread("t_pin", "p1", "Pinned", 1);
        pinned.thread.pinned_at = Some(1);
        SidebarBootstrapResponse {
            sections: vec![
                ThreadSection {
                    id: "s_work".into(),
                    name: "Work".into(),
                    ..ThreadSection::default()
                },
                ThreadSection {
                    id: "s_empty".into(),
                    name: "Empty".into(),
                    ..ThreadSection::default()
                },
            ],
            projects: vec![
                project("p1", "alpha", vec![work, ghost, pinned]),
                project("p2", "beta", vec![later, loose]),
            ],
            ..SidebarBootstrapResponse::default()
        }
    }

    #[test]
    fn chronological_mode_lists_sections_then_threads_in_manual_order() {
        let sidebar = sectioned();
        let mut prefs = prefs(OrganizationMode::Chronological);
        assert_eq!(
            rows(&sidebar, &prefs),
            vec![
                "# Pinned",
                "Pinned",
                "# Work",
                "Later item",
                "Work item",
                "# Empty",
                "# Threads",
                "Loose",
            ]
        );
        prefs.manual_section_order = vec!["threads".into(), "section:s_empty".into()];
        prefs.collapsed_thread_sections = vec!["chronological::s_work".into()];
        assert_eq!(
            rows(&sidebar, &prefs),
            vec![
                "# Pinned",
                "Pinned",
                "# Threads",
                "Loose",
                "# Empty",
                "# Work",
            ]
        );
    }

    #[test]
    fn chronological_sort_field_and_direction_follow_the_web_comparator() {
        let sidebar = sectioned();
        let mut prefs = prefs(OrganizationMode::Chronological);
        prefs.chronological_sort = ChronologicalSort::Alpha;
        let work = |prefs: &ThreadListPreferences| {
            let all = rows(&sidebar, prefs);
            let start = all.iter().position(|row| row == "# Work").unwrap();
            all[start + 1..start + 3].to_vec()
        };
        assert_eq!(work(&prefs), ["Later item", "Work item"]);
        prefs.sort_direction = SortDirection::Descending;
        assert_eq!(work(&prefs), ["Work item", "Later item"]);
        prefs.chronological_sort = ChronologicalSort::Created;
        prefs.sort_direction = SortDirection::Default;
        assert_eq!(work(&prefs), ["Later item", "Work item"]);
        prefs.sort_direction = SortDirection::Ascending;
        assert_eq!(work(&prefs), ["Work item", "Later item"]);
    }

    #[test]
    fn reversed_updated_sort_keeps_running_threads_first() {
        let mut running = thread("t_run", "p1", "Running", 1);
        running.thread.status = ThreadStatus::Active;
        let sidebar = SidebarBootstrapResponse {
            projects: vec![project(
                "p1",
                "alpha",
                vec![
                    thread("t_old", "p1", "Old", 10),
                    running,
                    thread("t_new", "p1", "New", 90),
                ],
            )],
            ..SidebarBootstrapResponse::default()
        };
        let mut prefs = prefs(OrganizationMode::Chronological);
        assert_eq!(
            rows(&sidebar, &prefs),
            ["# Threads", "Running", "New", "Old"]
        );
        prefs.sort_direction = SortDirection::Ascending;
        assert_eq!(
            rows(&sidebar, &prefs),
            ["# Threads", "Running", "Old", "New"]
        );
    }

    fn host(id: &str, name: &str) -> Host {
        Host {
            id: id.into(),
            name: name.into(),
            ..Host::default()
        }
    }

    fn on_host(mut entry: ThreadListEntry, host: Option<&str>) -> ThreadListEntry {
        entry.environment_host_id = host.map(Into::into);
        entry
    }

    #[test]
    fn machine_mode_orders_known_hosts_unknown_hosts_then_no_machine() {
        let sidebar = SidebarBootstrapResponse {
            projects: vec![project(
                "p1",
                "alpha",
                vec![
                    on_host(thread("thr_1", "p1", "One", 10), Some("host_b")),
                    on_host(thread("thr_2", "p1", "Two", 20), None),
                    on_host(thread("thr_3", "p1", "Three", 30), Some("host_gone")),
                    on_host(thread("thr_4", "p1", "Four", 40), Some("host_a")),
                    on_host(thread("thr_5", "p1", "Five", 50), Some("host_b")),
                ],
            )],
            ..SidebarBootstrapResponse::default()
        };
        let hosts = vec![
            host("host_a", "Laptop"),
            host("host_b", "Desktop"),
            host("host_c", "Idle box"),
        ];
        let mut prefs = prefs(OrganizationMode::Machine);
        let source = SidebarSource {
            hosts: &hosts,
            ..SidebarSource::new(&sidebar, &prefs)
        };
        let expanded = HashSet::new();
        assert_eq!(
            labels(&build_rows(source, SidebarView::plain(&expanded))),
            vec![
                "# Laptop",
                "Four",
                "# Desktop",
                "Five",
                "One",
                "# Idle box",
                "# host_gone",
                "Three",
                "# No machine",
                "Two",
            ]
        );
        prefs.machine_section_order = vec!["machine:no-machine".into(), "machines".into()];
        prefs.collapsed_machines = vec!["host_b".into()];
        let source = SidebarSource {
            hosts: &hosts,
            ..SidebarSource::new(&sidebar, &prefs)
        };
        assert_eq!(
            labels(&build_rows(source, SidebarView::plain(&expanded)))[..4],
            ["# No machine", "Two", "# Laptop", "Four"]
        );
    }

    #[test]
    fn machine_groups_fall_back_to_unknown_labels_sorted_by_id() {
        let first = thread("thr_1", "p1", "One", 1);
        let second = thread("thr_2", "p1", "Two", 2);
        let a = on_host(second, Some("host_b"));
        let b = on_host(first, Some("host_a"));
        let groups = machine_groups(&[&a, &b], &[]);
        let keys: Vec<(&str, &str)> = groups
            .iter()
            .map(|(key, label, _)| (key.as_str(), label.as_str()))
            .collect();
        assert_eq!(
            keys,
            [("host_a", "Unknown machine"), ("host_b", "Unknown machine")]
        );
    }

    #[test]
    fn machine_mode_without_machines_shows_the_threads_section() {
        let sidebar = SidebarBootstrapResponse {
            projects: vec![project("p1", "alpha", vec![])],
            ..SidebarBootstrapResponse::default()
        };
        assert_eq!(
            rows(&sidebar, &prefs(OrganizationMode::Machine)),
            ["# Threads"]
        );
    }

    fn worktree(id: &str, title: &str, at: i64, environment: &str) -> ThreadListEntry {
        let mut entry = thread(id, "p1", title, at);
        entry.thread.environment_id = Some(environment.into());
        entry.environment_is_worktree = Some(true);
        entry.environment_branch_name = Some(format!("branch-{environment}"));
        entry
    }

    #[test]
    fn environment_grouping_follows_auto_mode_and_preference() {
        let sidebar = SidebarBootstrapResponse {
            projects: vec![project(
                "p1",
                "alpha",
                vec![
                    worktree("t_a", "First", 10, "env_1"),
                    worktree("t_b", "Second", 30, "env_1"),
                    worktree("t_c", "Single", 20, "env_2"),
                ],
            )],
            ..SidebarBootstrapResponse::default()
        };
        let mut prefs = prefs(OrganizationMode::Project);
        assert_eq!(
            rows(&sidebar, &prefs),
            ["# alpha", "# branch-env_1", "  Second", "  First", "Single"]
        );
        prefs.collapsed_environments = vec!["env_1".into()];
        assert_eq!(
            rows(&sidebar, &prefs),
            ["# alpha", "# branch-env_1", "Single"]
        );
        prefs.environment_grouping = EnvironmentGrouping::Off;
        assert_eq!(
            rows(&sidebar, &prefs),
            ["# alpha", "Second", "Single", "First"]
        );
        let chronological = super::tests::prefs(OrganizationMode::Chronological);
        assert_eq!(
            rows(&sidebar, &chronological),
            ["# Threads", "Second", "Single", "First"]
        );
    }

    #[test]
    fn archived_lifecycle_merges_archived_pages_and_offers_show_more() {
        let sidebar = sample();
        let mut archived = thread("t_arch", "p2", "Archived work", 15);
        archived.thread.archived_at = Some(100);
        let archived = vec![archived];
        let mut prefs = prefs(OrganizationMode::Project);
        prefs.thread_lifecycles = vec![ThreadLifecycle::Active, ThreadLifecycle::Archived];
        let expanded = HashSet::new();
        let source = SidebarSource {
            archived: &archived,
            archived_status: ArchivedStatus::Ready {
                has_more: true,
                loading_more: false,
                more_failed: false,
            },
            ..SidebarSource::new(&sidebar, &prefs)
        };
        let built = build_rows(source, SidebarView::plain(&expanded));
        assert_eq!(
            labels(&built)[6..],
            [
                "# beta",
                "Fix the bug",
                "Archived work",
                "Orphan child",
                "# Show more"
            ]
        );
        assert!(built[8].thread().is_some_and(|row| row.archived));
        prefs.thread_lifecycles = vec![ThreadLifecycle::Archived];
        let source = SidebarSource {
            archived: &archived,
            archived_status: ArchivedStatus::Loading,
            ..SidebarSource::new(&sidebar, &prefs)
        };
        assert_eq!(
            labels(&build_rows(source, SidebarView::plain(&expanded))),
            [
                "# alpha",
                "# beta",
                "Archived work",
                "# Loading archived threads…"
            ]
        );
    }

    #[test]
    fn read_status_grouping_lifts_unread_threads_and_holds_the_open_one() {
        let mut sidebar = sample();
        sidebar.projects[1]
            .threads
            .push(thread("t_late", "p2", "Late read", 90));
        let mut prefs = prefs(OrganizationMode::Project);
        let beta = |prefs: &ThreadListPreferences, held: Option<(&str, bool)>| {
            let expanded = HashSet::new();
            let view = SidebarView {
                held_read: held,
                ..SidebarView::plain(&expanded)
            };
            let all = labels(&build_rows(SidebarSource::new(&sidebar, prefs), view));
            let start = all.iter().position(|row| row == "# beta").unwrap();
            all[start + 1..].to_vec()
        };
        assert_eq!(
            beta(&prefs, None),
            ["Late read", "Fix the bug", "Orphan child"]
        );
        prefs.group_by_read_status = true;
        assert_eq!(
            beta(&prefs, None),
            ["Fix the bug", "Late read", "Orphan child"]
        );
        assert_eq!(
            beta(&prefs, Some(("t_unread", false))),
            ["Late read", "Fix the bug", "Orphan child"]
        );
        assert_eq!(
            beta(&prefs, Some(("t_late", true))),
            ["Late read", "Fix the bug", "Orphan child"]
        );
    }

    #[test]
    fn pinned_roots_follow_pin_sort_keys_by_codepoint_and_take_descendants() {
        let mut upper = thread("t_upper", "p1", "Upper", 1);
        upper.thread.pinned_at = Some(1);
        upper.pin_sort_key = Some("Z".into());
        let mut lower = thread("t_lower", "p1", "Lower", 2);
        lower.thread.pinned_at = Some(2);
        lower.pin_sort_key = Some("a".into());
        let mut child = thread("t_kid", "p1", "Kid", 3);
        child.thread.parent_thread_id = Some("t_lower".into());
        let sidebar = SidebarBootstrapResponse {
            projects: vec![project("p1", "alpha", vec![lower, upper, child])],
            ..SidebarBootstrapResponse::default()
        };
        assert_eq!(
            rows(&sidebar, &prefs(OrganizationMode::Project)),
            ["# Pinned", "Upper", "Lower", "  Kid", "# alpha"]
        );
    }

    #[test]
    fn section_order_normalization_matches_the_web_reconciliation() {
        let s = |values: &[&str]| values.iter().map(|v| (*v).to_owned()).collect::<Vec<_>>();
        let entities = s(&["project:a", "project:b"]);
        assert_eq!(
            normalize_section_order(
                &s(&["threads", "projects", "pinned"]),
                &entities,
                "projects",
                true,
                true
            ),
            s(&["threads", "project:a", "project:b", "pinned"])
        );
        assert_eq!(
            normalize_section_order(
                &s(&["project:b", "pinned", "threads", "project:a"]),
                &entities,
                "projects",
                true,
                true
            ),
            s(&["project:b", "pinned", "threads", "project:a"])
        );
        assert_eq!(
            normalize_section_order(
                &s(&["project:removed", "threads", "project:b"]),
                &s(&["project:a", "project:b", "project:c"]),
                "projects",
                true,
                true
            ),
            s(&["pinned", "threads", "project:b", "project:a", "project:c"])
        );
        assert_eq!(
            normalize_section_order(
                &s(&["project:a", "threads", "project:b"]),
                &entities,
                "projects",
                true,
                false
            ),
            s(&["pinned", "project:a", "project:b"])
        );
        assert_eq!(
            normalize_section_order(
                &s(&["pinned", "sections", "threads"]),
                &s(&["section:work"]),
                "sections",
                true,
                true
            ),
            s(&["pinned", "section:work", "threads"])
        );
    }

    #[test]
    fn cycles_are_cut_without_dropping_members() {
        let mut a = thread("t_a", "p1", "A", 1);
        a.thread.parent_thread_id = Some("t_b".into());
        let mut b = thread("t_b", "p1", "B", 2);
        b.thread.parent_thread_id = Some("t_a".into());
        let sidebar = SidebarBootstrapResponse {
            projects: vec![project("p1", "alpha", vec![a, b])],
            ..SidebarBootstrapResponse::default()
        };
        let all = rows(&sidebar, &prefs(OrganizationMode::Project));
        assert_eq!(all.len(), 3, "{all:?}");
        assert!(all.iter().any(|row| row.trim() == "A") && all.iter().any(|row| row.trim() == "B"));
    }

    #[test]
    fn attention_counts_unread_and_waiting() {
        let mut sidebar = sample();
        sidebar.projects[0].threads[0].has_pending_interaction = true;
        assert_eq!(attention_count(&sidebar), 2);
    }

    fn sourced(
        id: &str,
        name: &str,
        remote: Option<&str>,
        path: &str,
    ) -> ProjectWithThreadsResponse {
        let mut project = project(id, name, vec![]);
        project.project.git_remote_url = remote.map(Into::into);
        project.sources = vec![bb_wire::project::ProjectSource {
            path: path.into(),
            is_default: true,
            ..Default::default()
        }];
        project
    }

    #[test]
    fn duplicate_project_names_get_remote_path_or_id_qualifiers() {
        let sidebar = SidebarBootstrapResponse {
            projects: vec![
                sourced(
                    "p1",
                    "bb",
                    Some("git@github.com:get-bb/bb.git"),
                    "/home/a/work/bb",
                ),
                sourced(
                    "p2",
                    "BB",
                    Some("https://github.com/fork/bb"),
                    "/home/a/tui/bb",
                ),
                sourced("p3", "app", None, "/home/a/work/app"),
                sourced("p4", "app", None, "/srv/other/app"),
                sourced("proj_abcdef123", "web", None, "/x/web"),
                sourced("proj_zzzzzz999", "web", None, "/x/web"),
                sourced("p7", "solo", None, "/x/solo"),
            ],
            ..SidebarBootstrapResponse::default()
        };
        let labels = project_labels(&sidebar);
        assert_eq!(labels["p1"], "bb (get-bb/bb)");
        assert_eq!(labels["p2"], "BB (fork/bb)");
        assert_eq!(labels["p3"], "app (work/app)");
        assert_eq!(labels["p4"], "app (other/app)");
        assert_eq!(labels["proj_abcdef123"], "web (abcdef12)");
        assert_eq!(labels["proj_zzzzzz999"], "web (zzzzzz99)");
        assert_eq!(labels["p7"], "solo");
    }
}
