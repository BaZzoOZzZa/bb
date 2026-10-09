use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use bb_store::{Interest, ResourceKey, Store};
use crossterm::event::{Event, KeyEventKind};
use futures::{Stream, StreamExt};
use ratatui::Terminal;
use ratatui::backend::Backend as TerminalBackend;
use tokio::sync::mpsc;

use bb_wire::project::WorkspacePathsQuery;
use bb_wire::system::SystemEnvironmentProvidersQuery;
use bb_wire::thread::ThreadSearchQuery;

use crate::api::{Api, AttachmentUpload};
use crate::app::{Action, AppState, Effect, ToastLevel, update};
use crate::attach::{ATTACHMENT_MAX_BYTES, UploadRequest, file_name, mime_type_for};
use crate::new_thread::{NewThreadLoaded, NewThreadRequest};
use crate::palette::{
    FILE_SEARCH_LIMIT, PaletteLoaded, PaletteRequest, THREAD_SEARCH_LIMIT_PER_GROUP,
};
use crate::platform::Platform;
use crate::time::now_ms;
use crate::typeahead::{TypeaheadLoaded, TypeaheadRequest};
use crate::ui;
use bb_wire::project::UploadedPromptAttachment;

pub const TICK: Duration = Duration::from_millis(200);

pub struct EffectRunner<A: Api, P: Platform> {
    store: Store,
    api: Arc<A>,
    platform: Arc<P>,
    tx: mpsc::UnboundedSender<Action>,
    _sidebar: Interest,
    thread: Option<(String, Interest)>,
    palette_tasks: HashMap<&'static str, tokio::task::JoinHandle<()>>,
}

impl<A: Api, P: Platform> EffectRunner<A, P> {
    pub fn new(
        store: Store,
        api: Arc<A>,
        platform: Arc<P>,
        tx: mpsc::UnboundedSender<Action>,
    ) -> Self {
        let sidebar = store.watch_sidebar();
        Self {
            store,
            api,
            platform,
            tx,
            _sidebar: sidebar,
            thread: None,
            palette_tasks: HashMap::new(),
        }
    }

    pub fn load_keymap(&self) {
        let api = self.api.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = api.keybinding_overrides().await;
            let _ = tx.send(Action::KeymapLoaded(result));
        });
    }

    pub fn run(&mut self, effect: Effect) {
        match effect {
            Effect::Mutate(mutation) => {
                let api = self.api.clone();
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let result = api.mutate(mutation.clone()).await;
                    let _ = tx.send(Action::MutationFinished { mutation, result });
                });
            }
            Effect::WatchThread(None) => self.thread = None,
            Effect::WatchThread(Some(thread_id)) => {
                if self.thread.as_ref().map(|(id, _)| id) != Some(&thread_id) {
                    let interest = self.store.watch_thread(&thread_id);
                    self.thread = Some((thread_id, interest));
                }
            }
            Effect::RefreshSidebar => self.store.refresh(ResourceKey::Sidebar),
            Effect::RefreshAll => self.store.refresh_all(),
            Effect::LoadOlder(thread_id) => self.store.load_older_timeline_rows(&thread_id),
            Effect::LoadTurnDetails(key) => self.store.load_turn_details(key),
            Effect::LoadOlderTurnDetails(key) => self.store.load_older_turn_details(key),
            Effect::LoadDeferred { thread_id, row } => {
                self.store.load_deferred_content(&thread_id, &row);
            }
            Effect::Copy(text) => {
                let (message, level) = match self.platform.copy(&text) {
                    Ok(()) => (
                        format!("Copied {} characters", text.chars().count()),
                        ToastLevel::Info,
                    ),
                    Err(error) => (format!("Copy failed: {error}"), ToastLevel::Error),
                };
                let _ = self.tx.send(Action::Notify { message, level });
            }
            Effect::OpenExternal(target) => {
                let (message, level) = match self.platform.open(&target) {
                    Ok(()) => (format!("Opening {target}"), ToastLevel::Info),
                    Err(error) => (
                        format!("Failed to open {target}: {error}"),
                        ToastLevel::Error,
                    ),
                };
                let _ = self.tx.send(Action::Notify { message, level });
            }
            Effect::Refresh(key) => self.store.refresh(key),
            Effect::LoadDraft(thread_id) => {
                let raw = self.platform.load_draft(&thread_id);
                let _ = self.tx.send(Action::DraftLoaded { thread_id, raw });
            }
            Effect::SaveDraft { thread_id, raw } => {
                if let Err(error) = self.platform.save_draft(&thread_id, raw.as_deref()) {
                    let _ = self.tx.send(Action::Notify {
                        message: format!("Failed to save draft: {error}"),
                        level: ToastLevel::Error,
                    });
                }
            }
            Effect::FetchExecutionOptions { thread_id, query } => {
                let api = self.api.clone();
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let result = api.execution_options(query).await;
                    let _ = tx.send(Action::ExecutionOptionsLoaded { thread_id, result });
                });
            }
            Effect::FetchDefaults(thread_id) => {
                let api = self.api.clone();
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let result = api.default_execution_options(thread_id.clone()).await;
                    let _ = tx.send(Action::DefaultsLoaded { thread_id, result });
                });
            }
            Effect::FetchPromptHistory(thread_id) => {
                let api = self.api.clone();
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let result = api.prompt_history(thread_id.clone()).await;
                    let _ = tx.send(Action::PromptHistoryLoaded { thread_id, result });
                });
            }
            Effect::NewThread {
                generation,
                request,
            } => {
                let api = self.api.clone();
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let loaded = run_new_thread_request(api.as_ref(), request).await;
                    let _ = tx.send(Action::NewThreadLoaded { generation, loaded });
                });
            }
            Effect::Palette {
                generation,
                request,
            } => {
                let api = self.api.clone();
                let tx = self.tx.clone();
                let slot = request.slot();
                let delay = request.debounce();
                let task = tokio::spawn(async move {
                    if !delay.is_zero() {
                        tokio::time::sleep(delay).await;
                    }
                    let loaded = run_palette_request(api.as_ref(), request).await;
                    let _ = tx.send(Action::PaletteLoaded { generation, loaded });
                });
                if let Some(slot) = slot
                    && let Some(previous) = self.palette_tasks.insert(slot, task)
                {
                    previous.abort();
                }
            }
            Effect::Typeahead(request) => {
                let api = self.api.clone();
                let tx = self.tx.clone();
                let slot = request.slot();
                let delay = request.debounce();
                let task = tokio::spawn(async move {
                    if !delay.is_zero() {
                        tokio::time::sleep(delay).await;
                    }
                    let loaded = run_typeahead_request(api.as_ref(), request).await;
                    let _ = tx.send(Action::TypeaheadLoaded(loaded));
                });
                if let Some(slot) = slot
                    && let Some(previous) = self.palette_tasks.insert(slot, task)
                {
                    previous.abort();
                }
            }
            Effect::Upload(request) => {
                let api = self.api.clone();
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let result = upload_attachment(api.as_ref(), &request).await;
                    let _ = tx.send(Action::AttachmentUploaded {
                        target: request.target,
                        path: request.path,
                        result,
                    });
                });
            }
            Effect::SavePaletteRecents(recents) => {
                if let Err(error) = self.platform.save_palette_recents(&recents) {
                    tracing::debug!(%error, "failed to save palette recents");
                }
            }
            Effect::Quit => {}
        }
    }

    pub fn watched_thread(&self) -> Option<&str> {
        self.thread.as_ref().map(|(id, _)| id.as_str())
    }
}

pub async fn run_new_thread_request<A: Api>(api: &A, request: NewThreadRequest) -> NewThreadLoaded {
    match request {
        NewThreadRequest::Providers { project_id } => {
            let result = api
                .environment_providers(SystemEnvironmentProvidersQuery {
                    project_id: project_id.clone(),
                    host_id: None,
                })
                .await;
            NewThreadLoaded::Providers { project_id, result }
        }
        NewThreadRequest::Config => NewThreadLoaded::Config(api.creation_config().await),
        NewThreadRequest::ProjectDefaults { project_id } => {
            let result = api
                .project_default_execution_options(project_id.clone())
                .await;
            NewThreadLoaded::ProjectDefaults { project_id, result }
        }
        NewThreadRequest::Branches { project_id, query } => {
            let host_id = query.host_id.clone();
            let result = api
                .branch_options(project_id.clone(), query)
                .await
                .map(Box::new);
            NewThreadLoaded::Branches {
                project_id,
                host_id,
                result,
            }
        }
        NewThreadRequest::Catalog { query } => {
            let result = api.execution_options(query.clone()).await.map(Box::new);
            NewThreadLoaded::Catalog { query, result }
        }
        NewThreadRequest::Create(request) => {
            NewThreadLoaded::Created(api.create_thread(*request).await.map(Box::new))
        }
    }
}

pub async fn run_palette_request<A: Api>(api: &A, request: PaletteRequest) -> PaletteLoaded {
    match request {
        PaletteRequest::Contributions => {
            PaletteLoaded::Contributions(api.plugin_contributions().await)
        }
        PaletteRequest::SearchThreads { query } => {
            let result = api
                .search_threads(ThreadSearchQuery {
                    query: query.clone(),
                    limit_per_group: Some(THREAD_SEARCH_LIMIT_PER_GROUP),
                })
                .await;
            PaletteLoaded::Threads { query, result }
        }
        PaletteRequest::Paths { source, query } => {
            let result = api
                .workspace_paths(
                    source,
                    WorkspacePathsQuery {
                        host_id: None,
                        environment_id: None,
                        query: Some(query.clone()),
                        limit: Some(FILE_SEARCH_LIMIT),
                        include_files: true,
                        include_directories: false,
                    },
                )
                .await;
            PaletteLoaded::Paths { query, result }
        }
        PaletteRequest::File { source, path } => {
            let result = api.workspace_file(source, path.clone()).await;
            PaletteLoaded::File { path, result }
        }
        PaletteRequest::RunCli { plugin_id, request } => {
            let label = std::iter::once(plugin_id.clone())
                .chain(request.argv.iter().cloned())
                .collect::<Vec<_>>()
                .join(" ");
            let result = api.run_plugin_cli(plugin_id, request).await;
            PaletteLoaded::Cli { label, result }
        }
    }
}

pub async fn run_typeahead_request<A: Api>(api: &A, request: TypeaheadRequest) -> TypeaheadLoaded {
    let paths_query = |query: &str| WorkspacePathsQuery {
        host_id: None,
        environment_id: None,
        query: Some(query.to_owned()),
        limit: Some(TypeaheadRequest::path_limit()),
        include_files: true,
        include_directories: true,
    };
    match request {
        TypeaheadRequest::MentionProviders => {
            TypeaheadLoaded::MentionProviders(api.mention_providers().await)
        }
        TypeaheadRequest::WorkspacePaths { source, query } => {
            let result = api.workspace_paths(source, paths_query(&query)).await;
            TypeaheadLoaded::WorkspacePaths { query, result }
        }
        TypeaheadRequest::StoragePaths { thread_id, query } => {
            let result = api
                .thread_storage_paths(thread_id, paths_query(&query))
                .await;
            TypeaheadLoaded::StoragePaths { query, result }
        }
        TypeaheadRequest::PluginMentions(search) => {
            let trigger = search.trigger.clone();
            let query = search.q.clone();
            let result = api.plugin_mentions(search).await;
            TypeaheadLoaded::PluginMentions {
                trigger,
                query,
                result,
            }
        }
        TypeaheadRequest::Commands(key) => {
            let result = api
                .project_commands(key.project_id.clone(), key.query())
                .await;
            TypeaheadLoaded::Commands { key, result }
        }
    }
}

pub async fn upload_attachment<A: Api>(
    api: &A,
    request: &UploadRequest,
) -> Result<UploadedPromptAttachment, String> {
    let path = request.path.clone();
    let bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, String> {
        let metadata = std::fs::metadata(&path).map_err(|error| error.to_string())?;
        if !metadata.is_file() {
            return Err("not a regular file".into());
        }
        if metadata.len() > ATTACHMENT_MAX_BYTES {
            return Err(format!(
                "{:.1}MB is over the {}MB attachment limit",
                metadata.len() as f64 / (1024.0 * 1024.0),
                ATTACHMENT_MAX_BYTES / (1024 * 1024)
            ));
        }
        std::fs::read(&path).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())??;
    api.upload_attachment(AttachmentUpload {
        project_id: request.project_id.clone(),
        file_name: file_name(&request.path),
        mime_type: mime_type_for(&request.path),
        bytes,
    })
    .await
}

pub fn terminal_action(event: std::io::Result<Event>) -> Option<Action> {
    match event {
        Ok(Event::Key(key)) if key.kind != KeyEventKind::Release => Some(Action::Key(key)),
        Ok(Event::Resize(_, _)) => Some(Action::Resize),
        Ok(Event::Paste(text)) => Some(Action::Paste(text)),
        _ => None,
    }
}

pub fn terminal_events() -> impl Stream<Item = Action> + Unpin {
    crossterm::event::EventStream::new()
        .filter_map(|event| std::future::ready(terminal_action(event)))
}

pub async fn run<B, A, P, S>(
    terminal: &mut Terminal<B>,
    state: &mut AppState,
    store: Store,
    api: Arc<A>,
    platform: Arc<P>,
    mut events: S,
) -> Result<()>
where
    B: TerminalBackend,
    B::Error: Send + Sync + 'static,
    A: Api,
    P: Platform,
    S: Stream<Item = Action> + Unpin,
{
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut snapshots = store.subscribe();
    state.palette_recents = platform.load_palette_recents();
    let mut runner = EffectRunner::new(store, api, platform, tx);
    runner.load_keymap();
    let initial = snapshots.borrow_and_update().clone();
    update(state, Action::Snapshot(initial));
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut events_open = true;

    while !state.quit {
        terminal
            .draw(|frame| ui::render(state, frame))
            .map_err(|error| anyhow!(error))?;
        let action = tokio::select! {
            biased;
            Some(action) = rx.recv() => action,
            changed = snapshots.changed() => match changed {
                Ok(()) => Action::Snapshot(snapshots.borrow_and_update().clone()),
                Err(_) => return Err(anyhow!("store stopped")),
            },
            event = events.next(), if events_open => match event {
                Some(action) => action,
                None => {
                    events_open = false;
                    continue;
                }
            },
            _ = tick.tick() => Action::Tick { now: Instant::now(), now_ms: now_ms() },
        };
        dispatch(state, &mut runner, action);
        while let Ok(action) = rx.try_recv() {
            dispatch(state, &mut runner, action);
        }
    }
    Ok(())
}

fn dispatch<A: Api, P: Platform>(
    state: &mut AppState,
    runner: &mut EffectRunner<A, P>,
    action: Action,
) {
    for effect in update(state, action) {
        runner.run(effect);
    }
}
