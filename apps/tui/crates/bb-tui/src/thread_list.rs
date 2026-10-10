use std::collections::{HashMap, HashSet};

use bb_store::ConnectionState;
use bb_wire::common::JsonValue;
use bb_wire::thread::{ThreadListEntry, ThreadListQuery};
use bb_wire::thread_list::{
    ChronologicalSort, EnvironmentGrouping, OrganizationMode, PreferenceKey, SortDirection,
    ThreadLifecycle, ThreadListPreferences, parse_preference_signal, parse_set_preference_reply,
};
use serde_json::json;

use crate::app::{AppState, Effect, ToastLevel};
use crate::sidebar::{ArchivedStatus, Collapse};

pub const ARCHIVED_PAGE_SIZE: u32 = 50;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PrefsStatus {
    #[default]
    Idle,
    Loading,
    Ready,
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArchivedLoad {
    #[default]
    Idle,
    Loading,
    Ready,
    Error,
}

#[derive(Debug, Clone, Default)]
pub struct ArchivedPages {
    pub entries: Vec<ThreadListEntry>,
    pub load: ArchivedLoad,
    pub has_more: bool,
    pub loading_more: bool,
    pub more_failed: bool,
    pub refreshing: bool,
    pub refresh_queued: bool,
    pub generation: u64,
    pub sidebar_seen: usize,
}

impl ArchivedPages {
    pub fn status(&self, enabled: bool) -> ArchivedStatus {
        if !enabled {
            return ArchivedStatus::Off;
        }
        match self.load {
            ArchivedLoad::Idle | ArchivedLoad::Loading => ArchivedStatus::Loading,
            ArchivedLoad::Error => ArchivedStatus::Error,
            ArchivedLoad::Ready => ArchivedStatus::Ready {
                has_more: self.has_more,
                loading_more: self.loading_more,
                more_failed: self.more_failed,
            },
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ThreadListState {
    pub prefs: ThreadListPreferences,
    pub status: PrefsStatus,
    pub generation: u64,
    pub pending: HashMap<PreferenceKey, u32>,
    pub archived: ArchivedPages,
    pub more_open: bool,
    pub expanded_while_grouped: HashSet<String>,
    pub held_read: Option<(String, bool)>,
    pub loaded_once: bool,
}

impl ThreadListState {
    pub fn awaiting_first_load(&self) -> bool {
        self.status == PrefsStatus::Loading && !self.loaded_once
    }

    pub fn is_collapsed(&self, collapse: &Collapse) -> bool {
        let prefs = &self.prefs;
        match collapse {
            Collapse::BuiltIn(id) => prefs.collapsed_sections.iter().any(|key| key == id),
            Collapse::Project(id) => prefs.collapsed_projects.contains(id),
            Collapse::ThreadSection(key) => prefs.collapsed_thread_sections.contains(key),
            Collapse::Machine(key) => prefs.collapsed_machines.contains(key),
            Collapse::Environment(id) => prefs.collapsed_environments.contains(id),
            Collapse::Thread(id) => {
                if prefs.group_by_read_status {
                    !self.expanded_while_grouped.contains(id)
                } else {
                    prefs.collapsed_threads.contains(id)
                }
            }
            Collapse::More => !self.more_open,
        }
    }

    fn collapse_list(&mut self, collapse: &Collapse) -> Option<(PreferenceKey, &mut Vec<String>)> {
        let prefs = &mut self.prefs;
        Some(match collapse {
            Collapse::BuiltIn(_) => (
                PreferenceKey::CollapsedSections,
                &mut prefs.collapsed_sections,
            ),
            Collapse::Project(_) => (
                PreferenceKey::CollapsedProjects,
                &mut prefs.collapsed_projects,
            ),
            Collapse::ThreadSection(_) => (
                PreferenceKey::CollapsedThreadSections,
                &mut prefs.collapsed_thread_sections,
            ),
            Collapse::Machine(_) => (
                PreferenceKey::CollapsedMachines,
                &mut prefs.collapsed_machines,
            ),
            Collapse::Environment(_) => (
                PreferenceKey::CollapsedEnvironments,
                &mut prefs.collapsed_environments,
            ),
            Collapse::Thread(_) if !prefs.group_by_read_status => (
                PreferenceKey::CollapsedThreads,
                &mut prefs.collapsed_threads,
            ),
            Collapse::Thread(_) | Collapse::More => return None,
        })
    }

    fn set_local(&mut self, collapse: &Collapse, collapsed: bool) -> Option<PreferenceKey> {
        match collapse {
            Collapse::More => {
                self.more_open = !collapsed;
                return None;
            }
            Collapse::Thread(id) if self.prefs.group_by_read_status => {
                if collapsed {
                    self.expanded_while_grouped.remove(id);
                } else {
                    self.expanded_while_grouped.insert(id.clone());
                }
                return None;
            }
            _ => {}
        }
        let id = match collapse {
            Collapse::BuiltIn(id) => (*id).to_owned(),
            Collapse::Project(id)
            | Collapse::ThreadSection(id)
            | Collapse::Machine(id)
            | Collapse::Environment(id)
            | Collapse::Thread(id) => id.clone(),
            Collapse::More => return None,
        };
        let (key, list) = self.collapse_list(collapse)?;
        let present = list.contains(&id);
        if present == collapsed {
            return None;
        }
        if collapsed {
            list.push(id);
        } else {
            list.retain(|known| *known != id);
        }
        Some(key)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ThreadListRequest {
    Load {
        generation: u64,
    },
    Set {
        key: PreferenceKey,
        value: JsonValue,
    },
    Archived {
        generation: u64,
        offset: u32,
        limit: u32,
        append: bool,
    },
}

impl ThreadListRequest {
    pub fn archived_query(offset: u32, limit: u32) -> ThreadListQuery {
        ThreadListQuery {
            archived: Some(true),
            limit: Some(limit),
            offset: Some(offset),
            ..ThreadListQuery::default()
        }
    }
}

#[derive(Debug, Clone)]
pub enum ThreadListLoaded {
    Preferences {
        generation: u64,
        result: Result<JsonValue, String>,
    },
    Set {
        key: PreferenceKey,
        result: Result<JsonValue, String>,
    },
    Archived {
        generation: u64,
        offset: u32,
        limit: u32,
        append: bool,
        result: Result<Vec<ThreadListEntry>, String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListOption {
    Organization,
    Sort,
    SortDirection,
    Lifecycle,
    Archived,
    EnvironmentGrouping,
    ReadGrouping,
}

impl ListOption {
    pub fn key(self) -> PreferenceKey {
        match self {
            Self::Organization => PreferenceKey::OrganizationMode,
            Self::Sort => PreferenceKey::ChronologicalSort,
            Self::SortDirection => PreferenceKey::SortDirection,
            Self::Lifecycle | Self::Archived => PreferenceKey::ThreadLifecycles,
            Self::EnvironmentGrouping => PreferenceKey::EnvironmentGrouping,
            Self::ReadGrouping => PreferenceKey::GroupByReadStatus,
        }
    }
}

pub fn option_display(prefs: &ThreadListPreferences, option: ListOption) -> String {
    match option {
        ListOption::Organization => prefs.organization_mode.as_str().to_owned(),
        ListOption::Sort => match prefs.chronological_sort {
            ChronologicalSort::None => "updated".to_owned(),
            other => other.as_str().to_owned(),
        },
        ListOption::SortDirection => match prefs.sort_direction {
            SortDirection::Default => "default",
            SortDirection::Ascending => "asc",
            SortDirection::Descending => "desc",
        }
        .to_owned(),
        ListOption::Lifecycle => lifecycle_name(prefs).to_owned(),
        ListOption::Archived => prefs.shows_archived().to_string(),
        ListOption::EnvironmentGrouping => match prefs.environment_grouping {
            EnvironmentGrouping::Auto => "auto",
            EnvironmentGrouping::On => "on",
            EnvironmentGrouping::Off => "off",
        }
        .to_owned(),
        ListOption::ReadGrouping => prefs.group_by_read_status.to_string(),
    }
}

fn lifecycle_name(prefs: &ThreadListPreferences) -> &'static str {
    match (prefs.shows_active(), prefs.shows_archived()) {
        (true, true) => "both",
        (false, true) => "archived",
        _ => "active",
    }
}

fn lifecycles(active: bool, archived: bool) -> JsonValue {
    let mut values = Vec::new();
    if active {
        values.push(ThreadLifecycle::Active.as_str());
    }
    if archived {
        values.push(ThreadLifecycle::Archived.as_str());
    }
    json!(values)
}

pub fn parse_option_value(
    prefs: &ThreadListPreferences,
    option: ListOption,
    raw: &str,
) -> Option<JsonValue> {
    let raw = raw.trim();
    Some(match option {
        ListOption::Organization => {
            let mode = match raw {
                "project" | "projects" => OrganizationMode::Project,
                "chronological" | "chrono" | "sections" => OrganizationMode::Chronological,
                "machine" | "machines" => OrganizationMode::Machine,
                _ => return None,
            };
            json!(mode.as_str())
        }
        ListOption::Sort => {
            let sort = match raw {
                "updated" | "recent" => ChronologicalSort::Updated,
                "created" => ChronologicalSort::Created,
                "alpha" | "title" => ChronologicalSort::Alpha,
                _ => return None,
            };
            json!(sort.as_str())
        }
        ListOption::SortDirection => {
            let direction = match raw {
                "default" => SortDirection::Default,
                "asc" | "ascending" => SortDirection::Ascending,
                "desc" | "descending" => SortDirection::Descending,
                _ => return None,
            };
            json!(direction.as_str())
        }
        ListOption::Lifecycle => match raw {
            "active" => lifecycles(true, false),
            "archived" => lifecycles(false, true),
            "both" | "all" => lifecycles(true, true),
            _ => return None,
        },
        ListOption::Archived => match raw {
            "true" | "on" => lifecycles(true, true),
            "false" | "off" => lifecycles(true, false),
            _ => return None,
        },
        ListOption::EnvironmentGrouping => match raw {
            "auto" => json!("auto"),
            "on" | "true" => json!(true),
            "off" | "false" => json!(false),
            _ => return None,
        },
        ListOption::ReadGrouping => match raw {
            "true" | "on" => json!(true),
            "false" | "off" => json!(false),
            _ => return None,
        },
    })
    .filter(|value| {
        let mut copy = prefs.clone();
        copy.apply(option.key(), value)
    })
}

pub fn next_option_value(prefs: &ThreadListPreferences, option: ListOption) -> JsonValue {
    match option {
        ListOption::Organization => {
            let next = match prefs.organization_mode {
                OrganizationMode::Project => OrganizationMode::Chronological,
                OrganizationMode::Chronological => OrganizationMode::Machine,
                OrganizationMode::Machine => OrganizationMode::Project,
            };
            json!(next.as_str())
        }
        ListOption::Sort => {
            let next = match prefs.chronological_sort {
                ChronologicalSort::Updated | ChronologicalSort::None => ChronologicalSort::Created,
                ChronologicalSort::Created => ChronologicalSort::Alpha,
                ChronologicalSort::Alpha => ChronologicalSort::Updated,
            };
            json!(next.as_str())
        }
        ListOption::SortDirection => {
            let next = match prefs.sort_direction {
                SortDirection::Default => SortDirection::Ascending,
                SortDirection::Ascending => SortDirection::Descending,
                SortDirection::Descending => SortDirection::Default,
            };
            json!(next.as_str())
        }
        ListOption::Lifecycle => match lifecycle_name(prefs) {
            "active" => lifecycles(true, true),
            "both" => lifecycles(false, true),
            _ => lifecycles(true, false),
        },
        ListOption::Archived => lifecycles(true, !prefs.shows_archived()),
        ListOption::EnvironmentGrouping => match prefs.environment_grouping {
            EnvironmentGrouping::Auto => json!(true),
            EnvironmentGrouping::On => json!(false),
            EnvironmentGrouping::Off => json!("auto"),
        },
        ListOption::ReadGrouping => json!(!prefs.group_by_read_status),
    }
}

fn sidebar_id(state: &AppState) -> usize {
    state
        .snapshot
        .sidebar
        .data
        .as_ref()
        .map_or(0, |sidebar| std::sync::Arc::as_ptr(sidebar) as usize)
}

impl AppState {
    pub fn thread_list_start(&mut self) -> Vec<Effect> {
        self.thread_list.generation += 1;
        if self.thread_list.status != PrefsStatus::Ready {
            self.thread_list.status = PrefsStatus::Loading;
        }
        vec![Effect::ThreadList(ThreadListRequest::Load {
            generation: self.thread_list.generation,
        })]
    }

    pub fn thread_list_loaded(&mut self, loaded: ThreadListLoaded) -> Vec<Effect> {
        match loaded {
            ThreadListLoaded::Preferences { generation, result } => {
                if generation != self.thread_list.generation {
                    return Vec::new();
                }
                match result {
                    Ok(value) => {
                        let fresh = ThreadListPreferences::from_list_response(&value);
                        let current = self.thread_list.prefs.clone();
                        let mut next = fresh;
                        for key in self.thread_list.pending.keys() {
                            next.apply(*key, &current.value(*key));
                        }
                        self.thread_list.prefs = next;
                        self.thread_list.status = PrefsStatus::Ready;
                    }
                    Err(error) => {
                        tracing::debug!(%error, "thread list preferences unavailable");
                        self.thread_list.status = PrefsStatus::Failed(error);
                    }
                }
                let first = !self.thread_list.loaded_once;
                self.thread_list.loaded_once = true;
                let effects = self.prefs_changed();
                if first && self.selected_thread_id().is_none() {
                    self.select_first_thread();
                }
                effects
            }
            ThreadListLoaded::Set { key, result } => {
                let remaining = match self.thread_list.pending.get_mut(&key) {
                    Some(count) => {
                        *count = count.saturating_sub(1);
                        *count
                    }
                    None => 0,
                };
                if remaining == 0 {
                    self.thread_list.pending.remove(&key);
                }
                match result {
                    Ok(reply) => {
                        if remaining == 0
                            && let Some((reply_key, value)) = parse_set_preference_reply(&reply)
                            && reply_key == key
                            && self.thread_list.prefs.value(key) != value
                            && self.thread_list.prefs.apply(key, &value)
                        {
                            return self.prefs_changed();
                        }
                        Vec::new()
                    }
                    Err(error) => {
                        self.toast(
                            format!("Failed to save thread list preference {key}: {error}"),
                            ToastLevel::Error,
                        );
                        self.thread_list_start()
                    }
                }
            }
            ThreadListLoaded::Archived {
                generation,
                offset,
                limit,
                append,
                result,
            } => self.archived_loaded(generation, offset, limit, append, result),
        }
    }

    pub fn thread_list_signal(&mut self, payload: &JsonValue) -> Vec<Effect> {
        let Some((key, value)) = parse_preference_signal(payload) else {
            return Vec::new();
        };
        if self.thread_list.pending.contains_key(&key) {
            return Vec::new();
        }
        if self.thread_list.prefs.value(key) == value || !self.thread_list.prefs.apply(key, &value)
        {
            return Vec::new();
        }
        self.prefs_changed()
    }

    pub fn thread_list_connection(&mut self, previous: &ConnectionState) -> Vec<Effect> {
        let was_down = matches!(
            previous,
            ConnectionState::Disconnected { .. } | ConnectionState::Connecting { reconnect: true }
        );
        if was_down
            && self.snapshot.connection == ConnectionState::Connected
            && self.thread_list.status != PrefsStatus::Idle
        {
            return self.thread_list_start();
        }
        Vec::new()
    }

    fn prefs_changed(&mut self) -> Vec<Effect> {
        self.rebuild_rows();
        self.sync_archived(true)
    }

    pub fn set_thread_list_preference(
        &mut self,
        key: PreferenceKey,
        value: JsonValue,
    ) -> Vec<Effect> {
        if self.thread_list.prefs.value(key) == value {
            return Vec::new();
        }
        if !self.thread_list.prefs.apply(key, &value) {
            return Vec::new();
        }
        let mut effects = self.write_preference(key);
        effects.extend(self.prefs_changed());
        effects
    }

    fn write_preference(&mut self, key: PreferenceKey) -> Vec<Effect> {
        if self.read_only {
            return Vec::new();
        }
        *self.thread_list.pending.entry(key).or_insert(0) += 1;
        vec![Effect::ThreadList(ThreadListRequest::Set {
            key,
            value: self.thread_list.prefs.value(key),
        })]
    }

    pub fn set_collapsed(&mut self, collapse: &Collapse, collapsed: bool) -> Vec<Effect> {
        let key = self.thread_list.set_local(collapse, collapsed);
        self.rebuild_rows();
        match key {
            Some(key) => self.write_preference(key),
            None => Vec::new(),
        }
    }

    pub fn set_many_collapsed(&mut self, targets: &[Collapse], collapsed: bool) -> Vec<Effect> {
        let mut keys: Vec<PreferenceKey> = Vec::new();
        for target in targets {
            if let Some(key) = self.thread_list.set_local(target, collapsed)
                && !keys.contains(&key)
            {
                keys.push(key);
            }
        }
        self.rebuild_rows();
        keys.into_iter()
            .flat_map(|key| self.write_preference(key))
            .collect()
    }

    pub fn sync_archived(&mut self, prefs_changed: bool) -> Vec<Effect> {
        let enabled = self.thread_list.prefs.shows_archived();
        let archived = &mut self.thread_list.archived;
        if !enabled {
            if archived.load != ArchivedLoad::Idle || !archived.entries.is_empty() {
                *archived = ArchivedPages {
                    generation: archived.generation + 1,
                    ..ArchivedPages::default()
                };
                if prefs_changed {
                    self.rebuild_rows();
                }
            }
            return Vec::new();
        }
        let sidebar = sidebar_id(self);
        let archived = &mut self.thread_list.archived;
        if archived.load == ArchivedLoad::Idle {
            archived.load = ArchivedLoad::Loading;
            archived.generation += 1;
            archived.sidebar_seen = sidebar;
            archived.refreshing = true;
            let effects = vec![Effect::ThreadList(ThreadListRequest::Archived {
                generation: archived.generation,
                offset: 0,
                limit: ARCHIVED_PAGE_SIZE,
                append: false,
            })];
            self.rebuild_rows();
            return effects;
        }
        if sidebar == archived.sidebar_seen || sidebar == 0 {
            return Vec::new();
        }
        archived.sidebar_seen = sidebar;
        if archived.refreshing {
            archived.refresh_queued = true;
            return Vec::new();
        }
        self.refresh_archived()
    }

    fn refresh_archived(&mut self) -> Vec<Effect> {
        let archived = &mut self.thread_list.archived;
        archived.generation += 1;
        archived.refreshing = true;
        archived.refresh_queued = false;
        archived.loading_more = false;
        let pages = (archived.entries.len() as u32)
            .div_ceil(ARCHIVED_PAGE_SIZE)
            .max(1);
        vec![Effect::ThreadList(ThreadListRequest::Archived {
            generation: archived.generation,
            offset: 0,
            limit: pages * ARCHIVED_PAGE_SIZE,
            append: false,
        })]
    }

    pub fn load_more_sidebar_archived(&mut self) -> Vec<Effect> {
        let archived = &mut self.thread_list.archived;
        if archived.load != ArchivedLoad::Ready
            || !archived.has_more
            || archived.loading_more
            || archived.refreshing
        {
            return Vec::new();
        }
        archived.loading_more = true;
        archived.more_failed = false;
        let effects = vec![Effect::ThreadList(ThreadListRequest::Archived {
            generation: archived.generation,
            offset: archived.entries.len() as u32,
            limit: ARCHIVED_PAGE_SIZE,
            append: true,
        })];
        self.rebuild_rows();
        effects
    }

    fn archived_loaded(
        &mut self,
        generation: u64,
        offset: u32,
        limit: u32,
        append: bool,
        result: Result<Vec<ThreadListEntry>, String>,
    ) -> Vec<Effect> {
        let archived = &mut self.thread_list.archived;
        if generation != archived.generation {
            return Vec::new();
        }
        if append {
            archived.loading_more = false;
            match result {
                Ok(entries) if offset as usize == archived.entries.len() => {
                    archived.has_more = entries.len() as u32 >= limit;
                    archived.entries.extend(entries);
                }
                Ok(_) => {}
                Err(_) => archived.more_failed = true,
            }
        } else {
            archived.refreshing = false;
            match result {
                Ok(entries) => {
                    archived.has_more = entries.len() as u32 >= limit;
                    archived.entries = entries;
                    archived.load = ArchivedLoad::Ready;
                }
                Err(error) => {
                    tracing::debug!(%error, "archived threads unavailable");
                    if archived.load != ArchivedLoad::Ready {
                        archived.load = ArchivedLoad::Error;
                    }
                }
            }
        }
        let queued = archived.refresh_queued;
        self.rebuild_rows();
        if queued {
            return self.refresh_archived();
        }
        Vec::new()
    }

    pub fn archived_entry(&self, thread_id: &str) -> Option<&ThreadListEntry> {
        self.thread_list
            .archived
            .entries
            .iter()
            .find(|entry| entry.thread.id == thread_id)
    }

    pub fn list_option_set(
        &mut self,
        option: ListOption,
        raw: Option<&str>,
    ) -> Result<Vec<Effect>, String> {
        if self.read_only {
            return Err("Read-only mode: cannot change thread list preferences".into());
        }
        let value = match raw {
            Some(raw) => parse_option_value(&self.thread_list.prefs, option, raw)
                .ok_or_else(|| format!("E474: Invalid argument: {raw}"))?,
            None => next_option_value(&self.thread_list.prefs, option),
        };
        let effects = self.set_thread_list_preference(option.key(), value);
        self.keys.message(format!(
            "  {}",
            option_display(&self.thread_list.prefs, option)
        ));
        Ok(effects)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn option_values_parse_and_cycle_like_the_web_menu() {
        let prefs = ThreadListPreferences::default();
        assert_eq!(
            parse_option_value(&prefs, ListOption::Organization, "machine"),
            Some(json!("machine"))
        );
        assert_eq!(
            parse_option_value(&prefs, ListOption::Organization, "tree"),
            None
        );
        assert_eq!(
            parse_option_value(&prefs, ListOption::SortDirection, "asc"),
            Some(json!("ascending"))
        );
        assert_eq!(
            parse_option_value(&prefs, ListOption::Lifecycle, "both"),
            Some(json!(["active", "archived"]))
        );
        assert_eq!(
            parse_option_value(&prefs, ListOption::Archived, "off"),
            Some(json!(["active"]))
        );
        assert_eq!(
            next_option_value(&prefs, ListOption::Organization),
            json!("machine")
        );
        assert_eq!(
            next_option_value(&prefs, ListOption::Archived),
            json!(["active", "archived"])
        );
        assert_eq!(
            next_option_value(&prefs, ListOption::Sort),
            json!("created")
        );
        assert_eq!(option_display(&prefs, ListOption::Lifecycle), "active");
    }

    #[test]
    fn collapse_toggles_update_the_matching_preference_list() {
        let mut state = ThreadListState::default();
        assert_eq!(
            state.set_local(&Collapse::Project("p1".into()), true),
            Some(PreferenceKey::CollapsedProjects)
        );
        assert_eq!(state.set_local(&Collapse::Project("p1".into()), true), None);
        assert_eq!(
            state.set_local(&Collapse::BuiltIn("pinned"), true),
            Some(PreferenceKey::CollapsedSections)
        );
        assert_eq!(state.set_local(&Collapse::More, false), None);
        assert!(state.more_open);
        assert!(state.is_collapsed(&Collapse::Project("p1".into())));
        assert_eq!(state.prefs.collapsed_sections, ["pinned"]);
        state.prefs.group_by_read_status = true;
        assert!(state.is_collapsed(&Collapse::Thread("t1".into())));
        assert_eq!(state.set_local(&Collapse::Thread("t1".into()), false), None);
        assert!(!state.is_collapsed(&Collapse::Thread("t1".into())));
        assert!(state.prefs.collapsed_threads.is_empty());
    }
}
