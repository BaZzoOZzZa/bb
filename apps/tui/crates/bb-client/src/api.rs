use bb_wire::environment::{
    BranchListQuery, Environment, EnvironmentActionRequest, EnvironmentActionResponse,
    EnvironmentArchiveThreadsResponse, EnvironmentDiffFileQuery, EnvironmentDiffFileResponse,
    EnvironmentDiffFilesResponse, EnvironmentDiffPatchRequest, EnvironmentDiffPatchResponse,
    EnvironmentDiffQuery, EnvironmentDiffResponse, EnvironmentPullRequestResponse,
    EnvironmentStatusQuery, EnvironmentStatusResponse, GitBranchOptions, Host,
    HostProviderCliStatusResponse, ListEnvironmentsQuery, UpdateEnvironmentRequest,
    UpdateHostPermissionCeilingRequest, UpdateHostRequest,
};
use bb_wire::interaction::{
    PendingInteraction, ResolvePendingInteractionRequest, RespondPluginInteractionRequest,
    ThreadPendingInteractionsResponse,
};
use bb_wire::plugin::{
    PluginCliRequest, PluginCliResult, PluginContributionsResponse, PluginListResponse,
    PluginMentionSearchQuery, PluginMentionSearchResponse, PluginRpcResponse,
    PluginSettingsResponse, PluginSettingsUpdateRequest,
};
use bb_wire::project::{
    CommandListResponse, CreateProjectRequest, CreateThreadSectionRequest,
    DeleteThreadSectionRequest, ProjectBranchesQuery, ProjectBranchesResponse,
    ProjectCommandsQuery, ProjectListQuery, ProjectResponse, ProjectWithThreadsResponse,
    PromptHistoryQuery, ReorderProjectRequest, SidebarBootstrapResponse, ThreadSection,
    ThreadSectionMutationResponse, UpdateProjectRequest, UpdateThreadSectionRequest,
    UploadedPromptAttachment, WorkspaceFileListResponse, WorkspaceFilesQuery,
    WorkspacePathListResponse, WorkspacePathsQuery,
};
use bb_wire::prompt::{
    ProjectExecutionDefaults, PromptHistoryEntry, ResolvedThreadExecutionOptions,
};
use bb_wire::queue::{
    CreateQueuedMessageRequest, EditMessageRequest, EditMessageResponse,
    QueuedMessageEditHoldResponse, QueuedMessageListQuery, ReorderQueuedMessageRequest,
    RetryTurnRequest, RetryTurnResponse, SendMessageRequest, SendMessageResponse,
    SendQueuedMessageRequest, SendQueuedMessageResponse, SetQueuedMessageGroupBoundaryRequest,
    ThreadQueuedMessage, ThreadQueuedMessageListResponse, UpdateQueuedMessageRequest,
};
use bb_wire::realtime::{DeliveredResponse, ThreadOpenRequest};
use bb_wire::routes::{
    environments, hosts, plugins, projects, queue, system, terminals, thread_sections, threads,
};
use bb_wire::system::{
    AppKeybindingOverrides, ProviderInfo, SystemAttentionResponse, SystemConfigResponse,
    SystemEnvironmentProvidersQuery, SystemEnvironmentProvidersResponse,
    SystemExecutionOptionsQuery, SystemExecutionOptionsResponse, SystemProviderCatalogEntry,
    SystemProviderEnabledRequest, SystemProvidersQuery, SystemVersionQuery, SystemVersionResponse,
};
use bb_wire::terminal::{
    CloseTerminalRequest, CreateTerminalRequest, RestartTerminalRequest, TerminalInputRequest,
    TerminalListQuery, TerminalListResponse, TerminalOutputQuery, TerminalOutputResponse,
    TerminalResizeRequest, TerminalSession, UpdateTerminalRequest,
};
use bb_wire::thread::{
    CreateThreadRequest, DeleteThreadRequest, ReorderPinnedThreadRequest,
    ResolveThreadMentionsRequest, ThreadArchiveAllResponse, ThreadChildSummaryResponse,
    ThreadContextResponse, ThreadCountQuery, ThreadCountResponse, ThreadGetQuery, ThreadListQuery,
    ThreadListResponse, ThreadOutputResponse, ThreadPluginMetadata, ThreadResponse,
    ThreadSearchQuery, ThreadSearchResponse, ThreadStorageLocationResponse, UpdateThreadRequest,
};
use bb_wire::timeline::{
    ThreadMessageResponse, ThreadTimelineQuery, ThreadTimelineResponse, TurnSummaryDetailsQuery,
    TurnSummaryDetailsResponse,
};
use bb_wire::{JsonObject, JsonValue, OkResponse};
use serde::Serialize;

use crate::error::ClientError;
use crate::http::{BbClient, MultipartFile, RawRequest, RequestBody};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PathQuery<'a> {
    path: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginIdQuery<'a> {
    plugin_id: &'a str,
}

#[derive(Serialize)]
struct TailQuery {
    tail: u32,
}

impl BbClient {
    pub async fn system_version(
        &self,
        query: &SystemVersionQuery,
    ) -> Result<SystemVersionResponse, ClientError> {
        self.get_with(system::version(), query).await
    }

    pub async fn system_attention(&self) -> Result<SystemAttentionResponse, ClientError> {
        self.get(system::attention()).await
    }

    pub async fn system_config(&self) -> Result<SystemConfigResponse, ClientError> {
        self.get(system::config()).await
    }

    pub async fn reload_system_config(&self) -> Result<JsonValue, ClientError> {
        self.call(system::reload_config()).await
    }

    pub async fn system_execution_options(
        &self,
        query: &SystemExecutionOptionsQuery,
    ) -> Result<SystemExecutionOptionsResponse, ClientError> {
        self.get_with(system::execution_options(), query).await
    }

    pub async fn system_environment_providers(
        &self,
        query: &SystemEnvironmentProvidersQuery,
    ) -> Result<SystemEnvironmentProvidersResponse, ClientError> {
        self.get_with(system::environment_providers(), query).await
    }

    pub async fn system_machine_providers(&self) -> Result<JsonValue, ClientError> {
        self.get(system::machine_providers()).await
    }

    pub async fn system_providers(
        &self,
        query: &SystemProvidersQuery,
    ) -> Result<Vec<ProviderInfo>, ClientError> {
        self.get_with(system::providers(), query).await
    }

    pub async fn system_provider_states(
        &self,
        query: &SystemProvidersQuery,
    ) -> Result<JsonValue, ClientError> {
        self.get_with(system::provider_states(), query).await
    }

    pub async fn system_provider_catalog(
        &self,
    ) -> Result<Vec<SystemProviderCatalogEntry>, ClientError> {
        self.get(system::provider_catalog()).await
    }

    pub async fn set_provider_enabled(
        &self,
        provider_id: &str,
        enabled: bool,
    ) -> Result<Vec<SystemProviderCatalogEntry>, ClientError> {
        self.call_with(
            system::set_provider_enabled(provider_id),
            &SystemProviderEnabledRequest { enabled },
        )
        .await
    }

    pub async fn system_usage_limits(&self) -> Result<JsonValue, ClientError> {
        self.get(system::usage_limits()).await
    }

    pub async fn update_general_settings(
        &self,
        update: &JsonObject,
    ) -> Result<JsonValue, ClientError> {
        self.call_with(system::update_general_settings(), update)
            .await
    }

    pub async fn update_keyboard_settings(
        &self,
        overrides: &AppKeybindingOverrides,
    ) -> Result<AppKeybindingOverrides, ClientError> {
        self.call_with(system::update_keyboard_settings(), overrides)
            .await
    }

    pub async fn list_projects(
        &self,
        query: &ProjectListQuery,
    ) -> Result<Vec<ProjectResponse>, ClientError> {
        self.get_with(projects::list(), query).await
    }

    pub async fn list_projects_with_threads(
        &self,
        query: &ProjectListQuery,
    ) -> Result<Vec<ProjectWithThreadsResponse>, ClientError> {
        self.get_with(projects::list(), query).await
    }

    pub async fn create_project(
        &self,
        request: &CreateProjectRequest,
    ) -> Result<ProjectResponse, ClientError> {
        self.call_with(projects::create(), request).await
    }

    pub async fn sidebar_bootstrap(&self) -> Result<SidebarBootstrapResponse, ClientError> {
        self.get(projects::sidebar_bootstrap()).await
    }

    pub async fn get_project(&self, id: &str) -> Result<ProjectResponse, ClientError> {
        self.get(projects::get(id)).await
    }

    pub async fn update_project(
        &self,
        id: &str,
        request: &UpdateProjectRequest,
    ) -> Result<ProjectResponse, ClientError> {
        self.call_with(projects::update(id), request).await
    }

    pub async fn delete_project(&self, id: &str) -> Result<OkResponse, ClientError> {
        self.call(projects::delete(id)).await
    }

    pub async fn reorder_project(
        &self,
        id: &str,
        request: &ReorderProjectRequest,
    ) -> Result<Vec<ProjectResponse>, ClientError> {
        self.call_with(projects::reorder(id), request).await
    }

    pub async fn project_default_execution_options(
        &self,
        id: &str,
    ) -> Result<Option<ProjectExecutionDefaults>, ClientError> {
        self.get(projects::default_execution_options(id)).await
    }

    pub async fn project_prompt_history(
        &self,
        id: &str,
        query: &PromptHistoryQuery,
    ) -> Result<Vec<PromptHistoryEntry>, ClientError> {
        self.get_with(projects::prompt_history(id), query).await
    }

    pub async fn project_files(
        &self,
        id: &str,
        query: &WorkspaceFilesQuery,
    ) -> Result<WorkspaceFileListResponse, ClientError> {
        self.get_with(projects::files(id), query).await
    }

    pub async fn project_file(&self, id: &str, file_path: &str) -> Result<Vec<u8>, ClientError> {
        self.get_bytes(projects::file(id, file_path)).await
    }

    pub async fn project_paths(
        &self,
        id: &str,
        query: &WorkspacePathsQuery,
    ) -> Result<WorkspacePathListResponse, ClientError> {
        self.get_with(projects::paths(id), query).await
    }

    pub async fn project_commands(
        &self,
        id: &str,
        query: &ProjectCommandsQuery,
    ) -> Result<CommandListResponse, ClientError> {
        self.get_with(projects::commands(id), query).await
    }

    pub async fn project_branches(
        &self,
        id: &str,
        query: &ProjectBranchesQuery,
    ) -> Result<ProjectBranchesResponse, ClientError> {
        self.get_with(projects::branches(id), query).await
    }

    pub async fn project_branch_options(
        &self,
        id: &str,
        query: &ProjectBranchesQuery,
    ) -> Result<ProjectBranchesResponse, ClientError> {
        self.get_with(projects::branch_options(id), query).await
    }

    pub async fn upload_project_attachment(
        &self,
        id: &str,
        file_name: &str,
        mime_type: Option<&str>,
        bytes: Vec<u8>,
    ) -> Result<UploadedPromptAttachment, ClientError> {
        let mut request = RawRequest::api(&projects::upload_attachment(id));
        request.body = RequestBody::Multipart(MultipartFile {
            field: "file".into(),
            file_name: file_name.into(),
            mime_type: mime_type.map(str::to_owned),
            bytes,
        });
        self.execute_json(&request).await
    }

    pub async fn project_attachment_content(
        &self,
        id: &str,
        path: &str,
    ) -> Result<Vec<u8>, ClientError> {
        let request =
            RawRequest::api(&projects::attachment_content(id)).with_query(&PathQuery { path })?;
        self.execute_bytes(&request).await
    }

    pub async fn list_thread_sections(&self) -> Result<Vec<ThreadSection>, ClientError> {
        self.get(thread_sections::list()).await
    }

    pub async fn create_thread_section(
        &self,
        request: &CreateThreadSectionRequest,
    ) -> Result<ThreadSectionMutationResponse, ClientError> {
        self.call_with(thread_sections::create(), request).await
    }

    pub async fn update_thread_section(
        &self,
        request: &UpdateThreadSectionRequest,
    ) -> Result<ThreadSectionMutationResponse, ClientError> {
        self.call_with(thread_sections::update(), request).await
    }

    pub async fn delete_thread_section(
        &self,
        request: &DeleteThreadSectionRequest,
    ) -> Result<ThreadSectionMutationResponse, ClientError> {
        self.call_with(thread_sections::delete(), request).await
    }

    pub async fn list_threads(
        &self,
        query: &ThreadListQuery,
    ) -> Result<ThreadListResponse, ClientError> {
        self.get_with(threads::list(), query).await
    }

    pub async fn count_threads(
        &self,
        query: &ThreadCountQuery,
    ) -> Result<ThreadCountResponse, ClientError> {
        self.get_with(threads::count(), query).await
    }

    pub async fn running_threads(&self) -> Result<JsonValue, ClientError> {
        self.get(threads::running()).await
    }

    pub async fn search_threads(
        &self,
        query: &ThreadSearchQuery,
    ) -> Result<ThreadSearchResponse, ClientError> {
        self.get_with(threads::search(), query).await
    }

    pub async fn resolve_thread_mentions(
        &self,
        request: &ResolveThreadMentionsRequest,
    ) -> Result<JsonValue, ClientError> {
        self.call_with(threads::resolve_mentions(), request).await
    }

    pub async fn create_thread(
        &self,
        request: &CreateThreadRequest,
    ) -> Result<ThreadResponse, ClientError> {
        self.call_with(threads::create(), request).await
    }

    pub async fn fork_thread(&self, request: &JsonValue) -> Result<ThreadResponse, ClientError> {
        self.call_with(threads::fork(), request).await
    }

    pub async fn get_thread(
        &self,
        id: &str,
        query: &ThreadGetQuery,
    ) -> Result<ThreadResponse, ClientError> {
        self.get_with(threads::get(id), query).await
    }

    pub async fn update_thread(
        &self,
        id: &str,
        request: &UpdateThreadRequest,
    ) -> Result<ThreadResponse, ClientError> {
        self.call_with(threads::update(id), request).await
    }

    pub async fn delete_thread(
        &self,
        id: &str,
        request: &DeleteThreadRequest,
    ) -> Result<OkResponse, ClientError> {
        self.call_with(threads::delete(id), request).await
    }

    pub async fn thread_plugin_metadata(
        &self,
        id: &str,
        plugin_id: &str,
    ) -> Result<JsonValue, ClientError> {
        self.get_with(threads::plugin_metadata(id), &PluginIdQuery { plugin_id })
            .await
    }

    pub async fn update_thread_plugin_metadata(
        &self,
        id: &str,
        plugin_id: &str,
        set: Option<&ThreadPluginMetadata>,
        remove: &[String],
    ) -> Result<JsonValue, ClientError> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Body<'a> {
            plugin_id: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            set: Option<&'a ThreadPluginMetadata>,
            #[serde(skip_serializing_if = "<[String]>::is_empty")]
            remove: &'a [String],
        }
        self.call_with(
            threads::update_plugin_metadata(id),
            &Body {
                plugin_id,
                set,
                remove,
            },
        )
        .await
    }

    pub async fn thread_child_summary(
        &self,
        id: &str,
    ) -> Result<ThreadChildSummaryResponse, ClientError> {
        self.get(threads::child_summary(id)).await
    }

    pub async fn send_message(
        &self,
        id: &str,
        request: &SendMessageRequest,
    ) -> Result<SendMessageResponse, ClientError> {
        self.call_with(threads::send(id), request).await
    }

    pub async fn edit_message(
        &self,
        id: &str,
        request: &EditMessageRequest,
    ) -> Result<EditMessageResponse, ClientError> {
        self.call_with(threads::edit_message(id), request).await
    }

    pub async fn retry_turn(
        &self,
        id: &str,
        request: &RetryTurnRequest,
    ) -> Result<RetryTurnResponse, ClientError> {
        self.call_with(threads::retry(id), request).await
    }

    pub async fn stop_thread(&self, id: &str) -> Result<OkResponse, ClientError> {
        self.call(threads::stop(id)).await
    }

    pub async fn compact_thread(&self, id: &str) -> Result<OkResponse, ClientError> {
        self.call(threads::compact(id)).await
    }

    pub async fn clear_thread_context(&self, id: &str) -> Result<OkResponse, ClientError> {
        self.call(threads::clear_context(id)).await
    }

    pub async fn cancel_thread_plan(&self, id: &str) -> Result<OkResponse, ClientError> {
        self.call(threads::cancel_plan(id)).await
    }

    pub async fn clear_thread_goal(&self, id: &str) -> Result<OkResponse, ClientError> {
        self.call(threads::clear_goal(id)).await
    }

    pub async fn open_thread(
        &self,
        id: &str,
        request: &ThreadOpenRequest,
    ) -> Result<DeliveredResponse, ClientError> {
        self.call_with(threads::open(id), request).await
    }

    pub async fn pin_thread(&self, id: &str) -> Result<ThreadResponse, ClientError> {
        self.call(threads::pin(id)).await
    }

    pub async fn unpin_thread(&self, id: &str) -> Result<ThreadResponse, ClientError> {
        self.call(threads::unpin(id)).await
    }

    pub async fn reorder_pinned_thread(
        &self,
        id: &str,
        request: &ReorderPinnedThreadRequest,
    ) -> Result<ThreadListResponse, ClientError> {
        self.call_with(threads::pin_order(id), request).await
    }

    pub async fn archive_thread(&self, id: &str) -> Result<ThreadArchiveAllResponse, ClientError> {
        self.call(threads::archive_all(id)).await
    }

    pub async fn unarchive_thread(&self, id: &str) -> Result<OkResponse, ClientError> {
        self.call(threads::unarchive(id)).await
    }

    pub async fn restore_thread_environment(
        &self,
        id: &str,
    ) -> Result<ThreadResponse, ClientError> {
        self.call(threads::restore_environment(id)).await
    }

    pub async fn mark_thread_read(&self, id: &str) -> Result<ThreadResponse, ClientError> {
        self.call(threads::mark_read(id)).await
    }

    pub async fn mark_thread_unread(&self, id: &str) -> Result<ThreadResponse, ClientError> {
        self.call(threads::mark_unread(id)).await
    }

    pub async fn thread_timeline(
        &self,
        id: &str,
        query: &ThreadTimelineQuery,
    ) -> Result<ThreadTimelineResponse, ClientError> {
        self.get_with(threads::timeline(id), query).await
    }

    pub async fn thread_turn_summary_details(
        &self,
        id: &str,
        query: &TurnSummaryDetailsQuery,
    ) -> Result<TurnSummaryDetailsResponse, ClientError> {
        self.get_with(threads::timeline_turn_summary_details(id), query)
            .await
    }

    pub async fn thread_context(&self, id: &str) -> Result<ThreadContextResponse, ClientError> {
        self.get(threads::context(id)).await
    }

    pub async fn thread_output(&self, id: &str) -> Result<ThreadOutputResponse, ClientError> {
        self.get(threads::output(id)).await
    }

    pub async fn thread_message(
        &self,
        id: &str,
        seq: i64,
    ) -> Result<ThreadMessageResponse, ClientError> {
        self.get(threads::message(id, seq)).await
    }

    pub async fn thread_default_execution_options(
        &self,
        id: &str,
    ) -> Result<Option<ResolvedThreadExecutionOptions>, ClientError> {
        self.get(threads::default_execution_options(id)).await
    }

    pub async fn thread_prompt_history(
        &self,
        id: &str,
        query: &PromptHistoryQuery,
    ) -> Result<Vec<PromptHistoryEntry>, ClientError> {
        self.get_with(threads::prompt_history(id), query).await
    }

    pub async fn queued_messages(
        &self,
        id: &str,
    ) -> Result<ThreadQueuedMessageListResponse, ClientError> {
        self.get(threads::queued_messages(id)).await
    }

    pub async fn create_queued_message(
        &self,
        id: &str,
        request: &CreateQueuedMessageRequest,
    ) -> Result<ThreadQueuedMessage, ClientError> {
        self.call_with(threads::create_queued_message(id), request)
            .await
    }

    pub async fn set_queued_message_group_boundary(
        &self,
        id: &str,
        request: &SetQueuedMessageGroupBoundaryRequest,
    ) -> Result<ThreadQueuedMessageListResponse, ClientError> {
        self.call_with(threads::set_queued_message_group_boundary(id), request)
            .await
    }

    pub async fn update_queued_message(
        &self,
        id: &str,
        queued_message_id: &str,
        request: &UpdateQueuedMessageRequest,
    ) -> Result<ThreadQueuedMessage, ClientError> {
        self.call_with(
            threads::update_queued_message(id, queued_message_id),
            request,
        )
        .await
    }

    pub async fn delete_queued_message(
        &self,
        id: &str,
        queued_message_id: &str,
    ) -> Result<OkResponse, ClientError> {
        self.call(threads::delete_queued_message(id, queued_message_id))
            .await
    }

    pub async fn send_queued_message(
        &self,
        id: &str,
        queued_message_id: &str,
        request: &SendQueuedMessageRequest,
    ) -> Result<SendQueuedMessageResponse, ClientError> {
        self.call_with(threads::send_queued_message(id, queued_message_id), request)
            .await
    }

    pub async fn hold_queued_message_for_edit(
        &self,
        id: &str,
        queued_message_id: &str,
    ) -> Result<QueuedMessageEditHoldResponse, ClientError> {
        self.call(threads::hold_queued_message_for_edit(id, queued_message_id))
            .await
    }

    pub async fn release_queued_message_edit_hold(
        &self,
        id: &str,
        queued_message_id: &str,
    ) -> Result<OkResponse, ClientError> {
        self.call(threads::release_queued_message_edit_hold(
            id,
            queued_message_id,
        ))
        .await
    }

    pub async fn reorder_queued_message(
        &self,
        id: &str,
        queued_message_id: &str,
        request: &ReorderQueuedMessageRequest,
    ) -> Result<ThreadQueuedMessageListResponse, ClientError> {
        self.call_with(
            threads::reorder_queued_message(id, queued_message_id),
            request,
        )
        .await
    }

    pub async fn thread_interactions(
        &self,
        id: &str,
    ) -> Result<ThreadPendingInteractionsResponse, ClientError> {
        self.get(threads::interactions(id)).await
    }

    pub async fn thread_interaction(
        &self,
        id: &str,
        interaction_id: &str,
    ) -> Result<PendingInteraction, ClientError> {
        self.get(threads::interaction(id, interaction_id)).await
    }

    pub async fn resolve_interaction(
        &self,
        id: &str,
        interaction_id: &str,
        request: &ResolvePendingInteractionRequest,
    ) -> Result<PendingInteraction, ClientError> {
        self.call_with(threads::resolve_interaction(id, interaction_id), request)
            .await
    }

    pub async fn respond_to_interaction(
        &self,
        id: &str,
        interaction_id: &str,
        request: &RespondPluginInteractionRequest,
    ) -> Result<PendingInteraction, ClientError> {
        self.call_with(threads::respond_to_interaction(id, interaction_id), request)
            .await
    }

    pub async fn cancel_interaction(
        &self,
        id: &str,
        interaction_id: &str,
    ) -> Result<PendingInteraction, ClientError> {
        self.call(threads::cancel_interaction(id, interaction_id))
            .await
    }

    pub async fn thread_storage_location(
        &self,
        id: &str,
    ) -> Result<ThreadStorageLocationResponse, ClientError> {
        self.get(threads::storage_location(id)).await
    }

    pub async fn thread_storage_files(
        &self,
        id: &str,
        query: &WorkspaceFilesQuery,
    ) -> Result<WorkspaceFileListResponse, ClientError> {
        self.get_with(threads::storage_files(id), query).await
    }

    pub async fn thread_storage_paths(
        &self,
        id: &str,
        query: &WorkspacePathsQuery,
    ) -> Result<WorkspacePathListResponse, ClientError> {
        self.get_with(threads::storage_paths(id), query).await
    }

    pub async fn thread_storage_file(
        &self,
        id: &str,
        file_path: &str,
    ) -> Result<Vec<u8>, ClientError> {
        self.get_bytes(threads::storage_file(id, file_path)).await
    }

    pub async fn list_queued_messages(
        &self,
        query: &QueuedMessageListQuery,
    ) -> Result<ThreadQueuedMessageListResponse, ClientError> {
        self.get_with(queue::list(), query).await
    }

    pub async fn list_environments(
        &self,
        query: &ListEnvironmentsQuery,
    ) -> Result<Vec<Environment>, ClientError> {
        self.get_with(environments::list(), query).await
    }

    pub async fn get_environment(&self, id: &str) -> Result<Environment, ClientError> {
        self.get(environments::get(id)).await
    }

    pub async fn update_environment(
        &self,
        id: &str,
        request: &UpdateEnvironmentRequest,
    ) -> Result<Environment, ClientError> {
        self.call_with(environments::update(id), request).await
    }

    pub async fn delete_environment(&self, id: &str) -> Result<JsonValue, ClientError> {
        self.call(environments::delete(id)).await
    }

    pub async fn cleanup_environment(&self, id: &str) -> Result<OkResponse, ClientError> {
        self.call(environments::cleanup(id)).await
    }

    pub async fn environment_status(
        &self,
        id: &str,
        query: &EnvironmentStatusQuery,
    ) -> Result<EnvironmentStatusResponse, ClientError> {
        self.get_with(environments::status(id), query).await
    }

    pub async fn environment_pull_request(
        &self,
        id: &str,
    ) -> Result<EnvironmentPullRequestResponse, ClientError> {
        self.get(environments::pull_request(id)).await
    }

    pub async fn environment_diff(
        &self,
        id: &str,
        query: &EnvironmentDiffQuery,
    ) -> Result<EnvironmentDiffResponse, ClientError> {
        self.get_with(environments::diff(id), query).await
    }

    pub async fn environment_diff_files(
        &self,
        id: &str,
        query: &EnvironmentDiffQuery,
    ) -> Result<EnvironmentDiffFilesResponse, ClientError> {
        self.get_with(environments::diff_files(id), query).await
    }

    pub async fn environment_diff_patch(
        &self,
        id: &str,
        request: &EnvironmentDiffPatchRequest,
    ) -> Result<EnvironmentDiffPatchResponse, ClientError> {
        self.call_with(environments::diff_patch(id), request).await
    }

    pub async fn environment_diff_file(
        &self,
        id: &str,
        query: &EnvironmentDiffFileQuery,
    ) -> Result<EnvironmentDiffFileResponse, ClientError> {
        self.get_with(environments::diff_file(id), query).await
    }

    pub async fn environment_diff_branches(
        &self,
        id: &str,
        query: &BranchListQuery,
    ) -> Result<GitBranchOptions, ClientError> {
        self.get_with(environments::diff_branches(id), query).await
    }

    pub async fn environment_paths(
        &self,
        id: &str,
        query: &WorkspacePathsQuery,
    ) -> Result<WorkspacePathListResponse, ClientError> {
        self.get_with(environments::paths(id), query).await
    }

    pub async fn environment_file(
        &self,
        id: &str,
        file_path: &str,
    ) -> Result<Vec<u8>, ClientError> {
        self.get_bytes(environments::file(id, file_path)).await
    }

    pub async fn environment_action(
        &self,
        id: &str,
        request: &EnvironmentActionRequest,
    ) -> Result<EnvironmentActionResponse, ClientError> {
        self.call_with(environments::actions(id), request).await
    }

    pub async fn archive_environment_threads(
        &self,
        id: &str,
    ) -> Result<EnvironmentArchiveThreadsResponse, ClientError> {
        self.call(environments::archive_threads(id)).await
    }

    pub async fn list_hosts(&self) -> Result<Vec<Host>, ClientError> {
        self.get(hosts::list()).await
    }

    pub async fn get_host(&self, id: &str) -> Result<Host, ClientError> {
        self.get(hosts::get(id)).await
    }

    pub async fn update_host(
        &self,
        id: &str,
        request: &UpdateHostRequest,
    ) -> Result<Host, ClientError> {
        self.call_with(hosts::update(id), request).await
    }

    pub async fn update_host_permission_ceiling(
        &self,
        id: &str,
        request: &UpdateHostPermissionCeilingRequest,
    ) -> Result<Host, ClientError> {
        self.call_with(hosts::permission_ceiling(id), request).await
    }

    pub async fn host_provider_cli_status(
        &self,
        id: &str,
    ) -> Result<HostProviderCliStatusResponse, ClientError> {
        self.get(hosts::provider_cli_status(id)).await
    }

    pub async fn list_terminals(
        &self,
        query: &TerminalListQuery,
    ) -> Result<TerminalListResponse, ClientError> {
        self.get_with(terminals::list(), query).await
    }

    pub async fn create_terminal(
        &self,
        request: &CreateTerminalRequest,
    ) -> Result<TerminalSession, ClientError> {
        self.call_with(terminals::create(), request).await
    }

    pub async fn get_terminal(&self, id: &str) -> Result<TerminalSession, ClientError> {
        self.get(terminals::get(id)).await
    }

    pub async fn update_terminal(
        &self,
        id: &str,
        request: &UpdateTerminalRequest,
    ) -> Result<TerminalSession, ClientError> {
        self.call_with(terminals::update(id), request).await
    }

    pub async fn restart_terminal(&self, id: &str) -> Result<TerminalSession, ClientError> {
        self.call_with(terminals::restart(id), &RestartTerminalRequest {})
            .await
    }

    pub async fn close_terminal(
        &self,
        id: &str,
        request: &CloseTerminalRequest,
    ) -> Result<TerminalSession, ClientError> {
        self.call_with(terminals::close(id), request).await
    }

    pub async fn terminal_input(
        &self,
        id: &str,
        request: &TerminalInputRequest,
    ) -> Result<TerminalSession, ClientError> {
        self.call_with(terminals::input(id), request).await
    }

    pub async fn resize_terminal(
        &self,
        id: &str,
        request: &TerminalResizeRequest,
    ) -> Result<TerminalSession, ClientError> {
        self.call_with(terminals::resize(id), request).await
    }

    pub async fn terminal_output(
        &self,
        id: &str,
        query: &TerminalOutputQuery,
    ) -> Result<TerminalOutputResponse, ClientError> {
        self.get_with(terminals::output(id), query).await
    }

    pub async fn list_plugins(&self) -> Result<PluginListResponse, ClientError> {
        self.get(plugins::list()).await
    }

    pub async fn plugin_contributions(&self) -> Result<PluginContributionsResponse, ClientError> {
        self.get(plugins::contributions()).await
    }

    pub async fn plugin_mention_search(
        &self,
        query: &PluginMentionSearchQuery,
    ) -> Result<PluginMentionSearchResponse, ClientError> {
        self.get_with(plugins::mention_search(), query).await
    }

    pub async fn run_plugin_cli(
        &self,
        plugin_id: &str,
        request: &PluginCliRequest,
    ) -> Result<PluginCliResult, ClientError> {
        let raw = RawRequest::api(&plugins::cli(plugin_id)).with_json(request)?;
        let body = self.execute_text(&raw).await?;
        PluginCliResult::parse_body(&body).map_err(|source| ClientError::Decode {
            route: raw.path.clone(),
            source,
        })
    }

    pub async fn plugin_settings(
        &self,
        plugin_id: &str,
    ) -> Result<PluginSettingsResponse, ClientError> {
        self.get(plugins::settings(plugin_id)).await
    }

    pub async fn update_plugin_settings(
        &self,
        plugin_id: &str,
        request: &PluginSettingsUpdateRequest,
    ) -> Result<PluginSettingsResponse, ClientError> {
        self.call_with(plugins::update_settings(plugin_id), request)
            .await
    }

    pub async fn enable_plugin(&self, plugin_id: &str) -> Result<JsonValue, ClientError> {
        self.call(plugins::enable(plugin_id)).await
    }

    pub async fn disable_plugin(&self, plugin_id: &str) -> Result<JsonValue, ClientError> {
        self.call(plugins::disable(plugin_id)).await
    }

    pub async fn plugin_logs(&self, plugin_id: &str, tail: u32) -> Result<JsonValue, ClientError> {
        self.get_with(plugins::logs(plugin_id), &TailQuery { tail })
            .await
    }

    pub async fn plugin_rpc<B: Serialize + ?Sized>(
        &self,
        plugin_id: &str,
        method: &str,
        input: &B,
    ) -> Result<JsonValue, ClientError> {
        let response: PluginRpcResponse = self
            .call_with(plugins::rpc(plugin_id, method), input)
            .await?;
        response
            .into_result()
            .map_err(|error| ClientError::Plugin(error.message()))
    }

    pub async fn plugin_rpc_without_input(
        &self,
        plugin_id: &str,
        method: &str,
    ) -> Result<JsonValue, ClientError> {
        let response: PluginRpcResponse = self.call(plugins::rpc(plugin_id, method)).await?;
        response
            .into_result()
            .map_err(|error| ClientError::Plugin(error.message()))
    }
}
