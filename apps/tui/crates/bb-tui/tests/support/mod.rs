#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use bb_store::{
    Backend, FetchError, FetchResult, LatestTimeline, LoadedTimelineState, StoreSnapshot,
    TimelineEntry, TurnDetailsEntry, TurnDetailsKey,
};
use bb_tui::api::{Api, ApiFuture, Mutation, MutationReply};
use bb_tui::app::{Action, AppState, Effect, ServerInfo, update};
use bb_tui::platform::Platform;
use bb_tui::ui;
use bb_wire::environment::Host;
use bb_wire::interaction::PendingInteraction;
use bb_wire::project::{
    Project, ProjectKind, ProjectWithThreadsResponse, SidebarBootstrapResponse,
};
use bb_wire::prompt::{PromptHistoryEntry, ResolvedThreadExecutionOptions};
use bb_wire::queue::{SendMessageResponse, ThreadQueuedMessage};
use bb_wire::system::{
    AppKeybindingOverride, SystemAttentionResponse, SystemExecutionOptionsQuery,
    SystemExecutionOptionsResponse,
};
use bb_wire::thread::{ThreadListEntry, ThreadResponse, ThreadStatus};
use bb_wire::timeline::{
    ConversationRole, ConversationRow, ThreadTimelineQuery, ThreadTimelineResponse, TimelineRow,
    TimelineRowBase, TimelineRowStatus, TimelineWorkRow, TurnRow, TurnSummaryDetailsQuery,
    TurnSummaryDetailsResponse,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

pub const NOW_MS: i64 = 1_800_000_000_000;
pub const MINUTE: i64 = 60_000;

pub fn thread(id: &str, project: &str, title: &str, minutes_ago: i64) -> ThreadListEntry {
    let at = NOW_MS - minutes_ago * MINUTE;
    let mut entry = ThreadListEntry::default();
    entry.thread.id = id.into();
    entry.thread.project_id = project.into();
    entry.thread.provider_id = "codex".into();
    entry.thread.title = Some(title.into());
    entry.thread.status = ThreadStatus::Idle;
    entry.thread.latest_attention_at = at;
    entry.thread.last_read_at = Some(at);
    entry.thread.created_at = at;
    entry.thread.updated_at = at;
    entry
}

pub fn project(id: &str, name: &str, threads: Vec<ThreadListEntry>) -> ProjectWithThreadsResponse {
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

pub fn sample_sidebar() -> SidebarBootstrapResponse {
    let mut waiting = thread("thr_wait", "proj_a", "Needs approval", 3);
    waiting.has_pending_interaction = true;
    let mut unread = thread("thr_unread", "proj_a", "Fix flaky test", 30);
    unread.thread.last_read_at = Some(0);
    let older = thread("thr_old", "proj_a", "Write docs", 60 * 30);
    let other = thread("thr_other", "proj_b", "Refactor parser", 10);
    SidebarBootstrapResponse {
        sections: Vec::new(),
        projects: vec![
            project("proj_a", "alpha", vec![older, unread, waiting]),
            project("proj_b", "beta", vec![other]),
        ],
        personal_project: project("proj_personal", "Personal", Vec::new()),
    }
}

#[derive(Default)]
pub struct FakeBackend {
    pub sidebar: Mutex<SidebarBootstrapResponse>,
    pub sidebar_fetches: Mutex<usize>,
    pub timelines: Mutex<HashMap<String, ThreadTimelineResponse>>,
    pub older_pages: Mutex<HashMap<String, ThreadTimelineResponse>>,
    pub turn_details: Mutex<HashMap<String, TurnSummaryDetailsResponse>>,
    pub timeline_queries: Mutex<Vec<ThreadTimelineQuery>>,
    pub details_queries: Mutex<Vec<TurnSummaryDetailsQuery>>,
    pub queued: Mutex<HashMap<String, Vec<ThreadQueuedMessage>>>,
    pub interactions: Mutex<HashMap<String, Vec<PendingInteraction>>>,
}

impl FakeBackend {
    pub fn new(sidebar: SidebarBootstrapResponse) -> Arc<Self> {
        Arc::new(Self {
            sidebar: Mutex::new(sidebar),
            ..Self::default()
        })
    }

    pub fn patch_thread(&self, thread_id: &str, patch: impl Fn(&mut ThreadListEntry)) {
        let mut guard = self.sidebar.lock().unwrap();
        let sidebar = &mut *guard;
        for project in sidebar
            .projects
            .iter_mut()
            .chain(std::iter::once(&mut sidebar.personal_project))
        {
            for entry in &mut project.threads {
                if entry.thread.id == thread_id {
                    patch(entry);
                }
            }
        }
    }

    fn entry(&self, thread_id: &str) -> Option<ThreadListEntry> {
        let sidebar = self.sidebar.lock().unwrap();
        sidebar
            .projects
            .iter()
            .chain(std::iter::once(&sidebar.personal_project))
            .flat_map(|project| project.threads.iter())
            .find(|entry| entry.thread.id == thread_id)
            .cloned()
    }
}

impl Backend for FakeBackend {
    async fn sidebar_bootstrap(&self) -> FetchResult<SidebarBootstrapResponse> {
        *self.sidebar_fetches.lock().unwrap() += 1;
        Ok(self.sidebar.lock().unwrap().clone())
    }

    async fn hosts(&self) -> FetchResult<Vec<Host>> {
        Ok(Vec::new())
    }

    async fn attention(&self) -> FetchResult<SystemAttentionResponse> {
        Ok(SystemAttentionResponse::default())
    }

    async fn thread(&self, thread_id: String) -> FetchResult<ThreadResponse> {
        let entry = self
            .entry(&thread_id)
            .ok_or_else(|| FetchError::http(404, Some("not_found"), "missing"))?;
        Ok(ThreadResponse {
            thread: entry.thread,
            runtime: entry.runtime,
            ..ThreadResponse::default()
        })
    }

    async fn timeline(
        &self,
        thread_id: String,
        query: ThreadTimelineQuery,
    ) -> FetchResult<ThreadTimelineResponse> {
        self.timeline_queries.lock().unwrap().push(query.clone());
        let pages = if query.before_anchor_seq.is_some() {
            &self.older_pages
        } else {
            &self.timelines
        };
        Ok(pages
            .lock()
            .unwrap()
            .get(&thread_id)
            .cloned()
            .unwrap_or_default())
    }

    async fn turn_summary_details(
        &self,
        _thread_id: String,
        query: TurnSummaryDetailsQuery,
    ) -> FetchResult<TurnSummaryDetailsResponse> {
        self.details_queries.lock().unwrap().push(query.clone());
        let key = query
            .item_id
            .clone()
            .unwrap_or_else(|| query.turn_id.clone());
        self.turn_details
            .lock()
            .unwrap()
            .get(&key)
            .cloned()
            .ok_or_else(|| FetchError::http(404, Some("not_found"), "no details"))
    }

    async fn interactions(&self, thread_id: String) -> FetchResult<Vec<PendingInteraction>> {
        Ok(self
            .interactions
            .lock()
            .unwrap()
            .get(&thread_id)
            .cloned()
            .unwrap_or_default())
    }

    async fn queued_messages(&self, thread_id: String) -> FetchResult<Vec<ThreadQueuedMessage>> {
        Ok(self
            .queued
            .lock()
            .unwrap()
            .get(&thread_id)
            .cloned()
            .unwrap_or_default())
    }
}

pub struct MockApi {
    pub backend: Arc<FakeBackend>,
    pub mutations: Mutex<Vec<Mutation>>,
    pub overrides: Vec<AppKeybindingOverride>,
    pub fail: bool,
    pub execution_options: SystemExecutionOptionsResponse,
    pub defaults: Option<ResolvedThreadExecutionOptions>,
    pub history: Vec<PromptHistoryEntry>,
    pub execution_queries: Mutex<Vec<SystemExecutionOptionsQuery>>,
}

impl MockApi {
    pub fn new(backend: Arc<FakeBackend>) -> Arc<Self> {
        Arc::new(Self {
            backend,
            mutations: Mutex::new(Vec::new()),
            overrides: Vec::new(),
            fail: false,
            execution_options: SystemExecutionOptionsResponse::default(),
            defaults: None,
            history: Vec::new(),
            execution_queries: Mutex::new(Vec::new()),
        })
    }

    pub fn recorded(&self) -> Vec<Mutation> {
        self.mutations.lock().unwrap().clone()
    }
}

impl Api for MockApi {
    fn mutate(&self, mutation: Mutation) -> ApiFuture<MutationReply> {
        self.mutations.lock().unwrap().push(mutation.clone());
        if self.fail {
            return Box::pin(async { Err("server said no".to_string()) });
        }
        let id = mutation.thread_id().to_owned();
        if let Some(interaction_id) = mutation.interaction_id() {
            let mut interactions = self.backend.interactions.lock().unwrap();
            let list = interactions.entry(id.clone()).or_default();
            list.retain(|interaction| interaction.id != interaction_id);
            let empty = list.is_empty();
            drop(interactions);
            if empty {
                self.backend
                    .patch_thread(&id, |entry| entry.has_pending_interaction = false);
            }
            return Box::pin(async { Ok(MutationReply::Done) });
        }
        if mutation.is_composer() {
            let reply = self.composer_reply(&mutation);
            return Box::pin(async move { Ok(reply) });
        }
        self.backend.patch_thread(&id, |entry| match &mutation {
            Mutation::Rename { title, .. } => entry.thread.title = Some(title.clone()),
            Mutation::Archive { .. } => entry.thread.archived_at = Some(NOW_MS),
            Mutation::Unarchive { .. } => entry.thread.archived_at = None,
            Mutation::MarkRead { .. } => {
                entry.thread.last_read_at = Some(entry.thread.latest_attention_at)
            }
            Mutation::MarkUnread { .. } => entry.thread.last_read_at = Some(0),
            Mutation::Pin { .. } => entry.thread.pinned_at = Some(NOW_MS),
            Mutation::Unpin { .. } => entry.thread.pinned_at = None,
            _ => {}
        });
        Box::pin(async { Ok(MutationReply::Done) })
    }

    fn execution_options(
        &self,
        query: SystemExecutionOptionsQuery,
    ) -> ApiFuture<SystemExecutionOptionsResponse> {
        self.execution_queries.lock().unwrap().push(query);
        let options = self.execution_options.clone();
        Box::pin(async move { Ok(options) })
    }

    fn default_execution_options(
        &self,
        _thread_id: String,
    ) -> ApiFuture<Option<ResolvedThreadExecutionOptions>> {
        let defaults = self.defaults.clone();
        Box::pin(async move { Ok(defaults) })
    }

    fn prompt_history(&self, _thread_id: String) -> ApiFuture<Vec<PromptHistoryEntry>> {
        let history = self.history.clone();
        Box::pin(async move { Ok(history) })
    }

    fn keybinding_overrides(&self) -> ApiFuture<Vec<AppKeybindingOverride>> {
        let overrides = self.overrides.clone();
        Box::pin(async move { Ok(overrides) })
    }
}

impl MockApi {
    fn composer_reply(&self, mutation: &Mutation) -> MutationReply {
        let mut queued = self.backend.queued.lock().unwrap();
        let list = queued.entry(mutation.thread_id().to_owned()).or_default();
        match mutation {
            Mutation::QueueMessage { thread_id, request } => {
                list.push(ThreadQueuedMessage {
                    id: format!("q{}", list.len() + 1),
                    thread_id: thread_id.clone(),
                    content: request.input.clone(),
                    editable: true,
                    updated_at: NOW_MS,
                    ..ThreadQueuedMessage::default()
                });
                MutationReply::Done
            }
            Mutation::SendMessage { .. } => {
                MutationReply::Delivery(SendMessageResponse::Sent { ok: true })
            }
            Mutation::SendQueued {
                queued_message_id, ..
            }
            | Mutation::DeleteQueued {
                queued_message_id, ..
            } => {
                list.retain(|message| &message.id != queued_message_id);
                MutationReply::Delivery(SendMessageResponse::Sent { ok: true })
            }
            Mutation::UpdateQueued {
                queued_message_id,
                request,
                ..
            } => {
                for message in list.iter_mut() {
                    if &message.id == queued_message_id {
                        message.content = request.input.clone();
                        message.updated_at += 1;
                    }
                }
                MutationReply::Done
            }
            Mutation::HoldQueued { .. } => MutationReply::Lease { lease_ms: 30_000 },
            _ => MutationReply::Done,
        }
    }
}

pub fn snapshot_with(sidebar: SidebarBootstrapResponse) -> Arc<StoreSnapshot> {
    let mut snapshot = StoreSnapshot::default();
    snapshot.sidebar.data = Some(Arc::new(sidebar));
    snapshot.connection = bb_store::ConnectionState::Connected;
    Arc::new(snapshot)
}

pub fn loaded_state(sidebar: SidebarBootstrapResponse) -> AppState {
    let mut state = AppState::new(
        ServerInfo {
            url: "http://127.0.0.1:4000/".into(),
            profile: None,
        },
        Instant::now(),
        NOW_MS,
    );
    update(&mut state, Action::Snapshot(snapshot_with(sidebar)));
    state
}

pub fn render_lines(state: &AppState, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal
        .draw(|frame| ui::render(state, frame))
        .expect("draw");
    buffer_lines(&terminal)
}

pub fn buffer_lines(terminal: &Terminal<TestBackend>) -> Vec<String> {
    let buffer = terminal.backend().buffer();
    let width = usize::from(buffer.area.width);
    buffer
        .content()
        .chunks(width)
        .map(|row| {
            let mut line = String::new();
            let mut skip = 0;
            for cell in row {
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                let symbol = cell.symbol();
                skip = unicode_width::UnicodeWidthStr::width(symbol).saturating_sub(1);
                line.push_str(symbol);
            }
            line.trim_end().to_owned()
        })
        .collect()
}

#[derive(Default)]
pub struct MockPlatform {
    pub copied: Mutex<Vec<String>>,
    pub opened: Mutex<Vec<String>>,
    pub drafts: Mutex<HashMap<String, String>>,
}

impl Platform for MockPlatform {
    fn copy(&self, text: &str) -> Result<(), String> {
        self.copied.lock().unwrap().push(text.to_owned());
        Ok(())
    }

    fn open(&self, target: &str) -> Result<(), String> {
        self.opened.lock().unwrap().push(target.to_owned());
        Ok(())
    }

    fn load_draft(&self, thread_id: &str) -> Option<String> {
        self.drafts.lock().unwrap().get(thread_id).cloned()
    }

    fn save_draft(&self, thread_id: &str, raw: Option<&str>) -> Result<(), String> {
        let mut drafts = self.drafts.lock().unwrap();
        match raw {
            Some(raw) => drafts.insert(thread_id.to_owned(), raw.to_owned()),
            None => drafts.remove(thread_id),
        };
        Ok(())
    }
}

pub const THREAD: &str = "thr_wait";

pub fn base(id: &str, seq: i64) -> TimelineRowBase {
    TimelineRowBase {
        id: id.into(),
        thread_id: THREAD.into(),
        turn_id: Some(format!("turn-{}", seq / 100)),
        source_seq_start: seq,
        source_seq_end: seq,
        started_at: NOW_MS - 600_000 + seq * 1_000,
        created_at: NOW_MS - 600_000 + seq * 1_000,
    }
}

pub fn message(id: &str, seq: i64, role: ConversationRole, text: &str) -> TimelineRow {
    TimelineRow::Conversation(Box::new(ConversationRow {
        base: base(id, seq),
        role,
        message_seq: seq,
        text: text.into(),
        ..ConversationRow::default()
    }))
}

pub fn user(id: &str, seq: i64, text: &str) -> TimelineRow {
    message(id, seq, ConversationRole::User, text)
}

pub fn assistant(id: &str, seq: i64, text: &str) -> TimelineRow {
    message(id, seq, ConversationRole::Assistant, text)
}

pub fn command(id: &str, seq: i64, command: &str, output: &str) -> TimelineRow {
    TimelineRow::Work(TimelineWorkRow::Command(Box::new(
        bb_wire::timeline::CommandWorkRow {
            base: base(id, seq),
            status: TimelineRowStatus::Completed,
            call_id: format!("call-{id}"),
            command: command.into(),
            output: output.into(),
            exit_code: Some(0),
            completed_at: Some(NOW_MS - 600_000 + seq * 1_000 + 2_000),
            ..bb_wire::timeline::CommandWorkRow::default()
        },
    )))
}

pub fn turn(id: &str, turn_id: &str, start: i64, end: i64) -> TimelineRow {
    let mut base = base(id, start);
    base.turn_id = Some(turn_id.into());
    base.source_seq_end = end;
    TimelineRow::Turn(Box::new(TurnRow {
        base,
        status: TimelineRowStatus::Completed,
        summary_count: 3,
        completed_at: Some(NOW_MS - 300_000),
        children: None,
    }))
}

pub fn row_from_json(value: serde_json::Value) -> TimelineRow {
    serde_json::from_value(value).expect("timeline row")
}

pub fn timeline_response(rows: Vec<TimelineRow>, has_older: bool) -> ThreadTimelineResponse {
    let mut response = ThreadTimelineResponse {
        rows,
        max_seq: 10_000,
        ..ThreadTimelineResponse::default()
    };
    if has_older {
        response.timeline_page.has_older_rows = true;
        response.timeline_page.older_cursor = Some(bb_wire::timeline::TimelinePaginationCursor {
            anchor_seq: 1,
            anchor_id: "anchor".into(),
        });
    }
    response
}

pub fn timeline_entry(response: ThreadTimelineResponse) -> Arc<TimelineEntry> {
    let latest = LatestTimeline::from_response(response);
    let loaded = LoadedTimelineState::from_latest(&latest, "surface", None);
    Arc::new(TimelineEntry {
        latest: Some(Arc::new(latest)),
        loaded,
        ..TimelineEntry::default()
    })
}

pub fn with_timeline(
    snapshot: &StoreSnapshot,
    rows: Vec<TimelineRow>,
    has_older: bool,
) -> Arc<StoreSnapshot> {
    let mut snapshot = snapshot.clone();
    snapshot.timelines.insert(
        THREAD.into(),
        timeline_entry(timeline_response(rows, has_older)),
    );
    snapshot.version += 1;
    Arc::new(snapshot)
}

pub fn with_turn_details(
    snapshot: &StoreSnapshot,
    key: TurnDetailsKey,
    rows: Vec<TimelineRow>,
) -> Arc<StoreSnapshot> {
    let mut snapshot = snapshot.clone();
    snapshot.turn_details.insert(
        key,
        Arc::new(TurnDetailsEntry {
            rows: Arc::new(rows),
            loaded: true,
            ..TurnDetailsEntry::default()
        }),
    );
    snapshot.version += 1;
    Arc::new(snapshot)
}

pub fn press(state: &mut AppState, code: crossterm::event::KeyCode) -> Vec<Effect> {
    update(state, Action::Key(crossterm::event::KeyEvent::from(code)))
}

pub fn press_mod(
    state: &mut AppState,
    code: crossterm::event::KeyCode,
    modifiers: crossterm::event::KeyModifiers,
) -> Vec<Effect> {
    update(
        state,
        Action::Key(crossterm::event::KeyEvent::new(code, modifiers)),
    )
}

pub fn without_composer_loads(effects: Vec<Effect>) -> Vec<Effect> {
    effects
        .into_iter()
        .filter(|effect| {
            !matches!(
                effect,
                Effect::LoadDraft(_)
                    | Effect::FetchDefaults(_)
                    | Effect::FetchExecutionOptions { .. }
                    | Effect::FetchPromptHistory(_)
            )
        })
        .collect()
}
