use std::collections::{BTreeMap, BTreeSet};

use bb_wire::realtime::{
    ChangedMessage, EnvironmentChangeKind, HostChangeKind, ProjectChangeKind, SystemChangeKind,
    ThreadChangeKind, ThreadChangeMetadata, ThreadStatusChange,
};

pub const TURN_COMPLETED_EVENT: &str = "turn/completed";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlushPriority {
    Debounced,
    Immediate,
}

pub fn thread_change_flush_priority(kind: ThreadChangeKind) -> FlushPriority {
    match kind {
        ThreadChangeKind::HistoryRewritten
        | ThreadChangeKind::StatusChanged
        | ThreadChangeKind::EnvironmentChanged
        | ThreadChangeKind::TabsChanged => FlushPriority::Immediate,
        _ => FlushPriority::Debounced,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Target {
    Sidebar,
    Hosts,
    Attention,
    Thread(Option<String>),
    Timeline(Option<String>),
    Interactions(Option<String>),
    QueuedMessages(Option<String>),
    TurnDetails(Option<String>),
    ThreadsInEnvironment(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TimelineRefresh {
    Delta,
    Terminal,
    Full,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Generation {
    System(SystemChangeKind),
    Environment(String),
    Hosts,
    ThreadTerminals(Option<String>),
    ThreadTabs(Option<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Refetch(Target),
    MarkStale(Target),
    RefreshTimeline {
        thread_id: Option<String>,
        mode: TimelineRefresh,
    },
    PatchStatus {
        thread_id: String,
        change: ThreadStatusChange,
    },
    PatchPendingInteraction {
        thread_id: String,
        has_pending_interaction: bool,
    },
    NoteTimelineSequence {
        thread_id: String,
        sequence: i64,
    },
    Bump(Generation),
}

#[derive(Debug, Default)]
struct Effects(Vec<Effect>);

impl Effects {
    fn push(&mut self, effect: Effect) {
        if !self.0.contains(&effect) {
            self.0.push(effect);
        }
    }
}

fn merge_event_types(
    current: Option<&Vec<String>>,
    next: Option<&Vec<String>>,
) -> Option<Vec<String>> {
    match (current, next) {
        (None, next) => next.cloned(),
        (current, None) => current.cloned(),
        (Some(current), Some(next)) => {
            let mut merged = current.clone();
            for event in next {
                if !merged.contains(event) {
                    merged.push(event.clone());
                }
            }
            Some(merged)
        }
    }
}

pub fn merge_thread_change_metadata(
    current: Option<&ThreadChangeMetadata>,
    next: &ThreadChangeMetadata,
    status_changed: bool,
) -> ThreadChangeMetadata {
    let timeline_sequence = match (
        current.and_then(|current| current.timeline_sequence),
        next.timeline_sequence,
    ) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (left, right) => left.or(right),
    };
    ThreadChangeMetadata {
        background_activity_changed: next
            .background_activity_changed
            .or(current.and_then(|current| current.background_activity_changed)),
        event_types: merge_event_types(
            current.and_then(|current| current.event_types.as_ref()),
            next.event_types.as_ref(),
        ),
        timeline_sequence,
        has_pending_interaction: next
            .has_pending_interaction
            .or(current.and_then(|current| current.has_pending_interaction)),
        project_id: next
            .project_id
            .clone()
            .or_else(|| current.and_then(|current| current.project_id.clone())),
        status_change: if status_changed {
            next.status_change.clone()
        } else {
            next.status_change
                .clone()
                .or_else(|| current.and_then(|current| current.status_change.clone()))
        },
    }
}

pub fn has_turn_completed(metadata: Option<&ThreadChangeMetadata>) -> bool {
    metadata
        .and_then(|metadata| metadata.event_types.as_ref())
        .is_some_and(|events| events.iter().any(|event| event == TURN_COMPLETED_EVENT))
}

pub fn thread_change_effects(
    thread_id: Option<&str>,
    kinds: &BTreeSet<ThreadChangeKind>,
    metadata: Option<&ThreadChangeMetadata>,
) -> Vec<Effect> {
    let id = thread_id.map(str::to_owned);
    let mut effects = Effects::default();
    let background_activity_changed =
        metadata.and_then(|metadata| metadata.background_activity_changed) == Some(true);
    let turn_completed = has_turn_completed(metadata);
    for kind in kinds {
        match kind {
            ThreadChangeKind::ThreadCreated | ThreadChangeKind::ThreadDeleted => {
                effects.push(Effect::Refetch(Target::Sidebar));
                effects.push(Effect::Refetch(Target::Thread(id.clone())));
                effects.push(Effect::RefreshTimeline {
                    thread_id: id.clone(),
                    mode: TimelineRefresh::Delta,
                });
            }
            ThreadChangeKind::EventsAppended => {
                if background_activity_changed {
                    effects.push(Effect::Refetch(Target::Sidebar));
                    effects.push(Effect::Refetch(Target::Thread(id.clone())));
                }
                if let (Some(thread_id), Some(sequence)) = (
                    thread_id,
                    metadata.and_then(|metadata| metadata.timeline_sequence),
                ) {
                    effects.push(Effect::NoteTimelineSequence {
                        thread_id: thread_id.to_owned(),
                        sequence,
                    });
                }
                effects.push(Effect::RefreshTimeline {
                    thread_id: id.clone(),
                    mode: if turn_completed {
                        TimelineRefresh::Terminal
                    } else {
                        TimelineRefresh::Delta
                    },
                });
                if turn_completed {
                    effects.push(Effect::Refetch(Target::Attention));
                }
            }
            ThreadChangeKind::HistoryRewritten => {
                effects.push(Effect::Refetch(Target::Sidebar));
                effects.push(Effect::Refetch(Target::Thread(id.clone())));
                effects.push(Effect::RefreshTimeline {
                    thread_id: id.clone(),
                    mode: TimelineRefresh::Full,
                });
                effects.push(Effect::Refetch(Target::TurnDetails(id.clone())));
                effects.push(Effect::Refetch(Target::QueuedMessages(id.clone())));
                effects.push(Effect::Refetch(Target::Interactions(id.clone())));
            }
            ThreadChangeKind::HistoryCompacted => {
                effects.push(Effect::RefreshTimeline {
                    thread_id: id.clone(),
                    mode: TimelineRefresh::Full,
                });
                effects.push(Effect::Refetch(Target::TurnDetails(id.clone())));
            }
            ThreadChangeKind::InteractionsChanged => {
                effects.push(Effect::Refetch(Target::Interactions(id.clone())));
                effects.push(Effect::Refetch(Target::Attention));
                if let (Some(thread_id), Some(has_pending_interaction)) = (
                    thread_id,
                    metadata.and_then(|metadata| metadata.has_pending_interaction),
                ) {
                    effects.push(Effect::PatchPendingInteraction {
                        thread_id: thread_id.to_owned(),
                        has_pending_interaction,
                    });
                }
            }
            ThreadChangeKind::StatusChanged => {
                match (
                    thread_id,
                    metadata.and_then(|metadata| metadata.status_change.as_ref()),
                ) {
                    (Some(thread_id), Some(change)) => effects.push(Effect::PatchStatus {
                        thread_id: thread_id.to_owned(),
                        change: change.clone(),
                    }),
                    _ => effects.push(Effect::Refetch(Target::Sidebar)),
                }
                effects.push(Effect::Refetch(Target::Thread(id.clone())));
                effects.push(Effect::Refetch(Target::Attention));
            }
            ThreadChangeKind::TitleChanged
            | ThreadChangeKind::ArchivedChanged
            | ThreadChangeKind::PinStateChanged
            | ThreadChangeKind::ParentChanged
            | ThreadChangeKind::EnvironmentChanged => {
                effects.push(Effect::Refetch(Target::Sidebar));
                effects.push(Effect::Refetch(Target::Thread(id.clone())));
            }
            ThreadChangeKind::QueueChanged => {
                effects.push(Effect::Refetch(Target::QueuedMessages(id.clone())));
                effects.push(Effect::Refetch(Target::Sidebar));
            }
            ThreadChangeKind::ReadStateChanged => {
                effects.push(Effect::MarkStale(Target::Thread(id.clone())));
                effects.push(Effect::Refetch(Target::Sidebar));
                effects.push(Effect::Refetch(Target::Attention));
            }
            ThreadChangeKind::TabsChanged => {
                effects.push(Effect::Bump(Generation::ThreadTabs(id.clone())));
            }
            ThreadChangeKind::TerminalsChanged => {
                effects.push(Effect::Bump(Generation::ThreadTerminals(id.clone())));
            }
            ThreadChangeKind::Unknown => {}
        }
    }
    effects.0
}

pub fn project_change_effects(kinds: &BTreeSet<ProjectChangeKind>) -> Vec<Effect> {
    if kinds.iter().any(|kind| kind.is_known()) {
        vec![Effect::Refetch(Target::Sidebar)]
    } else {
        Vec::new()
    }
}

pub fn host_change_effects(kinds: &BTreeSet<HostChangeKind>) -> Vec<Effect> {
    let mut effects = Effects::default();
    for kind in kinds {
        match kind {
            HostChangeKind::HostConnected | HostChangeKind::HostDisconnected => {
                effects.push(Effect::Refetch(Target::Hosts));
                effects.push(Effect::Refetch(Target::Sidebar));
                effects.push(Effect::Bump(Generation::Hosts));
            }
            HostChangeKind::ProviderModelCatalogChanged => {
                effects.push(Effect::Bump(Generation::Hosts));
            }
            HostChangeKind::Unknown => {}
        }
    }
    effects.0
}

pub fn environment_change_effects(
    environment_id: &str,
    kinds: &BTreeSet<EnvironmentChangeKind>,
) -> Vec<Effect> {
    let mut effects = Effects::default();
    for kind in kinds {
        match kind {
            EnvironmentChangeKind::EnvironmentCreated
            | EnvironmentChangeKind::EnvironmentDeleted
            | EnvironmentChangeKind::StatusChanged => {
                effects.push(Effect::Refetch(Target::ThreadsInEnvironment(
                    environment_id.to_owned(),
                )));
            }
            EnvironmentChangeKind::MetadataChanged => {
                effects.push(Effect::Refetch(Target::ThreadsInEnvironment(
                    environment_id.to_owned(),
                )));
                effects.push(Effect::Refetch(Target::Sidebar));
            }
            EnvironmentChangeKind::WorkStatusChanged
            | EnvironmentChangeKind::GitRefsChanged
            | EnvironmentChangeKind::ThreadStorageChanged => {}
            EnvironmentChangeKind::Unknown => continue,
        }
        effects.push(Effect::Bump(Generation::Environment(
            environment_id.to_owned(),
        )));
    }
    effects.0
}

pub fn system_change_effects(kinds: &BTreeSet<SystemChangeKind>) -> Vec<Effect> {
    let mut effects = Effects::default();
    for kind in kinds {
        if !kind.is_known() {
            continue;
        }
        if *kind == SystemChangeKind::ConfigChanged {
            effects.push(Effect::RefreshTimeline {
                thread_id: None,
                mode: TimelineRefresh::Delta,
            });
        }
        effects.push(Effect::Bump(Generation::System(*kind)));
    }
    effects.0
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct PendingChanges {
    thread_kinds: BTreeMap<Option<String>, BTreeSet<ThreadChangeKind>>,
    thread_metadata: BTreeMap<String, ThreadChangeMetadata>,
    project_kinds: BTreeSet<ProjectChangeKind>,
    host_kinds: BTreeSet<HostChangeKind>,
    environment_kinds: BTreeMap<String, BTreeSet<EnvironmentChangeKind>>,
    system_kinds: BTreeSet<SystemChangeKind>,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct RecordOutcome {
    pub scheduled: bool,
    pub flush_now: bool,
    pub immediate: Vec<Effect>,
}

impl PendingChanges {
    pub fn is_empty(&self) -> bool {
        self.thread_kinds.is_empty()
            && self.project_kinds.is_empty()
            && self.host_kinds.is_empty()
            && self.environment_kinds.is_empty()
            && self.system_kinds.is_empty()
    }

    fn record_thread(
        &mut self,
        id: Option<&String>,
        kinds: impl IntoIterator<Item = ThreadChangeKind>,
        metadata: Option<&ThreadChangeMetadata>,
    ) {
        let kinds: Vec<ThreadChangeKind> = kinds.into_iter().collect();
        if kinds.is_empty() {
            return;
        }
        let status_changed = kinds.contains(&ThreadChangeKind::StatusChanged);
        self.thread_kinds
            .entry(id.cloned())
            .or_default()
            .extend(kinds);
        if let Some(id) = id
            && (metadata.is_some() || status_changed)
        {
            let merged = merge_thread_change_metadata(
                self.thread_metadata.get(id),
                metadata.unwrap_or(&ThreadChangeMetadata::default()),
                status_changed,
            );
            self.thread_metadata.insert(id.clone(), merged);
        }
    }

    pub fn record(&mut self, message: &ChangedMessage) -> RecordOutcome {
        let mut outcome = RecordOutcome::default();
        match message {
            ChangedMessage::Thread {
                id,
                metadata,
                changes,
            } => {
                let metadata = id.as_ref().and(metadata.as_ref());
                if let (Some(thread_id), Some(sequence)) =
                    (id, metadata.and_then(|metadata| metadata.timeline_sequence))
                    && changes.contains(&ThreadChangeKind::EventsAppended)
                {
                    outcome.immediate.push(Effect::NoteTimelineSequence {
                        thread_id: thread_id.clone(),
                        sequence,
                    });
                }
                if has_turn_completed(metadata) {
                    self.record_thread(id.as_ref(), changes.iter().copied(), metadata);
                    outcome.scheduled = true;
                    outcome.flush_now = true;
                    return outcome;
                }
                let (immediate, debounced): (Vec<_>, Vec<_>) =
                    changes.iter().copied().partition(|kind| {
                        thread_change_flush_priority(*kind) == FlushPriority::Immediate
                    });
                if !debounced.is_empty() {
                    self.record_thread(id.as_ref(), debounced, metadata);
                    outcome.scheduled = true;
                }
                if !immediate.is_empty() {
                    let status_changed = immediate.contains(&ThreadChangeKind::StatusChanged);
                    let merged = metadata.map(|metadata| {
                        merge_thread_change_metadata(None, metadata, status_changed)
                    });
                    let kinds: BTreeSet<ThreadChangeKind> = immediate.into_iter().collect();
                    outcome.immediate.extend(thread_change_effects(
                        id.as_deref(),
                        &kinds,
                        merged.as_ref(),
                    ));
                }
            }
            ChangedMessage::Project { changes, .. } => {
                if !changes.is_empty() {
                    self.project_kinds.extend(changes.iter().copied());
                    outcome.scheduled = true;
                }
            }
            ChangedMessage::Host { changes, .. } => {
                if !changes.is_empty() {
                    self.host_kinds.extend(changes.iter().copied());
                    outcome.scheduled = true;
                }
            }
            ChangedMessage::Environment { id, changes } => {
                if let Some(id) = id
                    && !changes.is_empty()
                {
                    self.environment_kinds
                        .entry(id.clone())
                        .or_default()
                        .extend(changes.iter().copied());
                    outcome.scheduled = true;
                }
            }
            ChangedMessage::System { changes } => {
                if !changes.is_empty() {
                    self.system_kinds.extend(changes.iter().copied());
                    outcome.scheduled = true;
                }
            }
            ChangedMessage::Unknown(_) => {}
        }
        outcome
    }

    pub fn take_effects(&mut self) -> Vec<Effect> {
        let pending = std::mem::take(self);
        let mut effects = Effects::default();
        let mut extend = |list: Vec<Effect>| {
            for effect in list {
                effects.push(effect);
            }
        };
        for (id, kinds) in &pending.thread_kinds {
            let metadata = id.as_ref().and_then(|id| pending.thread_metadata.get(id));
            extend(thread_change_effects(id.as_deref(), kinds, metadata));
        }
        extend(project_change_effects(&pending.project_kinds));
        extend(host_change_effects(&pending.host_kinds));
        for (id, kinds) in &pending.environment_kinds {
            extend(environment_change_effects(id, kinds));
        }
        extend(system_change_effects(&pending.system_kinds));
        effects.0
    }
}
