use std::future::Future;
use std::pin::Pin;

use bb_client::BbClient;
use bb_wire::system::AppKeybindingOverride;
use bb_wire::thread::UpdateThreadRequest;

pub type ApiFuture<T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mutation {
    Rename { thread_id: String, title: String },
    Archive { thread_id: String },
    Unarchive { thread_id: String },
    MarkRead { thread_id: String },
    MarkUnread { thread_id: String },
    Pin { thread_id: String },
    Unpin { thread_id: String },
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
            | Self::Unpin { thread_id } => thread_id,
        }
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
        }
    }

    pub fn is_silent(&self) -> bool {
        matches!(self, Self::MarkRead { .. })
    }
}

pub trait Api: Send + Sync + 'static {
    fn mutate(&self, mutation: Mutation) -> ApiFuture<()>;

    fn keybinding_overrides(&self) -> ApiFuture<Vec<AppKeybindingOverride>>;
}

impl Api for BbClient {
    fn mutate(&self, mutation: Mutation) -> ApiFuture<()> {
        let client = self.clone();
        Box::pin(async move {
            let result = match &mutation {
                Mutation::Rename { thread_id, title } => client
                    .update_thread(thread_id, &UpdateThreadRequest::rename(title.clone()))
                    .await
                    .map(drop),
                Mutation::Archive { thread_id } => client.archive_thread(thread_id).await.map(drop),
                Mutation::Unarchive { thread_id } => {
                    client.unarchive_thread(thread_id).await.map(drop)
                }
                Mutation::MarkRead { thread_id } => {
                    client.mark_thread_read(thread_id).await.map(drop)
                }
                Mutation::MarkUnread { thread_id } => {
                    client.mark_thread_unread(thread_id).await.map(drop)
                }
                Mutation::Pin { thread_id } => client.pin_thread(thread_id).await.map(drop),
                Mutation::Unpin { thread_id } => client.unpin_thread(thread_id).await.map(drop),
            };
            result.map_err(|error| error.to_string())
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
}
