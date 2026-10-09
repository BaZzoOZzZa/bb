#![allow(dead_code)]

pub mod ws;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use bb_store::{
    Backend, FetchError, FetchResult, LatestTimeline, LoadedTimelineState, StoreSnapshot,
    TimelineEntry, TurnDetailsEntry, TurnDetailsKey,
};
use bb_tui::api::{Api, ApiFuture, AttachmentUpload, Mutation, MutationReply};
use bb_tui::app::{Action, AppState, Effect, ServerInfo, update};
use bb_tui::new_thread::CreationConfig;
use bb_tui::palette::FileSource;
use bb_tui::platform::Platform;
use bb_tui::ui;
use bb_wire::environment::{
    CreateThreadEnvironmentArgs, DiffPatchEntry, EnvironmentDiffFilesResponse,
    EnvironmentDiffPatchRequest, EnvironmentDiffPatchResponse, EnvironmentDiffQuery, Host,
};
use bb_wire::interaction::PendingInteraction;
use bb_wire::plugin::{
    PluginCliContribution, PluginCliRequest, PluginCliResult, PluginMentionProviderContribution,
    PluginMentionSearchGroup, PluginMentionSearchQuery,
};
use bb_wire::project::{
    Project, ProjectBranchesQuery, ProjectBranchesResponse, ProjectCommandsQuery, ProjectKind,
    ProjectWithThreadsResponse, ProviderCommand, SidebarBootstrapResponse, UploadedAttachmentType,
    UploadedPromptAttachment, WorkspacePathEntry, WorkspacePathListResponse, WorkspacePathsQuery,
};
use bb_wire::prompt::{
    ProjectExecutionDefaults, PromptHistoryEntry, ResolvedThreadExecutionOptions,
};
use bb_wire::queue::{SendMessageResponse, ThreadQueuedMessage};
use bb_wire::system::{
    AppKeybindingOverride, SystemAttentionResponse, SystemEnvironmentProvider,
    SystemEnvironmentProvidersQuery, SystemExecutionOptionsQuery, SystemExecutionOptionsResponse,
};
use bb_wire::thread::{
    CreateThreadRequest, ThreadListEntry, ThreadResponse, ThreadSearchQuery, ThreadSearchResponse,
    ThreadStatus,
};
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
    pub hosts: Mutex<Vec<Host>>,
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
        Ok(self.hosts.lock().unwrap().clone())
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

#[derive(Default)]
pub struct SettingsFake {
    pub config: Mutex<bb_wire::system::SystemConfigResponse>,
    pub version: bb_wire::system::SystemVersionResponse,
    pub catalog: Mutex<Vec<bb_wire::system::SystemProviderCatalogEntry>>,
    pub host_clis: HashMap<String, bb_wire::environment::HostProviderCliStatusResponse>,
    pub archived: Mutex<Vec<ThreadListEntry>>,
    pub thread_queries: Mutex<Vec<bb_wire::thread::ThreadListQuery>>,
    pub child_counts: HashMap<String, u32>,
    pub plugins: Mutex<Vec<bb_wire::plugin::InstalledPlugin>>,
    pub plugin_settings: Mutex<HashMap<String, bb_wire::plugin::PluginSettingsResponse>>,
    pub mutations: Mutex<Vec<bb_tui::settings::SettingsMutation>>,
    pub reject: Mutex<Option<String>>,
}

impl SettingsFake {
    pub fn recorded(&self) -> Vec<bb_tui::settings::SettingsMutation> {
        self.mutations.lock().unwrap().clone()
    }

    fn apply(
        &self,
        mutation: &bb_tui::settings::SettingsMutation,
    ) -> Result<bb_tui::settings::SettingsReply, String> {
        use bb_tui::settings::{SettingsMutation, SettingsReply};
        self.mutations.lock().unwrap().push(mutation.clone());
        if let Some(error) = self.reject.lock().unwrap().clone() {
            return Err(error);
        }
        Ok(match mutation {
            SettingsMutation::UpdateGeneral { settings, .. } => {
                let value = serde_json::Value::Object(settings.clone());
                self.config.lock().unwrap().general_settings = value.clone();
                SettingsReply::General(value)
            }
            SettingsMutation::SetProviderEnabled {
                provider_id,
                enabled,
            } => {
                let mut catalog = self.catalog.lock().unwrap();
                for entry in catalog.iter_mut() {
                    if &entry.id == provider_id {
                        entry.enabled = *enabled;
                    }
                }
                SettingsReply::Catalog(catalog.clone())
            }
            SettingsMutation::UnarchiveThread { thread_id }
            | SettingsMutation::DeleteThread { thread_id, .. } => {
                self.archived
                    .lock()
                    .unwrap()
                    .retain(|entry| &entry.thread.id != thread_id);
                SettingsReply::Done
            }
            SettingsMutation::SetPluginEnabled { plugin_id, enabled } => {
                for plugin in self.plugins.lock().unwrap().iter_mut() {
                    if &plugin.id == plugin_id {
                        plugin.enabled = *enabled;
                    }
                }
                SettingsReply::Done
            }
            SettingsMutation::UpdatePluginSetting {
                plugin_id,
                key,
                value,
                secret,
            } => {
                let mut all = self.plugin_settings.lock().unwrap();
                let settings = all.entry(plugin_id.clone()).or_default();
                let stored = if *secret {
                    serde_json::json!({"set": !value.as_str().unwrap_or("").is_empty()})
                } else {
                    value.clone()
                };
                settings.values.insert(key.clone(), stored);
                SettingsReply::PluginSettings(Box::new(settings.clone()))
            }
            _ => SettingsReply::Done,
        })
    }
}

#[derive(Default)]
pub struct AutomationsFake {
    pub automations: Mutex<Vec<serde_json::Value>>,
    pub runs: Mutex<HashMap<String, Vec<serde_json::Value>>>,
    pub calls: Mutex<Vec<(String, serde_json::Value)>>,
    pub reject: Mutex<Option<String>>,
}

pub const RUNS_PAGE: usize = 2;
const READ_METHODS: &[&str] = &[
    "automations_overview",
    "automations_get",
    "automations_runs",
];

impl AutomationsFake {
    pub fn calls(&self) -> Vec<(String, serde_json::Value)> {
        self.calls.lock().unwrap().clone()
    }

    pub fn mutations(&self) -> Vec<(String, serde_json::Value)> {
        self.calls()
            .into_iter()
            .filter(|(method, _)| !READ_METHODS.contains(&method.as_str()))
            .collect()
    }

    fn handle(
        &self,
        sidebar: &SidebarBootstrapResponse,
        method: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        use serde_json::json;
        self.calls
            .lock()
            .unwrap()
            .push((method.to_owned(), input.clone()));
        if !READ_METHODS.contains(&method)
            && let Some(error) = self.reject.lock().unwrap().clone()
        {
            return Err(error);
        }
        let id = input
            .get("automationId")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let mut automations = self.automations.lock().unwrap();
        let position = automations
            .iter()
            .position(|automation| automation["id"] == json!(id));
        let missing = || "Automation not found".to_owned();
        match method {
            "automations_overview" => {
                let entries: Vec<serde_json::Value> = automations
                    .iter()
                    .map(|automation| {
                        let project_id = automation["projectId"].as_str().unwrap_or_default();
                        let name = sidebar
                            .projects
                            .iter()
                            .chain(std::iter::once(&sidebar.personal_project))
                            .find(|project| project.project.id == project_id)
                            .map_or(project_id.to_owned(), |project| project.project.name.clone());
                        json!({"automation": automation, "project": {"id": project_id, "name": name}})
                    })
                    .collect();
                Ok(json!({"automations": entries}))
            }
            "automations_get" => position
                .map(|index| automations[index].clone())
                .ok_or_else(missing),
            "automations_runs" => {
                let runs = self
                    .runs
                    .lock()
                    .unwrap()
                    .get(&id)
                    .cloned()
                    .unwrap_or_default();
                let start: usize = input
                    .get("cursor")
                    .and_then(serde_json::Value::as_str)
                    .and_then(|cursor| cursor.parse().ok())
                    .unwrap_or(0);
                let end = (start + RUNS_PAGE).min(runs.len());
                let next = (end < runs.len()).then(|| end.to_string());
                Ok(json!({"runs": runs[start.min(end)..end], "nextCursor": next}))
            }
            "automations_create" => {
                let enabled = input["enabled"].as_bool().unwrap_or(true);
                let created = json!({
                    "id": "auto_new",
                    "projectId": input["projectId"],
                    "name": input["name"],
                    "enabled": enabled,
                    "trigger": input["trigger"],
                    "execution": input["execution"],
                    "origin": input["origin"],
                    "createdByThreadId": null,
                    "nextRunAt": if enabled { json!(NOW_MS + 60 * MINUTE) } else { json!(null) },
                    "lastRunAt": null,
                    "runCount": 0,
                    "lastRunStatus": null,
                    "lastRunThreadId": null,
                    "lastError": null,
                    "createdAt": NOW_MS,
                    "updatedAt": NOW_MS,
                });
                automations.push(created.clone());
                Ok(created)
            }
            "automations_update" => {
                let index = position.ok_or_else(missing)?;
                let automation = &mut automations[index];
                for key in ["name", "trigger"] {
                    if let Some(value) = input.get(key) {
                        automation[key] = value.clone();
                    }
                }
                if let Some(agent) = input.get("agent").and_then(serde_json::Value::as_object) {
                    for (key, value) in agent {
                        if key == "target" {
                            automation["execution"]["environment"] = value["environment"].clone();
                        } else {
                            automation["execution"][key] = value.clone();
                        }
                    }
                }
                Ok(automation.clone())
            }
            "automations_pause" | "automations_resume" => {
                let index = position.ok_or_else(missing)?;
                let enabled = method == "automations_resume";
                automations[index]["enabled"] = json!(enabled);
                automations[index]["nextRunAt"] = if enabled {
                    json!(NOW_MS + 60 * MINUTE)
                } else {
                    json!(null)
                };
                Ok(automations[index].clone())
            }
            "automations_delete" => {
                let index = position.ok_or_else(missing)?;
                automations.remove(index);
                Ok(json!({"ok": true}))
            }
            "automations_run" => {
                position.ok_or_else(missing)?;
                let run = json!({
                    "id": "run_new",
                    "automationId": id,
                    "runMode": "agent",
                    "threadId": "thr_run_new",
                    "status": "running",
                    "trigger": "manual",
                    "skipReason": null,
                    "error": null,
                    "output": null,
                    "exitCode": null,
                    "scheduledFor": NOW_MS,
                    "startedAt": NOW_MS,
                    "finishedAt": null,
                });
                self.runs
                    .lock()
                    .unwrap()
                    .entry(id)
                    .or_default()
                    .insert(0, run.clone());
                Ok(json!({"run": run}))
            }
            other => Err(format!(
                "plugin \"automations\" has no rpc method \"{other}\""
            )),
        }
    }
}

pub struct MockApi {
    pub backend: Arc<FakeBackend>,
    pub settings: SettingsFake,
    pub automations: AutomationsFake,
    pub mutations: Mutex<Vec<Mutation>>,
    pub overrides: Vec<AppKeybindingOverride>,
    pub fail: bool,
    pub execution_options: SystemExecutionOptionsResponse,
    pub defaults: Option<ResolvedThreadExecutionOptions>,
    pub history: Vec<PromptHistoryEntry>,
    pub execution_queries: Mutex<Vec<SystemExecutionOptionsQuery>>,
    pub environment_providers: Vec<SystemEnvironmentProvider>,
    pub project_environment_providers: HashMap<String, Vec<SystemEnvironmentProvider>>,
    pub creation_config: CreationConfig,
    pub project_defaults: HashMap<String, ProjectExecutionDefaults>,
    pub branches: ProjectBranchesResponse,
    pub branch_queries: Mutex<Vec<(String, ProjectBranchesQuery)>>,
    pub provider_queries: Mutex<Vec<SystemEnvironmentProvidersQuery>>,
    pub created: Mutex<Vec<CreateThreadRequest>>,
    pub contributions: Vec<PluginCliContribution>,
    pub search_results: HashMap<String, ThreadSearchResponse>,
    pub search_queries: Mutex<Vec<ThreadSearchQuery>>,
    pub paths: Vec<WorkspacePathEntry>,
    pub path_queries: Mutex<Vec<(FileSource, WorkspacePathsQuery)>>,
    pub files: HashMap<String, Vec<u8>>,
    pub cli_runs: Mutex<Vec<(String, PluginCliRequest)>>,
    pub cli_result: PluginCliResult,
    pub mention_providers: Vec<PluginMentionProviderContribution>,
    pub plugin_groups: Vec<PluginMentionSearchGroup>,
    pub plugin_queries: Mutex<Vec<PluginMentionSearchQuery>>,
    pub storage_paths: Vec<WorkspacePathEntry>,
    pub storage_queries: Mutex<Vec<(String, WorkspacePathsQuery)>>,
    pub commands: Vec<ProviderCommand>,
    pub command_queries: Mutex<Vec<(String, ProjectCommandsQuery)>>,
    pub uploads: Mutex<Vec<AttachmentUpload>>,
    pub diff_files: Option<EnvironmentDiffFilesResponse>,
    pub diff_patch_requests: Mutex<Vec<(String, EnvironmentDiffPatchRequest)>>,
    pub diff_queries: Mutex<Vec<(String, EnvironmentDiffQuery)>>,
    pub terminal_sessions: Mutex<Vec<bb_wire::terminal::TerminalSession>>,
    pub terminal_queries: Mutex<Vec<bb_wire::terminal::TerminalListQuery>>,
    pub terminal_created: Mutex<Vec<bb_wire::terminal::CreateTerminalRequest>>,
    pub terminal_closed: Mutex<Vec<String>>,
    pub terminal_client: Option<bb_client::BbClient>,
    pub terminal_connects: Mutex<Vec<(String, u64)>>,
}

impl MockApi {
    pub fn new(backend: Arc<FakeBackend>) -> Arc<Self> {
        Arc::new(Self {
            backend,
            settings: SettingsFake::default(),
            automations: AutomationsFake::default(),
            mutations: Mutex::new(Vec::new()),
            overrides: Vec::new(),
            fail: false,
            execution_options: SystemExecutionOptionsResponse::default(),
            defaults: None,
            history: Vec::new(),
            execution_queries: Mutex::new(Vec::new()),
            environment_providers: Vec::new(),
            project_environment_providers: HashMap::new(),
            creation_config: CreationConfig {
                primary_host_id: None,
                allow_fast_service_tier: true,
            },
            project_defaults: HashMap::new(),
            branches: ProjectBranchesResponse::default(),
            branch_queries: Mutex::new(Vec::new()),
            provider_queries: Mutex::new(Vec::new()),
            created: Mutex::new(Vec::new()),
            contributions: Vec::new(),
            search_results: HashMap::new(),
            search_queries: Mutex::new(Vec::new()),
            paths: Vec::new(),
            path_queries: Mutex::new(Vec::new()),
            files: HashMap::new(),
            cli_runs: Mutex::new(Vec::new()),
            cli_result: PluginCliResult::default(),
            mention_providers: Vec::new(),
            plugin_groups: Vec::new(),
            plugin_queries: Mutex::new(Vec::new()),
            storage_paths: Vec::new(),
            storage_queries: Mutex::new(Vec::new()),
            commands: Vec::new(),
            command_queries: Mutex::new(Vec::new()),
            uploads: Mutex::new(Vec::new()),
            diff_files: None,
            diff_patch_requests: Mutex::new(Vec::new()),
            diff_queries: Mutex::new(Vec::new()),
            terminal_sessions: Mutex::new(Vec::new()),
            terminal_queries: Mutex::new(Vec::new()),
            terminal_created: Mutex::new(Vec::new()),
            terminal_closed: Mutex::new(Vec::new()),
            terminal_client: None,
            terminal_connects: Mutex::new(Vec::new()),
        })
    }

    pub fn recorded(&self) -> Vec<Mutation> {
        self.mutations.lock().unwrap().clone()
    }
}

impl Api for MockApi {
    fn plugin_rpc(
        &self,
        plugin_id: String,
        method: String,
        input: serde_json::Value,
    ) -> ApiFuture<serde_json::Value> {
        let result = if plugin_id == "automations" {
            let sidebar = self.backend.sidebar.lock().unwrap().clone();
            self.automations.handle(&sidebar, &method, input)
        } else {
            Err(format!("unknown plugin \"{plugin_id}\""))
        };
        Box::pin(async move { result })
    }

    fn system_config(&self) -> ApiFuture<bb_wire::system::SystemConfigResponse> {
        let config = self.settings.config.lock().unwrap().clone();
        Box::pin(async move { Ok(config) })
    }

    fn server_version(&self) -> ApiFuture<bb_wire::system::SystemVersionResponse> {
        let version = self.settings.version.clone();
        Box::pin(async move { Ok(version) })
    }

    fn provider_catalog(&self) -> ApiFuture<Vec<bb_wire::system::SystemProviderCatalogEntry>> {
        let catalog = self.settings.catalog.lock().unwrap().clone();
        Box::pin(async move { Ok(catalog) })
    }

    fn host_provider_clis(
        &self,
        host_id: String,
    ) -> ApiFuture<bb_wire::environment::HostProviderCliStatusResponse> {
        let status = self.settings.host_clis.get(&host_id).cloned();
        Box::pin(async move { status.ok_or_else(|| "host offline".to_string()) })
    }

    fn list_threads(
        &self,
        query: bb_wire::thread::ThreadListQuery,
    ) -> ApiFuture<Vec<ThreadListEntry>> {
        self.settings
            .thread_queries
            .lock()
            .unwrap()
            .push(query.clone());
        let offset = query.offset.unwrap_or(0) as usize;
        let limit = query.limit.unwrap_or(50) as usize;
        let threads: Vec<ThreadListEntry> = self
            .settings
            .archived
            .lock()
            .unwrap()
            .iter()
            .filter(|entry| {
                query
                    .project_id
                    .as_ref()
                    .is_none_or(|id| &entry.thread.project_id == id)
                    && query
                        .has_parent
                        .is_none_or(|parent| entry.thread.parent_thread_id.is_some() == parent)
            })
            .skip(offset)
            .take(limit)
            .cloned()
            .collect();
        Box::pin(async move { Ok(threads) })
    }

    fn thread_child_summary(
        &self,
        thread_id: String,
    ) -> ApiFuture<bb_wire::thread::ThreadChildSummaryResponse> {
        let count = self
            .settings
            .child_counts
            .get(&thread_id)
            .copied()
            .unwrap_or(0);
        Box::pin(async move {
            Ok(bb_wire::thread::ThreadChildSummaryResponse {
                non_deleted_child_count: count,
                unarchived_descendant_count: 0,
            })
        })
    }

    fn list_plugins(&self) -> ApiFuture<Vec<bb_wire::plugin::InstalledPlugin>> {
        let plugins = self.settings.plugins.lock().unwrap().clone();
        Box::pin(async move { Ok(plugins) })
    }

    fn plugin_settings(
        &self,
        plugin_id: String,
    ) -> ApiFuture<bb_wire::plugin::PluginSettingsResponse> {
        let settings = self
            .settings
            .plugin_settings
            .lock()
            .unwrap()
            .get(&plugin_id)
            .cloned();
        Box::pin(async move { settings.ok_or_else(|| "plugin_not_found".to_string()) })
    }

    fn settings_mutation(
        &self,
        mutation: bb_tui::settings::SettingsMutation,
    ) -> ApiFuture<bb_tui::settings::SettingsReply> {
        let result = self.settings.apply(&mutation);
        Box::pin(async move { result })
    }

    fn list_terminals(
        &self,
        query: bb_wire::terminal::TerminalListQuery,
    ) -> ApiFuture<Vec<bb_wire::terminal::TerminalSession>> {
        self.terminal_queries.lock().unwrap().push(query.clone());
        let sessions: Vec<_> = self
            .terminal_sessions
            .lock()
            .unwrap()
            .iter()
            .filter(|session| match (&query.thread_id, &query.environment_id) {
                (Some(thread_id), _) => session.thread_id.as_ref() == Some(thread_id),
                (None, Some(environment_id)) => {
                    session.thread_id.is_none()
                        && session.environment_id.as_ref() == Some(environment_id)
                }
                _ => false,
            })
            .cloned()
            .collect();
        Box::pin(async move { Ok(sessions) })
    }

    fn create_terminal(
        &self,
        request: bb_wire::terminal::CreateTerminalRequest,
    ) -> ApiFuture<bb_wire::terminal::TerminalSession> {
        self.terminal_created.lock().unwrap().push(request.clone());
        let mut sessions = self.terminal_sessions.lock().unwrap();
        let thread_id = match &request.target {
            bb_wire::terminal::TerminalCreateTarget::Thread { thread_id } => {
                Some(thread_id.clone())
            }
            _ => None,
        };
        let session = terminal_session(
            &format!("term_new{}", sessions.len() + 1),
            thread_id.as_deref(),
            None,
            bb_wire::terminal::TerminalSessionStatus::Running,
        );
        sessions.push(session.clone());
        Box::pin(async move { Ok(session) })
    }

    fn close_terminal(
        &self,
        terminal_id: String,
        _request: bb_wire::terminal::CloseTerminalRequest,
    ) -> ApiFuture<bb_wire::terminal::TerminalSession> {
        self.terminal_closed
            .lock()
            .unwrap()
            .push(terminal_id.clone());
        let mut sessions = self.terminal_sessions.lock().unwrap();
        let closed = sessions
            .iter()
            .find(|session| session.id == terminal_id)
            .cloned()
            .map(|session| bb_wire::terminal::TerminalSession {
                status: bb_wire::terminal::TerminalSessionStatus::Exited,
                ..session
            });
        sessions.retain(|session| session.id != terminal_id);
        Box::pin(async move { closed.ok_or_else(|| "terminal_not_found".to_string()) })
    }

    fn connect_terminal(
        &self,
        terminal_id: &str,
        since_seq: u64,
    ) -> Result<bb_tui::api::TerminalLink, String> {
        self.terminal_connects
            .lock()
            .unwrap()
            .push((terminal_id.to_owned(), since_seq));
        let client = self
            .terminal_client
            .clone()
            .ok_or_else(|| "no terminal server".to_string())?;
        let config = bb_client::TerminalConfig {
            reconnect_delays: vec![std::time::Duration::from_millis(10)],
            ..bb_tui::api::terminal_config(since_seq)
        };
        let (connection, events) =
            bb_client::TerminalConnection::spawn(client, terminal_id, config);
        Ok(bb_tui::api::TerminalLink {
            handle: Arc::new(connection),
            events,
        })
    }

    fn thread_interactions(&self, thread_id: String) -> ApiFuture<Vec<PendingInteraction>> {
        let list = self
            .backend
            .interactions
            .lock()
            .unwrap()
            .get(&thread_id)
            .cloned()
            .unwrap_or_default();
        Box::pin(async move { Ok(list) })
    }

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

    fn environment_providers(
        &self,
        query: SystemEnvironmentProvidersQuery,
    ) -> ApiFuture<Vec<SystemEnvironmentProvider>> {
        self.provider_queries.lock().unwrap().push(query.clone());
        let providers = match &query.project_id {
            Some(project_id) => self
                .project_environment_providers
                .get(project_id)
                .cloned()
                .unwrap_or_else(|| self.environment_providers.clone()),
            None => self.environment_providers.clone(),
        };
        Box::pin(async move { Ok(providers) })
    }

    fn creation_config(&self) -> ApiFuture<CreationConfig> {
        let config = self.creation_config.clone();
        Box::pin(async move { Ok(config) })
    }

    fn project_default_execution_options(
        &self,
        project_id: String,
    ) -> ApiFuture<Option<ProjectExecutionDefaults>> {
        let defaults = self.project_defaults.get(&project_id).cloned();
        Box::pin(async move { Ok(defaults) })
    }

    fn branch_options(
        &self,
        project_id: String,
        query: ProjectBranchesQuery,
    ) -> ApiFuture<ProjectBranchesResponse> {
        self.branch_queries
            .lock()
            .unwrap()
            .push((project_id, query));
        let branches = self.branches.clone();
        Box::pin(async move { Ok(branches) })
    }

    fn create_thread(&self, request: CreateThreadRequest) -> ApiFuture<ThreadResponse> {
        self.created.lock().unwrap().push(request.clone());
        if self.fail {
            return Box::pin(async { Err("server said no".to_string()) });
        }
        let id = format!("thr_new{}", self.created.lock().unwrap().len());
        let mut entry = thread(&id, &request.project_id, "Created thread", 0);
        if let CreateThreadEnvironmentArgs::Reuse { environment_id } = &request.environment {
            entry.thread.environment_id = Some(environment_id.clone());
        }
        {
            let mut sidebar = self.backend.sidebar.lock().unwrap();
            let sidebar = &mut *sidebar;
            if let Some(project) = sidebar
                .projects
                .iter_mut()
                .chain(std::iter::once(&mut sidebar.personal_project))
                .find(|project| project.project.id == request.project_id)
            {
                project.threads.push(entry.clone());
            }
        }
        let response = ThreadResponse {
            thread: entry.thread,
            runtime: entry.runtime,
            ..ThreadResponse::default()
        };
        Box::pin(async move { Ok(response) })
    }

    fn plugin_contributions(&self) -> ApiFuture<Vec<PluginCliContribution>> {
        let contributions = self.contributions.clone();
        Box::pin(async move { Ok(contributions) })
    }

    fn search_threads(&self, query: ThreadSearchQuery) -> ApiFuture<ThreadSearchResponse> {
        self.search_queries.lock().unwrap().push(query.clone());
        let response = self
            .search_results
            .get(&query.query)
            .cloned()
            .unwrap_or_default();
        Box::pin(async move { Ok(response) })
    }

    fn workspace_paths(
        &self,
        source: FileSource,
        query: WorkspacePathsQuery,
    ) -> ApiFuture<WorkspacePathListResponse> {
        let response = self.palette_paths(&query);
        self.path_queries.lock().unwrap().push((source, query));
        Box::pin(async move { Ok(response) })
    }

    fn workspace_file(&self, _source: FileSource, path: String) -> ApiFuture<Vec<u8>> {
        let file = self.files.get(&path).cloned();
        Box::pin(async move { file.ok_or_else(|| format!("{path}: not found")) })
    }

    fn run_plugin_cli(
        &self,
        plugin_id: String,
        request: PluginCliRequest,
    ) -> ApiFuture<PluginCliResult> {
        self.cli_runs.lock().unwrap().push((plugin_id, request));
        let result = self.cli_result.clone();
        Box::pin(async move { Ok(result) })
    }

    fn mention_providers(&self) -> ApiFuture<Vec<PluginMentionProviderContribution>> {
        let providers = self.mention_providers.clone();
        Box::pin(async move { Ok(providers) })
    }

    fn plugin_mentions(
        &self,
        query: PluginMentionSearchQuery,
    ) -> ApiFuture<Vec<PluginMentionSearchGroup>> {
        self.plugin_queries.lock().unwrap().push(query);
        let groups = self.plugin_groups.clone();
        Box::pin(async move { Ok(groups) })
    }

    fn thread_storage_paths(
        &self,
        thread_id: String,
        query: WorkspacePathsQuery,
    ) -> ApiFuture<WorkspacePathListResponse> {
        self.storage_queries
            .lock()
            .unwrap()
            .push((thread_id, query));
        let paths = self.storage_paths.clone();
        Box::pin(async move {
            Ok(WorkspacePathListResponse {
                paths,
                ..WorkspacePathListResponse::default()
            })
        })
    }

    fn project_commands(
        &self,
        project_id: String,
        query: ProjectCommandsQuery,
    ) -> ApiFuture<Vec<ProviderCommand>> {
        self.command_queries
            .lock()
            .unwrap()
            .push((project_id, query));
        let commands = self.commands.clone();
        Box::pin(async move { Ok(commands) })
    }

    fn upload_attachment(&self, upload: AttachmentUpload) -> ApiFuture<UploadedPromptAttachment> {
        let response = UploadedPromptAttachment {
            source_project_id: Some(upload.project_id.clone()),
            attachment_type: if upload
                .mime_type
                .as_deref()
                .is_some_and(|mime| mime.starts_with("image/"))
            {
                UploadedAttachmentType::LocalImage
            } else {
                UploadedAttachmentType::LocalFile
            },
            path: format!("attachments/{}", upload.file_name),
            name: upload.file_name.clone(),
            mime_type: upload.mime_type.clone(),
            size_bytes: upload.bytes.len() as u64,
        };
        self.uploads.lock().unwrap().push(upload);
        Box::pin(async move { Ok(response) })
    }

    fn environment_diff_files(
        &self,
        environment_id: String,
        query: EnvironmentDiffQuery,
    ) -> ApiFuture<EnvironmentDiffFilesResponse> {
        self.diff_queries
            .lock()
            .unwrap()
            .push((environment_id, query));
        let files = self.diff_files.clone();
        Box::pin(async move { files.ok_or_else(|| "no diff".to_string()) })
    }

    fn environment_diff_patch(
        &self,
        environment_id: String,
        request: EnvironmentDiffPatchRequest,
    ) -> ApiFuture<EnvironmentDiffPatchResponse> {
        let patches = request
            .paths
            .iter()
            .map(|path| DiffPatchEntry {
                path: path.clone(),
                patch: format!(
                    "diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -1,1 +1,2 @@\n keep\n+added in {path}\n"
                ),
                truncated: false,
            })
            .collect();
        self.diff_patch_requests
            .lock()
            .unwrap()
            .push((environment_id, request));
        Box::pin(async move { Ok(EnvironmentDiffPatchResponse::Available { patches }) })
    }
}

impl MockApi {
    pub fn with_diff_files(
        backend: Arc<FakeBackend>,
        files: EnvironmentDiffFilesResponse,
    ) -> Arc<Self> {
        let mut api = Arc::try_unwrap(Self::new(backend)).ok().expect("fresh api");
        api.diff_files = Some(files);
        Arc::new(api)
    }

    fn palette_paths(&self, query: &WorkspacePathsQuery) -> WorkspacePathListResponse {
        let needle = query.query.clone().unwrap_or_default().to_lowercase();
        WorkspacePathListResponse {
            paths: self
                .paths
                .iter()
                .filter(|entry| entry.path.to_lowercase().contains(&needle))
                .cloned()
                .collect(),
            ..WorkspacePathListResponse::default()
        }
    }

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

pub fn terminal_session(
    id: &str,
    thread_id: Option<&str>,
    environment_id: Option<&str>,
    status: bb_wire::terminal::TerminalSessionStatus,
) -> bb_wire::terminal::TerminalSession {
    bb_wire::terminal::TerminalSession {
        id: id.into(),
        thread_id: thread_id.map(str::to_owned),
        environment_id: environment_id.map(str::to_owned),
        host_id: "host_1".into(),
        title: format!("shell {id}"),
        initial_cwd: "/w".into(),
        cols: 80,
        rows: 24,
        status,
        created_at: NOW_MS,
        updated_at: NOW_MS,
        ..bb_wire::terminal::TerminalSession::default()
    }
}

#[derive(Default)]
pub struct MockPlatform {
    pub alerts: Mutex<Vec<bb_tui::notifications::Alert>>,
    pub titles: Mutex<Vec<String>>,
    pub edited: Mutex<Vec<(String, Option<u32>)>>,
    pub copied: Mutex<Vec<String>>,
    pub opened: Mutex<Vec<String>>,
    pub drafts: Mutex<HashMap<String, String>>,
    pub raw_input: Mutex<Option<bb_tui::platform::RawInput>>,
    pub raw_output: Mutex<Vec<u8>>,
    pub screen_events: Mutex<Vec<&'static str>>,
    pub screen_size: Option<(u16, u16)>,
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

    fn edit_file(&self, path: &str, line: Option<u32>) -> Result<(), String> {
        self.edited.lock().unwrap().push((path.to_owned(), line));
        Ok(())
    }

    fn alert(&self, alert: &bb_tui::notifications::Alert) -> Result<(), String> {
        self.alerts.lock().unwrap().push(alert.clone());
        Ok(())
    }

    fn set_title(&self, title: &str) -> Result<(), String> {
        self.titles.lock().unwrap().push(title.to_owned());
        Ok(())
    }

    fn raw_input(&self) -> Option<bb_tui::platform::RawInput> {
        self.raw_input.lock().unwrap().take()
    }

    fn suspend_screen(&self) -> Result<(), String> {
        self.screen_events.lock().unwrap().push("suspend");
        Ok(())
    }

    fn resume_screen(&self) -> Result<(), String> {
        self.screen_events.lock().unwrap().push("resume");
        Ok(())
    }

    fn write_raw(&self, bytes: &[u8]) -> Result<(), String> {
        self.raw_output.lock().unwrap().extend_from_slice(bytes);
        Ok(())
    }

    fn screen_size(&self) -> Option<(u16, u16)> {
        self.screen_size
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

pub async fn run_settings(
    api: &MockApi,
    state: &mut AppState,
    effects: Vec<Effect>,
) -> Vec<Effect> {
    let mut pending = effects;
    let mut rest = Vec::new();
    for _ in 0..8 {
        let mut next = Vec::new();
        for effect in pending {
            match effect {
                Effect::Settings(request) => {
                    let loaded = bb_tui::runtime::run_settings_request(api, request).await;
                    next.extend(update(state, Action::SettingsLoaded(loaded)));
                }
                other => rest.push(other),
            }
        }
        if next.is_empty() {
            break;
        }
        pending = next;
    }
    rest
}

pub async fn run_automations(
    api: &MockApi,
    state: &mut AppState,
    effects: Vec<Effect>,
) -> Vec<Effect> {
    let mut pending = effects;
    let mut rest = Vec::new();
    for _ in 0..8 {
        let mut next = Vec::new();
        for effect in pending {
            match effect {
                Effect::Automations(request) => {
                    let loaded = bb_tui::runtime::run_automation_request(api, request).await;
                    next.extend(update(state, Action::AutomationsLoaded(loaded)));
                }
                other => rest.push(other),
            }
        }
        if next.is_empty() {
            break;
        }
        pending = next;
    }
    rest
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
