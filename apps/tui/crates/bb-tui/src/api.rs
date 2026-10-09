use std::future::Future;
use std::pin::Pin;

use bb_client::BbClient;
use bb_wire::project::PromptHistoryQuery;
use bb_wire::prompt::{PromptHistoryEntry, ResolvedThreadExecutionOptions};
use bb_wire::queue::{
    CreateQueuedMessageRequest, SendMessageRequest, SendMessageResponse, SendQueuedMessageMode,
    SendQueuedMessageRequest, UpdateQueuedMessageRequest,
};
use bb_wire::system::{
    AppKeybindingOverride, SystemExecutionOptionsQuery, SystemExecutionOptionsResponse,
};
use bb_wire::thread::UpdateThreadRequest;

pub type ApiFuture<T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send>>;

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
}

#[derive(Debug, Clone, PartialEq)]
pub enum MutationReply {
    Done,
    Delivery(SendMessageResponse),
    Lease { lease_ms: u64 },
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
            | Self::Stop { thread_id } => thread_id,
        }
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
}
