use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::common::JsonValue;
use crate::prompt::{PermissionMode, PromptMentionCommandTrigger, ReasoningLevel};
use crate::timeline::CompletedTurnDisplay;

wire_enum! {
    pub enum ServerMoveHealthState {
        Pending => "pending",
        Activating => "activating",
        Ready => "ready",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ServerMoveHealth {
    pub move_id: String,
    pub state: ServerMoveHealthState,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HealthResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub launch_id: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::common::lenient_option"
    )]
    pub server_move: Option<ServerMoveHealth>,
}

wire_enum! {
    pub enum InstallKind {
        Desktop => "desktop",
        Npm => "npm",
        Source => "source",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SystemVersionResponse {
    pub current_commit: Option<String>,
    pub install_kind: Option<InstallKind>,
    pub current_version: String,
    pub latest_version: Option<String>,
    pub source: String,
    pub update_available: bool,
    pub is_development: bool,
    pub upgrade_command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemVersionQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub force: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SystemAttentionResponse {
    pub has_attention: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ModelReasoningEffort {
    pub reasoning_effort: ReasoningLevel,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ModelServiceTier {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AvailableModel {
    pub id: String,
    pub model: String,
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_provider_id: Option<String>,
    pub description: String,
    pub supported_reasoning_efforts: Vec<ModelReasoningEffort>,
    pub default_reasoning_effort: ReasoningLevel,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supported_service_tiers: Option<Vec<ModelServiceTier>>,
    pub is_default: bool,
}

wire_enum! {
    pub enum ModelCatalogScope {
        Host => "host",
        Workspace => "workspace",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProviderCapabilities {
    pub supports_thread_archive: bool,
    pub supports_thread_rename: bool,
    pub supports_service_tier: bool,
    pub supports_native_user_question: bool,
    pub supports_fork: bool,
    pub supports_session_rewind: bool,
    pub permission_modes: Vec<PermissionMode>,
    pub model_catalog_scope: ModelCatalogScope,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProviderComposerCommand {
    pub trigger: PromptMentionCommandTrigger,
    pub name: String,
    pub trailing_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum ProviderComposerAction {
    Skills {
        trigger: PromptMentionCommandTrigger,
    },
    Plan {
        command: ProviderComposerCommand,
    },
    Goal {
        command: ProviderComposerCommand,
    },
    #[serde(untagged)]
    Unknown(JsonValue),
}

default_unknown!(ProviderComposerAction);

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProviderOptionDescriptor {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProviderMaintenance {
    pub health: bool,
    pub usage: bool,
    pub installation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProviderIcon {
    pub glyph: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProviderStrings {
    pub sign_in_hint: String,
    pub expired_hint: String,
    pub install_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brand_prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_mode_copy: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProviderExtensionKindInfo {
    pub item: bool,
    pub state: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProviderInfo {
    pub id: String,
    pub plugin_id: String,
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<ProviderIcon>,
    pub logo_url: Option<String>,
    pub maintenance: ProviderMaintenance,
    pub capabilities: ProviderCapabilities,
    pub composer_actions: Vec<ProviderComposerAction>,
    pub completed_turn_display: CompletedTurnDisplay,
    pub available: bool,
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::common::lenient_option"
    )]
    pub strings: Option<ProviderStrings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tiers: Option<Vec<ProviderOptionDescriptor>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_levels: Option<Vec<ProviderOptionDescriptor>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension_kinds: Option<BTreeMap<String, ProviderExtensionKindInfo>>,
}

wire_enum! {
    pub enum ModelLoadErrorCode {
        ProviderUnavailable => "provider_unavailable",
        MissingExecutable => "missing_executable",
        AuthRequired => "auth_required",
        Timeout => "timeout",
        Failed => "failed",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ModelLoadError {
    pub provider_id: String,
    pub code: ModelLoadErrorCode,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SystemExecutionOptionsResponse {
    pub providers: Vec<ProviderInfo>,
    pub permission_ceiling: PermissionMode,
    pub models: Vec<AvailableModel>,
    pub selected_only_models: Vec<AvailableModel>,
    pub model_load_error: Option<ModelLoadError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemExecutionOptionsQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemProvidersQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capability: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "status",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum ProviderAvailability {
    Available,
    SetupRequired {
        #[serde(default)]
        message: String,
    },
    Unavailable {
        #[serde(default)]
        message: String,
    },
    #[serde(untagged)]
    Unknown(JsonValue),
}

default_unknown!(ProviderAvailability);

impl ProviderAvailability {
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EnvironmentProviderRequirements {
    pub project_checkout: bool,
    pub git_checkout: bool,
    pub git_remote: bool,
    pub projectless: bool,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SystemEnvironmentProvider {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment_provider_id: Option<String>,
    pub machine_provider_id: Option<String>,
    pub id: String,
    pub display_name: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub logo_url: Option<String>,
    pub plugin_id: String,
    pub requires: EnvironmentProviderRequirements,
    pub inputs: Option<JsonValue>,
    pub accepts_empty_inputs: bool,
    pub availability: Option<ProviderAvailability>,
    pub machine_availability: BTreeMap<String, Option<ProviderAvailability>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub machine_inputs: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub machine_accepts_empty_inputs: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub machine_provider_plugin_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SystemEnvironmentProvidersResponse {
    pub providers: Vec<SystemEnvironmentProvider>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemEnvironmentProvidersQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppShortcut {
    pub key: String,
    #[serde(rename = "mod")]
    pub primary_modifier: bool,
    pub meta: bool,
    pub control: bool,
    pub alt: bool,
    pub shift: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppCommandWhen {
    pub all: Vec<String>,
    pub none: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppKeybinding {
    pub command: String,
    pub desktop_only: bool,
    pub shortcut: AppShortcut,
    pub when: AppCommandWhen,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppDefaultKeybinding {
    pub command: String,
    pub desktop_only: bool,
    pub shortcut: Option<AppShortcut>,
    pub when: AppCommandWhen,
}

wire_enum! {
    pub enum KeyboardPlatform {
        Mac => "mac",
        Windows => "windows",
        Linux => "linux",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppKeybindingOverride {
    pub command: String,
    pub shortcut: Option<AppShortcut>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<KeyboardPlatform>,
}

pub type AppKeybindingOverrides = Vec<AppKeybindingOverride>;

pub const APP_COMMAND_IDS: &[&str] = &[
    "palette.open",
    "thread.new",
    "thread.search",
    "thread.rename",
    "thread.archive",
    "thread.previous",
    "thread.next",
    "history.back",
    "history.forward",
    "thread.jump.1",
    "thread.jump.2",
    "thread.jump.3",
    "thread.jump.4",
    "thread.jump.5",
    "thread.jump.6",
    "thread.jump.7",
    "thread.jump.8",
    "thread.jump.9",
    "pane.focus.left",
    "pane.focus.right",
    "pane.focus.up",
    "pane.focus.down",
    "pane.focus.previous",
    "pane.focus.next",
    "pane.focus.1",
    "pane.focus.2",
    "pane.focus.3",
    "pane.focus.4",
    "pane.focus.5",
    "pane.focus.6",
    "pane.focus.7",
    "pane.focus.8",
    "pane.maximize.toggle",
    "pane.close",
    "window.new",
    "window.find",
    "window.reload",
    "app.back",
    "settings.open",
    "settings.openServers",
    "sidebar.toggle",
    "panel.previousTab",
    "panel.nextTab",
    "panel.previousNewTabItem",
    "panel.nextNewTabItem",
    "panel.newTab",
    "panel.reopenClosedTab",
    "panel.close",
    "panel.toggle",
    "panel.fullScreen.toggle",
    "file.quickOpen",
    "diff.toggle",
    "terminal.open",
    "composer.focus",
    "modelPicker.toggle",
    "modelPicker.cycleModel",
    "modelPicker.cycleModelBackward",
    "modelPicker.cycleProvider",
    "modelPicker.cycleProviderBackward",
    "modelPicker.cycleReasoning",
    "modelPicker.cycleReasoningBackward",
    "browser.focusLocation",
    "browser.reload",
    "browser.find",
    "workspace.openPreferred",
    "logs.openServerDaemon",
    "dataDirectory.open",
    "notifications.open",
    "plugins.enterSafeMode",
    "plugins.exitSafeMode",
    "plugins.pruneCache",
    "question.select.1",
    "question.select.2",
    "question.select.3",
    "question.select.4",
    "question.select.5",
    "question.select.6",
    "question.select.7",
    "question.select.8",
    "question.select.9",
];

pub const APP_COMMAND_CONTEXT_KEYS: &[&str] = &[
    "mainSurface",
    "modalOpen",
    "editableFocus",
    "terminalFocus",
    "browserFocus",
    "modelPickerOpen",
    "questionOpen",
    "promptAvailable",
    "splitActive",
    "webSurface",
    "macPlatform",
    "windowsPlatform",
    "linuxPlatform",
];

pub fn is_app_command_id(id: &str) -> bool {
    APP_COMMAND_IDS.contains(&id)
}

pub fn parse_plugin_command_id(id: &str) -> Option<(&str, &str)> {
    let rest = id.strip_prefix("plugin:")?;
    let (plugin_id, command_id) = rest.split_once('/')?;
    let plugin_ok = plugin_id
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
        && plugin_id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    let command_ok = !command_id.is_empty()
        && command_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    (plugin_ok && command_ok && id.len() <= 256).then_some((plugin_id, command_id))
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SystemConfigResponse {
    pub general_settings: JsonValue,
    #[serde(deserialize_with = "crate::common::lenient_vec")]
    pub keybindings: Vec<AppKeybinding>,
    #[serde(deserialize_with = "crate::common::lenient_vec")]
    pub default_keybindings: Vec<AppDefaultKeybinding>,
    #[serde(deserialize_with = "crate::common::lenient_vec")]
    pub keybinding_overrides: Vec<AppKeybindingOverride>,
    pub experiments: JsonValue,
    pub feature_flags: JsonValue,
    pub appearance: JsonValue,
    pub server_access: JsonValue,
    pub host_daemon_port: Option<u16>,
    pub server_url: String,
    pub primary_host_id: Option<String>,
    pub primary_host_platform: Option<String>,
    pub voice_transcription_enabled: bool,
    pub data_dir: String,
}
