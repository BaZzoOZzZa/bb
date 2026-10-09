use serde::{Deserialize, Serialize};

use crate::common::JsonValue;
use crate::prompt::{
    ExistingThreadExecutionInputSources, PermissionMode, PluginSubmission, PromptInput,
    ReasoningLevel, TurnInitiator,
};
use crate::thread::ThreadCreateOrigin;

wire_enum! {
    pub enum SendMessageMode {
        QueueIfActive => "queue-if-active",
        SteerIfActive => "steer-if-active",
        Auto => "auto",
        Start => "start",
        Steer => "steer",
    }
}

wire_enum! {
    pub enum SendQueuedMessageMode {
        Auto => "auto",
        Steer => "steer",
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum QueuedMessageWaitingOn {
    Time,
    ThreadBusy,
    Stopping,
    TurnStarting,
    Provisioning,
    HostOffline {
        #[serde(default)]
        host_name: String,
    },
    Interaction,
    Plugin {
        #[serde(default)]
        plugin_id: String,
        #[serde(default)]
        reason: String,
    },
    #[serde(untagged)]
    Unknown(JsonValue),
}

default_unknown!(QueuedMessageWaitingOn);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum QueuedMessagePayload {
    Inline,
    Retry {
        #[serde(default)]
        retry_of_turn_request_id: String,
        #[serde(default)]
        attempt: u32,
        #[serde(default)]
        reason: String,
    },
    #[serde(untagged)]
    Unknown(JsonValue),
}

default_unknown!(QueuedMessagePayload);

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadQueuedMessage {
    pub id: String,
    pub origin: Option<ThreadCreateOrigin>,
    pub origin_plugin_id: Option<String>,
    pub initiator: TurnInitiator,
    pub sender_thread_id: Option<String>,
    pub thread_id: String,
    pub content: Vec<PromptInput>,
    pub model: String,
    pub reasoning_level: ReasoningLevel,
    pub permission_mode: PermissionMode,
    pub service_tier: String,
    pub group_with_next: bool,
    pub send_at: Option<i64>,
    pub waiting_on: Option<QueuedMessageWaitingOn>,
    pub failure_reason: Option<String>,
    pub payload: QueuedMessagePayload,
    pub editable: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

pub type ThreadQueuedMessageListResponse = Vec<ThreadQueuedMessage>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendMessageRequest {
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
    pub execution_input_sources: Option<ExistingThreadExecutionInputSources>,
    pub mode: SendMessageMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_submission: Option<PluginSubmission>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send_at: Option<i64>,
}

impl SendMessageRequest {
    pub fn new(input: Vec<PromptInput>, mode: SendMessageMode) -> Self {
        Self {
            input,
            model: None,
            service_tier: None,
            reasoning_level: None,
            permission_mode: None,
            execution_input_sources: None,
            mode,
            sender_thread_id: None,
            plugin_submission: None,
            send_at: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "delivery",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum SendMessageResponse {
    Sent {
        #[serde(default)]
        ok: bool,
    },
    Queued {
        #[serde(default)]
        ok: bool,
        #[serde(default)]
        queued_message: Box<ThreadQueuedMessage>,
    },
    #[serde(untagged)]
    Unknown(JsonValue),
}

default_unknown!(SendMessageResponse);

pub type SendQueuedMessageResponse = SendMessageResponse;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditMessageRequest {
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
    pub execution_input_sources: Option<ExistingThreadExecutionInputSources>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_thread_id: Option<String>,
    pub operation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_request_sequence: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EditMessageResponse {
    pub ok: bool,
    pub operation_id: String,
    pub request_sequence: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryTurnRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "delivery",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum RetryTurnResponse {
    Sent {
        #[serde(default)]
        turn_request_id: String,
        #[serde(default)]
        attempt: u32,
    },
    Queued {
        #[serde(default)]
        turn_request_id: String,
        #[serde(default)]
        attempt: u32,
        #[serde(default)]
        queued_message_id: String,
        #[serde(default)]
        waiting_on: QueuedMessageWaitingOn,
        #[serde(default)]
        send_at: Option<i64>,
    },
    #[serde(untagged)]
    Unknown(JsonValue),
}

default_unknown!(RetryTurnResponse);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateQueuedMessageRequest {
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
    pub execution_input_sources: Option<ExistingThreadExecutionInputSources>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_thread_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateQueuedMessageRequest {
    pub expected_updated_at: i64,
    pub input: Vec<PromptInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendQueuedMessageRequest {
    pub mode: SendQueuedMessageMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReorderQueuedMessageRequest {
    pub previous_queued_message_id: Option<String>,
    pub next_queued_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_boundary_queued_message_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetQueuedMessageGroupBoundaryRequest {
    pub expected_grouped_prefix_queued_message_ids: Vec<String>,
    pub group_boundary_queued_message_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct QueuedMessageEditHoldResponse {
    pub lease_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedMessageListQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait_holder: Option<String>,
}
