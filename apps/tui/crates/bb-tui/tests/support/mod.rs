#![allow(dead_code)]

use std::sync::{Arc, Mutex};
use std::time::Instant;

use bb_store::{Backend, FetchError, FetchResult, StoreSnapshot};
use bb_tui::api::{Api, ApiFuture, Mutation};
use bb_tui::app::{Action, AppState, ServerInfo, update};
use bb_tui::ui;
use bb_wire::environment::Host;
use bb_wire::interaction::PendingInteraction;
use bb_wire::project::{
    Project, ProjectKind, ProjectWithThreadsResponse, SidebarBootstrapResponse,
};
use bb_wire::queue::ThreadQueuedMessage;
use bb_wire::system::{AppKeybindingOverride, SystemAttentionResponse};
use bb_wire::thread::{ThreadListEntry, ThreadResponse, ThreadStatus};
use bb_wire::timeline::{
    ThreadTimelineQuery, ThreadTimelineResponse, TurnSummaryDetailsQuery,
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
}

impl FakeBackend {
    pub fn new(sidebar: SidebarBootstrapResponse) -> Arc<Self> {
        Arc::new(Self {
            sidebar: Mutex::new(sidebar),
            sidebar_fetches: Mutex::new(0),
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
        _thread_id: String,
        _query: ThreadTimelineQuery,
    ) -> FetchResult<ThreadTimelineResponse> {
        Ok(ThreadTimelineResponse::default())
    }

    async fn turn_summary_details(
        &self,
        _thread_id: String,
        _query: TurnSummaryDetailsQuery,
    ) -> FetchResult<TurnSummaryDetailsResponse> {
        Ok(TurnSummaryDetailsResponse::default())
    }

    async fn interactions(&self, _thread_id: String) -> FetchResult<Vec<PendingInteraction>> {
        Ok(Vec::new())
    }

    async fn queued_messages(&self, _thread_id: String) -> FetchResult<Vec<ThreadQueuedMessage>> {
        Ok(Vec::new())
    }
}

pub struct MockApi {
    pub backend: Arc<FakeBackend>,
    pub mutations: Mutex<Vec<Mutation>>,
    pub overrides: Vec<AppKeybindingOverride>,
    pub fail: bool,
}

impl MockApi {
    pub fn new(backend: Arc<FakeBackend>) -> Arc<Self> {
        Arc::new(Self {
            backend,
            mutations: Mutex::new(Vec::new()),
            overrides: Vec::new(),
            fail: false,
        })
    }

    pub fn recorded(&self) -> Vec<Mutation> {
        self.mutations.lock().unwrap().clone()
    }
}

impl Api for MockApi {
    fn mutate(&self, mutation: Mutation) -> ApiFuture<()> {
        self.mutations.lock().unwrap().push(mutation.clone());
        if self.fail {
            return Box::pin(async { Err("server said no".to_string()) });
        }
        let id = mutation.thread_id().to_owned();
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
        });
        Box::pin(async { Ok(()) })
    }

    fn keybinding_overrides(&self) -> ApiFuture<Vec<AppKeybindingOverride>> {
        let overrides = self.overrides.clone();
        Box::pin(async move { Ok(overrides) })
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
