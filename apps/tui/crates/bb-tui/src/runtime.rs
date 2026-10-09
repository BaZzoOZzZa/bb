use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use bb_client::RealtimeEvent;
use bb_store::{Interest, ResourceKey, Store};
use crossterm::event::{Event, KeyEventKind};
use futures::{Stream, StreamExt};
use ratatui::Terminal;
use ratatui::backend::Backend as TerminalBackend;
use tokio::sync::mpsc;

use bb_wire::project::WorkspacePathsQuery;
use bb_wire::system::SystemEnvironmentProvidersQuery;
use bb_wire::thread::ThreadSearchQuery;

use crate::api::{Api, AttachmentUpload, TerminalHandle};
use crate::app::{Action, AppState, Effect, ToastLevel, update};
use crate::attach::{ATTACHMENT_MAX_BYTES, UploadRequest, file_name, mime_type_for};
use crate::automations::{AutomationLoaded, AutomationRequest};
use crate::git_diff::diff_query;
use crate::new_thread::{NewThreadLoaded, NewThreadRequest};
use crate::palette::{
    FILE_SEARCH_LIMIT, PaletteLoaded, PaletteRequest, THREAD_SEARCH_LIMIT_PER_GROUP,
};
use crate::panel::{PanelLoaded, PanelRequest};
use crate::platform::{Platform, RawEvent};
use crate::settings::{SettingsLoaded, SettingsRequest};
use crate::terminal::{TerminalLoaded, TerminalRequest, TerminalScope};
use crate::terminal_input::{DetachState, RESET_MODES};
use crate::time::now_ms;
use crate::typeahead::{TypeaheadLoaded, TypeaheadRequest};
use crate::ui;
use bb_wire::environment::{EnvironmentDiffPatchRequest, EnvironmentStatusQuery};
use bb_wire::project::UploadedPromptAttachment;

pub const TICK: Duration = Duration::from_millis(200);

pub struct EffectRunner<A: Api, P: Platform> {
    store: Store,
    api: Arc<A>,
    platform: Arc<P>,
    tx: mpsc::UnboundedSender<Action>,
    _sidebar: Interest,
    thread: Option<(String, Interest)>,
    splits: HashMap<String, Interest>,
    palette_tasks: HashMap<&'static str, tokio::task::JoinHandle<()>>,
    terminal_links: HashMap<String, (u64, Arc<dyn TerminalHandle>)>,
    fullscreen: Option<String>,
    redraw: bool,
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
            splits: HashMap::new(),
            palette_tasks: HashMap::new(),
            terminal_links: HashMap::new(),
            fullscreen: None,
            redraw: false,
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
            Effect::WatchSplits(thread_ids) => {
                self.splits.retain(|id, _| thread_ids.contains(id));
                for thread_id in thread_ids {
                    if !self.splits.contains_key(&thread_id) {
                        let interest = self.store.watch_thread(&thread_id);
                        self.splits.insert(thread_id, interest);
                    }
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
            Effect::Panel(request) => {
                let api = self.api.clone();
                let tx = self.tx.clone();
                let slot = request.slot();
                let task = tokio::spawn(async move {
                    let loaded = run_panel_request(api.as_ref(), request).await;
                    let _ = tx.send(Action::PanelLoaded(loaded));
                });
                if let Some(slot) = slot
                    && let Some(previous) = self.palette_tasks.insert(slot, task)
                {
                    previous.abort();
                }
            }
            Effect::EditFile { path, line } => {
                let result = self.platform.edit_file(&path, line);
                self.redraw = true;
                if let Err(error) = result {
                    let _ = self.tx.send(Action::Notify {
                        message: format!("Cannot open editor: {error}"),
                        level: ToastLevel::Error,
                    });
                }
            }
            Effect::SavePaletteRecents(recents) => {
                if let Err(error) = self.platform.save_palette_recents(&recents) {
                    tracing::debug!(%error, "failed to save palette recents");
                }
            }
            Effect::Alert(alert) => {
                if let Err(error) = self.platform.alert(&alert) {
                    tracing::debug!(%error, "notification alert failed");
                }
            }
            Effect::SetTitle(title) => {
                if let Err(error) = self.platform.set_title(&title) {
                    tracing::debug!(%error, "failed to set the terminal title");
                }
            }
            Effect::FetchNotificationInteractions(thread_id) => {
                let api = self.api.clone();
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let result = api.thread_interactions(thread_id.clone()).await;
                    let _ = tx.send(Action::NotificationInteractions { thread_id, result });
                });
            }
            Effect::Terminal(request) => self.run_terminal(request),
            Effect::Settings(request) => {
                let api = self.api.clone();
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let loaded = run_settings_request(api.as_ref(), request).await;
                    let _ = tx.send(Action::SettingsLoaded(loaded));
                });
            }
            Effect::Automations(request) => {
                let api = self.api.clone();
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let loaded = run_automation_request(api.as_ref(), request).await;
                    let _ = tx.send(Action::AutomationsLoaded(loaded));
                });
            }
            Effect::Quit => {
                for (_, (_, handle)) in self.terminal_links.drain() {
                    handle.close();
                }
            }
        }
    }

    fn terminal_link(&self, terminal_id: &str, link: u64) -> Option<&Arc<dyn TerminalHandle>> {
        self.terminal_links
            .get(terminal_id)
            .filter(|(current, _)| *current == link)
            .map(|(_, handle)| handle)
    }

    fn run_terminal(&mut self, request: TerminalRequest) {
        match request {
            TerminalRequest::List { scope } => {
                let api = self.api.clone();
                let tx = self.tx.clone();
                let task = tokio::spawn(async move {
                    let result = list_terminals(api.as_ref(), &scope).await;
                    let _ = tx.send(Action::TerminalLoaded(TerminalLoaded::List {
                        scope,
                        result,
                    }));
                });
                if let Some(previous) = self.palette_tasks.insert("terminal-list", task) {
                    previous.abort();
                }
            }
            TerminalRequest::Create { scope, request } => {
                let api = self.api.clone();
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let result = api.create_terminal(request).await;
                    let _ = tx.send(Action::TerminalLoaded(TerminalLoaded::Created {
                        scope,
                        result,
                    }));
                });
            }
            TerminalRequest::Close {
                terminal_id,
                request,
            } => {
                let api = self.api.clone();
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let result = api.close_terminal(terminal_id.clone(), request).await;
                    let _ = tx.send(Action::TerminalLoaded(TerminalLoaded::Closed {
                        terminal_id,
                        result,
                    }));
                });
            }
            TerminalRequest::Connect {
                terminal_id,
                link,
                since_seq,
                size,
                visible,
            } => {
                if let Some((_, previous)) = self.terminal_links.remove(&terminal_id) {
                    previous.close();
                }
                match self.api.connect_terminal(&terminal_id, since_seq) {
                    Ok(connection) => {
                        if let Some((cols, rows)) = size {
                            connection.handle.resize(cols, rows);
                        }
                        if !visible {
                            connection.handle.set_visible(false);
                        }
                        self.terminal_links
                            .insert(terminal_id.clone(), (link, connection.handle));
                        let tx = self.tx.clone();
                        let mut events = connection.events;
                        tokio::spawn(async move {
                            while let Some(event) = events.recv().await {
                                let action = Action::TerminalEvent {
                                    terminal_id: terminal_id.clone(),
                                    link,
                                    event,
                                };
                                if tx.send(action).is_err() {
                                    break;
                                }
                            }
                        });
                    }
                    Err(error) => {
                        let _ =
                            self.tx
                                .send(Action::TerminalLoaded(TerminalLoaded::ConnectFailed {
                                    terminal_id,
                                    link,
                                    error,
                                }));
                    }
                }
            }
            TerminalRequest::Disconnect { terminal_id, link } => {
                if self.terminal_link(&terminal_id, link).is_some()
                    && let Some((_, handle)) = self.terminal_links.remove(&terminal_id)
                {
                    handle.close();
                }
            }
            TerminalRequest::Input {
                terminal_id,
                link,
                bytes,
            } => {
                if let Some(handle) = self.terminal_link(&terminal_id, link) {
                    handle.send_input(&bytes);
                }
            }
            TerminalRequest::Resize {
                terminal_id,
                link,
                cols,
                rows,
            } => {
                if let Some(handle) = self.terminal_link(&terminal_id, link) {
                    handle.resize(cols, rows);
                }
            }
            TerminalRequest::Ack {
                terminal_id,
                link,
                next_seq,
            } => {
                if let Some(handle) = self.terminal_link(&terminal_id, link) {
                    handle.acknowledge_output(next_seq);
                }
            }
            TerminalRequest::Visible {
                terminal_id,
                link,
                visible,
            } => {
                if let Some(handle) = self.terminal_link(&terminal_id, link) {
                    handle.set_visible(visible);
                }
            }
            TerminalRequest::Fullscreen { terminal_id, link } => {
                if self.terminal_link(&terminal_id, link).is_some() {
                    self.fullscreen = Some(terminal_id);
                }
            }
        }
    }

    pub fn take_fullscreen(&mut self) -> Option<String> {
        self.fullscreen.take()
    }

    pub fn take_redraw(&mut self) -> bool {
        std::mem::take(&mut self.redraw)
    }

    pub fn watched_thread(&self) -> Option<&str> {
        self.thread.as_ref().map(|(id, _)| id.as_str())
    }

    pub fn watched_splits(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.splits.keys().cloned().collect();
        ids.sort();
        ids
    }
}

pub async fn list_terminals<A: Api>(
    api: &A,
    scope: &TerminalScope,
) -> Result<Vec<bb_wire::terminal::TerminalSession>, String> {
    let mut sessions = Vec::new();
    for (index, query) in scope.queries().into_iter().enumerate() {
        match api.list_terminals(query).await {
            Ok(found) => sessions.extend(found),
            Err(error) if index == 0 => return Err(error),
            Err(error) => tracing::debug!(%error, "environment terminals unavailable"),
        }
    }
    Ok(sessions)
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

pub async fn run_settings_request<A: Api>(api: &A, request: SettingsRequest) -> SettingsLoaded {
    match request {
        SettingsRequest::Config => SettingsLoaded::Config(api.system_config().await.map(Box::new)),
        SettingsRequest::Version => {
            SettingsLoaded::Version(api.server_version().await.map(Box::new))
        }
        SettingsRequest::Catalog => SettingsLoaded::Catalog(api.provider_catalog().await),
        SettingsRequest::Models { provider_id } => {
            let result = api
                .execution_options(bb_wire::system::SystemExecutionOptionsQuery {
                    provider_id: Some(provider_id.clone()),
                    ..bb_wire::system::SystemExecutionOptionsQuery::default()
                })
                .await
                .map(|options| options.models);
            SettingsLoaded::Models {
                provider_id,
                result,
            }
        }
        SettingsRequest::HostClis { host_id } => {
            let result = api.host_provider_clis(host_id.clone()).await;
            SettingsLoaded::HostClis { host_id, result }
        }
        SettingsRequest::ProjectDefaults { project_id } => {
            let result = api
                .project_default_execution_options(project_id.clone())
                .await;
            SettingsLoaded::ProjectDefaults { project_id, result }
        }
        SettingsRequest::Plugins => SettingsLoaded::Plugins(api.list_plugins().await),
        SettingsRequest::PluginSettings { plugin_id } => {
            let result = api.plugin_settings(plugin_id.clone()).await;
            SettingsLoaded::PluginSettings { plugin_id, result }
        }
        SettingsRequest::Archived(query) => {
            let result = api.list_threads(query.list_query()).await;
            SettingsLoaded::Archived { query, result }
        }
        SettingsRequest::ChildSummary { thread_id } => {
            let result = api.thread_child_summary(thread_id.clone()).await;
            SettingsLoaded::ChildSummary { thread_id, result }
        }
        SettingsRequest::Mutate(mutation) => {
            let result = api.settings_mutation(mutation.clone()).await;
            SettingsLoaded::Mutated { mutation, result }
        }
    }
}

async fn automations_rpc<A: Api, T: serde::de::DeserializeOwned>(
    api: &A,
    method: &str,
    input: bb_wire::common::JsonValue,
) -> Result<T, String> {
    let value = api
        .plugin_rpc(
            crate::automation_data::PLUGIN_ID.to_owned(),
            method.to_owned(),
            input,
        )
        .await?;
    serde_json::from_value(value).map_err(|error| format!("unexpected {method} response: {error}"))
}

pub async fn run_automation_request<A: Api>(
    api: &A,
    request: AutomationRequest,
) -> AutomationLoaded {
    use crate::automation_data::{Automation, OverviewResponse, RunPage};
    match request {
        AutomationRequest::Overview => AutomationLoaded::Overview(
            automations_rpc::<A, OverviewResponse>(
                api,
                "automations_overview",
                bb_wire::common::JsonValue::Null,
            )
            .await
            .map(|response| response.automations),
        ),
        AutomationRequest::Detail(route) => {
            let input = serde_json::json!({
                "projectId": route.project_id,
                "automationId": route.automation_id,
            });
            let result = automations_rpc::<A, Automation>(api, "automations_get", input)
                .await
                .map(Box::new);
            AutomationLoaded::Detail { route, result }
        }
        AutomationRequest::Runs { route, cursor } => {
            let mut input = serde_json::json!({
                "projectId": route.project_id,
                "automationId": route.automation_id,
            });
            if let (Some(cursor), Some(object)) = (cursor.as_ref(), input.as_object_mut()) {
                object.insert("cursor".into(), serde_json::json!(cursor));
            }
            let result = automations_rpc::<A, RunPage>(api, "automations_runs", input).await;
            AutomationLoaded::Runs {
                route,
                cursor,
                result,
            }
        }
        AutomationRequest::Providers => AutomationLoaded::Providers(api.provider_catalog().await),
        AutomationRequest::Models { provider_id } => {
            let result = api
                .execution_options(bb_wire::system::SystemExecutionOptionsQuery {
                    provider_id: Some(provider_id.clone()),
                    ..bb_wire::system::SystemExecutionOptionsQuery::default()
                })
                .await;
            AutomationLoaded::Models {
                provider_id,
                result,
            }
        }
        AutomationRequest::ProjectDefaults { project_id } => {
            let result = api
                .project_default_execution_options(project_id.clone())
                .await;
            AutomationLoaded::ProjectDefaults { project_id, result }
        }
        AutomationRequest::Mutate(mutation) => {
            let result = api
                .plugin_rpc(
                    crate::automation_data::PLUGIN_ID.to_owned(),
                    mutation.method().to_owned(),
                    mutation.input(),
                )
                .await;
            AutomationLoaded::Mutated { mutation, result }
        }
    }
}

pub async fn run_panel_request<A: Api>(api: &A, request: PanelRequest) -> PanelLoaded {
    match request {
        PanelRequest::Status {
            environment_id,
            merge_base_branch,
        } => {
            let result = api
                .environment_status(
                    environment_id.clone(),
                    EnvironmentStatusQuery {
                        merge_base_branch: merge_base_branch.clone(),
                    },
                )
                .await;
            PanelLoaded::Status {
                environment_id,
                merge_base_branch,
                result,
            }
        }
        PanelRequest::PullRequest { environment_id } => {
            let result = api.environment_pull_request(environment_id.clone()).await;
            PanelLoaded::PullRequest {
                environment_id,
                result,
            }
        }
        PanelRequest::DiffFiles {
            environment_id,
            target,
        } => {
            let result = api
                .environment_diff_files(environment_id.clone(), diff_query(&target))
                .await;
            PanelLoaded::DiffFiles {
                environment_id,
                target,
                result,
            }
        }
        PanelRequest::DiffPatches {
            environment_id,
            target,
            paths,
        } => {
            let result = api
                .environment_diff_patch(
                    environment_id.clone(),
                    EnvironmentDiffPatchRequest {
                        target: target.clone(),
                        paths: paths.clone(),
                    },
                )
                .await;
            PanelLoaded::DiffPatches {
                environment_id,
                target,
                paths,
                result,
            }
        }
        PanelRequest::File {
            environment_id,
            path,
            query,
        } => {
            let result = api
                .environment_diff_file(environment_id.clone(), query)
                .await;
            PanelLoaded::File {
                environment_id,
                path,
                result,
            }
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
        Ok(Event::Mouse(mouse)) => Some(Action::Mouse(mouse)),
        Ok(Event::FocusGained) => Some(Action::TerminalFocus(true)),
        Ok(Event::FocusLost) => Some(Action::TerminalFocus(false)),
        _ => None,
    }
}

pub fn terminal_events() -> impl Stream<Item = Action> + Unpin {
    crossterm::event::EventStream::new()
        .filter_map(|event| std::future::ready(terminal_action(event)))
}

pub fn realtime_actions(
    events: tokio::sync::broadcast::Receiver<RealtimeEvent>,
) -> impl Stream<Item = Action> + Unpin {
    Box::pin(futures::stream::unfold(events, |mut events| async move {
        loop {
            match events.recv().await {
                Ok(RealtimeEvent::PluginSignal {
                    plugin_id,
                    channel,
                    payload,
                }) => {
                    let action = Action::PluginSignal {
                        plugin_id,
                        channel,
                        payload,
                    };
                    return Some((action, events));
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    }))
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
    dispatch(state, &mut runner, Action::Snapshot(initial));
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut events_open = true;

    while !state.quit {
        terminal
            .draw(|frame| ui::render(state, frame))
            .map_err(|error| anyhow!(error))?;
        let deadline = state.keys.deadline();
        let which_key = async move {
            match deadline {
                Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
                None => std::future::pending::<()>().await,
            }
        };
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
            () = which_key => Action::Tick { now: Instant::now(), now_ms: now_ms() },
            _ = tick.tick() => Action::Tick { now: Instant::now(), now_ms: now_ms() },
        };
        dispatch(state, &mut runner, action);
        if state.keys.needs_arming() {
            dispatch(
                state,
                &mut runner,
                Action::Tick {
                    now: Instant::now(),
                    now_ms: now_ms(),
                },
            );
        }
        while let Ok(action) = rx.try_recv() {
            dispatch(state, &mut runner, action);
        }
        if let Some(terminal_id) = runner.take_fullscreen() {
            run_fullscreen(state, &mut runner, &mut rx, &mut snapshots, terminal_id).await;
            terminal.clear().map_err(|error| anyhow!(error))?;
        }
        if runner.take_redraw() {
            terminal.clear().map_err(|error| anyhow!(error))?;
        }
    }
    Ok(())
}

fn fullscreen_output(state: &AppState, action: &Action, terminal_id: &str) -> Option<Vec<u8>> {
    let Action::TerminalEvent {
        terminal_id: id,
        link,
        event:
            bb_client::TerminalEvent::Message(bb_wire::terminal::TerminalServerMessage::Output {
                chunk,
            }),
    } = action
    else {
        return None;
    };
    let view = state.terminals.views.get(id)?;
    if id != terminal_id || view.link != Some(*link) || chunk.seq < view.next_seq {
        return None;
    }
    chunk.decode().ok()
}

pub async fn run_fullscreen<A: Api, P: Platform>(
    state: &mut AppState,
    runner: &mut EffectRunner<A, P>,
    rx: &mut mpsc::UnboundedReceiver<Action>,
    snapshots: &mut tokio::sync::watch::Receiver<Arc<bb_store::StoreSnapshot>>,
    terminal_id: String,
) {
    let platform = runner.platform.clone();
    let Some(mut input) = platform.raw_input() else {
        dispatch(
            state,
            runner,
            Action::FullscreenEnded("Fullscreen attach needs an interactive terminal".into()),
        );
        return;
    };
    if let Err(error) = platform.suspend_screen() {
        drop(input);
        let _ = platform.resume_screen();
        dispatch(
            state,
            runner,
            Action::FullscreenEnded(format!("Cannot attach fullscreen: {error}")),
        );
        return;
    }
    if let Some(screen) = state.fullscreen_screen() {
        let _ = platform.write_raw(&screen);
    }
    if let Some((cols, rows)) = platform.screen_size() {
        dispatch(state, runner, Action::FullscreenResize { cols, rows });
    }
    let title = state
        .terminals
        .views
        .get(&terminal_id)
        .map_or_else(|| "the terminal".to_owned(), |view| view.title());
    let mut detach = DetachState::default();
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let message = loop {
        if state.terminals.fullscreen.as_deref() != Some(terminal_id.as_str()) {
            break state
                .terminals
                .views
                .get(&terminal_id)
                .and_then(|view| view.notice.clone())
                .unwrap_or_else(|| format!("{title} ended"));
        }
        tokio::select! {
            biased;
            Some(action) = rx.recv() => {
                if let Some(bytes) = fullscreen_output(state, &action, &terminal_id) {
                    let _ = platform.write_raw(&bytes);
                }
                dispatch(state, runner, action);
            }
            event = input.next() => match event {
                None => break "Terminal input closed".to_owned(),
                Some(RawEvent::Resize { cols, rows }) => {
                    dispatch(state, runner, Action::FullscreenResize { cols, rows });
                }
                Some(RawEvent::Bytes(bytes)) => {
                    let split = detach.feed(&bytes);
                    if !split.forward.is_empty() {
                        dispatch(state, runner, Action::TerminalRawInput(split.forward));
                    }
                    if split.detach {
                        break format!("Detached from {title}");
                    }
                }
            },
            changed = snapshots.changed() => match changed {
                Ok(()) => {
                    let snapshot = snapshots.borrow_and_update().clone();
                    dispatch(state, runner, Action::Snapshot(snapshot));
                }
                Err(_) => break "The store stopped".to_owned(),
            },
            _ = tick.tick() => dispatch(state, runner, Action::Tick { now: Instant::now(), now_ms: now_ms() }),
        }
    };
    drop(input);
    let _ = platform.write_raw(RESET_MODES);
    let resumed = platform.resume_screen();
    dispatch(state, runner, Action::FullscreenEnded(message));
    if let Err(error) = resumed {
        dispatch(
            state,
            runner,
            Action::Notify {
                message: format!("Failed to restore the screen: {error}"),
                level: ToastLevel::Error,
            },
        );
    }
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
