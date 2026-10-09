use serde::{Deserialize, Serialize};

use crate::common::{JsonObject, double_option};
use crate::environment::{
    CreateThreadEnvironmentArgs, Environment, EnvironmentWorkspaceDisplayKind, Host,
};
use crate::prompt::{
    CreateExecutionInputSources, PermissionMode, PluginSubmission, PromptInput, ReasoningLevel,
};

wire_enum! {
    pub enum ThreadStatus {
        Pending => "pending",
        Idle => "idle",
        Starting => "starting",
        Active => "active",
        Stopping => "stopping",
        Error => "error",
    }
}

wire_enum! {
    pub enum ThreadDisplayStatus {
        Pending => "pending",
        Idle => "idle",
        Starting => "starting",
        Active => "active",
        Stopping => "stopping",
        Error => "error",
        Provisioning => "provisioning",
        WaitingForHost => "waiting-for-host",
    }
}

wire_enum! {
    pub enum ThreadVisibility {
        Visible => "visible",
        Hidden => "hidden",
    }
}

wire_enum! {
    pub enum ThreadOriginKind {
        Fork => "fork",
    }
}

wire_enum! {
    pub enum ThreadCreateOrigin {
        App => "app",
        Cli => "cli",
        Sdk => "sdk",
        Plugin => "plugin",
    }
}

wire_enum! {
    pub enum ThreadQueuedWork {
        None => "none",
        Waiting => "waiting",
        Failed => "failed",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadRuntimeState {
    pub display_status: ThreadDisplayStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadActivityState {
    pub active_workflow_count: u32,
    pub active_background_agent_count: u32,
    pub active_background_command_count: u32,
    pub active_plan_mode_count: u32,
    pub active_goal_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Thread {
    pub id: String,
    pub project_id: String,
    pub environment_id: Option<String>,
    pub provider_id: String,
    pub title: Option<String>,
    pub title_fallback: Option<String>,
    pub section_id: Option<String>,
    pub status: ThreadStatus,
    pub parent_thread_id: Option<String>,
    pub lifecycle_owner_thread_id: Option<String>,
    pub source_thread_id: Option<String>,
    pub origin_kind: Option<ThreadOriginKind>,
    pub origin_plugin_id: Option<String>,
    pub visibility: ThreadVisibility,
    pub archived_at: Option<i64>,
    pub pinned_at: Option<i64>,
    pub deleted_at: Option<i64>,
    pub last_read_at: Option<i64>,
    pub latest_attention_at: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Thread {
    pub fn title_or_fallback(&self) -> Option<&str> {
        self.title.as_deref().or(self.title_fallback.as_deref())
    }

    pub fn is_pinned(&self) -> bool {
        self.pinned_at.is_some()
    }

    pub fn is_archived(&self) -> bool {
        self.archived_at.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadListEntry {
    #[serde(flatten)]
    pub thread: Thread,
    pub runtime: ThreadRuntimeState,
    pub activity: ThreadActivityState,
    pub queued_work: ThreadQueuedWork,
    pub pin_sort_key: Option<String>,
    pub has_pending_interaction: bool,
    pub environment_host_id: Option<String>,
    pub environment_name: Option<String>,
    pub environment_branch_name: Option<String>,
    pub environment_path: Option<String>,
    pub environment_provider_id: Option<String>,
    pub environment_is_worktree: Option<bool>,
    pub environment_workspace_display_kind: EnvironmentWorkspaceDisplayKind,
}

pub type ThreadListResponse = Vec<ThreadListEntry>;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadResponse {
    #[serde(flatten)]
    pub thread: Thread,
    pub runtime: ThreadRuntimeState,
    pub active_background_agent_count: u32,
    pub can_restore_environment: bool,
    pub can_spawn_child: bool,
    pub queued_message_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment: Option<Environment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<Host>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment_host_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadGetQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include: Option<String>,
}

impl ThreadGetQuery {
    pub fn with_environment_and_host() -> Self {
        Self {
            include: Some("environment,host".into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadListQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_thread_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_thread_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unsectioned: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_parent: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin_kind: Option<ThreadOriginKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin_plugin_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_hidden: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
}

wire_enum! {
    pub enum ThreadCountGroupBy {
        Host => "host",
        Provider => "provider",
        Project => "project",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadCountQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<ThreadStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_thread_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_by: Option<ThreadCountGroupBy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_archived: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_hidden: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadCountGroup {
    pub key: Option<String>,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadCountResponse {
    pub total: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<ThreadCountGroup>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSearchQuery {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_per_group: Option<u32>,
}

wire_enum! {
    pub enum ThreadSearchSourceKind {
        Title => "title",
        TitleFallback => "title_fallback",
        UserMessage => "user_message",
        AssistantMessage => "assistant_message",
        SystemMessage => "system_message",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HighlightRange {
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadSearchMatch {
    pub source_kind: ThreadSearchSourceKind,
    pub text: String,
    pub highlight_ranges: Vec<HighlightRange>,
    pub source_seq: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadSearchResult {
    pub thread: ThreadListEntry,
    pub matches: Vec<ThreadSearchMatch>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadSearchResultGroup {
    pub total: u64,
    pub results: Vec<ThreadSearchResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadSearchResponse {
    pub active: ThreadSearchResultGroup,
    pub archived: ThreadSearchResultGroup,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateThreadRequest {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub title: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub section_id: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub parent_thread_id: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub model: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub reasoning_level: Option<Option<ReasoningLevel>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<ThreadVisibility>,
}

impl UpdateThreadRequest {
    pub fn rename(title: impl Into<String>) -> Self {
        Self {
            title: Some(Some(title.into())),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReorderPinnedThreadRequest {
    pub previous_thread_id: Option<String>,
    pub next_thread_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteThreadRequest {
    pub child_threads_confirmed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadChildSummaryResponse {
    pub non_deleted_child_count: u32,
    pub unarchived_descendant_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadArchiveAllResponse {
    pub ok: bool,
    pub archived_thread_ids: Vec<String>,
}

wire_enum! {
    pub enum StartedOnBehalfOfInitiator {
        Agent => "agent",
        System => "system",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StartedOnBehalfOf {
    pub initiator: StartedOnBehalfOfInitiator,
    pub sender_thread_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateThreadRequest {
    pub project_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    pub origin: ThreadCreateOrigin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_plugin_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_metadata: Option<JsonObject>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle_owner_thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<ThreadVisibility>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub input: Vec<PromptInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_level: Option<ReasoningLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<PermissionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_input_sources: Option<CreateExecutionInputSources>,
    pub environment: CreateThreadEnvironmentArgs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_thread_id: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub section_id: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_seq_end: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_on_behalf_of: Option<StartedOnBehalfOf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_kind: Option<ThreadOriginKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_submission: Option<PluginSubmission>,
}

impl CreateThreadRequest {
    pub fn new(
        project_id: impl Into<String>,
        environment: CreateThreadEnvironmentArgs,
        input: Vec<PromptInput>,
    ) -> Self {
        Self {
            project_id: project_id.into(),
            provider_id: None,
            origin: ThreadCreateOrigin::App,
            origin_plugin_id: None,
            plugin_metadata: None,
            lifecycle_owner_thread_id: None,
            visibility: None,
            title: None,
            input,
            model: None,
            service_tier: None,
            reasoning_level: None,
            permission_mode: None,
            execution_input_sources: None,
            environment,
            parent_thread_id: None,
            section_id: None,
            pinned: None,
            source_thread_id: None,
            source_seq_end: None,
            started_on_behalf_of: None,
            origin_kind: None,
            send_at: None,
            plugin_submission: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ContextEntry {
    pub id: String,
    pub label: String,
    pub tokens: u64,
}

wire_enum! {
    pub enum ContextCategoryKind {
        Used => "used",
        Free => "free",
        Reserved => "reserved",
        Deferred => "deferred",
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ContextCategory {
    pub id: String,
    pub label: String,
    pub kind: ContextCategoryKind,
    pub tokens: u64,
    pub entries: Vec<ContextEntry>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ContextSnapshot {
    pub captured_at: String,
    pub provider_session_id: String,
    pub provider_turn_id: Option<String>,
    pub model: String,
    pub used_tokens: u64,
    pub context_window_tokens: u64,
    pub auto_compact_at_tokens: Option<u64>,
    pub estimated: bool,
    pub categories: Vec<ContextCategory>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadContextWindowUsage {
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::common::lenient_option"
    )]
    pub snapshot: Option<ContextSnapshot>,
    pub used_tokens: f64,
    pub model_context_window: f64,
    pub estimated: bool,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadContextResponse {
    pub usage: Option<ThreadContextWindowUsage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadStorageLocationResponse {
    pub host_id: String,
    pub storage_root_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadOutputResponse {
    pub output: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadMentionResolution {
    pub thread_id: String,
    pub project_id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveThreadMentionsRequest {
    pub thread_ids: Vec<String>,
}

pub type ThreadPluginMetadata = JsonObject;
