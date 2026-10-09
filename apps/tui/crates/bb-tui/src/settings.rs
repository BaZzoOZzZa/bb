use std::collections::{HashMap, HashSet};

use bb_wire::common::{JsonObject, JsonValue};
use bb_wire::environment::{
    Host, HostProviderCliStatusResponse, HostStatus, HostType, MachinePhase,
};
use bb_wire::plugin::{InstalledPlugin, PluginRuntimeStatus, PluginSettingsResponse};
use bb_wire::project::{PERSONAL_PROJECT_ID, ProjectWithThreadsResponse};
use bb_wire::prompt::{PermissionMode, ProjectExecutionDefaults};
use bb_wire::system::{
    AvailableModel, SystemConfigResponse, SystemProviderCatalogEntry, SystemVersionResponse,
};
use bb_wire::thread::{ThreadChildSummaryResponse, ThreadListEntry};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{AppState, Effect, ToastLevel};
use crate::execution::Load;
use crate::input::{InputOutcome, LineInput};
use crate::keymap::Command;
use crate::prompt::{PERMISSION_MODE_OPTIONS, permission_mode_label};
use crate::settings_form::{
    FieldKind, descriptor_description, field_display, general_bool, initial_edit_text,
    merge_general, parse_field_input, unchanged, validate_branch_prefix, validate_server_url,
};
use crate::sidebar::thread_title;
use crate::time::compact_relative_time;

pub const ARCHIVED_PAGE_SIZE: u32 = 50;
const PAGE: isize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SettingsSection {
    General,
    Providers,
    Keyboard,
    Projects,
    Machines,
    Plugins,
    Archived,
}

impl SettingsSection {
    pub const ALL: [SettingsSection; 7] = [
        Self::General,
        Self::Providers,
        Self::Keyboard,
        Self::Projects,
        Self::Machines,
        Self::Plugins,
        Self::Archived,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Providers => "providers",
            Self::Keyboard => "keyboard",
            Self::Projects => "projects",
            Self::Machines => "machines",
            Self::Plugins => "plugins",
            Self::Archived => "archived",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Providers => "Providers",
            Self::Keyboard => "Keyboard",
            Self::Projects => "Projects",
            Self::Machines => "Machines",
            Self::Plugins => "Installed plugins",
            Self::Archived => "Archived threads",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|section| section.id() == id)
    }

    pub fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|section| *section == self)
            .unwrap_or(0)
    }

    fn filterable(self) -> bool {
        matches!(
            self,
            Self::Keyboard | Self::Projects | Self::Plugins | Self::Archived
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsFocus {
    Nav,
    Content,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Detail {
    Project(String),
    Machine(String),
    Plugin(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArchivedKind {
    #[default]
    All,
    Root,
    Child,
}

impl ArchivedKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All threads",
            Self::Root => "Root threads",
            Self::Child => "Child threads",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::All => Self::Root,
            Self::Root => Self::Child,
            Self::Child => Self::All,
        }
    }

    fn has_parent(self) -> Option<bool> {
        match self {
            Self::All => None,
            Self::Root => Some(false),
            Self::Child => Some(true),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ArchivedQuery {
    pub project_id: Option<String>,
    pub kind: ArchivedKind,
    pub offset: u32,
}

impl ArchivedQuery {
    pub fn list_query(&self) -> bb_wire::thread::ThreadListQuery {
        bb_wire::thread::ThreadListQuery {
            project_id: self.project_id.clone(),
            has_parent: self.kind.has_parent(),
            archived: Some(true),
            limit: Some(ARCHIVED_PAGE_SIZE),
            offset: Some(self.offset),
            ..bb_wire::thread::ThreadListQuery::default()
        }
    }

    fn same_filters(&self, other: &ArchivedQuery) -> bool {
        self.project_id == other.project_id && self.kind == other.kind
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ArchivedList {
    pub query: ArchivedQuery,
    pub threads: Vec<ThreadListEntry>,
    pub loading: bool,
    pub loaded: bool,
    pub has_more: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditTarget {
    General(&'static str),
    PluginField { plugin_id: String, key: String },
    ProjectName(String),
    HostName(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldEdit {
    pub target: EditTarget,
    pub label: String,
    pub input: LineInput,
    pub secret: bool,
    pub hint: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChoiceTarget {
    PluginField { plugin_id: String, key: String },
    HostPermission(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub title: String,
    pub target: ChoiceTarget,
    pub options: Vec<(String, String)>,
    pub selected: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmAction {
    DeleteProject { project_id: String, name: String },
    DeleteThread { thread_id: String, title: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct SettingsConfirm {
    pub action: ConfirmAction,
    pub children: Load<ThreadChildSummaryResponse>,
}

impl SettingsConfirm {
    pub fn title(&self) -> &'static str {
        match self.action {
            ConfirmAction::DeleteProject { .. } => "Delete project",
            ConfirmAction::DeleteThread { .. } => "Delete thread",
        }
    }

    pub fn lines(&self) -> Vec<String> {
        match &self.action {
            ConfirmAction::DeleteProject { name, .. } => vec![
                format!("Delete {name}?"),
                "This removes the project and every thread in it, including archived ones.".into(),
                "Checkouts stay on disk.".into(),
            ],
            ConfirmAction::DeleteThread { title, .. } => {
                let mut lines = vec![
                    format!("Delete \"{title}\" permanently?"),
                    "Its messages and history cannot be restored.".into(),
                ];
                match &self.children {
                    Load::Loaded(summary) if summary.non_deleted_child_count > 0 => {
                        lines.push(format!(
                            "Its {} child thread(s) are deleted too.",
                            summary.non_deleted_child_count
                        ));
                    }
                    Load::Loading | Load::Idle => lines.push("Checking child threads…".into()),
                    Load::Failed(error) => {
                        lines.push(format!("Could not check child threads: {error}"))
                    }
                    Load::Loaded(_) => {}
                }
                lines
            }
        }
    }

    pub fn ready(&self) -> bool {
        match self.action {
            ConfirmAction::DeleteProject { .. } => true,
            ConfirmAction::DeleteThread { .. } => matches!(self.children, Load::Loaded(_)),
        }
    }
}

#[derive(Clone, PartialEq)]
pub enum SettingsMutation {
    UpdateGeneral {
        key: &'static str,
        settings: JsonObject,
    },
    SetProviderEnabled {
        provider_id: String,
        enabled: bool,
    },
    RenameProject {
        project_id: String,
        name: String,
    },
    DeleteProject {
        project_id: String,
    },
    RenameHost {
        host_id: String,
        name: String,
    },
    SetHostPermissionCeiling {
        host_id: String,
        mode: PermissionMode,
    },
    UnarchiveThread {
        thread_id: String,
    },
    DeleteThread {
        thread_id: String,
        child_threads_confirmed: bool,
    },
    SetPluginEnabled {
        plugin_id: String,
        enabled: bool,
    },
    UpdatePluginSetting {
        plugin_id: String,
        key: String,
        value: JsonValue,
        secret: bool,
    },
}

impl std::fmt::Debug for SettingsMutation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UpdatePluginSetting {
                plugin_id,
                key,
                value,
                secret,
            } => {
                let mut debug = f.debug_struct("UpdatePluginSetting");
                debug.field("plugin_id", plugin_id).field("key", key);
                if *secret {
                    debug.field("value", &"<redacted>");
                } else {
                    debug.field("value", value);
                }
                debug.finish()
            }
            Self::UpdateGeneral { key, settings } => f
                .debug_struct("UpdateGeneral")
                .field("key", key)
                .field("value", &settings.get(*key))
                .finish(),
            Self::SetProviderEnabled {
                provider_id,
                enabled,
            } => f
                .debug_struct("SetProviderEnabled")
                .field("provider_id", provider_id)
                .field("enabled", enabled)
                .finish(),
            Self::RenameProject { project_id, name } => f
                .debug_struct("RenameProject")
                .field("project_id", project_id)
                .field("name", name)
                .finish(),
            Self::DeleteProject { project_id } => f
                .debug_struct("DeleteProject")
                .field("project_id", project_id)
                .finish(),
            Self::RenameHost { host_id, name } => f
                .debug_struct("RenameHost")
                .field("host_id", host_id)
                .field("name", name)
                .finish(),
            Self::SetHostPermissionCeiling { host_id, mode } => f
                .debug_struct("SetHostPermissionCeiling")
                .field("host_id", host_id)
                .field("mode", mode)
                .finish(),
            Self::UnarchiveThread { thread_id } => f
                .debug_struct("UnarchiveThread")
                .field("thread_id", thread_id)
                .finish(),
            Self::DeleteThread {
                thread_id,
                child_threads_confirmed,
            } => f
                .debug_struct("DeleteThread")
                .field("thread_id", thread_id)
                .field("child_threads_confirmed", child_threads_confirmed)
                .finish(),
            Self::SetPluginEnabled { plugin_id, enabled } => f
                .debug_struct("SetPluginEnabled")
                .field("plugin_id", plugin_id)
                .field("enabled", enabled)
                .finish(),
        }
    }
}

impl SettingsMutation {
    pub fn key(&self) -> String {
        match self {
            Self::UpdateGeneral { key, .. } => format!("general:{key}"),
            Self::SetProviderEnabled { provider_id, .. } => format!("provider:{provider_id}"),
            Self::RenameProject { project_id, .. } | Self::DeleteProject { project_id } => {
                format!("project:{project_id}")
            }
            Self::RenameHost { host_id, .. } | Self::SetHostPermissionCeiling { host_id, .. } => {
                format!("host:{host_id}")
            }
            Self::UnarchiveThread { thread_id } | Self::DeleteThread { thread_id, .. } => {
                format!("thread:{thread_id}")
            }
            Self::SetPluginEnabled { plugin_id, .. } => format!("plugin:{plugin_id}"),
            Self::UpdatePluginSetting { plugin_id, key, .. } => {
                format!("plugin-setting:{plugin_id}:{key}")
            }
        }
    }

    pub fn failure_verb(&self) -> &'static str {
        match self {
            Self::UpdateGeneral { .. } => "change settings",
            Self::SetProviderEnabled { enabled: true, .. } => "enable the provider",
            Self::SetProviderEnabled { enabled: false, .. } => "disable the provider",
            Self::RenameProject { .. } => "rename the project",
            Self::DeleteProject { .. } => "delete the project",
            Self::RenameHost { .. } => "rename the machine",
            Self::SetHostPermissionCeiling { .. } => "change the permission limit",
            Self::UnarchiveThread { .. } => "unarchive",
            Self::DeleteThread { .. } => "delete the thread",
            Self::SetPluginEnabled { enabled: true, .. } => "enable the plugin",
            Self::SetPluginEnabled { enabled: false, .. } => "disable the plugin",
            Self::UpdatePluginSetting { .. } => "save the plugin setting",
        }
    }

    fn success_message(&self) -> Option<&'static str> {
        Some(match self {
            Self::UpdateGeneral { .. } | Self::UpdatePluginSetting { .. } => return None,
            Self::SetProviderEnabled { enabled: true, .. } => "Provider enabled",
            Self::SetProviderEnabled { enabled: false, .. } => "Provider disabled",
            Self::RenameProject { .. } => "Project renamed",
            Self::DeleteProject { .. } => "Project deleted",
            Self::RenameHost { .. } => "Machine renamed",
            Self::SetHostPermissionCeiling { .. } => "Permission limit changed",
            Self::UnarchiveThread { .. } => "Unarchived",
            Self::DeleteThread { .. } => "Thread deleted",
            Self::SetPluginEnabled { enabled: true, .. } => "Plugin enabled",
            Self::SetPluginEnabled { enabled: false, .. } => "Plugin disabled",
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SettingsReply {
    Done,
    General(JsonValue),
    Catalog(Vec<SystemProviderCatalogEntry>),
    Host(Box<Host>),
    PluginSettings(Box<PluginSettingsResponse>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum SettingsRequest {
    Config,
    Version,
    Catalog,
    Models { provider_id: String },
    HostClis { host_id: String },
    ProjectDefaults { project_id: String },
    Plugins,
    PluginSettings { plugin_id: String },
    Archived(ArchivedQuery),
    ChildSummary { thread_id: String },
    Mutate(SettingsMutation),
}

#[derive(Debug, Clone, PartialEq)]
pub enum SettingsLoaded {
    Config(Result<Box<SystemConfigResponse>, String>),
    Version(Result<Box<SystemVersionResponse>, String>),
    Catalog(Result<Vec<SystemProviderCatalogEntry>, String>),
    Models {
        provider_id: String,
        result: Result<Vec<AvailableModel>, String>,
    },
    HostClis {
        host_id: String,
        result: Result<HostProviderCliStatusResponse, String>,
    },
    ProjectDefaults {
        project_id: String,
        result: Result<Option<ProjectExecutionDefaults>, String>,
    },
    Plugins(Result<Vec<InstalledPlugin>, String>),
    PluginSettings {
        plugin_id: String,
        result: Result<PluginSettingsResponse, String>,
    },
    Archived {
        query: ArchivedQuery,
        result: Result<Vec<ThreadListEntry>, String>,
    },
    ChildSummary {
        thread_id: String,
        result: Result<ThreadChildSummaryResponse, String>,
    },
    Mutated {
        mutation: SettingsMutation,
        result: Result<SettingsReply, String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct SettingsView {
    pub section: SettingsSection,
    pub focus: SettingsFocus,
    pub cursors: HashMap<SettingsSection, usize>,
    pub detail: Option<Detail>,
    pub detail_cursor: usize,
    pub filters: HashMap<SettingsSection, String>,
    pub filter_input: Option<LineInput>,
    pub config: Load<SystemConfigResponse>,
    pub version: Load<SystemVersionResponse>,
    pub catalog: Load<Vec<SystemProviderCatalogEntry>>,
    pub expanded_provider: Option<String>,
    pub models: HashMap<String, Load<Vec<AvailableModel>>>,
    pub host_clis: HashMap<String, Load<HostProviderCliStatusResponse>>,
    pub project_defaults: HashMap<String, Load<Option<ProjectExecutionDefaults>>>,
    pub plugins: Load<Vec<InstalledPlugin>>,
    pub plugin_settings: HashMap<String, Load<PluginSettingsResponse>>,
    pub field_errors: HashMap<String, String>,
    pub archived: ArchivedList,
    pub edit: Option<FieldEdit>,
    pub choice: Option<Choice>,
    pub confirm: Option<SettingsConfirm>,
    pub pending: HashSet<String>,
    pub offset: std::cell::Cell<usize>,
}

impl SettingsView {
    pub fn captures_keys(&self) -> bool {
        self.edit.is_some()
            || self.choice.is_some()
            || self.confirm.is_some()
            || self.filter_input.is_some()
    }

    pub fn new(section: SettingsSection) -> Self {
        Self {
            section,
            focus: SettingsFocus::Content,
            cursors: HashMap::new(),
            detail: None,
            detail_cursor: 0,
            filters: HashMap::new(),
            filter_input: None,
            config: Load::Loading,
            version: Load::Loading,
            catalog: Load::Loading,
            expanded_provider: None,
            models: HashMap::new(),
            host_clis: HashMap::new(),
            project_defaults: HashMap::new(),
            plugins: Load::Loading,
            plugin_settings: HashMap::new(),
            field_errors: HashMap::new(),
            archived: ArchivedList::default(),
            edit: None,
            choice: None,
            confirm: None,
            pending: HashSet::new(),
            offset: std::cell::Cell::new(0),
        }
    }

    pub fn filter(&self, section: SettingsSection) -> &str {
        if section == self.section
            && let Some(input) = &self.filter_input
        {
            return input.text();
        }
        self.filters.get(&section).map_or("", String::as_str)
    }

    pub fn cursor(&self) -> usize {
        if self.detail.is_some() {
            self.detail_cursor
        } else {
            self.cursors.get(&self.section).copied().unwrap_or(0)
        }
    }

    fn set_cursor(&mut self, cursor: usize) {
        if self.detail.is_some() {
            self.detail_cursor = cursor;
        } else {
            self.cursors.insert(self.section, cursor);
        }
    }

    pub fn general(&self) -> Option<&JsonValue> {
        self.config
            .loaded()
            .map(|config| &config.general_settings)
            .filter(|general| general.is_object())
    }

    fn initial_requests() -> Vec<SettingsRequest> {
        vec![
            SettingsRequest::Config,
            SettingsRequest::Version,
            SettingsRequest::Catalog,
            SettingsRequest::Plugins,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Muted,
    Good,
    Warn,
    Bad,
    Accent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Badge {
    pub text: String,
    pub tone: Tone,
}

fn badge(text: impl Into<String>, tone: Tone) -> Badge {
    Badge {
        text: text.into(),
        tone,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowStyle {
    Header,
    Item,
    Note,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowTarget {
    GeneralToggle(&'static str),
    GeneralText(&'static str),
    Copy(String),
    Provider(String),
    FastServiceTier,
    Binding(Command),
    Project(String),
    ProjectRename(String),
    ProjectDelete(String),
    Host(String),
    HostRename(String),
    HostPermission(String),
    Plugin(String),
    PluginEnabled(String),
    PluginField { plugin_id: String, key: String },
    Archived(String),
    ArchivedMore,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsRow {
    pub target: Option<RowTarget>,
    pub label: String,
    pub value: String,
    pub badges: Vec<Badge>,
    pub style: RowStyle,
    pub indent: u16,
}

impl SettingsRow {
    fn header(label: impl Into<String>) -> Self {
        Self {
            target: None,
            label: label.into(),
            value: String::new(),
            badges: Vec::new(),
            style: RowStyle::Header,
            indent: 0,
        }
    }

    fn note(label: impl Into<String>) -> Self {
        Self {
            style: RowStyle::Note,
            indent: 2,
            ..Self::header(label)
        }
    }

    fn error(label: impl Into<String>) -> Self {
        Self {
            style: RowStyle::Error,
            indent: 2,
            ..Self::header(label)
        }
    }

    fn info(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            target: None,
            label: label.into(),
            value: value.into(),
            badges: Vec::new(),
            style: RowStyle::Item,
            indent: 0,
        }
    }

    fn item(target: RowTarget, label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            target: Some(target),
            ..Self::info(label, value)
        }
    }

    fn with_badge(mut self, text: impl Into<String>, tone: Tone) -> Self {
        self.badges.push(badge(text, tone));
        self
    }

    fn indented(mut self, indent: u16) -> Self {
        self.indent = indent;
        self
    }

    pub fn selectable(&self) -> bool {
        self.target.is_some()
    }
}

struct GeneralToggle {
    key: &'static str,
    label: &'static str,
    on: &'static str,
    off: &'static str,
    description: &'static str,
}

const THREAD_TOGGLES: &[GeneralToggle] = &[
    GeneralToggle {
        key: "steerActiveThreadOnEnter",
        label: "Default thread followup behavior",
        on: "Steer",
        off: "Queue",
        description: "What Enter does in the prompt box while the thread runs.",
    },
    GeneralToggle {
        key: "showGitChanges",
        label: "Show Git changes and Commit button",
        on: "on",
        off: "off",
        description: "Show Git and PR controls above the composer, and Commit in the header.",
    },
    GeneralToggle {
        key: "confirmThreadArchive",
        label: "Thread archive confirmation",
        on: "on",
        off: "off",
        description: "Ask before archiving a thread with unarchived children.",
    },
    GeneralToggle {
        key: "keepHistoryAfterContextClear",
        label: "Show messages from before a context clear",
        on: "on",
        off: "off",
        description: "Earlier messages stay in the thread for you to read. The agent doesn't see them.",
    },
];

const PRIVACY_TOGGLES: &[GeneralToggle] = &[
    GeneralToggle {
        key: "streamerMode",
        label: "Streamer mode",
        on: "on",
        off: "off",
        description: "Hide the custom models from config.json in every model picker.",
    },
    GeneralToggle {
        key: "telemetryEnabled",
        label: "Share anonymous usage data",
        on: "on",
        off: "off",
        description: "Send anonymous app starts, thread and message counts, and plugin installs.",
    },
    GeneralToggle {
        key: "showDiagnosticEvents",
        label: "Show diagnostic events",
        on: "on",
        off: "off",
        description: "Show provider environment resolution and unhandled provider events.",
    },
];

const BRANCH_PREFIX_KEY: &str = "managedBranchPrefix";
const MACHINE_SERVER_URL_KEY: &str = "machineServerUrl";
const FAST_TIER_KEY: &str = "allowFastServiceTier";
const DEFAULT_PROVIDER_KEY: &str = "defaultProviderId";
const COMPLETED_TURN_KEY: &str = "providerCompletedTurnDisplay";

fn load_rows<T>(load: &Load<T>, what: &str, rows: &mut Vec<SettingsRow>) -> bool {
    match load {
        Load::Loaded(_) => true,
        Load::Failed(error) => {
            rows.push(SettingsRow::error(format!("Couldn't load {what}: {error}")));
            false
        }
        Load::Idle | Load::Loading => {
            rows.push(SettingsRow::note(format!("Loading {what}…")));
            false
        }
    }
}

fn matches_filter(filter: &str, texts: &[&str]) -> bool {
    let filter = filter.trim().to_lowercase();
    filter.is_empty()
        || texts
            .iter()
            .any(|text| text.to_lowercase().contains(&filter))
}

fn pluralize(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

pub fn format_git_remote(remote: &str) -> String {
    let trimmed = remote.trim_end_matches('/').trim_end_matches(".git");
    if let Some(rest) = trimmed.strip_prefix("git@") {
        return rest.replacen(':', "/", 1);
    }
    for prefix in ["https://", "http://", "ssh://git@", "ssh://", "git://"] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            return rest.to_owned();
        }
    }
    trimmed.to_owned()
}

impl AppState {
    fn settings_projects(&self) -> Vec<&ProjectWithThreadsResponse> {
        self.snapshot
            .sidebar
            .data
            .as_deref()
            .map(|sidebar| sidebar.projects.iter().collect())
            .unwrap_or_default()
    }

    fn settings_project(&self, project_id: &str) -> Option<&ProjectWithThreadsResponse> {
        let sidebar = self.snapshot.sidebar.data.as_deref()?;
        sidebar
            .projects
            .iter()
            .chain(std::iter::once(&sidebar.personal_project))
            .find(|project| project.project.id == project_id)
    }

    fn settings_project_name(&self, project_id: &str) -> Option<String> {
        self.settings_project(project_id)
            .map(|project| project.project.name.clone())
    }

    fn settings_hosts(&self) -> &[Host] {
        self.snapshot
            .hosts
            .data
            .as_deref()
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    fn settings_host(&self, host_id: &str) -> Option<&Host> {
        self.settings_hosts().iter().find(|host| host.id == host_id)
    }

    fn primary_host_id(&self) -> Option<&str> {
        self.settings
            .as_ref()?
            .config
            .loaded()?
            .primary_host_id
            .as_deref()
            .filter(|id| !id.is_empty())
    }
}

pub fn settings_rows(state: &AppState) -> Vec<SettingsRow> {
    let Some(view) = state.settings.as_ref() else {
        return Vec::new();
    };
    match &view.detail {
        Some(Detail::Project(id)) => project_detail_rows(state, view, id),
        Some(Detail::Machine(id)) => machine_detail_rows(state, view, id),
        Some(Detail::Plugin(id)) => plugin_detail_rows(state, view, id),
        None => match view.section {
            SettingsSection::General => general_rows(state, view),
            SettingsSection::Providers => provider_rows(view),
            SettingsSection::Keyboard => keyboard_rows(state, view),
            SettingsSection::Projects => project_rows(state, view),
            SettingsSection::Machines => machine_rows(state, view),
            SettingsSection::Plugins => plugin_rows(view),
            SettingsSection::Archived => archived_rows(state, view),
        },
    }
}

fn pending_badge(view: &SettingsView, key: &str, row: SettingsRow) -> SettingsRow {
    if view.pending.contains(key) {
        row.with_badge("saving…", Tone::Muted)
    } else {
        row
    }
}

fn toggle_rows(
    view: &SettingsView,
    general: &JsonValue,
    toggles: &[GeneralToggle],
) -> Vec<SettingsRow> {
    let mut rows = Vec::new();
    for toggle in toggles {
        let Some(value) = general_bool(general, toggle.key) else {
            continue;
        };
        let row = SettingsRow::item(
            RowTarget::GeneralToggle(toggle.key),
            toggle.label,
            if value { toggle.on } else { toggle.off },
        );
        rows.push(pending_badge(view, &format!("general:{}", toggle.key), row));
        rows.push(SettingsRow::note(toggle.description));
    }
    rows
}

fn general_rows(state: &AppState, view: &SettingsView) -> Vec<SettingsRow> {
    let mut rows = vec![SettingsRow::header("Server")];
    rows.push(SettingsRow::item(
        RowTarget::Copy(state.server.url.clone()),
        "Server URL",
        state.server.url.clone(),
    ));
    match &view.version {
        Load::Loaded(version) => {
            let mut row = SettingsRow::info("Version", version.current_version.clone());
            if version.update_available {
                let latest = version.latest_version.clone().unwrap_or_default();
                row = row.with_badge(format!("update to {latest}"), Tone::Warn);
            } else if version.is_development {
                row = row.with_badge("development", Tone::Muted);
            }
            rows.push(row);
            if version.update_available && !version.upgrade_command.is_empty() {
                rows.push(SettingsRow::item(
                    RowTarget::Copy(version.upgrade_command.clone()),
                    "Upgrade command",
                    version.upgrade_command.clone(),
                ));
            }
        }
        Load::Failed(error) => rows.push(SettingsRow::info(
            "Version",
            format!("unavailable: {error}"),
        )),
        Load::Idle | Load::Loading => rows.push(SettingsRow::info("Version", "…")),
    }
    if !load_rows(&view.config, "settings", &mut rows) {
        return rows;
    }
    let Some(config) = view.config.loaded() else {
        return rows;
    };
    if !config.data_dir.is_empty() {
        rows.push(SettingsRow::item(
            RowTarget::Copy(config.data_dir.clone()),
            "Data directory",
            config.data_dir.clone(),
        ));
    }
    if let Some(primary) = state.primary_host_id() {
        let name = state
            .settings_host(primary)
            .map_or_else(|| primary.to_owned(), |host| host.name.clone());
        rows.push(SettingsRow::info("Server machine", name));
    }
    let Some(general) = view.general() else {
        rows.push(SettingsRow::error("The server sent no general settings"));
        return rows;
    };
    let threads = toggle_rows(view, general, THREAD_TOGGLES);
    if !threads.is_empty() {
        rows.push(SettingsRow::header("Threads & editing"));
        rows.extend(threads);
    }
    if let Some(prefix) = general.get(BRANCH_PREFIX_KEY) {
        rows.push(SettingsRow::header("Git"));
        let value = prefix.as_str().unwrap_or("");
        let row = SettingsRow::item(
            RowTarget::GeneralText(BRANCH_PREFIX_KEY),
            "New branch prefix",
            if value.is_empty() { "(none)" } else { value },
        );
        rows.push(pending_badge(view, "general:managedBranchPrefix", row));
        rows.push(SettingsRow::note(
            "Prefix for branches bb creates for new git worktree threads.",
        ));
    }
    if general.get(MACHINE_SERVER_URL_KEY).is_some() {
        rows.push(SettingsRow::header("Machines"));
        let value = general
            .get(MACHINE_SERVER_URL_KEY)
            .and_then(JsonValue::as_str)
            .unwrap_or("(server URL)");
        let row = SettingsRow::item(
            RowTarget::GeneralText(MACHINE_SERVER_URL_KEY),
            "Machine server URL",
            value,
        );
        rows.push(pending_badge(view, "general:machineServerUrl", row));
        rows.push(SettingsRow::note(
            "The address new machines use to reach this server. Empty uses the server URL.",
        ));
    }
    let privacy = toggle_rows(view, general, PRIVACY_TOGGLES);
    if !privacy.is_empty() {
        rows.push(SettingsRow::header("Privacy & diagnostics"));
        rows.extend(privacy);
    }
    rows
}

pub fn ordered_providers(view: &SettingsView) -> Vec<&SystemProviderCatalogEntry> {
    let Some(catalog) = view.catalog.loaded() else {
        return Vec::new();
    };
    let order: Vec<String> = view
        .general()
        .and_then(|general| general.get("providerOrder"))
        .and_then(JsonValue::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(JsonValue::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let mut enabled: Vec<&SystemProviderCatalogEntry> = catalog
        .iter()
        .filter(|entry| entry.enabled && entry.plugin_enabled)
        .collect();
    enabled.sort_by_key(|entry| {
        order
            .iter()
            .position(|id| id == &entry.id)
            .unwrap_or(usize::MAX)
    });
    let disabled = catalog
        .iter()
        .filter(|entry| !(entry.enabled && entry.plugin_enabled));
    enabled.into_iter().chain(disabled).collect()
}

pub fn default_provider_id(view: &SettingsView) -> Option<String> {
    let explicit = view
        .general()
        .and_then(|general| general.get(DEFAULT_PROVIDER_KEY))
        .and_then(JsonValue::as_str)
        .map(str::to_owned);
    let providers = ordered_providers(view);
    match explicit {
        Some(id) => Some(id),
        None => providers
            .first()
            .filter(|entry| entry.enabled && entry.plugin_enabled)
            .map(|entry| entry.id.clone()),
    }
}

fn completed_turn_display(view: &SettingsView, entry: &SystemProviderCatalogEntry) -> String {
    view.general()
        .and_then(|general| general.get(COMPLETED_TURN_KEY))
        .and_then(|map| map.get(&entry.id))
        .and_then(JsonValue::as_str)
        .map(str::to_owned)
        .or_else(|| {
            entry
                .info
                .as_ref()
                .map(|info| info.completed_turn_display.as_str().to_owned())
        })
        .unwrap_or_else(|| "flat".into())
}

fn provider_rows(view: &SettingsView) -> Vec<SettingsRow> {
    let mut rows = vec![SettingsRow::header("Providers")];
    if !load_rows(&view.catalog, "providers", &mut rows) {
        return rows;
    }
    let providers = ordered_providers(view);
    if providers.is_empty() {
        rows.push(SettingsRow::note(
            "No providers available. Install a provider plugin first.",
        ));
        return rows;
    }
    let default = default_provider_id(view);
    for entry in providers {
        let enabled = entry.enabled && entry.plugin_enabled;
        let mut value = if enabled {
            format!("finished turns: {}", completed_turn_display(view, entry))
        } else if !entry.plugin_enabled {
            format!("plugin {} is disabled", entry.plugin_id)
        } else {
            String::new()
        };
        if enabled && view.expanded_provider.as_deref() == Some(entry.id.as_str()) {
            value.push_str(" · models ▾");
        }
        let mut row = SettingsRow::item(
            RowTarget::Provider(entry.id.clone()),
            entry.display_name.clone(),
            value,
        );
        if !enabled {
            row = row.with_badge("Disabled", Tone::Muted);
        } else if !entry.available {
            row = row.with_badge("Unavailable", Tone::Warn);
        } else if default.as_deref() == Some(entry.id.as_str()) {
            row = row.with_badge("Default", Tone::Accent);
        }
        rows.push(pending_badge(view, &format!("provider:{}", entry.id), row));
        if view.expanded_provider.as_deref() == Some(entry.id.as_str()) {
            match view.models.get(&entry.id) {
                Some(Load::Loaded(models)) if models.is_empty() => {
                    rows.push(SettingsRow::note("No models reported").indented(4));
                }
                Some(Load::Loaded(models)) => {
                    for model in models {
                        let mut row = SettingsRow::info(
                            if model.display_name.is_empty() {
                                model.model.clone()
                            } else {
                                model.display_name.clone()
                            },
                            model.model.clone(),
                        )
                        .indented(4);
                        if model.is_default {
                            row = row.with_badge("default", Tone::Muted);
                        }
                        rows.push(row);
                    }
                }
                Some(Load::Failed(error)) => {
                    rows.push(
                        SettingsRow::error(format!("Couldn't load models: {error}")).indented(4),
                    );
                }
                _ => rows.push(SettingsRow::note("Loading models…").indented(4)),
            }
        }
    }
    rows.push(SettingsRow::header("Configuration"));
    if let Some(fast) = view
        .general()
        .and_then(|general| general_bool(general, FAST_TIER_KEY))
    {
        let row = SettingsRow::item(
            RowTarget::FastServiceTier,
            "Allow faster service tiers",
            if fast { "on" } else { "off" },
        );
        rows.push(pending_badge(view, "general:allowFastServiceTier", row));
        rows.push(SettingsRow::note(
            "Turn this off to use the default service tier for all new turns.",
        ));
    }
    rows
}

fn keyboard_rows(state: &AppState, view: &SettingsView) -> Vec<SettingsRow> {
    let filter = view.filter(SettingsSection::Keyboard);
    let mut rows = vec![SettingsRow::header("Keyboard shortcuts")];
    if let Some(config) = view.config.loaded()
        && !config.keybinding_overrides.is_empty()
    {
        rows.push(SettingsRow::note(format!(
            "{} keybinding override(s) from the server settings apply to commands with an app id.",
            config.keybinding_overrides.len()
        )));
    }
    let mut shown = 0;
    for command in Command::ALL.iter().copied() {
        let keys = state.keymap.labels_for(command);
        let description = command.description();
        let id = command.app_command_id().unwrap_or("");
        if !matches_filter(filter, &[&description, &keys, id]) {
            continue;
        }
        shown += 1;
        let mut row = SettingsRow::item(
            RowTarget::Binding(command),
            description,
            if keys.is_empty() { "—".into() } else { keys },
        );
        if !id.is_empty() {
            row = row.with_badge(id, Tone::Muted);
        }
        rows.push(row);
    }
    if shown == 0 {
        rows.push(SettingsRow::note("No shortcuts match the filter"));
    }
    rows.push(SettingsRow::note(
        "Rebind commands with an app id in the web app's keyboard settings.",
    ));
    rows
}

fn project_rows(state: &AppState, view: &SettingsView) -> Vec<SettingsRow> {
    let filter = view.filter(SettingsSection::Projects);
    let mut rows = vec![SettingsRow::header("Projects")];
    if state.snapshot.sidebar.data.is_none() {
        rows.push(SettingsRow::note("Loading projects…"));
        return rows;
    }
    let host_count = state
        .settings_hosts()
        .iter()
        .filter(|host| host.host_type == HostType::Persistent)
        .count();
    let projects = state.settings_projects();
    if projects.is_empty() {
        rows.push(SettingsRow::note("No projects yet."));
    }
    let mut shown = 0;
    for project in projects {
        let remote = project
            .project
            .git_remote_url
            .as_deref()
            .map(format_git_remote)
            .unwrap_or_else(|| "No git remote".into());
        if !matches_filter(filter, &[&project.project.name, &remote]) {
            continue;
        }
        shown += 1;
        let machines: HashSet<&str> = project
            .sources
            .iter()
            .map(|source| source.host_id.as_str())
            .collect();
        let machines_label = if host_count > 1 {
            format!("{} of {}", machines.len(), pluralize(host_count, "machine"))
        } else {
            pluralize(machines.len(), "machine")
        };
        let mut row = SettingsRow::item(
            RowTarget::Project(project.project.id.clone()),
            project.project.name.clone(),
            format!(
                "{remote} · {machines_label} · {}",
                pluralize(project.threads.len(), "thread")
            ),
        );
        if machines.is_empty() {
            row = row.with_badge("needs setup", Tone::Warn);
        }
        rows.push(row);
    }
    if shown == 0 && !filter.is_empty() {
        rows.push(SettingsRow::note("No projects match the filter"));
    }
    rows
}

fn project_detail_rows(
    state: &AppState,
    view: &SettingsView,
    project_id: &str,
) -> Vec<SettingsRow> {
    let Some(project) = state.settings_project(project_id) else {
        return vec![SettingsRow::note("This project no longer exists.")];
    };
    let mut rows = vec![SettingsRow::header(format!(
        "Project · {}",
        project.project.name
    ))];
    rows.push(pending_badge(
        view,
        &format!("project:{project_id}"),
        SettingsRow::item(
            RowTarget::ProjectRename(project_id.to_owned()),
            "Name",
            project.project.name.clone(),
        ),
    ));
    rows.push(SettingsRow::info(
        "Git remote",
        project
            .project
            .git_remote_url
            .clone()
            .unwrap_or_else(|| "None detected".into()),
    ));
    rows.push(SettingsRow::item(
        RowTarget::Copy(project_id.to_owned()),
        "Project ID",
        project_id,
    ));
    rows.push(SettingsRow::info(
        "Created",
        format!(
            "{} ago",
            compact_relative_time(project.project.created_at, state.now_ms)
        ),
    ));
    rows.push(SettingsRow::info(
        "Threads",
        project.threads.len().to_string(),
    ));
    rows.push(SettingsRow::header("Checkouts"));
    let hosts = state.settings_hosts();
    if hosts.is_empty() {
        rows.push(SettingsRow::note("No machines are paired yet."));
    }
    let primary = state.primary_host_id();
    for host in hosts {
        let source = project
            .sources
            .iter()
            .find(|source| source.host_id == host.id);
        let mut row = SettingsRow::item(
            RowTarget::Host(host.id.clone()),
            host.name.clone(),
            source.map_or_else(
                || "Not set up on this machine".to_owned(),
                |source| source.path.clone(),
            ),
        );
        if host.status != HostStatus::Connected {
            row = row.with_badge("offline", Tone::Muted);
        }
        if hosts.len() > 1 && primary == Some(host.id.as_str()) {
            row = row.with_badge("primary", Tone::Accent);
        }
        rows.push(row);
    }
    rows.push(SettingsRow::header("Thread defaults"));
    match view.project_defaults.get(project_id) {
        Some(Load::Loaded(Some(defaults))) => {
            rows.push(SettingsRow::info("Provider", defaults.provider_id.clone()));
            rows.push(SettingsRow::info("Model", defaults.model.clone()));
            rows.push(SettingsRow::info(
                "Permission mode",
                permission_mode_label(defaults.permission_mode),
            ));
            rows.push(SettingsRow::info(
                "Reasoning",
                defaults.reasoning_level.as_str(),
            ));
        }
        Some(Load::Loaded(None)) => rows.push(SettingsRow::note(
            "No threads have run here yet. The first thread's options become the defaults.",
        )),
        Some(Load::Failed(error)) => rows.push(SettingsRow::error(format!(
            "Couldn't load thread defaults: {error}"
        ))),
        _ => rows.push(SettingsRow::note("Loading…")),
    }
    rows.push(SettingsRow::header("Danger zone"));
    rows.push(
        SettingsRow::item(
            RowTarget::ProjectDelete(project_id.to_owned()),
            "Delete project",
            "removes the project and all its threads",
        )
        .with_badge("D", Tone::Bad),
    );
    rows
}

fn host_badges(state: &AppState, host: &Host, row: SettingsRow) -> SettingsRow {
    let mut row = row;
    if host.status == HostStatus::Connected {
        row = row.with_badge("online", Tone::Good);
    } else {
        row = row.with_badge("offline", Tone::Muted);
    }
    if state.primary_host_id() == Some(host.id.as_str()) {
        row = row.with_badge("server", Tone::Accent);
    }
    if host.lifecycle.phase != MachinePhase::Active && host.lifecycle.phase.is_known() {
        row = row.with_badge(host.lifecycle.phase.as_str(), Tone::Warn);
    }
    if host_needs_update(host) {
        row = row.with_badge("needs update", Tone::Warn);
    }
    row
}

pub fn host_needs_update(host: &Host) -> bool {
    host.status == HostStatus::Disconnected && host.last_rejected_protocol_version.is_some()
}

fn last_seen(host: &Host, now_ms: i64) -> String {
    if host.status == HostStatus::Connected {
        return "connected".into();
    }
    match host.last_seen_at {
        Some(at) => format!("last seen {} ago", compact_relative_time(at, now_ms)),
        None => "never connected".into(),
    }
}

fn machine_rows(state: &AppState, view: &SettingsView) -> Vec<SettingsRow> {
    let mut rows = vec![SettingsRow::header("Server")];
    match &view.version {
        Load::Loaded(version) => {
            let mut row = SettingsRow::info("bb version", version.current_version.clone());
            if version.update_available {
                row = row.with_badge(
                    format!(
                        "update to {}",
                        version.latest_version.as_deref().unwrap_or("?")
                    ),
                    Tone::Warn,
                );
            }
            rows.push(row);
        }
        Load::Failed(error) => rows.push(SettingsRow::info(
            "bb version",
            format!("unavailable: {error}"),
        )),
        _ => rows.push(SettingsRow::info("bb version", "…")),
    }
    rows.push(SettingsRow::header("Machines"));
    if state.snapshot.hosts.data.is_none() {
        rows.push(match &state.snapshot.hosts.error {
            Some(error) => SettingsRow::error(format!("Couldn't load machines: {error}")),
            None => SettingsRow::note("Loading machines…"),
        });
        return rows;
    }
    let hosts = state.settings_hosts();
    if hosts.is_empty() {
        rows.push(SettingsRow::note("No machines are paired yet."));
    }
    for host in hosts {
        let kind = match host.host_type {
            HostType::Ephemeral => "ephemeral · ",
            _ => "",
        };
        let row = SettingsRow::item(
            RowTarget::Host(host.id.clone()),
            host.name.clone(),
            format!("{kind}{}", last_seen(host, state.now_ms)),
        );
        rows.push(host_badges(state, host, row));
    }
    rows
}

fn machine_detail_rows(state: &AppState, view: &SettingsView, host_id: &str) -> Vec<SettingsRow> {
    let Some(host) = state.settings_host(host_id) else {
        return vec![SettingsRow::note("Machine is no longer paired.")];
    };
    let mut rows = vec![host_badges(
        state,
        host,
        SettingsRow::header(format!("Machine · {}", host.name)),
    )];
    let key = format!("host:{host_id}");
    rows.push(pending_badge(
        view,
        &key,
        SettingsRow::item(
            RowTarget::HostRename(host_id.to_owned()),
            "Name",
            host.name.clone(),
        ),
    ));
    rows.push(SettingsRow::item(
        RowTarget::Copy(host_id.to_owned()),
        "Machine ID",
        host_id,
    ));
    rows.push(SettingsRow::info("Status", last_seen(host, state.now_ms)));
    rows.push(SettingsRow::info("Type", host.host_type.as_str()));
    if let Some(provider) = &host.machine_provider_id {
        rows.push(SettingsRow::info("Machine provider", provider.clone()));
    }
    if let Some(message) = host.lifecycle.message.as_deref().filter(|m| !m.is_empty()) {
        rows.push(SettingsRow::info(
            "Lifecycle",
            format!("{} · {message}", host.lifecycle.phase),
        ));
    }
    rows.push(pending_badge(
        view,
        &key,
        SettingsRow::item(
            RowTarget::HostPermission(host_id.to_owned()),
            "Permission limit",
            permission_mode_label(host.max_permission_mode),
        ),
    ));
    rows.push(SettingsRow::note(
        "The most access any thread on this machine can get.",
    ));
    let update = if host_needs_update(host) {
        format!(
            "Needs update · daemon protocol {}",
            host.last_rejected_protocol_version.unwrap_or_default()
        )
    } else {
        "Up to date".into()
    };
    rows.push(SettingsRow::info("Updates", update));
    rows.push(SettingsRow::info(
        "Created",
        format!(
            "{} ago",
            compact_relative_time(host.created_at, state.now_ms)
        ),
    ));
    let projects: Vec<String> = state
        .settings_projects()
        .into_iter()
        .filter(|project| {
            project
                .sources
                .iter()
                .any(|source| source.host_id == host_id)
        })
        .map(|project| project.project.name.clone())
        .collect();
    rows.push(SettingsRow::info(
        "Projects",
        if projects.is_empty() {
            "None".into()
        } else {
            projects.join(" · ")
        },
    ));
    rows.push(SettingsRow::header("Provider CLIs"));
    if host.status != HostStatus::Connected {
        rows.push(SettingsRow::note("Unavailable while offline"));
        return rows;
    }
    match view.host_clis.get(host_id) {
        Some(Load::Loaded(status)) => {
            let installed: Vec<_> = status.iter().filter(|(_, entry)| entry.installed).collect();
            if installed.is_empty() {
                rows.push(SettingsRow::note("None installed"));
            }
            for (_, entry) in installed {
                let mut row = SettingsRow::info(
                    entry.display_name.clone(),
                    entry
                        .current_version
                        .clone()
                        .unwrap_or_else(|| "unknown version".into()),
                );
                if entry.version_unsupported {
                    row = row.with_badge("unsupported version", Tone::Bad);
                } else if entry.needs_update {
                    row = row.with_badge(
                        format!(
                            "update to {}",
                            entry.latest_version.as_deref().unwrap_or("latest")
                        ),
                        Tone::Warn,
                    );
                }
                rows.push(row);
            }
        }
        Some(Load::Failed(error)) => {
            rows.push(SettingsRow::error(format!("Status unavailable: {error}")))
        }
        _ => rows.push(SettingsRow::note("Checking…")),
    }
    rows
}

fn plugin_status_badge(plugin: &InstalledPlugin) -> Option<Badge> {
    match plugin.status {
        _ if !plugin.enabled => Some(badge("disabled", Tone::Muted)),
        PluginRuntimeStatus::Running => None,
        PluginRuntimeStatus::Starting => Some(badge("starting", Tone::Muted)),
        PluginRuntimeStatus::NeedsConfiguration => Some(badge("needs configuration", Tone::Warn)),
        PluginRuntimeStatus::Degraded => Some(badge("degraded", Tone::Warn)),
        PluginRuntimeStatus::Error
        | PluginRuntimeStatus::Incompatible
        | PluginRuntimeStatus::Missing => Some(badge(plugin.status.as_str(), Tone::Bad)),
        PluginRuntimeStatus::Disabled => Some(badge("disabled", Tone::Muted)),
        PluginRuntimeStatus::Unknown => None,
    }
}

fn plugin_name(plugin: &InstalledPlugin) -> String {
    plugin
        .name
        .clone()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| plugin.id.clone())
}

fn plugin_rows(view: &SettingsView) -> Vec<SettingsRow> {
    let filter = view.filter(SettingsSection::Plugins);
    let mut rows = vec![SettingsRow::header("Installed plugins")];
    if !load_rows(&view.plugins, "plugins", &mut rows) {
        return rows;
    }
    let plugins = view.plugins.loaded().map(Vec::as_slice).unwrap_or(&[]);
    if plugins.is_empty() {
        rows.push(SettingsRow::note("No plugins installed."));
    }
    let mut shown = 0;
    for plugin in plugins {
        let name = plugin_name(plugin);
        let description = plugin.description.clone().unwrap_or_default();
        if !matches_filter(filter, &[&name, &plugin.id, &description]) {
            continue;
        }
        shown += 1;
        let mut value = format!("{} · v{}", plugin.id, plugin.version);
        if plugin.has_settings {
            value.push_str(" · settings");
        }
        let mut row = SettingsRow::item(RowTarget::Plugin(plugin.id.clone()), name, value);
        if let Some(status) = plugin_status_badge(plugin) {
            row.badges.push(status);
        }
        rows.push(pending_badge(view, &format!("plugin:{}", plugin.id), row));
    }
    if shown == 0 && !filter.is_empty() {
        rows.push(SettingsRow::note("No plugins match the filter"));
    }
    rows
}

fn plugin_detail_rows(state: &AppState, view: &SettingsView, plugin_id: &str) -> Vec<SettingsRow> {
    let Some(plugin) = view
        .plugins
        .loaded()
        .and_then(|plugins| plugins.iter().find(|plugin| plugin.id == plugin_id))
    else {
        return vec![SettingsRow::note("This plugin is no longer installed.")];
    };
    let mut header = SettingsRow::header(format!("Plugin · {}", plugin_name(plugin)));
    if let Some(status) = plugin_status_badge(plugin) {
        header.badges.push(status);
    }
    let mut rows = vec![header];
    rows.push(pending_badge(
        view,
        &format!("plugin:{plugin_id}"),
        SettingsRow::item(
            RowTarget::PluginEnabled(plugin_id.to_owned()),
            "Enabled",
            if plugin.enabled { "on" } else { "off" },
        ),
    ));
    rows.push(SettingsRow::item(
        RowTarget::Copy(plugin_id.to_owned()),
        "Plugin ID",
        plugin_id,
    ));
    rows.push(SettingsRow::info("Version", plugin.version.clone()));
    rows.push(SettingsRow::info(
        "Source",
        if plugin.source_display.is_empty() {
            plugin.source.clone()
        } else {
            plugin.source_display.clone()
        },
    ));
    let mut status = plugin.status.as_str().to_owned();
    if let Some(detail) = plugin.status_detail.as_deref().filter(|d| !d.is_empty()) {
        status.push_str(" · ");
        status.push_str(detail);
    }
    rows.push(SettingsRow::info("Status", status));
    if let Some(cli) = &plugin.cli_command {
        rows.push(SettingsRow::info(
            "CLI command",
            format!("bb {} — {}", cli.name, cli.summary),
        ));
    }
    if !plugin.provider_ids.is_empty() {
        rows.push(SettingsRow::info(
            "Providers",
            plugin.provider_ids.join(", "),
        ));
    }
    if let Some(description) = plugin.description.as_deref().filter(|d| !d.is_empty()) {
        rows.push(SettingsRow::note(description));
    }
    if !plugin.has_settings {
        return rows;
    }
    rows.push(SettingsRow::header("Settings"));
    match view.plugin_settings.get(plugin_id) {
        Some(Load::Loaded(settings)) => {
            if settings.schema.is_empty() {
                rows.push(SettingsRow::note("This plugin has no settings."));
            }
            for (key, descriptor) in settings.schema.iter() {
                let kind = FieldKind::of(descriptor);
                let value = settings.values.get(key);
                let label = descriptor.label().filter(|l| !l.is_empty()).unwrap_or(key);
                let display = field_display(&kind, value, |id| state.settings_project_name(id));
                let mut row = SettingsRow::item(
                    RowTarget::PluginField {
                        plugin_id: plugin_id.to_owned(),
                        key: key.to_owned(),
                    },
                    label,
                    display,
                );
                match kind {
                    FieldKind::Secret => row = row.with_badge("secret", Tone::Muted),
                    FieldKind::Unsupported => row = row.with_badge("web only", Tone::Muted),
                    _ => {}
                }
                rows.push(pending_badge(
                    view,
                    &format!("plugin-setting:{plugin_id}:{key}"),
                    row,
                ));
                if let Some(error) = view.field_errors.get(&field_error_key(plugin_id, key)) {
                    rows.push(SettingsRow::error(error.clone()));
                }
                if let Some(description) = descriptor_description(descriptor) {
                    rows.push(SettingsRow::note(description));
                }
            }
        }
        Some(Load::Failed(error)) => rows.push(SettingsRow::error(format!(
            "Couldn't load settings: {error}"
        ))),
        _ => rows.push(SettingsRow::note("Loading settings…")),
    }
    rows
}

pub fn field_error_key(plugin_id: &str, key: &str) -> String {
    format!("{plugin_id}:{key}")
}

fn archived_rows(state: &AppState, view: &SettingsView) -> Vec<SettingsRow> {
    let filter = view.filter(SettingsSection::Archived);
    let archived = &view.archived;
    let project_label = archived
        .query
        .project_id
        .as_deref()
        .map(|id| {
            state
                .settings_project_name(id)
                .unwrap_or_else(|| id.to_owned())
        })
        .unwrap_or_else(|| "All projects".into());
    let mut rows = vec![SettingsRow::header("Archived threads")];
    rows.push(SettingsRow::note(format!(
        "{} · {} · t kind · p project · / search",
        archived.query.kind.label(),
        project_label
    )));
    if let Some(error) = &archived.error {
        rows.push(SettingsRow::error(format!(
            "Couldn't load archived threads: {error}"
        )));
    }
    if !archived.loaded {
        rows.push(SettingsRow::note("Loading archived threads…"));
        return rows;
    }
    let threads: Vec<&ThreadListEntry> = archived
        .threads
        .iter()
        .filter(|entry| matches_filter(filter, &[&thread_title(entry)]))
        .collect();
    if threads.is_empty() {
        rows.push(SettingsRow::note(if filter.trim().is_empty() {
            "No archived threads match these filters."
        } else {
            "No archived threads match your search."
        }));
    }
    let mut groups: Vec<(&str, Vec<&ThreadListEntry>)> = Vec::new();
    for entry in threads {
        let project = entry.thread.project_id.as_str();
        match groups.iter_mut().find(|(id, _)| *id == project) {
            Some((_, list)) => list.push(entry),
            None => groups.push((project, vec![entry])),
        }
    }
    for (project_id, entries) in groups {
        let name = state
            .settings_project_name(project_id)
            .unwrap_or_else(|| "Unknown project".into());
        let mut header = SettingsRow::header(name);
        header.value = pluralize(entries.len(), "thread");
        rows.push(header);
        for entry in entries {
            let archived_at = entry
                .thread
                .archived_at
                .map(|at| format!("archived {} ago", compact_relative_time(at, state.now_ms)))
                .unwrap_or_default();
            let mut row = SettingsRow::item(
                RowTarget::Archived(entry.thread.id.clone()),
                thread_title(entry),
                archived_at,
            )
            .indented(2);
            if entry.thread.parent_thread_id.is_some() {
                row = row.with_badge("child", Tone::Muted);
            }
            rows.push(pending_badge(
                view,
                &format!("thread:{}", entry.thread.id),
                row,
            ));
        }
    }
    if archived.has_more {
        rows.push(SettingsRow::item(
            RowTarget::ArchivedMore,
            if archived.loading {
                "Loading…"
            } else {
                "Load more"
            },
            "",
        ));
    }
    rows
}

fn first_selectable(rows: &[SettingsRow]) -> Option<usize> {
    rows.iter().position(SettingsRow::selectable)
}

pub fn clamp_cursor(rows: &[SettingsRow], cursor: usize) -> Option<usize> {
    if rows.get(cursor).is_some_and(SettingsRow::selectable) {
        return Some(cursor);
    }
    rows.iter()
        .enumerate()
        .skip(cursor.min(rows.len()))
        .find(|(_, row)| row.selectable())
        .map(|(index, _)| index)
        .or_else(|| {
            rows.iter()
                .enumerate()
                .take(cursor.min(rows.len()))
                .rev()
                .find(|(_, row)| row.selectable())
                .map(|(index, _)| index)
        })
}

fn step_cursor(rows: &[SettingsRow], cursor: usize, delta: isize) -> usize {
    let Some(current) = clamp_cursor(rows, cursor) else {
        return 0;
    };
    let selectable: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.selectable())
        .map(|(index, _)| index)
        .collect();
    let position = selectable
        .iter()
        .position(|index| *index == current)
        .unwrap_or(0);
    let next = position
        .saturating_add_signed(delta)
        .min(selectable.len().saturating_sub(1));
    selectable[next]
}

impl AppState {
    pub fn open_settings(&mut self, section: Option<SettingsSection>) -> Vec<Effect> {
        if let Some(view) = self.settings.as_mut() {
            if let Some(section) = section {
                view.section = section;
                view.detail = None;
                view.focus = SettingsFocus::Content;
            }
            return self.settings_section_effects();
        }
        self.settings = Some(SettingsView::new(
            section.unwrap_or(SettingsSection::General),
        ));
        let mut effects: Vec<Effect> = SettingsView::initial_requests()
            .into_iter()
            .map(Effect::Settings)
            .collect();
        effects.extend(self.settings_section_effects());
        effects
    }

    pub fn close_settings(&mut self) {
        self.settings = None;
    }

    fn settings_section_effects(&mut self) -> Vec<Effect> {
        let Some(view) = self.settings.as_mut() else {
            return Vec::new();
        };
        match view.section {
            SettingsSection::Archived if !view.archived.loaded && !view.archived.loading => {
                view.archived.loading = true;
                vec![Effect::Settings(SettingsRequest::Archived(
                    view.archived.query.clone(),
                ))]
            }
            SettingsSection::Machines => vec![Effect::Refresh(bb_store::ResourceKey::Hosts)],
            _ => Vec::new(),
        }
    }

    fn reload_settings_section(&mut self) -> Vec<Effect> {
        let Some(view) = self.settings.as_mut() else {
            return Vec::new();
        };
        let mut requests = Vec::new();
        match view.detail.clone() {
            Some(Detail::Project(project_id)) => {
                view.project_defaults
                    .insert(project_id.clone(), Load::Loading);
                requests.push(SettingsRequest::ProjectDefaults { project_id });
            }
            Some(Detail::Machine(host_id)) => {
                view.host_clis.insert(host_id.clone(), Load::Loading);
                requests.push(SettingsRequest::HostClis { host_id });
            }
            Some(Detail::Plugin(plugin_id)) => {
                requests.push(SettingsRequest::Plugins);
                view.plugin_settings
                    .insert(plugin_id.clone(), Load::Loading);
                requests.push(SettingsRequest::PluginSettings { plugin_id });
            }
            None => match view.section {
                SettingsSection::General => {
                    requests.push(SettingsRequest::Config);
                    requests.push(SettingsRequest::Version);
                }
                SettingsSection::Providers => {
                    requests.push(SettingsRequest::Catalog);
                    requests.push(SettingsRequest::Config);
                    view.models.clear();
                    if let Some(provider_id) = view.expanded_provider.clone() {
                        view.models.insert(provider_id.clone(), Load::Loading);
                        requests.push(SettingsRequest::Models { provider_id });
                    }
                }
                SettingsSection::Keyboard => requests.push(SettingsRequest::Config),
                SettingsSection::Plugins => requests.push(SettingsRequest::Plugins),
                SettingsSection::Archived => {
                    view.archived.query.offset = 0;
                    view.archived.loading = true;
                    requests.push(SettingsRequest::Archived(view.archived.query.clone()));
                }
                SettingsSection::Machines => requests.push(SettingsRequest::Version),
                SettingsSection::Projects => {}
            },
        }
        let mut effects: Vec<Effect> = requests.into_iter().map(Effect::Settings).collect();
        effects.push(match view.section {
            SettingsSection::Machines => Effect::Refresh(bb_store::ResourceKey::Hosts),
            _ => Effect::RefreshSidebar,
        });
        self.toast("Reloading", ToastLevel::Info);
        effects
    }

    pub fn settings_loaded(&mut self, loaded: SettingsLoaded) -> Vec<Effect> {
        let Some(view) = self.settings.as_mut() else {
            return Vec::new();
        };
        match loaded {
            SettingsLoaded::Config(result) => view.config = Load::from_result(result.map(|c| *c)),
            SettingsLoaded::Version(result) => view.version = Load::from_result(result.map(|v| *v)),
            SettingsLoaded::Catalog(result) => {
                if result.is_ok() || view.catalog.loaded().is_none() {
                    view.catalog = Load::from_result(result);
                }
            }
            SettingsLoaded::Models {
                provider_id,
                result,
            } => {
                view.models.insert(provider_id, Load::from_result(result));
            }
            SettingsLoaded::HostClis { host_id, result } => {
                view.host_clis.insert(host_id, Load::from_result(result));
            }
            SettingsLoaded::ProjectDefaults { project_id, result } => {
                view.project_defaults
                    .insert(project_id, Load::from_result(result));
            }
            SettingsLoaded::Plugins(result) => {
                if result.is_ok() || view.plugins.loaded().is_none() {
                    view.plugins = Load::from_result(result);
                }
            }
            SettingsLoaded::PluginSettings { plugin_id, result } => {
                view.plugin_settings
                    .insert(plugin_id, Load::from_result(result));
            }
            SettingsLoaded::Archived { query, result } => {
                if !query.same_filters(&view.archived.query) {
                    return Vec::new();
                }
                let archived = &mut view.archived;
                archived.loading = false;
                match result {
                    Ok(threads) => {
                        archived.has_more = threads.len() >= ARCHIVED_PAGE_SIZE as usize;
                        if query.offset == 0 {
                            archived.threads.clear();
                        }
                        for entry in threads {
                            if entry.thread.archived_at.is_some()
                                && !archived
                                    .threads
                                    .iter()
                                    .any(|existing| existing.thread.id == entry.thread.id)
                            {
                                archived.threads.push(entry);
                            }
                        }
                        archived.query.offset = query.offset;
                        archived.loaded = true;
                        archived.error = None;
                    }
                    Err(error) => {
                        archived.loaded = true;
                        archived.error = Some(error);
                    }
                }
            }
            SettingsLoaded::ChildSummary { thread_id, result } => {
                if let Some(confirm) = view.confirm.as_mut()
                    && matches!(&confirm.action, ConfirmAction::DeleteThread { thread_id: id, .. } if *id == thread_id)
                {
                    confirm.children = Load::from_result(result);
                }
            }
            SettingsLoaded::Mutated { mutation, result } => {
                return self.settings_mutation_finished(mutation, result);
            }
        }
        Vec::new()
    }

    fn settings_mutation_finished(
        &mut self,
        mutation: SettingsMutation,
        result: Result<SettingsReply, String>,
    ) -> Vec<Effect> {
        let Some(view) = self.settings.as_mut() else {
            return Vec::new();
        };
        view.pending.remove(&mutation.key());
        let reply = match result {
            Ok(reply) => reply,
            Err(error) => {
                let mut effects = Vec::new();
                if let SettingsMutation::UpdatePluginSetting { plugin_id, key, .. } = &mutation {
                    view.field_errors
                        .insert(field_error_key(plugin_id, key), error.clone());
                }
                if let SettingsMutation::UpdateGeneral { .. } = &mutation {
                    effects.push(Effect::Settings(SettingsRequest::Config));
                }
                self.toast(
                    format!("Failed to {}: {error}", mutation.failure_verb()),
                    ToastLevel::Error,
                );
                return effects;
            }
        };
        let mut effects = Vec::new();
        match (&mutation, reply) {
            (SettingsMutation::UpdateGeneral { settings, .. }, reply) => {
                if let Load::Loaded(config) = &mut view.config {
                    config.general_settings = match reply {
                        SettingsReply::General(value) if value.is_object() => value,
                        _ => JsonValue::Object(settings.clone()),
                    };
                }
            }
            (SettingsMutation::SetProviderEnabled { .. }, SettingsReply::Catalog(catalog)) => {
                view.catalog = Load::Loaded(catalog);
                effects.push(Effect::Settings(SettingsRequest::Config));
            }
            (SettingsMutation::SetProviderEnabled { .. }, _) => {
                effects.push(Effect::Settings(SettingsRequest::Catalog));
            }
            (SettingsMutation::RenameProject { .. }, _) => effects.push(Effect::RefreshSidebar),
            (SettingsMutation::DeleteProject { project_id }, _) => {
                if view.detail == Some(Detail::Project(project_id.clone())) {
                    view.detail = None;
                }
                effects.push(Effect::RefreshSidebar);
            }
            (
                SettingsMutation::RenameHost { .. }
                | SettingsMutation::SetHostPermissionCeiling { .. },
                _,
            ) => effects.push(Effect::Refresh(bb_store::ResourceKey::Hosts)),
            (
                SettingsMutation::UnarchiveThread { thread_id }
                | SettingsMutation::DeleteThread { thread_id, .. },
                _,
            ) => {
                view.archived
                    .threads
                    .retain(|entry| &entry.thread.id != thread_id);
                effects.push(Effect::RefreshSidebar);
            }
            (SettingsMutation::SetPluginEnabled { .. }, _) => {
                effects.push(Effect::Settings(SettingsRequest::Plugins));
                effects.push(Effect::Settings(SettingsRequest::Catalog));
            }
            (
                SettingsMutation::UpdatePluginSetting { plugin_id, key, .. },
                SettingsReply::PluginSettings(settings),
            ) => {
                view.field_errors.remove(&field_error_key(plugin_id, key));
                view.plugin_settings
                    .insert(plugin_id.clone(), Load::Loaded(*settings));
            }
            (SettingsMutation::UpdatePluginSetting { plugin_id, key, .. }, _) => {
                view.field_errors.remove(&field_error_key(plugin_id, key));
                effects.push(Effect::Settings(SettingsRequest::PluginSettings {
                    plugin_id: plugin_id.clone(),
                }));
            }
        }
        if let Some(message) = mutation.success_message() {
            self.toast(message, ToastLevel::Info);
        }
        effects
    }

    fn settings_mutate(&mut self, mutation: SettingsMutation) -> Vec<Effect> {
        let Some(view) = self.settings.as_mut() else {
            return Vec::new();
        };
        let key = mutation.key();
        if view.pending.contains(&key) {
            self.toast("Still saving the previous change", ToastLevel::Info);
            return Vec::new();
        }
        view.pending.insert(key);
        vec![Effect::Settings(SettingsRequest::Mutate(mutation))]
    }

    pub fn settings_refused(&mut self, mutation: &SettingsMutation) {
        if let Some(view) = self.settings.as_mut() {
            view.pending.remove(&mutation.key());
        }
        self.toast(
            format!("Read-only mode: cannot {}", mutation.failure_verb()),
            ToastLevel::Error,
        );
    }

    fn general_update(&mut self, key: &'static str, value: JsonValue) -> Vec<Effect> {
        let Some(general) = self.settings.as_ref().and_then(SettingsView::general) else {
            self.toast("Settings have not loaded yet", ToastLevel::Info);
            return Vec::new();
        };
        match merge_general(general, key, value) {
            Some(settings) => {
                self.settings_mutate(SettingsMutation::UpdateGeneral { key, settings })
            }
            None => Vec::new(),
        }
    }

    fn toggle_general(&mut self, key: &'static str) -> Vec<Effect> {
        let current = self
            .settings
            .as_ref()
            .and_then(SettingsView::general)
            .and_then(|general| general_bool(general, key));
        match current {
            Some(value) => self.general_update(key, JsonValue::Bool(!value)),
            None => Vec::new(),
        }
    }

    fn open_detail(&mut self, detail: Detail) -> Vec<Effect> {
        let Some(view) = self.settings.as_mut() else {
            return Vec::new();
        };
        let mut effects = Vec::new();
        match &detail {
            Detail::Project(project_id) => {
                view.project_defaults
                    .insert(project_id.clone(), Load::Loading);
                effects.push(Effect::Settings(SettingsRequest::ProjectDefaults {
                    project_id: project_id.clone(),
                }));
            }
            Detail::Machine(host_id) => {
                let connected = self
                    .snapshot
                    .hosts
                    .data
                    .as_deref()
                    .and_then(|hosts| hosts.iter().find(|host| &host.id == host_id))
                    .is_some_and(|host| host.status == HostStatus::Connected);
                if connected {
                    view.host_clis.insert(host_id.clone(), Load::Loading);
                    effects.push(Effect::Settings(SettingsRequest::HostClis {
                        host_id: host_id.clone(),
                    }));
                }
            }
            Detail::Plugin(plugin_id) => {
                let has_settings = view
                    .plugins
                    .loaded()
                    .and_then(|plugins| plugins.iter().find(|plugin| &plugin.id == plugin_id))
                    .is_some_and(|plugin| plugin.has_settings);
                if has_settings {
                    view.plugin_settings
                        .insert(plugin_id.clone(), Load::Loading);
                    effects.push(Effect::Settings(SettingsRequest::PluginSettings {
                        plugin_id: plugin_id.clone(),
                    }));
                }
            }
        }
        view.detail = Some(detail);
        view.detail_cursor = 0;
        view.focus = SettingsFocus::Content;
        effects
    }

    fn toggle_provider_models(&mut self, provider_id: String) -> Vec<Effect> {
        let Some(view) = self.settings.as_mut() else {
            return Vec::new();
        };
        if view.expanded_provider.as_deref() == Some(provider_id.as_str()) {
            view.expanded_provider = None;
            return Vec::new();
        }
        view.expanded_provider = Some(provider_id.clone());
        if view
            .models
            .get(&provider_id)
            .is_some_and(|load| load.loaded().is_some())
        {
            return Vec::new();
        }
        view.models.insert(provider_id.clone(), Load::Loading);
        vec![Effect::Settings(SettingsRequest::Models { provider_id })]
    }

    fn provider_entry(&self, provider_id: &str) -> Option<SystemProviderCatalogEntry> {
        self.settings
            .as_ref()?
            .catalog
            .loaded()?
            .iter()
            .find(|entry| entry.id == provider_id)
            .cloned()
    }

    fn toggle_provider(&mut self, provider_id: &str) -> Vec<Effect> {
        let Some(entry) = self.provider_entry(provider_id) else {
            return Vec::new();
        };
        let enabled = !(entry.enabled && entry.plugin_enabled);
        self.settings_mutate(SettingsMutation::SetProviderEnabled {
            provider_id: provider_id.to_owned(),
            enabled,
        })
    }

    fn make_default_provider(&mut self, provider_id: &str) -> Vec<Effect> {
        let Some(entry) = self.provider_entry(provider_id) else {
            return Vec::new();
        };
        if !(entry.enabled && entry.plugin_enabled && entry.available) {
            self.toast(
                "Only an enabled, available provider can be the default",
                ToastLevel::Info,
            );
            return Vec::new();
        }
        self.general_update(
            DEFAULT_PROVIDER_KEY,
            JsonValue::String(provider_id.to_owned()),
        )
    }

    fn toggle_completed_turns(&mut self, provider_id: &str) -> Vec<Effect> {
        let Some(entry) = self.provider_entry(provider_id) else {
            return Vec::new();
        };
        let Some(view) = self.settings.as_ref() else {
            return Vec::new();
        };
        let current = completed_turn_display(view, &entry);
        let next = if current == "collapse" {
            "flat"
        } else {
            "collapse"
        };
        let provider_default = entry
            .info
            .as_ref()
            .map(|info| info.completed_turn_display.as_str().to_owned());
        let mut overrides = view
            .general()
            .and_then(|general| general.get(COMPLETED_TURN_KEY))
            .and_then(JsonValue::as_object)
            .cloned()
            .unwrap_or_default();
        overrides.remove(provider_id);
        if provider_default.as_deref() != Some(next) {
            overrides.insert(provider_id.to_owned(), JsonValue::String(next.into()));
        }
        self.general_update(COMPLETED_TURN_KEY, JsonValue::Object(overrides))
    }

    fn start_edit(&mut self, target: EditTarget) {
        let Some(view) = self.settings.as_ref() else {
            return;
        };
        let edit = match &target {
            EditTarget::General(key) => {
                let value = view
                    .general()
                    .and_then(|general| general.get(*key))
                    .and_then(JsonValue::as_str)
                    .unwrap_or("")
                    .to_owned();
                let (label, hint) = if *key == BRANCH_PREFIX_KEY {
                    ("New branch prefix", "Empty means no prefix")
                } else {
                    ("Machine server URL", "Empty uses the server URL")
                };
                FieldEdit {
                    target: target.clone(),
                    label: label.into(),
                    input: LineInput::new(value),
                    secret: false,
                    hint: Some(hint.into()),
                    error: None,
                }
            }
            EditTarget::ProjectName(project_id) => FieldEdit {
                target: target.clone(),
                label: "Project name".into(),
                input: LineInput::new(self.settings_project_name(project_id).unwrap_or_default()),
                secret: false,
                hint: None,
                error: None,
            },
            EditTarget::HostName(host_id) => FieldEdit {
                target: target.clone(),
                label: "Machine name".into(),
                input: LineInput::new(
                    self.settings_host(host_id)
                        .map(|host| host.name.clone())
                        .unwrap_or_default(),
                ),
                secret: false,
                hint: None,
                error: None,
            },
            EditTarget::PluginField { plugin_id, key } => {
                let Some(settings) = view.plugin_settings.get(plugin_id).and_then(Load::loaded)
                else {
                    return;
                };
                let Some(descriptor) = settings.schema.get(key) else {
                    return;
                };
                let kind = FieldKind::of(descriptor);
                let hint = match &kind {
                    FieldKind::Secret => {
                        Some("Enter a new value · empty keeps the current one".into())
                    }
                    FieldKind::Number => Some("Empty clears the value".into()),
                    FieldKind::Text { multiline: true } => {
                        Some("\\n is a line break, \\\\ a backslash".into())
                    }
                    _ => None,
                };
                FieldEdit {
                    target: target.clone(),
                    label: descriptor.label().unwrap_or(key).to_owned(),
                    input: LineInput::new(initial_edit_text(&kind, settings.values.get(key))),
                    secret: kind == FieldKind::Secret,
                    hint,
                    error: None,
                }
            }
        };
        if let Some(view) = self.settings.as_mut() {
            view.edit = Some(edit);
        }
    }

    fn submit_edit(&mut self, edit: FieldEdit) -> Result<Vec<Effect>, String> {
        let text = edit.input.text().to_owned();
        match edit.target {
            EditTarget::General(key) if key == BRANCH_PREFIX_KEY => {
                validate_branch_prefix(&text)?;
                Ok(self.general_update(key, JsonValue::String(text)))
            }
            EditTarget::General(key) => {
                let value = validate_server_url(&text)?;
                Ok(self.general_update(key, value.map_or(JsonValue::Null, JsonValue::String)))
            }
            EditTarget::ProjectName(project_id) => {
                let name = text.trim().to_owned();
                if name.is_empty() {
                    return Err("Enter a name".into());
                }
                if self.settings_project_name(&project_id).as_deref() == Some(name.as_str()) {
                    return Ok(Vec::new());
                }
                Ok(self.settings_mutate(SettingsMutation::RenameProject { project_id, name }))
            }
            EditTarget::HostName(host_id) => {
                let name = text.trim().to_owned();
                if name.is_empty() {
                    return Err("Enter a name".into());
                }
                if name.chars().count() > 100 {
                    return Err("Use at most 100 characters".into());
                }
                if self.settings_host(&host_id).map(|host| host.name.as_str())
                    == Some(name.as_str())
                {
                    return Ok(Vec::new());
                }
                Ok(self.settings_mutate(SettingsMutation::RenameHost { host_id, name }))
            }
            EditTarget::PluginField { plugin_id, key } => {
                Ok(self.save_plugin_field(plugin_id, key, &text)?)
            }
        }
    }

    fn save_plugin_field(
        &mut self,
        plugin_id: String,
        key: String,
        text: &str,
    ) -> Result<Vec<Effect>, String> {
        let Some(settings) = self
            .settings
            .as_ref()
            .and_then(|view| view.plugin_settings.get(&plugin_id))
            .and_then(Load::loaded)
        else {
            return Err("Settings have not loaded yet".into());
        };
        let Some(descriptor) = settings.schema.get(&key) else {
            return Err("This setting no longer exists".into());
        };
        let kind = FieldKind::of(descriptor);
        let Some(value) = parse_field_input(&kind, text)? else {
            return Ok(Vec::new());
        };
        if unchanged(&kind, settings.values.get(&key), &value) {
            return Ok(Vec::new());
        }
        let secret = kind == FieldKind::Secret;
        if let Some(view) = self.settings.as_mut() {
            view.field_errors.remove(&field_error_key(&plugin_id, &key));
        }
        Ok(self.settings_mutate(SettingsMutation::UpdatePluginSetting {
            plugin_id,
            key,
            value,
            secret,
        }))
    }

    fn activate_plugin_field(&mut self, plugin_id: String, key: String) -> Vec<Effect> {
        let Some(settings) = self
            .settings
            .as_ref()
            .and_then(|view| view.plugin_settings.get(&plugin_id))
            .and_then(Load::loaded)
            .cloned()
        else {
            return Vec::new();
        };
        let Some(descriptor) = settings.schema.get(&key) else {
            return Vec::new();
        };
        let kind = FieldKind::of(descriptor);
        let stored = settings.values.get(&key);
        match kind {
            FieldKind::Boolean => {
                let next = !stored.and_then(JsonValue::as_bool).unwrap_or(false);
                self.save_plugin_field(plugin_id, key, if next { "true" } else { "false" })
                    .unwrap_or_default()
            }
            FieldKind::Select(options) => {
                let current = stored.and_then(JsonValue::as_str).unwrap_or("");
                let selected = options
                    .iter()
                    .position(|option| option == current)
                    .unwrap_or(0);
                let title = descriptor.label().unwrap_or(&key).to_owned();
                if let Some(view) = self.settings.as_mut() {
                    view.choice = Some(Choice {
                        title,
                        target: ChoiceTarget::PluginField { plugin_id, key },
                        options: options
                            .iter()
                            .map(|option| (option.clone(), option.clone()))
                            .collect(),
                        selected,
                    });
                }
                Vec::new()
            }
            FieldKind::Project => {
                let mut options = Vec::new();
                if let Some(sidebar) = self.snapshot.sidebar.data.as_deref() {
                    options.push((
                        sidebar.personal_project.project.id.clone(),
                        sidebar.personal_project.project.name.clone(),
                    ));
                    for project in &sidebar.projects {
                        options.push((project.project.id.clone(), project.project.name.clone()));
                    }
                }
                if options.is_empty() {
                    options.push((PERSONAL_PROJECT_ID.into(), "Personal".into()));
                }
                let current = stored.and_then(JsonValue::as_str).unwrap_or("");
                let selected = options
                    .iter()
                    .position(|(id, _)| id == current)
                    .unwrap_or(0);
                let title = descriptor.label().unwrap_or(&key).to_owned();
                if let Some(view) = self.settings.as_mut() {
                    view.choice = Some(Choice {
                        title,
                        target: ChoiceTarget::PluginField { plugin_id, key },
                        options,
                        selected,
                    });
                }
                Vec::new()
            }
            FieldKind::Unsupported => {
                self.toast(
                    "This setting can only be changed in the web app",
                    ToastLevel::Info,
                );
                Vec::new()
            }
            FieldKind::Number | FieldKind::Text { .. } | FieldKind::Secret => {
                self.start_edit(EditTarget::PluginField { plugin_id, key });
                Vec::new()
            }
        }
    }

    fn open_permission_choice(&mut self, host_id: String) {
        let Some(host) = self.settings_host(&host_id) else {
            return;
        };
        let current = host.max_permission_mode;
        let title = format!("Permission limit · {}", host.name);
        let options: Vec<(String, String)> = PERMISSION_MODE_OPTIONS
            .iter()
            .map(|option| {
                (
                    option.value.as_str().to_owned(),
                    format!("{} — {}", option.label, option.description),
                )
            })
            .collect();
        let selected = PERMISSION_MODE_OPTIONS
            .iter()
            .position(|option| option.value == current)
            .unwrap_or(0);
        if let Some(view) = self.settings.as_mut() {
            view.choice = Some(Choice {
                title,
                target: ChoiceTarget::HostPermission(host_id),
                options,
                selected,
            });
        }
    }

    fn choose(&mut self, choice: Choice) -> Vec<Effect> {
        let Some((value, _)) = choice.options.get(choice.selected).cloned() else {
            return Vec::new();
        };
        match choice.target {
            ChoiceTarget::PluginField { plugin_id, key } => {
                match self.save_plugin_field(plugin_id.clone(), key.clone(), &value) {
                    Ok(effects) => effects,
                    Err(error) => {
                        if let Some(view) = self.settings.as_mut() {
                            view.field_errors
                                .insert(field_error_key(&plugin_id, &key), error);
                        }
                        Vec::new()
                    }
                }
            }
            ChoiceTarget::HostPermission(host_id) => {
                let mode = PERMISSION_MODE_OPTIONS
                    .iter()
                    .map(|option| option.value)
                    .find(|mode| mode.as_str() == value)
                    .unwrap_or_default();
                if self
                    .settings_host(&host_id)
                    .map(|host| host.max_permission_mode)
                    == Some(mode)
                {
                    return Vec::new();
                }
                self.settings_mutate(SettingsMutation::SetHostPermissionCeiling { host_id, mode })
            }
        }
    }

    fn ask_delete_thread(&mut self, thread_id: String) -> Vec<Effect> {
        let title = self
            .settings
            .as_ref()
            .and_then(|view| {
                view.archived
                    .threads
                    .iter()
                    .find(|entry| entry.thread.id == thread_id)
            })
            .map(thread_title)
            .unwrap_or_else(|| "this thread".into());
        if let Some(view) = self.settings.as_mut() {
            view.confirm = Some(SettingsConfirm {
                action: ConfirmAction::DeleteThread {
                    thread_id: thread_id.clone(),
                    title,
                },
                children: Load::Loading,
            });
        }
        vec![Effect::Settings(SettingsRequest::ChildSummary {
            thread_id,
        })]
    }

    fn ask_delete_project(&mut self, project_id: String) {
        let name = self
            .settings_project_name(&project_id)
            .unwrap_or_else(|| project_id.clone());
        if let Some(view) = self.settings.as_mut() {
            view.confirm = Some(SettingsConfirm {
                action: ConfirmAction::DeleteProject { project_id, name },
                children: Load::Idle,
            });
        }
    }

    fn confirm_settings(&mut self, confirm: SettingsConfirm) -> Vec<Effect> {
        match confirm.action {
            ConfirmAction::DeleteProject { project_id, .. } => {
                self.settings_mutate(SettingsMutation::DeleteProject { project_id })
            }
            ConfirmAction::DeleteThread { thread_id, .. } => {
                let children = confirm
                    .children
                    .loaded()
                    .map_or(0, |summary| summary.non_deleted_child_count);
                self.settings_mutate(SettingsMutation::DeleteThread {
                    thread_id,
                    child_threads_confirmed: children > 0,
                })
            }
        }
    }

    fn set_archived_filters(&mut self, update: impl FnOnce(&mut ArchivedQuery)) -> Vec<Effect> {
        let Some(view) = self.settings.as_mut() else {
            return Vec::new();
        };
        update(&mut view.archived.query);
        view.archived.query.offset = 0;
        view.archived.threads.clear();
        view.archived.loaded = false;
        view.archived.loading = true;
        view.archived.has_more = false;
        view.cursors.insert(SettingsSection::Archived, 0);
        vec![Effect::Settings(SettingsRequest::Archived(
            view.archived.query.clone(),
        ))]
    }

    fn cycle_archived_project(&mut self) -> Vec<Effect> {
        let mut ids: Vec<Option<String>> = vec![None];
        if let Some(sidebar) = self.snapshot.sidebar.data.as_deref() {
            ids.push(Some(sidebar.personal_project.project.id.clone()));
            ids.extend(
                sidebar
                    .projects
                    .iter()
                    .map(|project| Some(project.project.id.clone())),
            );
        }
        let current = self
            .settings
            .as_ref()
            .map(|view| view.archived.query.project_id.clone())
            .unwrap_or_default();
        let index = ids.iter().position(|id| *id == current).unwrap_or(0);
        let next = ids[(index + 1) % ids.len()].clone();
        self.set_archived_filters(|query| query.project_id = next)
    }

    fn load_more_archived(&mut self) -> Vec<Effect> {
        let Some(view) = self.settings.as_mut() else {
            return Vec::new();
        };
        if view.archived.loading || !view.archived.has_more {
            return Vec::new();
        }
        view.archived.loading = true;
        let mut query = view.archived.query.clone();
        query.offset = view.archived.threads.len() as u32;
        vec![Effect::Settings(SettingsRequest::Archived(query))]
    }

    pub(crate) fn set_section(&mut self, section: SettingsSection) -> Vec<Effect> {
        if let Some(view) = self.settings.as_mut() {
            if view.section == section && view.detail.is_none() {
                return Vec::new();
            }
            view.section = section;
            view.detail = None;
            view.filter_input = None;
        }
        self.settings_section_effects()
    }
}

fn is_ctrl(key: &KeyEvent, ch: char) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char(ch)
}

pub fn handle_settings_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    let Some(view) = state.settings.as_mut() else {
        return Vec::new();
    };
    if let Some(confirm) = view.confirm.take() {
        return match key.code {
            KeyCode::Char('y') | KeyCode::Enter if confirm.ready() => {
                state.confirm_settings(confirm)
            }
            KeyCode::Char('y') | KeyCode::Enter => {
                view.confirm = Some(confirm);
                Vec::new()
            }
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q') => Vec::new(),
            _ => {
                view.confirm = Some(confirm);
                Vec::new()
            }
        };
    }
    if let Some(mut choice) = view.choice.take() {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {}
            KeyCode::Enter | KeyCode::Char(' ') => return state.choose(choice),
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                choice.selected = (choice.selected + 1).min(choice.options.len().saturating_sub(1));
                view.choice = Some(choice);
            }
            KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => {
                choice.selected = choice.selected.saturating_sub(1);
                view.choice = Some(choice);
            }
            KeyCode::Char(ch) if ch.is_ascii_digit() && ch != '0' => {
                let index = usize::from(ch as u8 - b'1');
                if index < choice.options.len() {
                    choice.selected = index;
                    return state.choose(choice);
                }
                view.choice = Some(choice);
            }
            _ => view.choice = Some(choice),
        }
        return Vec::new();
    }
    if let Some(mut edit) = view.edit.take() {
        return match edit.input.handle_key(key) {
            InputOutcome::Cancel => Vec::new(),
            InputOutcome::Submit => match state.submit_edit(edit.clone()) {
                Ok(effects) => effects,
                Err(error) => {
                    edit.error = Some(error);
                    if let Some(view) = state.settings.as_mut() {
                        view.edit = Some(edit);
                    }
                    Vec::new()
                }
            },
            _ => {
                edit.error = None;
                view.edit = Some(edit);
                Vec::new()
            }
        };
    }
    if let Some(mut input) = view.filter_input.take() {
        let section = view.section;
        match input.handle_key(key) {
            InputOutcome::Submit => {
                view.filters.insert(section, input.text().to_owned());
            }
            InputOutcome::Cancel => {
                view.filters.remove(&section);
            }
            _ => view.filter_input = Some(input),
        }
        if view.detail.is_none() {
            view.cursors.insert(section, 0);
        }
        return Vec::new();
    }
    if is_ctrl(&key, 'k') {
        return crate::app::run_command(state, Command::PaletteOpen);
    }
    if is_ctrl(&key, 'r') {
        return state.reload_settings_section();
    }
    if is_ctrl(&key, 'c') {
        state.close_settings();
        return Vec::new();
    }
    match view.focus {
        SettingsFocus::Nav => handle_nav_key(state, key),
        SettingsFocus::Content => handle_content_key(state, key),
    }
}

fn handle_nav_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    let Some(view) = state.settings.as_mut() else {
        return Vec::new();
    };
    let index = view.section.index();
    let last = SettingsSection::ALL.len() - 1;
    let next = match key.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char(',') => {
            state.close_settings();
            return Vec::new();
        }
        KeyCode::Down | KeyCode::Char('j') => index.saturating_add(1).min(last),
        KeyCode::Up | KeyCode::Char('k') => index.saturating_sub(1),
        KeyCode::Char('g') | KeyCode::Home => 0,
        KeyCode::Char('G') | KeyCode::End => last,
        KeyCode::Char(ch) if ch.is_ascii_digit() && ch != '0' => {
            usize::from(ch as u8 - b'1').min(last)
        }
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') | KeyCode::Tab => {
            view.focus = SettingsFocus::Content;
            return Vec::new();
        }
        _ => return Vec::new(),
    };
    state.set_section(SettingsSection::ALL[next])
}

fn handle_content_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    let rows = settings_rows(state);
    let Some(view) = state.settings.as_mut() else {
        return Vec::new();
    };
    let cursor = clamp_cursor(&rows, view.cursor()).unwrap_or(0);
    let target = rows.get(cursor).and_then(|row| row.target.clone());
    match key.code {
        KeyCode::Down | KeyCode::Char('j') => {
            view.set_cursor(step_cursor(&rows, cursor, 1));
            return Vec::new();
        }
        KeyCode::Up | KeyCode::Char('k') => {
            view.set_cursor(step_cursor(&rows, cursor, -1));
            return Vec::new();
        }
        KeyCode::PageDown => {
            view.set_cursor(step_cursor(&rows, cursor, PAGE));
            return Vec::new();
        }
        KeyCode::PageUp => {
            view.set_cursor(step_cursor(&rows, cursor, -PAGE));
            return Vec::new();
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            view.set_cursor(step_cursor(&rows, cursor, PAGE / 2));
            return Vec::new();
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            view.set_cursor(step_cursor(&rows, cursor, -PAGE / 2));
            return Vec::new();
        }
        KeyCode::Char('g') | KeyCode::Home => {
            view.set_cursor(first_selectable(&rows).unwrap_or(0));
            return Vec::new();
        }
        KeyCode::Char('G') | KeyCode::End => {
            view.set_cursor(step_cursor(&rows, cursor, isize::MAX / 2));
            return Vec::new();
        }
        KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left | KeyCode::BackTab => {
            if view.detail.take().is_some() {
                return Vec::new();
            }
            if key.code == KeyCode::Esc && !view.filter(view.section).is_empty() {
                view.filters.remove(&view.section);
                return Vec::new();
            }
            view.focus = SettingsFocus::Nav;
            return Vec::new();
        }
        KeyCode::Tab => {
            view.focus = SettingsFocus::Nav;
            return Vec::new();
        }
        KeyCode::Char('q') | KeyCode::Char(',') => {
            state.close_settings();
            return Vec::new();
        }
        KeyCode::Char(ch) if view.detail.is_none() && ('1'..='9').contains(&ch) => {
            let index = usize::from(ch as u8 - b'1');
            if let Some(section) = SettingsSection::ALL.get(index).copied() {
                return state.set_section(section);
            }
            return Vec::new();
        }
        KeyCode::Char('/') if view.detail.is_none() && view.section.filterable() => {
            view.filter_input = Some(LineInput::new(view.filter(view.section).to_owned()));
            return Vec::new();
        }
        KeyCode::Char('t')
            if view.detail.is_none() && view.section == SettingsSection::Archived =>
        {
            return state.set_archived_filters(|query| query.kind = query.kind.next());
        }
        KeyCode::Char('p')
            if view.detail.is_none() && view.section == SettingsSection::Archived =>
        {
            return state.cycle_archived_project();
        }
        _ => {}
    }
    match (view.detail.clone(), key.code) {
        (Some(Detail::Project(id)), KeyCode::Char('r')) => {
            state.start_edit(EditTarget::ProjectName(id));
            return Vec::new();
        }
        (Some(Detail::Project(id)), KeyCode::Char('D')) => {
            state.ask_delete_project(id);
            return Vec::new();
        }
        (Some(Detail::Machine(id)), KeyCode::Char('r')) => {
            state.start_edit(EditTarget::HostName(id));
            return Vec::new();
        }
        _ => {}
    }
    let Some(target) = target else {
        return Vec::new();
    };
    row_key(state, target, key)
}

fn row_key(state: &mut AppState, target: RowTarget, key: KeyEvent) -> Vec<Effect> {
    let activate = matches!(
        key.code,
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l')
    );
    let space = key.code == KeyCode::Char(' ');
    match (target, key.code) {
        (RowTarget::GeneralToggle(name), _) if activate || space => state.toggle_general(name),
        (RowTarget::GeneralText(name), _) if activate || key.code == KeyCode::Char('e') => {
            state.start_edit(EditTarget::General(name));
            Vec::new()
        }
        (RowTarget::Copy(text), KeyCode::Char('y')) | (RowTarget::Copy(text), KeyCode::Enter) => {
            state.toast("Copied", ToastLevel::Info);
            vec![Effect::Copy(text)]
        }
        (RowTarget::Provider(id), _) if space || key.code == KeyCode::Char('e') => {
            state.toggle_provider(&id)
        }
        (RowTarget::Provider(id), KeyCode::Char('d')) => state.make_default_provider(&id),
        (RowTarget::Provider(id), KeyCode::Char('c')) => state.toggle_completed_turns(&id),
        (RowTarget::Provider(id), _) if activate => state.toggle_provider_models(id),
        (RowTarget::FastServiceTier, _) if activate || space => state.toggle_general(FAST_TIER_KEY),
        (RowTarget::Binding(command), _) if activate => {
            state.close_settings();
            crate::app::run_command(state, command)
        }
        (RowTarget::Project(id), _) if activate => state.open_detail(Detail::Project(id)),
        (RowTarget::Project(id), KeyCode::Char('r'))
        | (RowTarget::ProjectRename(id), KeyCode::Char('r')) => {
            state.start_edit(EditTarget::ProjectName(id));
            Vec::new()
        }
        (RowTarget::ProjectRename(id), _) if activate => {
            state.start_edit(EditTarget::ProjectName(id));
            Vec::new()
        }
        (RowTarget::ProjectDelete(id), _) if activate || key.code == KeyCode::Char('D') => {
            state.ask_delete_project(id);
            Vec::new()
        }
        (RowTarget::Project(id), KeyCode::Char('D')) => {
            state.ask_delete_project(id);
            Vec::new()
        }
        (RowTarget::Host(id), _) if activate => state.open_detail(Detail::Machine(id)),
        (RowTarget::Host(id), KeyCode::Char('r')) | (RowTarget::HostRename(id), _)
            if activate || key.code == KeyCode::Char('r') =>
        {
            state.start_edit(EditTarget::HostName(id));
            Vec::new()
        }
        (RowTarget::HostPermission(id), _) if activate => {
            state.open_permission_choice(id);
            Vec::new()
        }
        (RowTarget::Plugin(id), _) if activate => state.open_detail(Detail::Plugin(id)),
        (RowTarget::Plugin(id), KeyCode::Char(' ')) | (RowTarget::PluginEnabled(id), _)
            if space || activate =>
        {
            let enabled = state
                .settings
                .as_ref()
                .and_then(|view| view.plugins.loaded())
                .and_then(|plugins| plugins.iter().find(|plugin| plugin.id == id))
                .map(|plugin| plugin.enabled);
            match enabled {
                Some(enabled) => state.settings_mutate(SettingsMutation::SetPluginEnabled {
                    plugin_id: id,
                    enabled: !enabled,
                }),
                None => Vec::new(),
            }
        }
        (
            RowTarget::PluginField {
                plugin_id,
                key: field,
            },
            _,
        ) if activate || space => state.activate_plugin_field(plugin_id, field),
        (RowTarget::Archived(id), KeyCode::Char('u')) => {
            state.settings_mutate(SettingsMutation::UnarchiveThread { thread_id: id })
        }
        (RowTarget::Archived(id), KeyCode::Char('D') | KeyCode::Char('x') | KeyCode::Delete) => {
            state.ask_delete_thread(id)
        }
        (RowTarget::Archived(id), _) if activate || key.code == KeyCode::Char('o') => {
            state.close_settings();
            state.open(id)
        }
        (RowTarget::ArchivedMore, _) if activate => state.load_more_archived(),
        _ => Vec::new(),
    }
}

pub fn settings_hints(view: &SettingsView) -> String {
    if view.confirm.is_some() {
        return "y/RET confirm · ESC cancel".into();
    }
    if view.choice.is_some() {
        return "1-9/RET choose · j/k move · ESC cancel".into();
    }
    if view.edit.is_some() {
        return "RET save · ESC cancel".into();
    }
    if view.filter_input.is_some() {
        return "type to filter · RET keep · ESC clear".into();
    }
    if view.focus == SettingsFocus::Nav {
        return "j/k section · RET/l open · gt/gT/{N}gt section · q close".into();
    }
    let common = "gr reload · ESC back · q close";
    match (&view.detail, view.section) {
        (Some(Detail::Project(_)), _) => {
            format!("RET edit · R rename · dd delete · yy yank · {common}")
        }
        (Some(Detail::Machine(_)), _) => format!("RET edit · R rename · yy yank · {common}"),
        (Some(Detail::Plugin(_)), _) => format!("RET edit · ~ toggle · yy yank · {common}"),
        (None, SettingsSection::General) => format!("RET change · ~ toggle · yy yank · {common}"),
        (None, SettingsSection::Providers) => {
            format!("RET models · ~ enable · gd default · zc collapse turns · {common}")
        }
        (None, SettingsSection::Keyboard) => format!("RET run · g/ filter · {common}"),
        (None, SettingsSection::Projects) => {
            format!("RET open · R rename · dd delete · g/ filter · {common}")
        }
        (None, SettingsSection::Machines) => format!("RET open · R rename · {common}"),
        (None, SettingsSection::Plugins) => {
            format!("RET open · ~ enable · g/ filter · {common}")
        }
        (None, SettingsSection::Archived) => {
            format!(
                "RET open · u unarchive · dd delete · t kind · p project · g/ search · {common}"
            )
        }
    }
}
