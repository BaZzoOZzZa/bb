use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use bb_client::{BbClient, RealtimeClient, RealtimeEvent};
use bb_wire::realtime::SubscriptionTarget;
use bb_wire::timeline::{
    ThreadTimelineQuery, ThreadTimelineResponse, TimelinePaginationCursor, TimelineRow,
    TurnSummaryDetailsResponse,
};
use tokio::sync::{broadcast, mpsc, watch};
use tokio::time::Instant;

use crate::backend::{Backend, FetchResult, RealtimeSubscriber, SubscriptionGuard};
use crate::deferred::TurnDetailsKey;
use crate::invalidation::{Effect, Generation, PendingChanges, Target, TimelineRefresh};
use crate::scheduler::DebounceScheduler;
use crate::state::{ConnectionState, Resource, StoreSnapshot};
use crate::timeline_merge::{
    LatestTimeline, LoadedTimelineState, apply_timeline_delta,
    are_timeline_pagination_cursors_equal, merge_loaded_timeline_with_latest, prepend_older_rows,
    prepend_older_timeline_rows, recover_loaded_timeline_after_stale_cursor,
    resolve_loaded_timeline_surface_key,
};

#[derive(Debug, Clone)]
pub struct StoreConfig {
    pub debounce: Duration,
    pub max_wait: Duration,
    pub timeline_segment_limit: Option<u32>,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            debounce: Duration::from_millis(50),
            max_wait: Duration::from_millis(200),
            timeline_segment_limit: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ResourceKey {
    Sidebar,
    Hosts,
    Attention,
    Thread(String),
    Timeline(String),
    Interactions(String),
    QueuedMessages(String),
    TurnDetails(TurnDetailsKey),
}

impl ResourceKey {
    fn thread_scoped(thread_id: &str) -> Vec<Self> {
        vec![
            Self::Thread(thread_id.to_owned()),
            Self::Timeline(thread_id.to_owned()),
            Self::Interactions(thread_id.to_owned()),
            Self::QueuedMessages(thread_id.to_owned()),
        ]
    }

    fn sidebar_scoped() -> Vec<Self> {
        vec![Self::Sidebar, Self::Hosts, Self::Attention]
    }
}

enum Fetched {
    Sidebar(FetchResult<bb_wire::project::SidebarBootstrapResponse>),
    Hosts(FetchResult<Vec<bb_wire::environment::Host>>),
    Attention(FetchResult<bb_wire::system::SystemAttentionResponse>),
    Thread(String, FetchResult<bb_wire::thread::ThreadResponse>),
    Interactions(
        String,
        FetchResult<Vec<bb_wire::interaction::PendingInteraction>>,
    ),
    QueuedMessages(
        String,
        FetchResult<Vec<bb_wire::queue::ThreadQueuedMessage>>,
    ),
    Timeline {
        thread_id: String,
        epoch: u64,
        mode: TimelineRefresh,
        result: FetchResult<LatestTimeline>,
    },
    OlderTimeline {
        thread_id: String,
        cursor: TimelinePaginationCursor,
        surface_key: String,
        result: FetchResult<ThreadTimelineResponse>,
    },
    TurnDetails {
        key: TurnDetailsKey,
        before_cursor: Option<String>,
        epoch: u64,
        result: FetchResult<TurnSummaryDetailsResponse>,
    },
}

enum Msg {
    Realtime(RealtimeEvent),
    Acquire(Vec<ResourceKey>),
    Release(Vec<ResourceKey>),
    Refresh(ResourceKey),
    RefreshAll,
    LoadOlderTimeline(String),
    LoadTurnDetails(TurnDetailsKey),
    LoadOlderTurnDetails(TurnDetailsKey),
    Fetched(Box<Fetched>),
    Shutdown,
}

struct StoreInner {
    tx: mpsc::UnboundedSender<Msg>,
    subscriber: Box<dyn RealtimeSubscriber>,
    snapshot: watch::Receiver<Arc<StoreSnapshot>>,
}

impl Drop for StoreInner {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Shutdown);
    }
}

#[derive(Clone)]
pub struct Store {
    inner: Arc<StoreInner>,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("version", &self.inner.snapshot.borrow().version)
            .finish()
    }
}

pub struct Interest {
    keys: Vec<ResourceKey>,
    tx: mpsc::UnboundedSender<Msg>,
    _subscriptions: Vec<SubscriptionGuard>,
}

impl std::fmt::Debug for Interest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Interest")
            .field("keys", &self.keys)
            .finish()
    }
}

impl Drop for Interest {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Release(std::mem::take(&mut self.keys)));
    }
}

impl Store {
    pub fn spawn<B: Backend, S: RealtimeSubscriber>(
        backend: Arc<B>,
        subscriber: S,
        realtime_events: Option<broadcast::Receiver<RealtimeEvent>>,
        config: StoreConfig,
    ) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let (publisher, snapshot) = watch::channel(Arc::new(StoreSnapshot::default()));
        let actor = Actor {
            backend,
            scheduler: DebounceScheduler::new(config.debounce, config.max_wait),
            config,
            state: StoreSnapshot::default(),
            tx: tx.clone(),
            rx,
            publisher,
            realtime: realtime_events,
            pending: PendingChanges::default(),
            active: HashMap::new(),
            in_flight: HashSet::new(),
            again: HashSet::new(),
            timeline_modes: HashMap::new(),
            timeline_epochs: HashMap::new(),
            details_epochs: HashMap::new(),
            stale_cursor_recovery: HashSet::new(),
            dirty: false,
        };
        tokio::spawn(actor.run());
        Self {
            inner: Arc::new(StoreInner {
                tx,
                subscriber: Box::new(subscriber),
                snapshot,
            }),
        }
    }

    pub fn connect(client: BbClient, realtime: RealtimeClient, config: StoreConfig) -> Self {
        let events = realtime.events();
        Self::spawn(Arc::new(client), realtime, Some(events), config)
    }

    pub fn snapshot(&self) -> Arc<StoreSnapshot> {
        self.inner.snapshot.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<Arc<StoreSnapshot>> {
        self.inner.snapshot.clone()
    }

    fn send(&self, msg: Msg) {
        let _ = self.inner.tx.send(msg);
    }

    fn interest(&self, keys: Vec<ResourceKey>, targets: Vec<SubscriptionTarget>) -> Interest {
        let subscriptions = targets
            .into_iter()
            .map(|target| self.inner.subscriber.subscribe(target))
            .collect();
        self.send(Msg::Acquire(keys.clone()));
        Interest {
            keys,
            tx: self.inner.tx.clone(),
            _subscriptions: subscriptions,
        }
    }

    pub fn watch_sidebar(&self) -> Interest {
        self.interest(
            ResourceKey::sidebar_scoped(),
            vec![
                SubscriptionTarget::ThreadList,
                SubscriptionTarget::ProjectList,
                SubscriptionTarget::EnvironmentList,
                SubscriptionTarget::HostList,
                SubscriptionTarget::System,
            ],
        )
    }

    pub fn watch_thread(&self, thread_id: &str) -> Interest {
        self.interest(
            ResourceKey::thread_scoped(thread_id),
            vec![SubscriptionTarget::ThreadDetail {
                thread_id: thread_id.to_owned(),
            }],
        )
    }

    pub fn refresh(&self, key: ResourceKey) {
        self.send(Msg::Refresh(key));
    }

    pub fn refresh_all(&self) {
        self.send(Msg::RefreshAll);
    }

    pub fn load_older_timeline_rows(&self, thread_id: &str) {
        self.send(Msg::LoadOlderTimeline(thread_id.to_owned()));
    }

    pub fn load_turn_details(&self, key: TurnDetailsKey) {
        self.send(Msg::LoadTurnDetails(key));
    }

    pub fn load_older_turn_details(&self, key: TurnDetailsKey) {
        self.send(Msg::LoadOlderTurnDetails(key));
    }

    pub fn load_deferred_content(&self, thread_id: &str, row: &TimelineRow) -> bool {
        match TurnDetailsKey::for_deferred_row(thread_id, row) {
            Some(key) => {
                self.load_turn_details(key);
                true
            }
            None => false,
        }
    }

    pub fn handle_realtime_event(&self, event: RealtimeEvent) {
        self.send(Msg::Realtime(event));
    }
}

struct Actor<B> {
    backend: Arc<B>,
    config: StoreConfig,
    state: StoreSnapshot,
    tx: mpsc::UnboundedSender<Msg>,
    rx: mpsc::UnboundedReceiver<Msg>,
    publisher: watch::Sender<Arc<StoreSnapshot>>,
    realtime: Option<broadcast::Receiver<RealtimeEvent>>,
    pending: PendingChanges,
    scheduler: DebounceScheduler,
    active: HashMap<ResourceKey, usize>,
    in_flight: HashSet<ResourceKey>,
    again: HashSet<ResourceKey>,
    timeline_modes: HashMap<String, TimelineRefresh>,
    timeline_epochs: HashMap<String, u64>,
    details_epochs: HashMap<TurnDetailsKey, u64>,
    stale_cursor_recovery: HashSet<String>,
    dirty: bool,
}

async fn next_realtime(
    receiver: &mut Option<broadcast::Receiver<RealtimeEvent>>,
) -> Result<RealtimeEvent, broadcast::error::RecvError> {
    match receiver {
        Some(receiver) => receiver.recv().await,
        None => std::future::pending().await,
    }
}

async fn fetch_latest_timeline<B: Backend>(
    backend: &B,
    thread_id: String,
    base: Option<Arc<LatestTimeline>>,
    segment_limit: Option<u32>,
) -> FetchResult<LatestTimeline> {
    let mut query = ThreadTimelineQuery::latest();
    query.segment_limit = segment_limit;
    let full_query = query.clone();
    if let Some(base) = &base {
        query.after_sequence = Some(base.max_seq());
    }
    let mut response = backend.timeline(thread_id.clone(), query).await?;
    let Some(delta) = response.delta.take() else {
        return Ok(LatestTimeline::from_response(response));
    };
    if let Some(rows) = base
        .as_deref()
        .and_then(|base| apply_timeline_delta(&base.rows, delta))
    {
        return Ok(LatestTimeline::with_rows(response, rows));
    }
    let mut full = backend.timeline(thread_id, full_query).await?;
    full.delta = None;
    Ok(LatestTimeline::from_response(full))
}

impl<B: Backend> Actor<B> {
    async fn run(mut self) {
        loop {
            let deadline = self.scheduler.deadline();
            tokio::select! {
                msg = self.rx.recv() => match msg {
                    None | Some(Msg::Shutdown) => break,
                    Some(msg) => self.handle(msg),
                },
                event = next_realtime(&mut self.realtime) => match event {
                    Ok(event) => self.handle_realtime(event),
                    Err(broadcast::error::RecvError::Lagged(_)) => self.invalidate_everything(),
                    Err(broadcast::error::RecvError::Closed) => self.realtime = None,
                },
                _ = tokio::time::sleep_until(deadline.unwrap_or_else(Instant::now)), if deadline.is_some() => {
                    self.flush();
                }
            }
            while let Ok(msg) = self.rx.try_recv() {
                if matches!(msg, Msg::Shutdown) {
                    return;
                }
                self.handle(msg);
            }
            if self.scheduler.is_due(Instant::now()) {
                self.flush();
            }
            self.publish();
        }
    }

    fn publish(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        self.state.version += 1;
        self.publisher.send_replace(Arc::new(self.state.clone()));
    }

    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Realtime(event) => self.handle_realtime(event),
            Msg::Acquire(keys) => self.acquire(keys),
            Msg::Release(keys) => self.release(keys),
            Msg::Refresh(key) => self.invalidate(key, true),
            Msg::RefreshAll => self.invalidate_everything(),
            Msg::LoadOlderTimeline(thread_id) => self.load_older_timeline(thread_id),
            Msg::LoadTurnDetails(key) => self.load_turn_details(key),
            Msg::LoadOlderTurnDetails(key) => self.load_older_turn_details(key),
            Msg::Fetched(fetched) => self.apply_fetched(*fetched),
            Msg::Shutdown => {}
        }
    }

    fn handle_realtime(&mut self, event: RealtimeEvent) {
        match event {
            RealtimeEvent::Connecting { reconnect } => {
                self.set_connection(ConnectionState::Connecting { reconnect });
            }
            RealtimeEvent::Connected => {
                self.set_connection(ConnectionState::Connected);
                self.bump(Generation::System(
                    bb_wire::realtime::SystemChangeKind::PluginsChanged,
                ));
                self.invalidate_everything();
            }
            RealtimeEvent::Resumed => {
                self.set_connection(ConnectionState::Connected);
                self.invalidate_everything();
            }
            RealtimeEvent::Disconnected { retry_in, error } => {
                self.set_connection(ConnectionState::Disconnected { retry_in, error });
            }
            RealtimeEvent::Changed(message) => {
                let outcome = self.pending.record(&message);
                for effect in outcome.immediate {
                    self.apply_effect(effect);
                }
                if outcome.flush_now {
                    self.flush();
                } else if outcome.scheduled {
                    self.scheduler.schedule(Instant::now());
                }
            }
            RealtimeEvent::PluginSignal { .. } | RealtimeEvent::Server(_) => {}
        }
    }

    fn set_connection(&mut self, connection: ConnectionState) {
        if self.state.connection != connection {
            self.state.connection = connection;
            self.dirty = true;
        }
    }

    fn flush(&mut self) {
        self.scheduler.reset();
        for effect in self.pending.take_effects() {
            self.apply_effect(effect);
        }
    }

    fn bump(&mut self, generation: Generation) {
        let generations = Arc::make_mut(&mut self.state.generations);
        let counter = match generation {
            Generation::System(kind) => generations.system.entry(kind).or_default(),
            Generation::Environment(id) => generations.environments.entry(id).or_default(),
            Generation::Hosts => &mut generations.hosts,
            Generation::ThreadTerminals(id) => generations.thread_terminals.entry(id).or_default(),
            Generation::ThreadTabs(id) => generations.thread_tabs.entry(id).or_default(),
        };
        *counter += 1;
        self.dirty = true;
    }

    fn apply_effect(&mut self, effect: Effect) {
        match effect {
            Effect::Refetch(target) => {
                for key in self.expand(target) {
                    self.invalidate(key, true);
                }
            }
            Effect::MarkStale(target) => {
                for key in self.expand(target) {
                    self.invalidate(key, false);
                }
            }
            Effect::RefreshTimeline { thread_id, mode } => {
                for key in self.expand(Target::Timeline(thread_id)) {
                    if let ResourceKey::Timeline(thread_id) = key {
                        self.refresh_timeline(thread_id, mode);
                    }
                }
            }
            Effect::PatchStatus { thread_id, change } => {
                if self.state.apply_status_change(&thread_id, &change) {
                    self.dirty = true;
                }
            }
            Effect::PatchPendingInteraction {
                thread_id,
                has_pending_interaction,
            } => {
                if self.state.patch_sidebar_thread(&thread_id, |entry| {
                    entry.has_pending_interaction = has_pending_interaction;
                }) {
                    self.dirty = true;
                }
            }
            Effect::NoteTimelineSequence {
                thread_id,
                sequence,
            } => {
                let entry = Arc::make_mut(self.state.timelines.entry(thread_id).or_default());
                entry.unseen_sequence = Some(entry.unseen_sequence.unwrap_or(0).max(sequence));
                self.dirty = true;
            }
            Effect::Bump(generation) => self.bump(generation),
        }
    }

    fn thread_keys(
        &self,
        make: fn(String) -> ResourceKey,
        cached: Vec<String>,
    ) -> Vec<ResourceKey> {
        let mut keys: Vec<ResourceKey> = cached.into_iter().map(make).collect();
        for key in self.active.keys() {
            let matches = matches!(
                (key, make(String::new())),
                (ResourceKey::Thread(_), ResourceKey::Thread(_))
                    | (ResourceKey::Timeline(_), ResourceKey::Timeline(_))
                    | (ResourceKey::Interactions(_), ResourceKey::Interactions(_))
                    | (
                        ResourceKey::QueuedMessages(_),
                        ResourceKey::QueuedMessages(_)
                    )
            );
            if matches && !keys.contains(key) {
                keys.push(key.clone());
            }
        }
        keys
    }

    fn expand(&self, target: Target) -> Vec<ResourceKey> {
        match target {
            Target::Sidebar => vec![ResourceKey::Sidebar],
            Target::Hosts => vec![ResourceKey::Hosts],
            Target::Attention => vec![ResourceKey::Attention],
            Target::Thread(Some(id)) => vec![ResourceKey::Thread(id)],
            Target::Timeline(Some(id)) => vec![ResourceKey::Timeline(id)],
            Target::Interactions(Some(id)) => vec![ResourceKey::Interactions(id)],
            Target::QueuedMessages(Some(id)) => vec![ResourceKey::QueuedMessages(id)],
            Target::Thread(None) => self.thread_keys(
                ResourceKey::Thread,
                self.state.threads.keys().cloned().collect(),
            ),
            Target::Timeline(None) => self.thread_keys(
                ResourceKey::Timeline,
                self.state.timelines.keys().cloned().collect(),
            ),
            Target::Interactions(None) => self.thread_keys(
                ResourceKey::Interactions,
                self.state.interactions.keys().cloned().collect(),
            ),
            Target::QueuedMessages(None) => self.thread_keys(
                ResourceKey::QueuedMessages,
                self.state.queued_messages.keys().cloned().collect(),
            ),
            Target::TurnDetails(thread_id) => self
                .state
                .turn_details
                .keys()
                .filter(|key| thread_id.as_ref().is_none_or(|id| *id == key.thread_id))
                .cloned()
                .map(ResourceKey::TurnDetails)
                .collect(),
            Target::ThreadsInEnvironment(environment_id) => self
                .state
                .thread_ids_in_environment(&environment_id)
                .into_iter()
                .map(ResourceKey::Thread)
                .collect(),
        }
    }

    fn is_active(&self, key: &ResourceKey) -> bool {
        match key {
            ResourceKey::TurnDetails(details) => self.state.turn_details.contains_key(details),
            _ => self.active.get(key).is_some_and(|count| *count > 0),
        }
    }

    fn mark_stale(&mut self, key: &ResourceKey) {
        fn stale<T>(resource: &mut Resource<T>) -> bool {
            if resource.data.is_none() && resource.error.is_none() {
                return false;
            }
            resource.stale = true;
            true
        }
        let changed = match key {
            ResourceKey::Sidebar => stale(&mut self.state.sidebar),
            ResourceKey::Hosts => stale(&mut self.state.hosts),
            ResourceKey::Attention => stale(&mut self.state.attention),
            ResourceKey::Thread(id) => self.state.threads.get_mut(id).is_some_and(stale),
            ResourceKey::Interactions(id) => self.state.interactions.get_mut(id).is_some_and(stale),
            ResourceKey::QueuedMessages(id) => {
                self.state.queued_messages.get_mut(id).is_some_and(stale)
            }
            ResourceKey::Timeline(id) => match self.state.timelines.get_mut(id) {
                Some(entry) => {
                    Arc::make_mut(entry).stale = true;
                    true
                }
                None => false,
            },
            ResourceKey::TurnDetails(details) => match self.state.turn_details.get_mut(details) {
                Some(entry) => {
                    Arc::make_mut(entry).stale = true;
                    true
                }
                None => false,
            },
        };
        self.dirty |= changed;
    }

    fn invalidate(&mut self, key: ResourceKey, refetch: bool) {
        if let ResourceKey::Timeline(thread_id) = &key
            && refetch
        {
            self.refresh_timeline(thread_id.clone(), TimelineRefresh::Delta);
            return;
        }
        self.mark_stale(&key);
        if refetch && self.is_active(&key) {
            self.request_fetch(key);
        }
    }

    fn should_refetch(&self, key: &ResourceKey) -> bool {
        match key {
            ResourceKey::Timeline(thread_id) => {
                self.is_active(key) || self.stale_cursor_recovery.contains(thread_id)
            }
            _ => self.is_active(key),
        }
    }

    fn refresh_timeline(&mut self, thread_id: String, mode: TimelineRefresh) {
        let key = ResourceKey::Timeline(thread_id.clone());
        self.mark_stale(&key);
        let pending = self
            .timeline_modes
            .entry(thread_id.clone())
            .or_insert(TimelineRefresh::Delta);
        *pending = (*pending).max(mode);
        if mode != TimelineRefresh::Delta {
            *self.timeline_epochs.entry(thread_id).or_default() += 1;
        }
        if self.should_refetch(&key) {
            self.request_fetch(key);
        }
    }

    fn invalidate_everything(&mut self) {
        let mut keys = vec![
            ResourceKey::Sidebar,
            ResourceKey::Hosts,
            ResourceKey::Attention,
        ];
        keys.extend(self.expand(Target::Thread(None)));
        keys.extend(self.expand(Target::Interactions(None)));
        keys.extend(self.expand(Target::QueuedMessages(None)));
        keys.extend(self.expand(Target::TurnDetails(None)));
        for key in keys {
            self.invalidate(key, true);
        }
        for key in self.expand(Target::Timeline(None)) {
            if let ResourceKey::Timeline(thread_id) = key {
                self.refresh_timeline(thread_id, TimelineRefresh::Delta);
            }
        }
    }

    fn acquire(&mut self, keys: Vec<ResourceKey>) {
        for key in keys {
            let count = self.active.entry(key.clone()).or_default();
            *count += 1;
            if *count == 1 && self.needs_fetch(&key) {
                self.request_fetch(key);
            }
        }
    }

    fn release(&mut self, keys: Vec<ResourceKey>) {
        for key in keys {
            if let Some(count) = self.active.get_mut(&key) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    self.active.remove(&key);
                }
            }
        }
    }

    fn needs_fetch(&self, key: &ResourceKey) -> bool {
        match key {
            ResourceKey::Sidebar => self.state.sidebar.needs_fetch(),
            ResourceKey::Hosts => self.state.hosts.needs_fetch(),
            ResourceKey::Attention => self.state.attention.needs_fetch(),
            ResourceKey::Thread(id) => self.state.threads.get(id).is_none_or(Resource::needs_fetch),
            ResourceKey::Interactions(id) => self
                .state
                .interactions
                .get(id)
                .is_none_or(Resource::needs_fetch),
            ResourceKey::QueuedMessages(id) => self
                .state
                .queued_messages
                .get(id)
                .is_none_or(Resource::needs_fetch),
            ResourceKey::Timeline(id) => {
                self.state.timelines.get(id).is_none_or(|entry| {
                    entry.latest.is_none() || entry.stale || entry.error.is_some()
                })
            }
            ResourceKey::TurnDetails(details) => self
                .state
                .turn_details
                .get(details)
                .is_none_or(|entry| !entry.loaded || entry.stale || entry.error.is_some()),
        }
    }

    fn request_fetch(&mut self, key: ResourceKey) {
        if self.in_flight.contains(&key) {
            self.again.insert(key);
            return;
        }
        self.in_flight.insert(key.clone());
        self.dirty = true;
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        let send = move |fetched: Fetched| {
            let _ = tx.send(Msg::Fetched(Box::new(fetched)));
        };
        match key {
            ResourceKey::Sidebar => {
                self.state.sidebar.begin_fetch();
                tokio::spawn(async move {
                    send(Fetched::Sidebar(backend.sidebar_bootstrap().await));
                });
            }
            ResourceKey::Hosts => {
                self.state.hosts.begin_fetch();
                tokio::spawn(async move { send(Fetched::Hosts(backend.hosts().await)) });
            }
            ResourceKey::Attention => {
                self.state.attention.begin_fetch();
                tokio::spawn(async move { send(Fetched::Attention(backend.attention().await)) });
            }
            ResourceKey::Thread(id) => {
                self.state
                    .threads
                    .entry(id.clone())
                    .or_default()
                    .begin_fetch();
                tokio::spawn(async move {
                    let result = backend.thread(id.clone()).await;
                    send(Fetched::Thread(id, result));
                });
            }
            ResourceKey::Interactions(id) => {
                self.state
                    .interactions
                    .entry(id.clone())
                    .or_default()
                    .begin_fetch();
                tokio::spawn(async move {
                    let result = backend.interactions(id.clone()).await;
                    send(Fetched::Interactions(id, result));
                });
            }
            ResourceKey::QueuedMessages(id) => {
                self.state
                    .queued_messages
                    .entry(id.clone())
                    .or_default()
                    .begin_fetch();
                tokio::spawn(async move {
                    let result = backend.queued_messages(id.clone()).await;
                    send(Fetched::QueuedMessages(id, result));
                });
            }
            ResourceKey::Timeline(thread_id) => {
                let mode = self
                    .timeline_modes
                    .remove(&thread_id)
                    .unwrap_or(TimelineRefresh::Delta);
                let epoch = self.timeline_epochs.get(&thread_id).copied().unwrap_or(0);
                let entry =
                    Arc::make_mut(self.state.timelines.entry(thread_id.clone()).or_default());
                entry.fetching = true;
                let base = match mode {
                    TimelineRefresh::Full => None,
                    _ => entry.latest.clone(),
                };
                let segment_limit = self.config.timeline_segment_limit;
                tokio::spawn(async move {
                    let result =
                        fetch_latest_timeline(&*backend, thread_id.clone(), base, segment_limit)
                            .await;
                    send(Fetched::Timeline {
                        thread_id,
                        epoch,
                        mode,
                        result,
                    });
                });
            }
            ResourceKey::TurnDetails(key) => {
                let epoch = self.details_epochs.get(&key).copied().unwrap_or(0);
                let entry = Arc::make_mut(self.state.turn_details.entry(key.clone()).or_default());
                entry.fetching = true;
                let query = key.query(None);
                tokio::spawn(async move {
                    let result = backend
                        .turn_summary_details(key.thread_id.clone(), query)
                        .await;
                    send(Fetched::TurnDetails {
                        key,
                        before_cursor: None,
                        epoch,
                        result,
                    });
                });
            }
        }
    }

    fn finish_fetch(&mut self, key: ResourceKey) {
        self.in_flight.remove(&key);
        self.dirty = true;
        if self.again.remove(&key) && self.should_refetch(&key) {
            self.request_fetch(key);
        }
    }

    fn apply_fetched(&mut self, fetched: Fetched) {
        let now = Instant::now();
        match fetched {
            Fetched::Sidebar(result) => {
                self.state.sidebar.finish(result.map(Arc::new), now);
                self.finish_fetch(ResourceKey::Sidebar);
            }
            Fetched::Hosts(result) => {
                self.state.hosts.finish(result.map(Arc::new), now);
                self.finish_fetch(ResourceKey::Hosts);
            }
            Fetched::Attention(result) => {
                self.state.attention.finish(result, now);
                self.finish_fetch(ResourceKey::Attention);
            }
            Fetched::Thread(id, result) => {
                self.state
                    .threads
                    .entry(id.clone())
                    .or_default()
                    .finish(result.map(Arc::new), now);
                self.finish_fetch(ResourceKey::Thread(id));
            }
            Fetched::Interactions(id, result) => {
                self.state
                    .interactions
                    .entry(id.clone())
                    .or_default()
                    .finish(result.map(Arc::new), now);
                self.finish_fetch(ResourceKey::Interactions(id));
            }
            Fetched::QueuedMessages(id, result) => {
                self.state
                    .queued_messages
                    .entry(id.clone())
                    .or_default()
                    .finish(result.map(Arc::new), now);
                self.finish_fetch(ResourceKey::QueuedMessages(id));
            }
            Fetched::Timeline {
                thread_id,
                epoch,
                mode,
                result,
            } => self.apply_timeline(thread_id, epoch, mode, result, now),
            Fetched::OlderTimeline {
                thread_id,
                cursor,
                surface_key,
                result,
            } => self.apply_older_timeline(thread_id, cursor, surface_key, result),
            Fetched::TurnDetails {
                key,
                before_cursor,
                epoch,
                result,
            } => self.apply_turn_details(key, before_cursor, epoch, result),
        }
    }

    fn apply_timeline(
        &mut self,
        thread_id: String,
        epoch: u64,
        mode: TimelineRefresh,
        result: FetchResult<LatestTimeline>,
        now: Instant,
    ) {
        let key = ResourceKey::Timeline(thread_id.clone());
        let current_epoch = self.timeline_epochs.get(&thread_id).copied().unwrap_or(0);
        if epoch != current_epoch {
            self.again.insert(key.clone());
            self.finish_fetch(key);
            return;
        }
        let recovering = self.stale_cursor_recovery.remove(&thread_id);
        let entry = Arc::make_mut(self.state.timelines.entry(thread_id.clone()).or_default());
        entry.fetching = false;
        match result {
            Ok(latest) => {
                let latest = Arc::new(latest);
                let surface_key =
                    resolve_loaded_timeline_surface_key(&thread_id, Some(&latest.response));
                entry.loaded = if mode == TimelineRefresh::Full {
                    LoadedTimelineState::from_latest(&latest, &surface_key, None)
                } else if recovering {
                    recover_loaded_timeline_after_stale_cursor(&entry.loaded, &latest, &surface_key)
                } else {
                    merge_loaded_timeline_with_latest(&entry.loaded, &latest, &surface_key)
                };
                if entry
                    .unseen_sequence
                    .is_some_and(|sequence| latest.max_seq() >= sequence)
                {
                    entry.unseen_sequence = None;
                }
                entry.latest = Some(latest);
                entry.stale = false;
                entry.error = None;
                entry.updated_at = Some(now);
            }
            Err(error) => {
                tracing::debug!(thread_id, %error, "timeline fetch failed");
                if recovering {
                    entry.loaded.older_cursor = None;
                }
                entry.error = Some(error);
            }
        }
        self.finish_fetch(key);
    }

    fn load_older_timeline(&mut self, thread_id: String) {
        let Some(entry) = self.state.timelines.get_mut(&thread_id) else {
            return;
        };
        if entry.loading_older {
            return;
        }
        let Some(cursor) = entry.loaded.older_cursor.clone() else {
            return;
        };
        let entry = Arc::make_mut(entry);
        entry.loading_older = true;
        entry.older_error = None;
        self.dirty = true;
        let surface_key = entry.loaded.surface_key.clone();
        let mut query = ThreadTimelineQuery::older(&cursor);
        query.segment_limit = self.config.timeline_segment_limit;
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend.timeline(thread_id.clone(), query).await;
            let _ = tx.send(Msg::Fetched(Box::new(Fetched::OlderTimeline {
                thread_id,
                cursor,
                surface_key,
                result,
            })));
        });
    }

    fn apply_older_timeline(
        &mut self,
        thread_id: String,
        cursor: TimelinePaginationCursor,
        surface_key: String,
        result: FetchResult<ThreadTimelineResponse>,
    ) {
        let Some(entry) = self.state.timelines.get_mut(&thread_id) else {
            return;
        };
        let entry = Arc::make_mut(entry);
        entry.loading_older = false;
        self.dirty = true;
        match result {
            Ok(response) => {
                if entry.loaded.surface_key != surface_key
                    || !are_timeline_pagination_cursors_equal(
                        entry.loaded.older_cursor.as_ref(),
                        Some(&cursor),
                    )
                {
                    return;
                }
                let older = LatestTimeline::from_response(response);
                entry.loaded.rows = prepend_older_timeline_rows(&entry.loaded.rows, &older.rows);
                entry.loaded.older_cursor = older.response.timeline_page.older_cursor;
            }
            Err(error) if error.is_stale_cursor() => {
                self.stale_cursor_recovery.insert(thread_id.clone());
                self.refresh_timeline(thread_id, TimelineRefresh::Delta);
            }
            Err(error) => entry.older_error = Some(error),
        }
    }

    fn load_turn_details(&mut self, key: TurnDetailsKey) {
        let resource = ResourceKey::TurnDetails(key);
        if self.in_flight.contains(&resource) || !self.needs_fetch(&resource) {
            return;
        }
        self.request_fetch(resource);
    }

    fn load_older_turn_details(&mut self, key: TurnDetailsKey) {
        let Some(entry) = self.state.turn_details.get_mut(&key) else {
            return;
        };
        if entry.loading_older {
            return;
        }
        let Some(cursor) = entry.older_cursor.clone() else {
            return;
        };
        Arc::make_mut(entry).loading_older = true;
        self.dirty = true;
        let epoch = self.details_epochs.get(&key).copied().unwrap_or(0);
        let query = key.query(Some(cursor.clone()));
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .turn_summary_details(key.thread_id.clone(), query)
                .await;
            let _ = tx.send(Msg::Fetched(Box::new(Fetched::TurnDetails {
                key,
                before_cursor: Some(cursor),
                epoch,
                result,
            })));
        });
    }

    fn apply_turn_details(
        &mut self,
        key: TurnDetailsKey,
        before_cursor: Option<String>,
        epoch: u64,
        result: FetchResult<TurnSummaryDetailsResponse>,
    ) {
        let is_newest = before_cursor.is_none();
        let current_epoch = self.details_epochs.get(&key).copied().unwrap_or(0);
        let Some(entry) = self.state.turn_details.get_mut(&key) else {
            if is_newest {
                self.finish_fetch(ResourceKey::TurnDetails(key));
            }
            return;
        };
        let entry = Arc::make_mut(entry);
        self.dirty = true;
        if is_newest {
            entry.fetching = false;
            match result {
                Ok(page) => {
                    entry.rows = Arc::new(page.rows);
                    entry.older_cursor = page.older_cursor;
                    entry.history_snapshot = page.history_snapshot;
                    entry.loaded = true;
                    entry.stale = false;
                    entry.error = None;
                    *self.details_epochs.entry(key.clone()).or_default() += 1;
                }
                Err(error) => entry.error = Some(error),
            }
            self.finish_fetch(ResourceKey::TurnDetails(key));
            return;
        }
        entry.loading_older = false;
        if epoch != current_epoch || entry.older_cursor != before_cursor {
            return;
        }
        match result {
            Ok(page) => {
                entry.rows = Arc::new(prepend_older_rows(&entry.rows, &page.rows));
                entry.older_cursor = page.older_cursor;
            }
            Err(error) => entry.error = Some(error),
        }
    }
}
