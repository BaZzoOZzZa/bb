use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::common::{JsonObject, JsonValue};

pub const PUSH_NOTIFICATIONS_PLUGIN_ID: &str = "push-notifications";
pub const PUSH_NOTIFICATION_CHANNEL: &str = "notification";

wire_enum! {
    pub enum PluginRuntimeStatus {
        Starting => "starting",
        Running => "running",
        Error => "error",
        Incompatible => "incompatible",
        Missing => "missing",
        Disabled => "disabled",
        Degraded => "degraded",
        NeedsConfiguration => "needs-configuration",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PluginCliCommandSummary {
    pub name: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct InstalledPlugin {
    pub id: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub version: String,
    pub source: String,
    pub source_display: String,
    pub enabled: bool,
    pub status: PluginRuntimeStatus,
    pub status_detail: Option<String>,
    pub has_settings: bool,
    pub icon: Option<String>,
    pub cli_command: Option<PluginCliCommandSummary>,
    pub provider_ids: Vec<String>,
    pub icons: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PluginListResponse {
    pub plugins: Vec<InstalledPlugin>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PluginCliCommandInfo {
    pub name: String,
    pub summary: String,
    pub usage: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PluginCliContribution {
    pub plugin_id: String,
    pub name: String,
    pub summary: String,
    pub commands: Vec<PluginCliCommandInfo>,
    pub renders_help: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PluginMentionProviderContribution {
    pub plugin_id: String,
    pub id: String,
    pub label: String,
    pub triggers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PluginContributionsResponse {
    pub cli_commands: Vec<PluginCliContribution>,
    pub mention_providers: Vec<PluginMentionProviderContribution>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginCliRequest {
    pub argv: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PluginCliResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonValue>,
}

impl PluginCliResult {
    pub fn parse_body(body: &str) -> Result<Self, serde_json::Error> {
        let payload = body
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("");
        serde_json::from_str(payload)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PluginMentionSearchItem {
    pub item_id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub icon: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PluginMentionSearchGroup {
    pub plugin_id: String,
    pub provider_id: String,
    pub label: String,
    pub items: Vec<PluginMentionSearchItem>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PluginMentionSearchResponse {
    pub ok: bool,
    pub groups: Vec<PluginMentionSearchGroup>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<PluginErrorValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginMentionSearchQuery {
    pub q: String,
    pub trigger: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PluginSettingDescriptor {
    String {
        #[serde(default)]
        label: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        secret: Option<bool>,
        #[serde(
            default,
            rename = "experimental_multiline",
            skip_serializing_if = "Option::is_none"
        )]
        multiline: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default: Option<String>,
    },
    Boolean {
        #[serde(default)]
        label: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default: Option<bool>,
    },
    Number {
        #[serde(default)]
        label: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default: Option<f64>,
    },
    Select {
        #[serde(default)]
        label: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default)]
        options: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default: Option<String>,
    },
    Project {
        #[serde(default)]
        label: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default: Option<String>,
    },
    #[serde(untagged)]
    Unknown(JsonValue),
}

default_unknown!(PluginSettingDescriptor);

impl PluginSettingDescriptor {
    pub fn label(&self) -> Option<&str> {
        match self {
            Self::String { label, .. }
            | Self::Boolean { label, .. }
            | Self::Number { label, .. }
            | Self::Select { label, .. }
            | Self::Project { label, .. } => Some(label),
            Self::Unknown(value) => value.get("label").and_then(JsonValue::as_str),
        }
    }

    pub fn is_secret(&self) -> bool {
        matches!(
            self,
            Self::String {
                secret: Some(true),
                ..
            }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PluginSettingsResponse {
    pub ok: bool,
    pub schema: BTreeMap<String, PluginSettingDescriptor>,
    pub values: JsonObject,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<PluginErrorValue>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginSettingsUpdateRequest {
    pub values: JsonObject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginCodedError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PluginErrorValue {
    Message(String),
    Coded(PluginCodedError),
    Other(JsonValue),
}

impl PluginErrorValue {
    pub fn message(&self) -> String {
        match self {
            Self::Message(message) => message.clone(),
            Self::Coded(error) => error.message.clone(),
            Self::Other(value) => value.to_string(),
        }
    }

    pub fn code(&self) -> Option<&str> {
        match self {
            Self::Coded(error) => Some(&error.code),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PluginRpcResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<PluginErrorValue>,
}

impl PluginRpcResponse {
    pub fn into_result(self) -> Result<JsonValue, PluginErrorValue> {
        if self.ok {
            Ok(self.result.unwrap_or(JsonValue::Null))
        } else {
            Err(self
                .error
                .unwrap_or_else(|| PluginErrorValue::Message("plugin rpc failed".into())))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PluginErrorEnvelope {
    pub ok: bool,
    pub error: Option<PluginErrorValue>,
}

wire_enum! {
    pub enum NotificationChannel {
        Web => "web",
        Desktop => "desktop",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ClientNotification {
    pub id: String,
    pub title: String,
    pub body: String,
    pub thread_id: Option<String>,
    pub channels: Vec<NotificationChannel>,
}
