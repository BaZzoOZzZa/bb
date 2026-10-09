use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::common::JsonValue;
use crate::timeline::ItemPresentation;

pub const USER_QUESTION_MAX_QUESTIONS: usize = 4;
pub const USER_QUESTION_MAX_OPTIONS: usize = 4;
pub const USER_QUESTION_MAX_SELECTED: usize = 4;
pub const USER_QUESTION_MAX_FREE_TEXT_LENGTH: usize = 4096;

pub const ASK_USER_QUESTION_RENDERER_ID: &str = "ask-user-question";
pub const SECRET_REQUEST_RENDERER_ID: &str = "secret-request";

wire_enum! {
    pub enum PendingInteractionStatus {
        Pending => "pending",
        Resolving => "resolving",
        Resolved => "resolved",
        Interrupted => "interrupted",
    }
}

wire_enum! {
    pub enum ApprovalDecision {
        AllowOnce => "allow_once",
        AllowForSession => "allow_for_session",
        Deny => "deny",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NetworkPermissions {
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FileSystemPermissions {
    pub read: Vec<String>,
    pub write: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PermissionProfile {
    pub network: Option<NetworkPermissions>,
    pub file_system: Option<FileSystemPermissions>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CommandAction {
    Read {
        #[serde(default)]
        command: String,
        #[serde(default)]
        name: String,
        #[serde(default)]
        path: String,
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

default_unknown!(CommandAction);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ApprovalSubject {
    Command {
        item_id: String,
        command: String,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        actions: Vec<CommandAction>,
        #[serde(default)]
        session_grant: Option<PermissionProfile>,
    },
    FileChange {
        item_id: String,
        #[serde(default)]
        write_scope: Option<String>,
        #[serde(default)]
        session_grant: Option<PermissionProfile>,
    },
    PermissionGrant {
        item_id: String,
        #[serde(default)]
        tool_name: Option<String>,
        #[serde(default)]
        permissions: PermissionProfile,
    },
    Plan {
        item_id: String,
        plan: String,
        #[serde(default)]
        plan_file_path: Option<String>,
    },
    ToolUse {
        item_id: String,
        tool: String,
        #[serde(default)]
        presentation: ItemPresentation,
    },
    #[serde(untagged)]
    Unknown(JsonValue),
}

default_unknown!(ApprovalSubject);

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ApprovalPayload {
    pub subject: ApprovalSubject,
    pub reason: Option<String>,
    pub available_decisions: Vec<ApprovalDecision>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UserQuestionOption {
    pub value: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UserQuestion {
    pub id: String,
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short_label: Option<String>,
    pub multi_select: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<UserQuestionOption>>,
    pub allow_free_text: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UserQuestionPayload {
    pub questions: Vec<UserQuestion>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UserAnswer {
    pub selected: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub free_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PluginInteractionPayload {
    pub title: String,
    pub data: JsonValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presentation: Option<ItemPresentation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "RawExtensionInteractionPayload")]
pub struct ExtensionInteractionPayload {
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub data: JsonValue,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawExtensionInteractionPayload {
    kind: String,
    title: String,
    #[serde(default)]
    data: JsonValue,
}

impl TryFrom<RawExtensionInteractionPayload> for ExtensionInteractionPayload {
    type Error = String;

    fn try_from(raw: RawExtensionInteractionPayload) -> Result<Self, Self::Error> {
        if !is_extension_kind(&raw.kind) {
            return Err(format!("{} is not an extension kind", raw.kind));
        }
        Ok(Self {
            kind: raw.kind,
            title: raw.title,
            data: raw.data,
        })
    }
}

pub fn is_extension_kind(kind: &str) -> bool {
    let valid = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    };
    kind.split_once('/')
        .is_some_and(|(plugin, name)| valid(plugin) && valid(name))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InteractionPayload {
    Approval(ApprovalPayload),
    UserQuestion(UserQuestionPayload),
    Plugin(PluginInteractionPayload),
    #[serde(untagged)]
    Extension(ExtensionInteractionPayload),
    #[serde(untagged)]
    Unknown(JsonValue),
}

default_unknown!(InteractionPayload);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum InteractionOrigin {
    Provider {
        provider_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_thread_id: Option<String>,
        #[serde(default)]
        provider_request_id: String,
    },
    Plugin {
        plugin_id: String,
        renderer_id: String,
    },
    #[serde(untagged)]
    Unknown(JsonValue),
}

default_unknown!(InteractionOrigin);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "decision",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ApprovalResolution {
    AllowOnce {
        #[serde(default)]
        granted_permissions: Option<PermissionProfile>,
    },
    AllowForSession {
        #[serde(default)]
        granted_permissions: Option<PermissionProfile>,
    },
    Deny,
}

impl ApprovalResolution {
    pub fn decision(&self) -> ApprovalDecision {
        match self {
            Self::AllowOnce { .. } => ApprovalDecision::AllowOnce,
            Self::AllowForSession { .. } => ApprovalDecision::AllowForSession,
            Self::Deny => ApprovalDecision::Deny,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PluginInteractionDescription {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<JsonValue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum KindedResolution {
    UserAnswer {
        answers: BTreeMap<String, UserAnswer>,
    },
    PluginSubmitted {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<PluginInteractionDescription>,
    },
    RequestAnswer {
        #[serde(default)]
        value: JsonValue,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InteractionResolution {
    Approval(ApprovalResolution),
    Kinded(KindedResolution),
    Unknown(JsonValue),
}

impl InteractionResolution {
    pub fn approval(decision: ApprovalDecision) -> Self {
        Self::Approval(match decision {
            ApprovalDecision::AllowForSession => ApprovalResolution::AllowForSession {
                granted_permissions: None,
            },
            ApprovalDecision::Deny => ApprovalResolution::Deny,
            ApprovalDecision::AllowOnce | ApprovalDecision::Unknown => {
                ApprovalResolution::AllowOnce {
                    granted_permissions: None,
                }
            }
        })
    }

    pub fn user_answer(answers: BTreeMap<String, UserAnswer>) -> Self {
        Self::Kinded(KindedResolution::UserAnswer { answers })
    }

    pub fn request_answer(value: JsonValue) -> Self {
        Self::Kinded(KindedResolution::RequestAnswer { value })
    }
}

pub type ResolvePendingInteractionRequest = InteractionResolution;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RespondPluginInteractionRequest {
    pub value: JsonValue,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PendingInteraction {
    pub id: String,
    pub thread_id: String,
    pub status: PendingInteractionStatus,
    pub status_reason: Option<String>,
    pub created_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    pub resolved_at: Option<i64>,
    pub turn_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_thread_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<InteractionOrigin>,
    pub payload: InteractionPayload,
    pub resolution: Option<InteractionResolution>,
}

impl PendingInteraction {
    pub fn renderer_id(&self) -> Option<&str> {
        match &self.origin {
            Some(InteractionOrigin::Plugin { renderer_id, .. }) => Some(renderer_id),
            _ => None,
        }
    }

    pub fn plugin_id(&self) -> Option<&str> {
        match &self.origin {
            Some(InteractionOrigin::Plugin { plugin_id, .. }) => Some(plugin_id),
            _ => None,
        }
    }

    pub fn is_open(&self) -> bool {
        matches!(self.status, PendingInteractionStatus::Pending)
    }

    pub fn plugin_data<T: serde::de::DeserializeOwned>(&self) -> Option<T> {
        match &self.payload {
            InteractionPayload::Plugin(payload) => {
                serde_json::from_value(payload.data.clone()).ok()
            }
            InteractionPayload::Extension(payload) => {
                serde_json::from_value(payload.data.clone()).ok()
            }
            _ => None,
        }
    }
}

pub type ThreadPendingInteractionsResponse = Vec<PendingInteraction>;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AskUserQuestionPluginPayload {
    pub questions: Vec<UserQuestion>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AskUserQuestionPluginResponse {
    pub answers: BTreeMap<String, UserAnswer>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SecretRequestDestination {
    pub kind: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SecretRequestField {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SecretRequestPayload {
    pub purpose: Option<String>,
    pub destination: SecretRequestDestination,
    pub fields: Vec<SecretRequestField>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SecretRequestResponse {
    pub values: BTreeMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_namespaced_kinds_parse_as_extension_payloads() {
        let extension: InteractionPayload =
            serde_json::from_value(json!({"kind": "acme/pick-1", "title": "Pick", "data": 1}))
                .unwrap();
        assert!(matches!(extension, InteractionPayload::Extension(_)));

        for kind in ["future_kind", "Acme/pick", "acme/", "/pick", "a/b/c"] {
            let payload: InteractionPayload =
                serde_json::from_value(json!({"kind": kind, "title": "Pick"})).unwrap();
            assert!(
                matches!(payload, InteractionPayload::Unknown(_)),
                "{kind} should not be an extension payload"
            );
        }
    }
}
