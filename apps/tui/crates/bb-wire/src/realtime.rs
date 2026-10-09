use serde::{Deserialize, Serialize};

use crate::common::{JsonValue, lenient_option};
use crate::thread::{ThreadActivityState, ThreadRuntimeState, ThreadStatus};

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum SubscriptionTarget {
    ThreadDetail { thread_id: String },
    ThreadList,
    ProjectDetail { project_id: String },
    ProjectList,
    EnvironmentDetail { environment_id: String },
    EnvironmentList,
    HostDetail { host_id: String },
    HostList,
    System,
}

impl SubscriptionTarget {
    pub fn key(&self) -> String {
        match self {
            Self::ThreadDetail { thread_id } => format!("thread-detail:{thread_id}"),
            Self::ThreadList => "thread-list".into(),
            Self::ProjectDetail { project_id } => format!("project-detail:{project_id}"),
            Self::ProjectList => "project-list".into(),
            Self::EnvironmentDetail { environment_id } => {
                format!("environment-detail:{environment_id}")
            }
            Self::EnvironmentList => "environment-list".into(),
            Self::HostDetail { host_id } => format!("host-detail:{host_id}"),
            Self::HostList => "host-list".into(),
            Self::System => "system".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ClientMessage {
    Subscribe { target: SubscriptionTarget },
    Unsubscribe { target: SubscriptionTarget },
    Ping,
}

impl ClientMessage {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

wire_enum! {
    pub enum ThreadChangeKind {
        ThreadCreated => "thread-created",
        ThreadDeleted => "thread-deleted",
        EventsAppended => "events-appended",
        HistoryRewritten => "history-rewritten",
        HistoryCompacted => "history-compacted",
        InteractionsChanged => "interactions-changed",
        StatusChanged => "status-changed",
        TitleChanged => "title-changed",
        QueueChanged => "queue-changed",
        ArchivedChanged => "archived-changed",
        PinStateChanged => "pin-state-changed",
        ParentChanged => "parent-changed",
        EnvironmentChanged => "environment-changed",
        ReadStateChanged => "read-state-changed",
        TabsChanged => "tabs-changed",
        TerminalsChanged => "terminals-changed",
    }
}

wire_enum! {
    pub enum ProjectChangeKind {
        ProjectCreated => "project-created",
        ProjectUpdated => "project-updated",
        ProjectDeleted => "project-deleted",
        ProjectSourcesChanged => "project-sources-changed",
        ThreadsChanged => "threads-changed",
        ProjectOrderChanged => "project-order-changed",
    }
}

wire_enum! {
    pub enum EnvironmentChangeKind {
        EnvironmentCreated => "environment-created",
        EnvironmentDeleted => "environment-deleted",
        MetadataChanged => "metadata-changed",
        StatusChanged => "status-changed",
        WorkStatusChanged => "work-status-changed",
        GitRefsChanged => "git-refs-changed",
        ThreadStorageChanged => "thread-storage-changed",
    }
}

wire_enum! {
    pub enum HostChangeKind {
        HostConnected => "host-connected",
        HostDisconnected => "host-disconnected",
        ProviderModelCatalogChanged => "provider-model-catalog-changed",
    }
}

wire_enum! {
    pub enum SystemChangeKind {
        ConfigChanged => "config-changed",
        PluginsChanged => "plugins-changed",
        PluginInstallJobsChanged => "plugin-install-jobs-changed",
        PluginUpdateJobsChanged => "plugin-update-jobs-changed",
        ProviderRegistrationsChanged => "provider-registrations-changed",
        UiPreferencesChanged => "ui-preferences-changed",
        EnvironmentAvailabilityChanged => "environment-availability-changed",
        ServerMoveChanged => "server-move-changed",
        AppUpdateChanged => "app-update-changed",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadStatusChange {
    pub status: ThreadStatus,
    pub runtime: ThreadRuntimeState,
    pub activity: ThreadActivityState,
    pub latest_attention_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadChangeMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_activity_changed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_types: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeline_sequence: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_pending_interaction: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "lenient_option"
    )]
    pub status_change: Option<ThreadStatusChange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "entity", rename_all = "kebab-case")]
pub enum ChangedMessage {
    Thread {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        metadata: Option<ThreadChangeMetadata>,
        #[serde(default)]
        changes: Vec<ThreadChangeKind>,
    },
    Project {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default)]
        changes: Vec<ProjectChangeKind>,
    },
    Environment {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default)]
        changes: Vec<EnvironmentChangeKind>,
    },
    Host {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default)]
        changes: Vec<HostChangeKind>,
    },
    System {
        #[serde(default)]
        changes: Vec<SystemChangeKind>,
    },
    #[serde(untagged)]
    Unknown(JsonValue),
}

impl ChangedMessage {
    pub fn without_unknown_kinds(mut self) -> Self {
        match &mut self {
            Self::Thread { changes, .. } => changes.retain(|kind| kind.is_known()),
            Self::Project { changes, .. } => changes.retain(|kind| kind.is_known()),
            Self::Environment { changes, .. } => changes.retain(|kind| kind.is_known()),
            Self::Host { changes, .. } => changes.retain(|kind| kind.is_known()),
            Self::System { changes } => changes.retain(|kind| kind.is_known()),
            Self::Unknown(_) => {}
        }
        self
    }
}

wire_enum! {
    pub enum ThreadOpenSplit {
        Right => "right",
        Down => "down",
        Left => "left",
        Top => "top",
        Replace => "replace",
    }
}

wire_enum! {
    pub enum PanelFileSource {
        Workspace => "workspace",
        ThreadStorage => "thread-storage",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadOpenFile {
    pub source: PanelFileSource,
    pub path: String,
    pub line_number: Option<u32>,
}

wire_enum! {
    pub enum ThreadPaneAction {
        Maximize => "maximize",
        Restore => "restore",
        Toggle => "toggle",
        Spotlight => "spotlight",
        ClearSpotlight => "clear-spotlight",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum ServerMessage {
    Changed(ChangedMessage),
    Pong,
    PluginSignal {
        plugin_id: String,
        channel: String,
        #[serde(default)]
        payload: JsonValue,
    },
    ThreadOpen {
        #[serde(default)]
        project_id: String,
        thread_id: String,
        #[serde(default)]
        split: ThreadOpenSplit,
        #[serde(default)]
        file: Option<ThreadOpenFile>,
    },
    ThreadPaneAction {
        #[serde(default)]
        project_id: String,
        thread_id: String,
        #[serde(default)]
        action: ThreadPaneAction,
    },
    #[serde(untagged)]
    Unknown(JsonValue),
}

impl ServerMessage {
    pub fn parse(text: &str) -> Self {
        serde_json::from_str(text).unwrap_or_else(|_| {
            Self::Unknown(serde_json::from_str(text).unwrap_or(JsonValue::String(text.into())))
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadOpenRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub split: Option<ThreadOpenSplit>,
    pub file: Option<ThreadOpenFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DeliveredResponse {
    pub delivered: u32,
}
