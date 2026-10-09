use std::future::Future;
use std::pin::Pin;

use bb_client::BbClient;
use bb_wire::common::JsonValue;
use bb_wire::environment::{
    EnvironmentActionRequest, EnvironmentActionResponse, EnvironmentDiffFileQuery,
    EnvironmentDiffFileResponse, EnvironmentDiffFilesResponse, EnvironmentDiffPatchRequest,
    EnvironmentDiffPatchResponse, EnvironmentDiffQuery, EnvironmentPullRequestResponse,
    EnvironmentStatusQuery, EnvironmentStatusResponse,
};
use bb_wire::interaction::{
    InteractionResolution, PendingInteraction, RespondPluginInteractionRequest,
};
use bb_wire::project::{ProjectBranchesQuery, ProjectBranchesResponse, PromptHistoryQuery};
use bb_wire::prompt::{
    ProjectExecutionDefaults, PromptHistoryEntry, ResolvedThreadExecutionOptions,
};
use bb_wire::queue::{
    CreateQueuedMessageRequest, SendMessageRequest, SendMessageResponse, SendQueuedMessageMode,
    SendQueuedMessageRequest, UpdateQueuedMessageRequest,
};
use bb_wire::system::{
    AppKeybindingOverride, SystemEnvironmentProvider, SystemEnvironmentProvidersQuery,
    SystemExecutionOptionsQuery, SystemExecutionOptionsResponse,
};
use bb_wire::thread::{CreateThreadRequest, ThreadResponse, UpdateThreadRequest};

use bb_wire::plugin::{
    PluginCliContribution, PluginCliRequest, PluginCliResult, PluginMentionProviderContribution,
    PluginMentionSearchGroup, PluginMentionSearchQuery,
};
use bb_wire::project::{
    ProjectCommandsQuery, ProviderCommand, UploadedPromptAttachment, WorkspacePathListResponse,
    WorkspacePathsQuery,
};
use bb_wire::thread::{ThreadSearchQuery, ThreadSearchResponse};

use crate::new_thread::CreationConfig;
use crate::palette::FileSource;

pub type ApiFuture<T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send>>;

#[derive(Clone, PartialEq)]
pub struct InteractionValue {
    pub value: JsonValue,
    pub sensitive: bool,
}

impl std::fmt::Debug for InteractionValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.sensitive {
            f.write_str("InteractionValue(<redacted>)")
        } else {
            write!(f, "InteractionValue({})", self.value)
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Mutation {
    Rename {
        thread_id: String,
        title: String,
    },
    Archive {
        thread_id: String,
    },
    Unarchive {
        thread_id: String,
    },
    MarkRead {
        thread_id: String,
    },
    MarkUnread {
        thread_id: String,
    },
    Pin {
        thread_id: String,
    },
    Unpin {
        thread_id: String,
    },
    SendMessage {
        thread_id: String,
        request: Box<SendMessageRequest>,
    },
    QueueMessage {
        thread_id: String,
        request: Box<CreateQueuedMessageRequest>,
    },
    SendQueued {
        thread_id: String,
        queued_message_id: String,
        mode: SendQueuedMessageMode,
    },
    UpdateQueued {
        thread_id: String,
        queued_message_id: String,
        request: Box<UpdateQueuedMessageRequest>,
    },
    DeleteQueued {
        thread_id: String,
        queued_message_id: String,
    },
    HoldQueued {
        thread_id: String,
        queued_message_id: String,
    },
    ReleaseQueued {
        thread_id: String,
        queued_message_id: String,
    },
    Stop {
        thread_id: String,
    },
    ResolveInteraction {
        thread_id: String,
        interaction_id: String,
        resolution: Box<InteractionResolution>,
    },
    RespondInteraction {
        thread_id: String,
        interaction_id: String,
        value: InteractionValue,
    },
    CancelInteraction {
        thread_id: String,
        interaction_id: String,
    },
    EnvironmentAction {
        thread_id: String,
        environment_id: String,
        request: EnvironmentActionRequest,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum MutationReply {
    Done,
    Delivery(SendMessageResponse),
    Lease { lease_ms: u64 },
    EnvironmentAction(Box<EnvironmentActionResponse>),
}

impl Mutation {
    pub fn thread_id(&self) -> &str {
        match self {
            Self::Rename { thread_id, .. }
            | Self::Archive { thread_id }
            | Self::Unarchive { thread_id }
            | Self::MarkRead { thread_id }
            | Self::MarkUnread { thread_id }
            | Self::Pin { thread_id }
            | Self::Unpin { thread_id }
            | Self::SendMessage { thread_id, .. }
            | Self::QueueMessage { thread_id, .. }
            | Self::SendQueued { thread_id, .. }
            | Self::UpdateQueued { thread_id, .. }
            | Self::DeleteQueued { thread_id, .. }
            | Self::HoldQueued { thread_id, .. }
            | Self::ReleaseQueued { thread_id, .. }
            | Self::Stop { thread_id }
            | Self::ResolveInteraction { thread_id, .. }
            | Self::RespondInteraction { thread_id, .. }
            | Self::CancelInteraction { thread_id, .. }
            | Self::EnvironmentAction { thread_id, .. } => thread_id,
        }
    }

    pub fn interaction_id(&self) -> Option<&str> {
        match self {
            Self::ResolveInteraction { interaction_id, .. }
            | Self::RespondInteraction { interaction_id, .. }
            | Self::CancelInteraction { interaction_id, .. } => Some(interaction_id),
            _ => None,
        }
    }

    pub fn is_interaction(&self) -> bool {
        self.interaction_id().is_some()
    }

    pub fn queued_message_id(&self) -> Option<&str> {
        match self {
            Self::SendQueued {
                queued_message_id, ..
            }
            | Self::UpdateQueued {
                queued_message_id, ..
            }
            | Self::DeleteQueued {
                queued_message_id, ..
            }
            | Self::HoldQueued {
                queued_message_id, ..
            }
            | Self::ReleaseQueued {
                queued_message_id, ..
            } => Some(queued_message_id),
            _ => None,
        }
    }

    pub fn is_environment_action(&self) -> bool {
        matches!(self, Self::EnvironmentAction { .. })
    }

    pub fn is_composer(&self) -> bool {
        matches!(
            self,
            Self::SendMessage { .. }
                | Self::QueueMessage { .. }
                | Self::SendQueued { .. }
                | Self::UpdateQueued { .. }
                | Self::DeleteQueued { .. }
                | Self::HoldQueued { .. }
                | Self::ReleaseQueued { .. }
                | Self::Stop { .. }
        )
    }

    pub fn success_message(&self) -> &'static str {
        match self {
            Self::Rename { .. } => "Renamed",
            Self::Archive { .. } => "Archived (u to undo)",
            Self::Unarchive { .. } => "Unarchived",
            Self::MarkRead { .. } => "Marked as read",
            Self::MarkUnread { .. } => "Marked as unread",
            Self::Pin { .. } => "Pinned",
            Self::Unpin { .. } => "Unpinned",
            Self::SendMessage { .. } => "Sent",
            Self::QueueMessage { .. } => "Queued",
            Self::SendQueued { .. } => "Sent queued message",
            Self::UpdateQueued { .. } => "Queued message updated",
            Self::DeleteQueued { .. } => "Queued message deleted",
            Self::HoldQueued { .. } | Self::ReleaseQueued { .. } => "",
            Self::Stop { .. } => "Stopping",
            Self::ResolveInteraction { .. } | Self::RespondInteraction { .. } => "Answered",
            Self::CancelInteraction { .. } => "Request cancelled",
            Self::EnvironmentAction { request, .. } => match request {
                EnvironmentActionRequest::Commit => "Commit created",
                EnvironmentActionRequest::PullRequestReady => "Pull request marked ready",
                EnvironmentActionRequest::PullRequestMerge { .. } => "Pull request merged",
                EnvironmentActionRequest::PullRequestDraft => "Pull request converted to draft",
            },
        }
    }

    pub fn failure_verb(&self) -> &'static str {
        match self {
            Self::Rename { .. } => "rename",
            Self::Archive { .. } => "archive",
            Self::Unarchive { .. } => "unarchive",
            Self::MarkRead { .. } => "mark read",
            Self::MarkUnread { .. } => "mark unread",
            Self::Pin { .. } => "pin",
            Self::Unpin { .. } => "unpin",
            Self::SendMessage { .. } => "send message",
            Self::QueueMessage { .. } => "queue message",
            Self::SendQueued { .. } => "send queued message",
            Self::UpdateQueued { .. } => "update queued message",
            Self::DeleteQueued { .. } => "delete queued message",
            Self::HoldQueued { .. } => "edit queued message",
            Self::ReleaseQueued { .. } => "release queued message",
            Self::Stop { .. } => "stop",
            Self::ResolveInteraction { .. } | Self::RespondInteraction { .. } => "answer request",
            Self::CancelInteraction { .. } => "cancel request",
            Self::EnvironmentAction { request, .. } => match request {
                EnvironmentActionRequest::Commit => "commit",
                EnvironmentActionRequest::PullRequestReady => "mark the pull request ready",
                EnvironmentActionRequest::PullRequestMerge { .. } => "merge the pull request",
                EnvironmentActionRequest::PullRequestDraft => "convert the pull request to draft",
            },
        }
    }

    pub fn is_silent(&self) -> bool {
        matches!(
            self,
            Self::MarkRead { .. } | Self::HoldQueued { .. } | Self::ReleaseQueued { .. }
        )
    }
}

pub trait Api: Send + Sync + 'static {
    fn mutate(&self, mutation: Mutation) -> ApiFuture<MutationReply>;

    fn keybinding_overrides(&self) -> ApiFuture<Vec<AppKeybindingOverride>>;

    fn execution_options(
        &self,
        query: SystemExecutionOptionsQuery,
    ) -> ApiFuture<SystemExecutionOptionsResponse>;

    fn default_execution_options(
        &self,
        thread_id: String,
    ) -> ApiFuture<Option<ResolvedThreadExecutionOptions>>;

    fn prompt_history(&self, thread_id: String) -> ApiFuture<Vec<PromptHistoryEntry>>;

    fn environment_providers(
        &self,
        query: SystemEnvironmentProvidersQuery,
    ) -> ApiFuture<Vec<SystemEnvironmentProvider>>;

    fn creation_config(&self) -> ApiFuture<CreationConfig>;

    fn project_default_execution_options(
        &self,
        project_id: String,
    ) -> ApiFuture<Option<ProjectExecutionDefaults>>;

    fn branch_options(
        &self,
        project_id: String,
        query: ProjectBranchesQuery,
    ) -> ApiFuture<ProjectBranchesResponse>;

    fn create_thread(&self, request: CreateThreadRequest) -> ApiFuture<ThreadResponse>;

    fn plugin_contributions(&self) -> ApiFuture<Vec<PluginCliContribution>>;

    fn search_threads(&self, query: ThreadSearchQuery) -> ApiFuture<ThreadSearchResponse>;

    fn workspace_paths(
        &self,
        source: FileSource,
        query: WorkspacePathsQuery,
    ) -> ApiFuture<WorkspacePathListResponse>;

    fn workspace_file(&self, source: FileSource, path: String) -> ApiFuture<Vec<u8>>;

    fn run_plugin_cli(
        &self,
        plugin_id: String,
        request: PluginCliRequest,
    ) -> ApiFuture<PluginCliResult>;
    fn mention_providers(&self) -> ApiFuture<Vec<PluginMentionProviderContribution>> {
        unsupported("mention providers")
    }
    fn plugin_mentions(
        &self,
        _query: PluginMentionSearchQuery,
    ) -> ApiFuture<Vec<PluginMentionSearchGroup>> {
        unsupported("plugin mention search")
    }
    fn thread_storage_paths(
        &self,
        _thread_id: String,
        _query: WorkspacePathsQuery,
    ) -> ApiFuture<WorkspacePathListResponse> {
        unsupported("thread storage paths")
    }
    fn project_commands(
        &self,
        _project_id: String,
        _query: ProjectCommandsQuery,
    ) -> ApiFuture<Vec<ProviderCommand>> {
        unsupported("project commands")
    }
    fn upload_attachment(&self, _upload: AttachmentUpload) -> ApiFuture<UploadedPromptAttachment> {
        unsupported("attachment upload")
    }
    fn environment_status(
        &self,
        _environment_id: String,
        _query: EnvironmentStatusQuery,
    ) -> ApiFuture<EnvironmentStatusResponse> {
        unsupported("workspace status")
    }
    fn environment_pull_request(
        &self,
        _environment_id: String,
    ) -> ApiFuture<EnvironmentPullRequestResponse> {
        unsupported("pull request status")
    }
    fn environment_diff_files(
        &self,
        _environment_id: String,
        _query: EnvironmentDiffQuery,
    ) -> ApiFuture<EnvironmentDiffFilesResponse> {
        unsupported("git diff")
    }
    fn environment_diff_patch(
        &self,
        _environment_id: String,
        _request: EnvironmentDiffPatchRequest,
    ) -> ApiFuture<EnvironmentDiffPatchResponse> {
        unsupported("git diff patches")
    }
    fn environment_diff_file(
        &self,
        _environment_id: String,
        _query: EnvironmentDiffFileQuery,
    ) -> ApiFuture<EnvironmentDiffFileResponse> {
        unsupported("diff file contents")
    }

    fn thread_interactions(&self, _thread_id: String) -> ApiFuture<Vec<PendingInteraction>> {
        unsupported("pending interactions")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct AttachmentUpload {
    pub project_id: String,
    pub file_name: String,
    pub mime_type: Option<String>,
    pub bytes: Vec<u8>,
}

impl std::fmt::Debug for AttachmentUpload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AttachmentUpload")
            .field("project_id", &self.project_id)
            .field("file_name", &self.file_name)
            .field("mime_type", &self.mime_type)
            .field("bytes", &self.bytes.len())
            .finish()
    }
}

fn unsupported<T: Send + 'static>(what: &'static str) -> ApiFuture<T> {
    Box::pin(async move { Err(format!("{what} is not available")) })
}

fn plugin_error_message(error: Option<&bb_wire::plugin::PluginErrorValue>) -> String {
    match error {
        Some(bb_wire::plugin::PluginErrorValue::Message(message)) => message.clone(),
        Some(other) => serde_json::to_string(other).unwrap_or_else(|_| "plugin error".into()),
        None => "plugin mention search failed".into(),
    }
}

pub fn creation_config_from(config: &bb_wire::system::SystemConfigResponse) -> CreationConfig {
    CreationConfig {
        primary_host_id: config.primary_host_id.clone().filter(|id| !id.is_empty()),
        allow_fast_service_tier: config
            .general_settings
            .get("allowFastServiceTier")
            .and_then(JsonValue::as_bool)
            .unwrap_or(true),
    }
}

fn done<T>(_: T) -> MutationReply {
    MutationReply::Done
}

async fn run_mutation(
    client: &BbClient,
    mutation: &Mutation,
) -> Result<MutationReply, bb_client::ClientError> {
    Ok(match mutation {
        Mutation::Rename { thread_id, title } => client
            .update_thread(thread_id, &UpdateThreadRequest::rename(title.clone()))
            .await
            .map(done)?,
        Mutation::Archive { thread_id } => client.archive_thread(thread_id).await.map(done)?,
        Mutation::Unarchive { thread_id } => client.unarchive_thread(thread_id).await.map(done)?,
        Mutation::MarkRead { thread_id } => client.mark_thread_read(thread_id).await.map(done)?,
        Mutation::MarkUnread { thread_id } => {
            client.mark_thread_unread(thread_id).await.map(done)?
        }
        Mutation::Pin { thread_id } => client.pin_thread(thread_id).await.map(done)?,
        Mutation::Unpin { thread_id } => client.unpin_thread(thread_id).await.map(done)?,
        Mutation::SendMessage { thread_id, request } => {
            MutationReply::Delivery(client.send_message(thread_id, request).await?)
        }
        Mutation::QueueMessage { thread_id, request } => client
            .create_queued_message(thread_id, request)
            .await
            .map(done)?,
        Mutation::SendQueued {
            thread_id,
            queued_message_id,
            mode,
        } => MutationReply::Delivery(
            client
                .send_queued_message(
                    thread_id,
                    queued_message_id,
                    &SendQueuedMessageRequest { mode: *mode },
                )
                .await?,
        ),
        Mutation::UpdateQueued {
            thread_id,
            queued_message_id,
            request,
        } => client
            .update_queued_message(thread_id, queued_message_id, request)
            .await
            .map(done)?,
        Mutation::DeleteQueued {
            thread_id,
            queued_message_id,
        } => client
            .delete_queued_message(thread_id, queued_message_id)
            .await
            .map(done)?,
        Mutation::HoldQueued {
            thread_id,
            queued_message_id,
        } => MutationReply::Lease {
            lease_ms: client
                .hold_queued_message_for_edit(thread_id, queued_message_id)
                .await?
                .lease_ms,
        },
        Mutation::ReleaseQueued {
            thread_id,
            queued_message_id,
        } => client
            .release_queued_message_edit_hold(thread_id, queued_message_id)
            .await
            .map(done)?,
        Mutation::Stop { thread_id } => client.stop_thread(thread_id).await.map(done)?,
        Mutation::ResolveInteraction {
            thread_id,
            interaction_id,
            resolution,
        } => client
            .resolve_interaction(thread_id, interaction_id, resolution)
            .await
            .map(done)?,
        Mutation::RespondInteraction {
            thread_id,
            interaction_id,
            value,
        } => client
            .respond_to_interaction(
                thread_id,
                interaction_id,
                &RespondPluginInteractionRequest {
                    value: value.value.clone(),
                },
            )
            .await
            .map(done)?,
        Mutation::CancelInteraction {
            thread_id,
            interaction_id,
        } => client
            .cancel_interaction(thread_id, interaction_id)
            .await
            .map(done)?,
        Mutation::EnvironmentAction {
            environment_id,
            request,
            ..
        } => MutationReply::EnvironmentAction(Box::new(
            client.environment_action(environment_id, request).await?,
        )),
    })
}

impl Api for BbClient {
    fn mutate(&self, mutation: Mutation) -> ApiFuture<MutationReply> {
        let client = self.clone();
        Box::pin(async move {
            run_mutation(&client, &mutation)
                .await
                .map_err(|error| error.to_string())
        })
    }

    fn keybinding_overrides(&self) -> ApiFuture<Vec<AppKeybindingOverride>> {
        let client = self.clone();
        Box::pin(async move {
            client
                .system_config()
                .await
                .map(|config| config.keybinding_overrides)
                .map_err(|error| error.to_string())
        })
    }

    fn execution_options(
        &self,
        query: SystemExecutionOptionsQuery,
    ) -> ApiFuture<SystemExecutionOptionsResponse> {
        let client = self.clone();
        Box::pin(async move {
            client
                .system_execution_options(&query)
                .await
                .map_err(|error| error.to_string())
        })
    }

    fn default_execution_options(
        &self,
        thread_id: String,
    ) -> ApiFuture<Option<ResolvedThreadExecutionOptions>> {
        let client = self.clone();
        Box::pin(async move {
            client
                .thread_default_execution_options(&thread_id)
                .await
                .map_err(|error| error.to_string())
        })
    }

    fn prompt_history(&self, thread_id: String) -> ApiFuture<Vec<PromptHistoryEntry>> {
        let client = self.clone();
        Box::pin(async move {
            client
                .thread_prompt_history(&thread_id, &PromptHistoryQuery::default())
                .await
                .map_err(|error| error.to_string())
        })
    }

    fn environment_providers(
        &self,
        query: SystemEnvironmentProvidersQuery,
    ) -> ApiFuture<Vec<SystemEnvironmentProvider>> {
        let client = self.clone();
        Box::pin(async move {
            client
                .system_environment_providers(&query)
                .await
                .map(|response| response.providers)
                .map_err(|error| error.to_string())
        })
    }

    fn creation_config(&self) -> ApiFuture<CreationConfig> {
        let client = self.clone();
        Box::pin(async move {
            client
                .system_config()
                .await
                .map(|config| creation_config_from(&config))
                .map_err(|error| error.to_string())
        })
    }

    fn project_default_execution_options(
        &self,
        project_id: String,
    ) -> ApiFuture<Option<ProjectExecutionDefaults>> {
        let client = self.clone();
        Box::pin(async move {
            client
                .project_default_execution_options(&project_id)
                .await
                .map_err(|error| error.to_string())
        })
    }

    fn branch_options(
        &self,
        project_id: String,
        query: ProjectBranchesQuery,
    ) -> ApiFuture<ProjectBranchesResponse> {
        let client = self.clone();
        Box::pin(async move {
            client
                .project_branch_options(&project_id, &query)
                .await
                .map_err(|error| error.to_string())
        })
    }

    fn create_thread(&self, request: CreateThreadRequest) -> ApiFuture<ThreadResponse> {
        let client = self.clone();
        Box::pin(async move {
            client
                .create_thread(&request)
                .await
                .map_err(|error| error.to_string())
        })
    }

    fn plugin_contributions(&self) -> ApiFuture<Vec<PluginCliContribution>> {
        let client = self.clone();
        Box::pin(async move {
            client
                .plugin_contributions()
                .await
                .map(|response| response.cli_commands)
                .map_err(|error| error.to_string())
        })
    }

    fn search_threads(&self, query: ThreadSearchQuery) -> ApiFuture<ThreadSearchResponse> {
        let client = self.clone();
        Box::pin(async move {
            client
                .search_threads(&query)
                .await
                .map_err(|error| error.to_string())
        })
    }

    fn workspace_paths(
        &self,
        source: FileSource,
        query: WorkspacePathsQuery,
    ) -> ApiFuture<WorkspacePathListResponse> {
        let client = self.clone();
        Box::pin(async move {
            match &source {
                FileSource::Environment { environment_id, .. } => {
                    client.environment_paths(environment_id, &query).await
                }
                FileSource::Project { project_id, .. } => {
                    client.project_paths(project_id, &query).await
                }
            }
            .map_err(|error| error.to_string())
        })
    }

    fn workspace_file(&self, source: FileSource, path: String) -> ApiFuture<Vec<u8>> {
        let client = self.clone();
        Box::pin(async move {
            match &source {
                FileSource::Environment { environment_id, .. } => {
                    client.environment_file(environment_id, &path).await
                }
                FileSource::Project { project_id, .. } => {
                    client.project_file(project_id, &path).await
                }
            }
            .map_err(|error| error.to_string())
        })
    }

    fn run_plugin_cli(
        &self,
        plugin_id: String,
        request: PluginCliRequest,
    ) -> ApiFuture<PluginCliResult> {
        let client = self.clone();
        Box::pin(async move {
            client
                .run_plugin_cli(&plugin_id, &request)
                .await
                .map_err(|error| error.to_string())
        })
    }

    fn mention_providers(&self) -> ApiFuture<Vec<PluginMentionProviderContribution>> {
        let client = self.clone();
        Box::pin(async move {
            client
                .plugin_contributions()
                .await
                .map(|response| response.mention_providers)
                .map_err(|error| error.to_string())
        })
    }

    fn plugin_mentions(
        &self,
        query: PluginMentionSearchQuery,
    ) -> ApiFuture<Vec<PluginMentionSearchGroup>> {
        let client = self.clone();
        Box::pin(async move {
            let response = client
                .plugin_mention_search(&query)
                .await
                .map_err(|error| error.to_string())?;
            if response.ok {
                Ok(response.groups)
            } else {
                Err(plugin_error_message(response.error.as_ref()))
            }
        })
    }

    fn thread_storage_paths(
        &self,
        thread_id: String,
        query: WorkspacePathsQuery,
    ) -> ApiFuture<WorkspacePathListResponse> {
        let client = self.clone();
        Box::pin(async move {
            client
                .thread_storage_paths(&thread_id, &query)
                .await
                .map_err(|error| error.to_string())
        })
    }

    fn project_commands(
        &self,
        project_id: String,
        query: ProjectCommandsQuery,
    ) -> ApiFuture<Vec<ProviderCommand>> {
        let client = self.clone();
        Box::pin(async move {
            client
                .project_commands(&project_id, &query)
                .await
                .map(|response| response.commands)
                .map_err(|error| error.to_string())
        })
    }

    fn upload_attachment(&self, upload: AttachmentUpload) -> ApiFuture<UploadedPromptAttachment> {
        let client = self.clone();
        Box::pin(async move {
            client
                .upload_project_attachment(
                    &upload.project_id,
                    &upload.file_name,
                    upload.mime_type.as_deref(),
                    upload.bytes,
                )
                .await
                .map_err(|error| error.to_string())
        })
    }
    fn environment_status(
        &self,
        environment_id: String,
        query: EnvironmentStatusQuery,
    ) -> ApiFuture<EnvironmentStatusResponse> {
        let client = self.clone();
        Box::pin(async move {
            client
                .environment_status(&environment_id, &query)
                .await
                .map_err(|error| error.to_string())
        })
    }

    fn environment_pull_request(
        &self,
        environment_id: String,
    ) -> ApiFuture<EnvironmentPullRequestResponse> {
        let client = self.clone();
        Box::pin(async move {
            client
                .environment_pull_request(&environment_id)
                .await
                .map_err(|error| error.to_string())
        })
    }

    fn environment_diff_files(
        &self,
        environment_id: String,
        query: EnvironmentDiffQuery,
    ) -> ApiFuture<EnvironmentDiffFilesResponse> {
        let client = self.clone();
        Box::pin(async move {
            client
                .environment_diff_files(&environment_id, &query)
                .await
                .map_err(|error| error.to_string())
        })
    }

    fn environment_diff_patch(
        &self,
        environment_id: String,
        request: EnvironmentDiffPatchRequest,
    ) -> ApiFuture<EnvironmentDiffPatchResponse> {
        let client = self.clone();
        Box::pin(async move {
            client
                .environment_diff_patch(&environment_id, &request)
                .await
                .map_err(|error| error.to_string())
        })
    }

    fn environment_diff_file(
        &self,
        environment_id: String,
        query: EnvironmentDiffFileQuery,
    ) -> ApiFuture<EnvironmentDiffFileResponse> {
        let client = self.clone();
        Box::pin(async move {
            client
                .environment_diff_file(&environment_id, &query)
                .await
                .map_err(|error| error.to_string())
        })
    }

    fn thread_interactions(&self, thread_id: String) -> ApiFuture<Vec<PendingInteraction>> {
        let client = self.clone();
        Box::pin(async move {
            client
                .thread_interactions(&thread_id)
                .await
                .map_err(|error| error.to_string())
        })
    }
}
