use serde::{Deserialize, Serialize};

use crate::common::JsonValue;

wire_enum! {
    pub enum ReasoningLevel {
        None => "none",
        Low => "low",
        Medium => "medium",
        High => "high",
        Xhigh => "xhigh",
        Ultracode => "ultracode",
        Max => "max",
        Ultra => "ultra",
    }
}

wire_enum! {
    pub enum PermissionMode {
        AcceptEdits => "accept-edits" | "workspace-write",
        Auto => "auto",
        Full => "full",
    }
}

wire_enum! {
    pub enum ExecutionInputSource {
        Explicit => "explicit",
        ClientPreference => "client-preference",
    }
}

wire_enum! {
    pub enum ThreadExecutionSource {
        ThreadStart => "client/thread/start",
        TurnRequested => "client/turn/requested",
        TurnStart => "client/turn/start",
    }
}

wire_enum! {
    pub enum TurnInitiator {
        User => "user",
        Agent => "agent",
        System => "system",
    }
}

wire_enum! {
    pub enum PromptInputVisibility {
        AgentOnly => "agent-only",
    }
}

wire_enum! {
    pub enum PromptMentionPathSource {
        Workspace => "workspace",
        ThreadStorage => "thread-storage",
    }
}

wire_enum! {
    pub enum PathEntryKind {
        File => "file",
        Directory => "directory",
    }
}

wire_enum! {
    pub enum PromptMentionCommandTrigger {
        Slash => "/",
        Dollar => "$",
    }
}

wire_enum! {
    pub enum PromptMentionCommandSource {
        Skill => "skill",
        Command => "command",
    }
}

wire_enum! {
    pub enum PromptMentionCommandOrigin {
        Builtin => "builtin",
        Project => "project",
        User => "user",
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum PromptMentionResource {
    Thread {
        thread_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        project_id: Option<String>,
        #[serde(default)]
        label: String,
    },
    Project {
        project_id: String,
        #[serde(default)]
        label: String,
    },
    #[serde(alias = "folder")]
    Section {
        #[serde(alias = "folderId")]
        section_id: String,
        #[serde(default)]
        label: String,
    },
    Path {
        source: PromptMentionPathSource,
        entry_kind: PathEntryKind,
        path: String,
        #[serde(default)]
        label: String,
    },
    Command {
        trigger: PromptMentionCommandTrigger,
        name: String,
        source: PromptMentionCommandSource,
        origin: PromptMentionCommandOrigin,
        #[serde(default)]
        label: String,
        #[serde(default)]
        argument_hint: Option<String>,
    },
    Plugin {
        plugin_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        icon: Option<String>,
        item_id: String,
        #[serde(default)]
        label: String,
    },
    #[serde(untagged)]
    Unknown(JsonValue),
}

default_unknown!(PromptMentionResource);

impl PromptMentionResource {
    pub fn label(&self) -> Option<&str> {
        match self {
            Self::Thread { label, .. }
            | Self::Project { label, .. }
            | Self::Section { label, .. }
            | Self::Path { label, .. }
            | Self::Command { label, .. }
            | Self::Plugin { label, .. } => Some(label),
            Self::Unknown(value) => value.get("label").and_then(JsonValue::as_str),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PromptTextMention {
    pub start: u32,
    pub end: u32,
    pub resource: PromptMentionResource,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TextPromptInput {
    pub text: String,
    pub mentions: Vec<PromptTextMention>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<PromptInputVisibility>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ImagePromptInput {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<PromptInputVisibility>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LocalImagePromptInput {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<PromptInputVisibility>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LocalFilePromptInput {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<PromptInputVisibility>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PromptInput {
    Text(TextPromptInput),
    Image(ImagePromptInput),
    LocalImage(LocalImagePromptInput),
    LocalFile(LocalFilePromptInput),
    #[serde(untagged)]
    Unknown(JsonValue),
}

default_unknown!(PromptInput);

impl PromptInput {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(TextPromptInput {
            text: text.into(),
            ..TextPromptInput::default()
        })
    }

    pub fn text_with_mentions(text: impl Into<String>, mentions: Vec<PromptTextMention>) -> Self {
        Self::Text(TextPromptInput {
            text: text.into(),
            mentions,
            visibility: None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ResolvedThreadExecutionOptions {
    pub model: String,
    pub service_tier: String,
    pub reasoning_level: ReasoningLevel,
    pub permission_mode: PermissionMode,
    pub source: ThreadExecutionSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProjectExecutionDefaults {
    pub provider_id: String,
    pub model: String,
    pub service_tier: String,
    pub reasoning_level: ReasoningLevel,
    pub permission_mode: PermissionMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExistingThreadExecutionInputSources {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<ExecutionInputSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<ExecutionInputSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_level: Option<ExecutionInputSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<ExecutionInputSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CreateExecutionInputSources {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<ExecutionInputSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<ExecutionInputSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<ExecutionInputSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_level: Option<ExecutionInputSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<ExecutionInputSource>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PluginSubmission {
    pub plugin_id: String,
    pub data: JsonValue,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PromptHistoryEntry {
    pub id: String,
    pub created_at: i64,
    pub input: Vec<PromptInput>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn legacy_folder_mention_becomes_section() {
        let mention: PromptTextMention = serde_json::from_value(json!({
            "start": 0,
            "end": 4,
            "resource": {"kind": "folder", "folderId": "sec_1", "label": "Work"}
        }))
        .unwrap();
        assert_eq!(
            mention.resource,
            PromptMentionResource::Section {
                section_id: "sec_1".into(),
                label: "Work".into()
            }
        );
        assert_eq!(
            serde_json::to_value(&mention.resource).unwrap(),
            json!({"kind": "section", "sectionId": "sec_1", "label": "Work"})
        );
    }

    #[test]
    fn text_input_serializes_without_absent_visibility() {
        let input = PromptInput::text("hello");
        assert_eq!(
            serde_json::to_value(&input).unwrap(),
            json!({"type": "text", "text": "hello", "mentions": []})
        );
    }
}
