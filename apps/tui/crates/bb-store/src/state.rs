use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use bb_wire::environment::Host;
use bb_wire::interaction::PendingInteraction;
use bb_wire::project::{ProjectWithThreadsResponse, SidebarBootstrapResponse};
use bb_wire::queue::ThreadQueuedMessage;
use bb_wire::realtime::{SystemChangeKind, ThreadStatusChange};
use bb_wire::system::SystemAttentionResponse;
use bb_wire::thread::{ThreadListEntry, ThreadResponse};
use bb_wire::timeline::{ThreadTimelineResponse, TimelineRow};
use tokio::time::Instant;

use crate::backend::FetchError;
use crate::deferred::{TurnDetailsKey, resolve_deferred_timeline_content};
use crate::timeline_merge::{LatestTimeline, LoadedTimelineState, RowRef};

#[derive(Debug, Clone, PartialEq)]
pub struct Resource<T> {
    pub data: Option<T>,
    pub stale: bool,
    pub fetching: bool,
    pub error: Option<FetchError>,
    pub updated_at: Option<Instant>,
}

impl<T> Default for Resource<T> {
    fn default() -> Self {
        Self {
            data: None,
            stale: false,
            fetching: false,
            error: None,
            updated_at: None,
        }
    }
}

impl<T> Resource<T> {
    pub fn is_loading(&self) -> bool {
        self.data.is_none() && self.error.is_none()
    }

    pub fn needs_fetch(&self) -> bool {
        self.data.is_none() || self.stale || self.error.is_some()
    }

    pub(crate) fn begin_fetch(&mut self) {
        self.fetching = true;
    }

    pub(crate) fn finish(&mut self, result: Result<T, FetchError>, now: Instant) {
        self.fetching = false;
        match result {
            Ok(data) => {
                self.data = Some(data);
                self.stale = false;
                self.error = None;
                self.updated_at = Some(now);
            }
            Err(error) => self.error = Some(error),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ConnectionState {
    #[default]
    Idle,
    Connecting {
        reconnect: bool,
    },
    Connected,
    Disconnected {
        retry_in: Option<Duration>,
        error: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TimelineEntry {
    pub latest: Option<Arc<LatestTimeline>>,
    pub loaded: LoadedTimelineState,
    pub stale: bool,
    pub fetching: bool,
    pub error: Option<FetchError>,
    pub loading_older: bool,
    pub older_error: Option<FetchError>,
    pub unseen_sequence: Option<i64>,
    pub updated_at: Option<Instant>,
}

impl TimelineEntry {
    pub fn head(&self) -> Option<&ThreadTimelineResponse> {
        self.latest.as_deref().map(|latest| &latest.response)
    }

    pub fn rows(&self) -> &[RowRef] {
        if !self.loaded.rows.is_empty() {
            return &self.loaded.rows;
        }
        self.latest
            .as_deref()
            .map_or(&[], |latest| latest.rows.as_slice())
    }

    pub fn has_older_rows(&self) -> bool {
        self.loaded.older_cursor.is_some()
    }

    pub fn max_seq(&self) -> Option<i64> {
        self.latest.as_deref().map(LatestTimeline::max_seq)
    }

    pub fn is_catching_up(&self) -> bool {
        self.unseen_sequence
            .is_some_and(|sequence| sequence > self.max_seq().unwrap_or(0))
            && !self.rows().is_empty()
    }

    pub fn is_loading(&self) -> bool {
        self.rows().is_empty() && self.latest.is_none() && self.error.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TurnDetailsEntry {
    pub rows: Arc<Vec<TimelineRow>>,
    pub older_cursor: Option<String>,
    pub history_snapshot: Option<String>,
    pub loaded: bool,
    pub stale: bool,
    pub fetching: bool,
    pub loading_older: bool,
    pub error: Option<FetchError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Generations {
    pub system: BTreeMap<SystemChangeKind, u64>,
    pub environments: BTreeMap<String, u64>,
    pub hosts: u64,
    pub thread_terminals: BTreeMap<Option<String>, u64>,
    pub thread_tabs: BTreeMap<Option<String>, u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DeferredContent<'a> {
    NotDeferred,
    Pending,
    Loaded(TimelineRow),
    Missing,
    Failed(&'a FetchError),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct StoreSnapshot {
    pub version: u64,
    pub connection: ConnectionState,
    pub sidebar: Resource<Arc<SidebarBootstrapResponse>>,
    pub hosts: Resource<Arc<Vec<Host>>>,
    pub attention: Resource<SystemAttentionResponse>,
    pub threads: BTreeMap<String, Resource<Arc<ThreadResponse>>>,
    pub timelines: BTreeMap<String, Arc<TimelineEntry>>,
    pub interactions: BTreeMap<String, Resource<Arc<Vec<PendingInteraction>>>>,
    pub queued_messages: BTreeMap<String, Resource<Arc<Vec<ThreadQueuedMessage>>>>,
    pub turn_details: BTreeMap<TurnDetailsKey, Arc<TurnDetailsEntry>>,
    pub generations: Arc<Generations>,
}

fn sidebar_projects(
    sidebar: &SidebarBootstrapResponse,
) -> impl Iterator<Item = &ProjectWithThreadsResponse> {
    sidebar
        .projects
        .iter()
        .chain(std::iter::once(&sidebar.personal_project))
}

fn sidebar_projects_mut(
    sidebar: &mut SidebarBootstrapResponse,
) -> impl Iterator<Item = &mut ProjectWithThreadsResponse> {
    sidebar
        .projects
        .iter_mut()
        .chain(std::iter::once(&mut sidebar.personal_project))
}

impl StoreSnapshot {
    pub fn sidebar_threads(&self) -> impl Iterator<Item = &ThreadListEntry> {
        self.sidebar
            .data
            .as_deref()
            .into_iter()
            .flat_map(sidebar_projects)
            .flat_map(|project| project.threads.iter())
    }

    pub fn sidebar_thread(&self, thread_id: &str) -> Option<&ThreadListEntry> {
        self.sidebar_threads()
            .find(|entry| entry.thread.id == thread_id)
    }

    pub fn thread(&self, thread_id: &str) -> Option<&ThreadResponse> {
        self.threads.get(thread_id)?.data.as_deref()
    }

    pub fn timeline(&self, thread_id: &str) -> Option<&TimelineEntry> {
        self.timelines.get(thread_id).map(Arc::as_ref)
    }

    pub fn pending_interactions(&self, thread_id: &str) -> &[PendingInteraction] {
        self.interactions
            .get(thread_id)
            .and_then(|resource| resource.data.as_deref())
            .map_or(&[], Vec::as_slice)
    }

    pub fn queued(&self, thread_id: &str) -> &[ThreadQueuedMessage] {
        self.queued_messages
            .get(thread_id)
            .and_then(|resource| resource.data.as_deref())
            .map_or(&[], Vec::as_slice)
    }

    pub fn resolve_deferred_row(&self, thread_id: &str, row: &TimelineRow) -> DeferredContent<'_> {
        let Some(key) = TurnDetailsKey::for_deferred_row(thread_id, row) else {
            return DeferredContent::NotDeferred;
        };
        let Some(entry) = self.turn_details.get(&key) else {
            return DeferredContent::Pending;
        };
        if !entry.loaded {
            return match &entry.error {
                Some(error) => DeferredContent::Failed(error),
                None => DeferredContent::Pending,
            };
        }
        match resolve_deferred_timeline_content(row, &entry.rows) {
            Some(resolved) => DeferredContent::Loaded(resolved),
            None => DeferredContent::Missing,
        }
    }

    pub(crate) fn thread_ids_in_environment(&self, environment_id: &str) -> Vec<String> {
        let mut ids: Vec<String> = self
            .threads
            .iter()
            .filter(|(_, resource)| {
                resource.data.as_deref().is_some_and(|thread| {
                    thread.thread.environment_id.as_deref() == Some(environment_id)
                })
            })
            .map(|(id, _)| id.clone())
            .collect();
        for entry in self.sidebar_threads() {
            if entry.thread.environment_id.as_deref() == Some(environment_id)
                && !ids.contains(&entry.thread.id)
            {
                ids.push(entry.thread.id.clone());
            }
        }
        ids
    }

    pub(crate) fn patch_sidebar_thread(
        &mut self,
        thread_id: &str,
        patch: impl Fn(&mut ThreadListEntry),
    ) -> bool {
        let Some(sidebar) = self.sidebar.data.as_mut() else {
            return false;
        };
        let contains = sidebar_projects(sidebar).any(|project| {
            project
                .threads
                .iter()
                .any(|entry| entry.thread.id == thread_id)
        });
        if !contains {
            return false;
        }
        for project in sidebar_projects_mut(Arc::make_mut(sidebar)) {
            for entry in &mut project.threads {
                if entry.thread.id == thread_id {
                    patch(entry);
                }
            }
        }
        true
    }

    pub(crate) fn apply_status_change(
        &mut self,
        thread_id: &str,
        change: &ThreadStatusChange,
    ) -> bool {
        let mut changed = self.patch_sidebar_thread(thread_id, |entry| {
            entry.thread.status = change.status;
            entry.runtime = change.runtime.clone();
            entry.activity = change.activity.clone();
            entry.thread.latest_attention_at = change.latest_attention_at;
            entry.thread.updated_at = change.updated_at;
        });
        if let Some(thread) = self
            .threads
            .get_mut(thread_id)
            .and_then(|resource| resource.data.as_mut())
        {
            let thread = Arc::make_mut(thread);
            thread.thread.status = change.status;
            thread.runtime = change.runtime.clone();
            thread.active_background_agent_count = change.activity.active_background_agent_count;
            thread.thread.latest_attention_at = change.latest_attention_at;
            thread.thread.updated_at = change.updated_at;
            changed = true;
        }
        changed
    }
}
