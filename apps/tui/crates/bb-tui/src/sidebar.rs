use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use bb_wire::project::{ProjectKind, ProjectWithThreadsResponse, SidebarBootstrapResponse};
use bb_wire::thread::{ThreadListEntry, ThreadStatus, ThreadVisibility};
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use crate::indicator::{Indicator, is_unread_done};

pub const PINNED_GROUP: &str = "@pinned";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RowKey {
    Group(String),
    Thread(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupRow {
    pub key: String,
    pub label: String,
    pub thread_count: usize,
    pub collapsed: bool,
    pub attention: usize,
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
    pub attention_at: i64,
    pub child_count: usize,
    pub expanded: bool,
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
}

#[derive(Debug, Clone, Copy)]
pub struct SidebarView<'a> {
    pub collapsed: &'a HashSet<String>,
    pub expanded_threads: &'a HashSet<String>,
    pub filter: &'a str,
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

fn compare_created_desc(left: &ThreadListEntry, right: &ThreadListEntry) -> Ordering {
    right
        .thread
        .created_at
        .cmp(&left.thread.created_at)
        .then_with(|| left.thread.id.cmp(&right.thread.id))
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
        let by_key = a.cmp(b);
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

struct Tree<'a> {
    children: HashMap<&'a str, Vec<&'a ThreadListEntry>>,
    visible: HashSet<&'a str>,
}

fn is_listed(entry: &ThreadListEntry) -> bool {
    entry.thread.visibility != ThreadVisibility::Hidden
        && entry.thread.deleted_at.is_none()
        && entry.thread.archived_at.is_none()
}

impl<'a> Tree<'a> {
    fn mark_visible(
        &mut self,
        entry: &'a ThreadListEntry,
        filter: &mut FuzzyFilter,
        stack: &mut Vec<&'a str>,
    ) -> bool {
        let id = entry.thread.id.as_str();
        if stack.contains(&id) {
            return false;
        }
        stack.push(id);
        let mut visible = filter.matches(&thread_title(entry));
        let children = self.children.get(id).cloned().unwrap_or_default();
        for child in children {
            visible |= self.mark_visible(child, filter, stack);
        }
        stack.pop();
        if visible {
            self.visible.insert(id);
        }
        visible
    }

    fn attention(&self, entry: &ThreadListEntry, depth: usize) -> usize {
        let own = usize::from(Indicator::for_entry(entry).needs_attention());
        if depth > 32 {
            return own;
        }
        own + self
            .children
            .get(entry.thread.id.as_str())
            .map_or(0, |children| {
                children
                    .iter()
                    .map(|child| self.attention(child, depth + 1))
                    .sum()
            })
    }
}

struct Emitter<'v, 'a> {
    view: SidebarView<'v>,
    filtering: bool,
    tree: Tree<'a>,
    rows: Vec<SidebarRow>,
}

impl<'a> Emitter<'_, 'a> {
    fn emit_thread(&mut self, entry: &'a ThreadListEntry, depth: usize, pinned: bool) {
        let id = entry.thread.id.as_str();
        if !self.tree.visible.contains(id) || depth > 32 {
            return;
        }
        let mut children: Vec<&ThreadListEntry> = self
            .tree
            .children
            .get(id)
            .map(|children| {
                children
                    .iter()
                    .copied()
                    .filter(|child| self.tree.visible.contains(child.thread.id.as_str()))
                    .collect()
            })
            .unwrap_or_default();
        children.sort_by(|a, b| compare_standard(a, b));
        let expanded = self.filtering || self.view.expanded_threads.contains(id);
        self.rows.push(SidebarRow::Thread(ThreadRow {
            id: entry.thread.id.clone(),
            project_id: entry.thread.project_id.clone(),
            title: thread_title(entry),
            indicator: Indicator::for_entry(entry),
            unread: is_unread_done(&entry.thread),
            depth,
            pinned,
            attention_at: entry.thread.latest_attention_at,
            child_count: children.len(),
            expanded,
        }));
        if expanded {
            for child in children {
                self.emit_thread(child, depth + 1, pinned);
            }
        }
    }

    fn emit_group(
        &mut self,
        key: String,
        label: String,
        roots: Vec<&'a ThreadListEntry>,
        pinned: bool,
    ) {
        let roots: Vec<&ThreadListEntry> = roots
            .into_iter()
            .filter(|entry| self.tree.visible.contains(entry.thread.id.as_str()))
            .collect();
        if self.filtering && roots.is_empty() {
            return;
        }
        let collapsed = !self.filtering && self.view.collapsed.contains(&key);
        let attention = roots
            .iter()
            .map(|entry| self.tree.attention(entry, 0))
            .sum();
        self.rows.push(SidebarRow::Group(GroupRow {
            key,
            label,
            thread_count: roots.len(),
            collapsed,
            attention,
        }));
        if !collapsed {
            for entry in roots {
                self.emit_thread(entry, 0, pinned);
            }
        }
    }
}

pub fn build_rows(sidebar: &SidebarBootstrapResponse, view: SidebarView<'_>) -> Vec<SidebarRow> {
    let projects: Vec<&ProjectWithThreadsResponse> = sidebar
        .projects
        .iter()
        .chain(std::iter::once(&sidebar.personal_project))
        .filter(|project| !project.project.id.is_empty() || !project.threads.is_empty())
        .collect();
    let listed: Vec<&ThreadListEntry> = projects
        .iter()
        .flat_map(|project| project.threads.iter())
        .filter(|entry| is_listed(entry))
        .collect();
    let ids: HashSet<&str> = listed
        .iter()
        .map(|entry| entry.thread.id.as_str())
        .collect();
    let mut children: HashMap<&str, Vec<&ThreadListEntry>> = HashMap::new();
    let mut is_root = HashMap::new();
    for entry in &listed {
        let parent = entry
            .thread
            .parent_thread_id
            .as_deref()
            .filter(|parent| ids.contains(parent) && *parent != entry.thread.id);
        is_root.insert(entry.thread.id.as_str(), parent.is_none());
        if let Some(parent) = parent {
            children.entry(parent).or_default().push(entry);
        }
    }
    let mut filter = FuzzyFilter::new(view.filter);
    let mut emitter = Emitter {
        view,
        filtering: filter.active(),
        tree: Tree {
            children,
            visible: HashSet::new(),
        },
        rows: Vec::new(),
    };
    let roots: Vec<&ThreadListEntry> = listed
        .iter()
        .copied()
        .filter(|entry| is_root.get(entry.thread.id.as_str()) == Some(&true))
        .collect();
    let mut stack = Vec::new();
    for root in &roots {
        emitter.tree.mark_visible(root, &mut filter, &mut stack);
    }

    let mut pinned: Vec<&ThreadListEntry> = roots
        .iter()
        .copied()
        .filter(|entry| entry.thread.is_pinned())
        .collect();
    pinned.sort_by(|a, b| compare_pinned(a, b));
    if !pinned.is_empty() {
        emitter.emit_group(PINNED_GROUP.into(), "Pinned".into(), pinned, true);
    }
    for project in projects {
        let mut project_roots: Vec<&ThreadListEntry> = roots
            .iter()
            .copied()
            .filter(|entry| {
                entry.thread.project_id == project.project.id && !entry.thread.is_pinned()
            })
            .collect();
        project_roots.sort_by(|a, b| compare_standard(a, b));
        emitter.emit_group(
            project.project.id.clone(),
            project_label(project),
            project_roots,
            false,
        );
    }
    emitter.rows
}

pub fn attention_count(sidebar: &SidebarBootstrapResponse) -> usize {
    sidebar
        .projects
        .iter()
        .chain(std::iter::once(&sidebar.personal_project))
        .flat_map(|project| project.threads.iter())
        .filter(|entry| is_listed(entry) && Indicator::for_entry(entry).needs_attention())
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

    fn labels(rows: &[SidebarRow]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                SidebarRow::Group(group) => format!("# {}", group.label),
                SidebarRow::Thread(thread) => {
                    format!("{}{}", "  ".repeat(thread.depth), thread.title)
                }
            })
            .collect()
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
    fn pinned_first_active_before_recent_children_collapsed_hidden_skipped() {
        let collapsed = HashSet::new();
        let expanded = HashSet::new();
        let rows = build_rows(
            &sample(),
            SidebarView {
                collapsed: &collapsed,
                expanded_threads: &expanded,
                filter: "",
            },
        );
        assert_eq!(
            labels(&rows),
            vec![
                "# Pinned",
                "Pinned one",
                "# alpha",
                "Running",
                "Recent",
                "# beta",
                "Fix the bug",
                "Orphan child",
                "# Personal",
            ]
        );
        let recent = rows.iter().find_map(SidebarRow::thread).map(|_| ());
        assert!(recent.is_some());
        let SidebarRow::Group(beta) = &rows[5] else {
            panic!("group")
        };
        assert_eq!(beta.attention, 1);
        assert_eq!(beta.thread_count, 2);
    }

    #[test]
    fn expanding_threads_and_collapsing_groups() {
        let collapsed: HashSet<String> = ["p2".to_string()].into();
        let expanded: HashSet<String> = ["t_recent".to_string()].into();
        let rows = build_rows(
            &sample(),
            SidebarView {
                collapsed: &collapsed,
                expanded_threads: &expanded,
                filter: "",
            },
        );
        assert_eq!(
            labels(&rows),
            vec![
                "# Pinned",
                "Pinned one",
                "# alpha",
                "Running",
                "Recent",
                "  Child agent",
                "# beta",
                "# Personal",
            ]
        );
    }

    #[test]
    fn filter_keeps_ancestors_and_drops_empty_groups() {
        let collapsed: HashSet<String> = ["p1".to_string()].into();
        let expanded = HashSet::new();
        let rows = build_rows(
            &sample(),
            SidebarView {
                collapsed: &collapsed,
                expanded_threads: &expanded,
                filter: "chld",
            },
        );
        assert_eq!(
            labels(&rows),
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
    fn attention_counts_unread_and_waiting() {
        let mut sidebar = sample();
        sidebar.projects[0].threads[0].has_pending_interaction = true;
        assert_eq!(attention_count(&sidebar), 2);
    }
}
