use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::common::{JsonObject, JsonValue, is_false};
use crate::interaction::{UserAnswer, UserQuestion};
use crate::prompt::{PromptTextMention, TurnInitiator};
use crate::thread::ThreadContextWindowUsage;

wire_enum! {
    pub enum TimelineRowStatus {
        Pending => "pending",
        Completed => "completed",
        Error => "error",
        Interrupted => "interrupted",
    }
}

wire_enum! {
    pub enum TimelineApprovalStatus {
        WaitingForApproval => "waiting_for_approval",
        Denied => "denied",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TimelineRowBase {
    pub id: String,
    pub thread_id: String,
    pub turn_id: Option<String>,
    pub source_seq_start: i64,
    pub source_seq_end: i64,
    pub started_at: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PresentationLabel {
    pub pending: String,
    pub completed: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PresentationIcon {
    pub glyph: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PresentationTint {
    pub light: String,
    pub dark: String,
}

wire_enum! {
    pub enum PresentationBadgeTone {
        Neutral => "neutral",
        Destructive => "destructive",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PresentationBadge {
    pub glyph: String,
    pub label: String,
    pub hint: String,
    pub tone: PresentationBadgeTone,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ItemPresentation {
    pub label: PresentationLabel,
    pub icon: PresentationIcon,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suppress: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tint: Option<PresentationTint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub badge: Option<PresentationBadge>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ConversationAttachments {
    pub web_images: u32,
    pub local_images: u32,
    pub local_files: u32,
    pub image_urls: Vec<String>,
    pub local_image_paths: Vec<String>,
    pub local_file_paths: Vec<String>,
}

wire_enum! {
    pub enum ConversationRole {
        User => "user",
        Assistant => "assistant",
    }
}

wire_enum! {
    pub enum TurnRequestKind {
        Message => "message",
        Steer => "steer",
    }
}

wire_enum! {
    pub enum TurnRequestStatus {
        Pending => "pending",
        Accepted => "accepted",
        Rejected => "rejected",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ConversationTurnRequest {
    pub is_grouped: bool,
    pub kind: TurnRequestKind,
    pub status: TurnRequestStatus,
}

wire_enum! {
    pub enum SystemMessageKind {
        OwnershipAssigned => "ownership-assigned",
        OwnershipRemoved => "ownership-removed",
        ChildNeedsAttention => "child-needs-attention",
        ChildCompleted => "child-completed",
        ChildFailed => "child-failed",
        ChildInterrupted => "child-interrupted",
        ChildOutcomeBatch => "child-outcome-batch",
        ToolResultDelivered => "tool-result-delivered",
        Unlabeled => "unlabeled",
    }
}

wire_enum! {
    pub enum ChildThreadOutcomeStatus {
        Completed => "completed",
        Failed => "failed",
        Interrupted => "interrupted",
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChildThreadOutcome {
    pub thread_id: String,
    pub status: ChildThreadOutcomeStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interruption: Option<JsonValue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum SystemMessageSubject {
    Thread {
        thread_id: String,
        #[serde(default)]
        thread_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        outcomes: Option<Vec<ChildThreadOutcome>>,
    },
    ThreadBatch {
        #[serde(default)]
        count: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        outcomes: Option<Vec<ChildThreadOutcome>>,
    },
    ToolCall {
        tool_name: String,
        #[serde(default)]
        suppress: bool,
    },
    #[serde(untagged)]
    Unknown(JsonValue),
}

default_unknown!(SystemMessageSubject);

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ConversationRow {
    #[serde(flatten)]
    pub base: TimelineRowBase,
    pub role: ConversationRole,
    pub message_seq: i64,
    pub text: String,
    pub attachments: Option<ConversationAttachments>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initiator: Option<TurnInitiator>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender_thread_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_message_kind: Option<SystemMessageKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_message_subject: Option<SystemMessageSubject>,
    pub turn_request: Option<ConversationTurnRequest>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub mentions: Vec<PromptTextMention>,
}

wire_enum! {
    pub enum SystemRowKind {
        Debug => "debug",
        Error => "error",
        Reconnect => "reconnect",
        Operation => "operation",
    }
}

wire_enum! {
    pub enum SystemOperationKind {
        Generic => "generic",
        Reasoning => "reasoning",
        Compaction => "compaction",
        ContextClear => "context-clear",
        ParentChange => "parent-change",
        ThreadProvisioning => "thread-provisioning",
        ThreadInterrupted => "thread-interrupted",
        ProviderUnhandled => "provider-unhandled",
        Warning => "warning",
        Deprecation => "deprecation",
    }
}

wire_enum! {
    pub enum ParentChangeAction {
        Assign => "assign",
        Release => "release",
        Transfer => "transfer",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ParentChange {
    pub action: ParentChangeAction,
    pub previous_parent_thread_id: Option<String>,
    pub previous_parent_thread_title: Option<String>,
    pub next_parent_thread_id: Option<String>,
    pub next_parent_thread_title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SystemRow {
    #[serde(flatten)]
    pub base: TimelineRowBase,
    pub title: String,
    pub detail: Option<String>,
    pub status: Option<TimelineRowStatus>,
    pub system_kind: SystemRowKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_kind: Option<SystemOperationKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_change: Option<ParentChange>,
    #[serde(skip_serializing_if = "is_false")]
    pub content_deferred: bool,
}

wire_enum! {
    pub enum FullOutputAvailability {
        Available => "available",
        DetailLimit => "detail-limit",
        RetentionExpired => "retention-expired",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OutputPreview {
    #[serde(rename = "experimental_fullOutputAvailability")]
    pub full_output_availability: FullOutputAvailability,
    pub total_chars: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ActivityIntent {
    Read {
        #[serde(default)]
        command: String,
        #[serde(default)]
        name: String,
        #[serde(default)]
        path: Option<String>,
    },
    ListFiles {
        #[serde(default)]
        command: String,
        #[serde(default)]
        path: Option<String>,
    },
    Search {
        #[serde(default)]
        command: String,
        #[serde(default)]
        query: Option<String>,
        #[serde(default)]
        path: Option<String>,
    },
    #[serde(rename = "unknown")]
    Generic {
        #[serde(default)]
        command: String,
    },
    #[serde(untagged)]
    Unknown(JsonValue),
}

default_unknown!(ActivityIntent);

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CommandWorkRow {
    #[serde(flatten)]
    pub base: TimelineRowBase,
    pub status: TimelineRowStatus,
    pub call_id: String,
    pub command: String,
    pub cwd: Option<String>,
    pub source: Option<String>,
    pub output: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_preview: Option<OutputPreview>,
    pub exit_code: Option<i64>,
    pub completed_at: Option<i64>,
    pub approval_status: Option<TimelineApprovalStatus>,
    pub activity_intents: Vec<ActivityIntent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presentation: Option<ItemPresentation>,
    #[serde(skip_serializing_if = "is_false")]
    pub content_deferred: bool,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ToolWorkRow {
    #[serde(flatten)]
    pub base: TimelineRowBase,
    pub status: TimelineRowStatus,
    pub call_id: String,
    pub tool_name: String,
    pub tool_args: Option<JsonObject>,
    pub output: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_preview: Option<OutputPreview>,
    pub completed_at: Option<i64>,
    pub approval_status: Option<TimelineApprovalStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presentation: Option<ItemPresentation>,
    #[serde(skip_serializing_if = "is_false")]
    pub content_deferred: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DiffStats {
    pub added: u64,
    pub removed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FileChange {
    pub path: String,
    pub kind: Option<String>,
    pub move_path: Option<String>,
    pub diff: Option<String>,
    pub diff_stats: DiffStats,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FileChangeWorkRow {
    #[serde(flatten)]
    pub base: TimelineRowBase,
    pub status: TimelineRowStatus,
    pub call_id: String,
    pub change: FileChange,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
    pub approval_status: Option<TimelineApprovalStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presentation: Option<ItemPresentation>,
    #[serde(skip_serializing_if = "is_false")]
    pub content_deferred: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WebSearchWorkRow {
    #[serde(flatten)]
    pub base: TimelineRowBase,
    pub status: TimelineRowStatus,
    pub call_id: String,
    pub queries: Vec<String>,
    pub completed_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presentation: Option<ItemPresentation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WebFetchWorkRow {
    #[serde(flatten)]
    pub base: TimelineRowBase,
    pub status: TimelineRowStatus,
    pub call_id: String,
    pub url: String,
    pub prompt: Option<String>,
    pub pattern: Option<String>,
    pub completed_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presentation: Option<ItemPresentation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ImageViewWorkRow {
    #[serde(flatten)]
    pub base: TimelineRowBase,
    pub status: TimelineRowStatus,
    pub call_id: String,
    pub path: String,
    pub completed_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presentation: Option<ItemPresentation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ImageGenerationWorkRow {
    #[serde(flatten)]
    pub base: TimelineRowBase,
    pub status: TimelineRowStatus,
    pub call_id: String,
    pub prompt: Option<String>,
    pub path: Option<String>,
    pub error: Option<String>,
    pub transparent_background: bool,
    pub completed_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presentation: Option<ItemPresentation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FileReadWorkRow {
    #[serde(flatten)]
    pub base: TimelineRowBase,
    pub status: TimelineRowStatus,
    pub call_id: String,
    pub path: String,
    pub cmd: Option<String>,
    pub completed_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presentation: Option<ItemPresentation>,
}

wire_enum! {
    pub enum SearchMode {
        Content => "content",
        Path => "path",
        List => "list",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SearchWorkRow {
    #[serde(flatten)]
    pub base: TimelineRowBase,
    pub status: TimelineRowStatus,
    pub call_id: String,
    pub mode: SearchMode,
    pub query: String,
    pub path: Option<String>,
    pub cmd: Option<String>,
    pub completed_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presentation: Option<ItemPresentation>,
}

wire_enum! {
    pub enum PlanStepStatus {
        Pending => "pending",
        Active => "active",
        Completed => "completed",
        Failed => "failed",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PlanStep {
    pub step: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<PlanStepStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PlanStepsWorkRow {
    #[serde(flatten)]
    pub base: TimelineRowBase,
    pub status: TimelineRowStatus,
    pub call_id: String,
    pub steps: Vec<PlanStep>,
    pub explanation: Option<String>,
    pub completed_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presentation: Option<ItemPresentation>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExtensionWorkRow {
    #[serde(flatten)]
    pub base: TimelineRowBase,
    pub status: TimelineRowStatus,
    pub call_id: String,
    pub extension_kind: String,
    pub payload: JsonValue,
    pub completed_at: Option<i64>,
    pub presentation: ItemPresentation,
}

wire_enum! {
    pub enum ApprovalKind {
        FileEdit => "file-edit",
        PermissionGrant => "permission-grant",
    }
}

wire_enum! {
    pub enum ApprovalLifecycle {
        Waiting => "waiting",
        Pending => "pending",
        Resolving => "resolving",
        Granted => "granted",
        Denied => "denied",
        Interrupted => "interrupted",
    }
}

wire_enum! {
    pub enum GrantScope {
        Turn => "turn",
        Session => "session",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ApprovalTarget {
    pub item_id: String,
    pub tool_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ApprovalWorkRow {
    #[serde(flatten)]
    pub base: TimelineRowBase,
    pub status: TimelineRowStatus,
    pub interaction_id: String,
    pub target: ApprovalTarget,
    pub approval_kind: ApprovalKind,
    pub lifecycle: ApprovalLifecycle,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant_scope: Option<GrantScope>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<String>,
}

wire_enum! {
    pub enum QuestionLifecycle {
        Pending => "pending",
        Resolving => "resolving",
        Answered => "answered",
        Interrupted => "interrupted",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct QuestionWorkRow {
    #[serde(flatten)]
    pub base: TimelineRowBase,
    pub status: TimelineRowStatus,
    pub interaction_id: String,
    pub lifecycle: QuestionLifecycle,
    pub questions: Vec<UserQuestion>,
    pub answers: Option<BTreeMap<String, UserAnswer>>,
    pub status_reason: Option<String>,
}

wire_enum! {
    pub enum FormLifecycle {
        Pending => "pending",
        Submitted => "submitted",
        Cancelled => "cancelled",
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FormWorkRow {
    #[serde(flatten)]
    pub base: TimelineRowBase,
    pub status: TimelineRowStatus,
    pub interaction_id: String,
    pub plugin_id: String,
    pub renderer_id: String,
    pub title: String,
    pub lifecycle: FormLifecycle,
    pub status_reason: Option<String>,
    pub presentation: ItemPresentation,
    pub payload: JsonValue,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DelegationWorkRow {
    #[serde(flatten)]
    pub base: TimelineRowBase,
    pub status: TimelineRowStatus,
    pub call_id: String,
    pub tool_name: String,
    pub child_ref: Option<String>,
    pub background: bool,
    pub subagent_type: Option<String>,
    pub description: Option<String>,
    pub output: String,
    pub completed_at: Option<i64>,
    pub child_rows: Option<Vec<TimelineRow>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presentation: Option<ItemPresentation>,
}

wire_enum! {
    pub enum BackgroundTaskStatus {
        Pending => "pending",
        Running => "running",
        Paused => "paused",
        Completed => "completed",
        Failed => "failed",
        Killed => "killed",
        Stopped => "stopped",
    }
}

wire_enum! {
    pub enum WorkflowAgentState {
        Queued => "queued",
        Running => "running",
        Done => "done",
        Failed => "failed",
        Skipped => "skipped",
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkflowAgentSnapshot {
    pub index: u32,
    pub label: String,
    pub state: WorkflowAgentState,
    pub model: String,
    pub attempt: u32,
    pub cached: bool,
    pub last_progress_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase_index: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isolation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queued_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_tool_summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_preview: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_preview: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkflowPhaseSnapshot {
    pub index: u32,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkflowProgressSnapshot {
    pub phases: Vec<WorkflowPhaseSnapshot>,
    pub agents: Vec<WorkflowAgentSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BackgroundTaskUsage {
    pub total_tokens: f64,
    pub tool_uses: f64,
    pub duration_ms: f64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkflowWorkRow {
    #[serde(flatten)]
    pub base: TimelineRowBase,
    pub status: TimelineRowStatus,
    pub item_id: String,
    pub task_type: String,
    pub workflow_name: Option<String>,
    pub description: String,
    pub model: Option<String>,
    pub task_status: BackgroundTaskStatus,
    pub workflow: Option<WorkflowProgressSnapshot>,
    pub usage: Option<BackgroundTaskUsage>,
    pub summary: Option<String>,
    pub error: Option<String>,
    pub completed_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presentation: Option<ItemPresentation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "workKind", rename_all = "kebab-case")]
pub enum TimelineWorkRow {
    Command(Box<CommandWorkRow>),
    Tool(Box<ToolWorkRow>),
    FileChange(Box<FileChangeWorkRow>),
    WebSearch(Box<WebSearchWorkRow>),
    WebFetch(Box<WebFetchWorkRow>),
    ImageGeneration(Box<ImageGenerationWorkRow>),
    ImageView(Box<ImageViewWorkRow>),
    FileRead(Box<FileReadWorkRow>),
    Search(Box<SearchWorkRow>),
    PlanSteps(Box<PlanStepsWorkRow>),
    Extension(Box<ExtensionWorkRow>),
    Approval(Box<ApprovalWorkRow>),
    Question(Box<QuestionWorkRow>),
    Form(Box<FormWorkRow>),
    Delegation(Box<DelegationWorkRow>),
    Workflow(Box<WorkflowWorkRow>),
    #[serde(untagged)]
    Unknown(JsonValue),
}

default_unknown!(TimelineWorkRow);

impl TimelineWorkRow {
    pub fn base(&self) -> Option<&TimelineRowBase> {
        Some(match self {
            Self::Command(row) => &row.base,
            Self::Tool(row) => &row.base,
            Self::FileChange(row) => &row.base,
            Self::WebSearch(row) => &row.base,
            Self::WebFetch(row) => &row.base,
            Self::ImageGeneration(row) => &row.base,
            Self::ImageView(row) => &row.base,
            Self::FileRead(row) => &row.base,
            Self::Search(row) => &row.base,
            Self::PlanSteps(row) => &row.base,
            Self::Extension(row) => &row.base,
            Self::Approval(row) => &row.base,
            Self::Question(row) => &row.base,
            Self::Form(row) => &row.base,
            Self::Delegation(row) => &row.base,
            Self::Workflow(row) => &row.base,
            Self::Unknown(_) => return None,
        })
    }

    pub fn status(&self) -> TimelineRowStatus {
        match self {
            Self::Command(row) => row.status,
            Self::Tool(row) => row.status,
            Self::FileChange(row) => row.status,
            Self::WebSearch(row) => row.status,
            Self::WebFetch(row) => row.status,
            Self::ImageGeneration(row) => row.status,
            Self::ImageView(row) => row.status,
            Self::FileRead(row) => row.status,
            Self::Search(row) => row.status,
            Self::PlanSteps(row) => row.status,
            Self::Extension(row) => row.status,
            Self::Approval(row) => row.status,
            Self::Question(row) => row.status,
            Self::Form(row) => row.status,
            Self::Delegation(row) => row.status,
            Self::Workflow(row) => row.status,
            Self::Unknown(value) => value
                .get("status")
                .and_then(|status| serde_json::from_value(status.clone()).ok())
                .unwrap_or_default(),
        }
    }

    pub fn presentation(&self) -> Option<&ItemPresentation> {
        match self {
            Self::Command(row) => row.presentation.as_ref(),
            Self::Tool(row) => row.presentation.as_ref(),
            Self::FileChange(row) => row.presentation.as_ref(),
            Self::WebSearch(row) => row.presentation.as_ref(),
            Self::WebFetch(row) => row.presentation.as_ref(),
            Self::ImageGeneration(row) => row.presentation.as_ref(),
            Self::ImageView(row) => row.presentation.as_ref(),
            Self::FileRead(row) => row.presentation.as_ref(),
            Self::Search(row) => row.presentation.as_ref(),
            Self::PlanSteps(row) => row.presentation.as_ref(),
            Self::Extension(row) => Some(&row.presentation),
            Self::Form(row) => Some(&row.presentation),
            Self::Delegation(row) => row.presentation.as_ref(),
            Self::Workflow(row) => row.presentation.as_ref(),
            Self::Approval(_) | Self::Question(_) | Self::Unknown(_) => None,
        }
    }

    pub fn content_deferred(&self) -> bool {
        match self {
            Self::Command(row) => row.content_deferred,
            Self::Tool(row) => row.content_deferred,
            Self::FileChange(row) => row.content_deferred,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TurnRow {
    #[serde(flatten)]
    pub base: TimelineRowBase,
    pub status: TimelineRowStatus,
    pub summary_count: u32,
    pub completed_at: Option<i64>,
    pub children: Option<Vec<TimelineRow>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum TimelineRow {
    Conversation(Box<ConversationRow>),
    Work(TimelineWorkRow),
    System(Box<SystemRow>),
    Turn(Box<TurnRow>),
    #[serde(untagged)]
    Unknown(JsonValue),
}

default_unknown!(TimelineRow);

impl TimelineRow {
    pub fn base(&self) -> Option<&TimelineRowBase> {
        match self {
            Self::Conversation(row) => Some(&row.base),
            Self::Work(row) => row.base(),
            Self::System(row) => Some(&row.base),
            Self::Turn(row) => Some(&row.base),
            Self::Unknown(_) => None,
        }
    }

    pub fn id(&self) -> Option<&str> {
        match self {
            Self::Work(TimelineWorkRow::Unknown(value)) | Self::Unknown(value) => {
                value.get("id").and_then(JsonValue::as_str)
            }
            _ => self.base().map(|base| base.id.as_str()),
        }
    }

    pub fn source_seq_end(&self) -> Option<i64> {
        match self {
            Self::Work(TimelineWorkRow::Unknown(value)) | Self::Unknown(value) => {
                value.get("sourceSeqEnd").and_then(JsonValue::as_i64)
            }
            _ => self.base().map(|base| base.source_seq_end),
        }
    }

    pub fn is_unknown(&self) -> bool {
        matches!(
            self,
            Self::Unknown(_) | Self::Work(TimelineWorkRow::Unknown(_))
        )
    }

    pub fn nested_rows(&self) -> Option<&[TimelineRow]> {
        match self {
            Self::Turn(row) => row.children.as_deref(),
            Self::Work(TimelineWorkRow::Delegation(row)) => row.child_rows.as_deref(),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TimelineDelta {
    pub upsert_rows: Vec<TimelineRow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row_order: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TimelinePaginationCursor {
    pub anchor_seq: i64,
    pub anchor_id: String,
}

wire_enum! {
    pub enum TimelinePageKind {
        Latest => "latest",
        Older => "older",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TimelineContentPage {
    pub anchor_seq: i64,
    pub start: u64,
    pub end: u64,
    pub total: u64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TimelinePageMetadata {
    pub kind: TimelinePageKind,
    pub segment_limit: u32,
    pub returned_segment_count: u32,
    pub has_older_rows: bool,
    pub older_cursor: Option<TimelinePaginationCursor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_snapshot: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub older_rows_source_seq_end: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub older_row_updates: Option<Vec<TimelineRow>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_page: Option<TimelineContentPage>,
}

wire_enum! {
    pub enum CompletedTurnDisplay {
        Collapse => "collapse",
        Flat => "flat",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ActivePromptMode {
    pub mode: String,
    pub provider_id: String,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ActiveThinking {
    pub id: String,
    pub text: String,
    pub started_at: i64,
    pub updated_at: i64,
}

wire_enum! {
    pub enum PendingTodoStatus {
        Pending => "pending",
        InProgress => "in_progress",
        Completed => "completed",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PendingTodoItem {
    pub id: String,
    pub text: String,
    pub status: PendingTodoStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PendingTodos {
    pub source_seq: i64,
    pub updated_at: i64,
    pub items: Vec<PendingTodoItem>,
}

wire_enum! {
    pub enum GoalStatus {
        Active => "active",
        Paused => "paused",
        BudgetLimited => "budgetLimited",
        Complete => "complete",
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadGoal {
    pub source_seq: i64,
    pub updated_at: i64,
    pub objective: String,
    pub status: GoalStatus,
    pub token_budget: Option<f64>,
    pub tokens_used: f64,
    pub time_used_seconds: f64,
}

wire_enum! {
    pub enum ModelFallbackReason {
        Refusal => "refusal",
        Provider => "provider",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ModelFallback {
    pub source_seq: i64,
    pub detected_at: i64,
    pub original_model: String,
    pub fallback_model: String,
    pub reason: ModelFallbackReason,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadTimelineResponse {
    pub rows: Vec<TimelineRow>,
    pub context_boundary_seq: Option<i64>,
    pub completed_turn_display: CompletedTurnDisplay,
    pub active_prompt_mode: Option<ActivePromptMode>,
    pub active_thinking: Option<ActiveThinking>,
    #[serde(deserialize_with = "crate::common::lenient_vec")]
    pub active_workflows: Vec<WorkflowWorkRow>,
    #[serde(deserialize_with = "crate::common::lenient_vec")]
    pub active_background_commands: Vec<WorkflowWorkRow>,
    #[serde(deserialize_with = "crate::common::lenient_option")]
    pub pending_todos: Option<PendingTodos>,
    #[serde(deserialize_with = "crate::common::lenient_option")]
    pub goal: Option<ThreadGoal>,
    #[serde(deserialize_with = "crate::common::lenient_option")]
    pub model_fallback: Option<ModelFallback>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::common::lenient_option"
    )]
    pub context_window_usage: Option<ThreadContextWindowUsage>,
    pub timeline_page: TimelinePageMetadata,
    pub max_seq: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta: Option<TimelineDelta>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadTimelineQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_nested_rows: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment_limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_anchor_seq: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_anchor_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary_only: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_sequence: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defer_content: Option<bool>,
}

impl ThreadTimelineQuery {
    pub fn latest() -> Self {
        Self {
            defer_content: Some(true),
            ..Self::default()
        }
    }

    pub fn after_sequence(max_seq: i64) -> Self {
        Self {
            after_sequence: Some(max_seq),
            defer_content: Some(true),
            ..Self::default()
        }
    }

    pub fn older(cursor: &TimelinePaginationCursor) -> Self {
        Self {
            before_anchor_seq: Some(cursor.anchor_seq),
            before_anchor_id: Some(cursor.anchor_id.clone()),
            defer_content: Some(true),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnSummaryDetailsQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defer_content: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
    pub turn_id: String,
    pub source_seq_start: i64,
    pub source_seq_end: i64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TurnSummaryDetailsResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub older_cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_snapshot: Option<String>,
    pub rows: Vec<TimelineRow>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadMessageResponse {
    pub message: ConversationRow,
    pub before: Vec<ConversationRow>,
    pub after: Vec<ConversationRow>,
}
