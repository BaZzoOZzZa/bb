use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use bb_wire::common::JsonValue;
use bb_wire::environment::{
    CreateThreadEnvironmentArgs, GitBranchSelection, GitCheckoutRef, Host, HostStatus, HostType,
    MachinePhase, MachineSelection, UnmanagedBranchSpec,
};
use bb_wire::project::{
    PERSONAL_PROJECT_ID, ProjectBranchesResponse, ProjectSource, WorkspaceGitOperationKind,
};
use bb_wire::prompt::{
    CreateExecutionInputSources, ExecutionInputSource, PermissionMode, ProjectExecutionDefaults,
    ReasoningLevel,
};
use bb_wire::system::{
    AvailableModel, ModelLoadError, ModelLoadErrorCode, ProviderAvailability, ProviderInfo,
    SystemEnvironmentProvider, SystemExecutionOptionsResponse,
};
use bb_wire::thread::ThreadListEntry;

use crate::execution::{
    PickerOption, format_model_label, resolve_model_catalog_selection, service_tier_options,
};
use crate::prompt::{
    PERMISSION_MODE_OPTIONS, permission_mode_rank, resolve_permission_mode_selection,
};
use crate::sidebar::thread_title;

pub const PROJECT_CHECKOUT_PROVIDER_ID: &str = "project-checkout";
pub const GIT_WORKTREE_PROVIDER_ID: &str = "git-worktree";
pub const PERSONAL_WORKSPACE_PROVIDER_ID: &str = "personal-workspace";
pub const REUSE_VALUE_WITHOUT_ENVIRONMENT: &str = "reuse";
pub const PROVIDER_INPUTS_CONTROL_MISSING_REASON: &str = "Needs its plugin's control";
pub const PERMISSION_CEILING_REASON: &str =
    "Above the selected machine's permission limit. Change it in Settings → Machines.";
const DEFAULT_SERVICE_TIER: &str = "default";
const NATIVE_INPUT_CONTROLS: &[&str] = &[GIT_WORKTREE_PROVIDER_ID, PROJECT_CHECKOUT_PROVIDER_ID];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvironmentValue {
    Reuse(Option<String>),
    Provider(String),
}

pub fn encode_reuse_value(environment_id: &str) -> String {
    format!("reuse:{environment_id}")
}

pub fn encode_provider_value(environment_provider_id: &str) -> String {
    format!("provider:{environment_provider_id}")
}

fn valid_provider_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

pub fn parse_environment_value(value: &str) -> Option<EnvironmentValue> {
    if value == REUSE_VALUE_WITHOUT_ENVIRONMENT {
        return Some(EnvironmentValue::Reuse(None));
    }
    if let Some(environment_id) = value.strip_prefix("reuse:")
        && !environment_id.is_empty()
    {
        return Some(EnvironmentValue::Reuse(Some(environment_id.to_owned())));
    }
    let provider = value.strip_prefix("provider:")?;
    valid_provider_id(provider).then(|| EnvironmentValue::Provider(provider.to_owned()))
}

pub fn is_projectless_project_id(project_id: &str) -> bool {
    project_id == PERSONAL_PROJECT_ID
}

pub fn find_local_path_source_for_host<'a>(
    sources: &'a [ProjectSource],
    host_id: &str,
) -> Option<&'a ProjectSource> {
    sources
        .iter()
        .find(|source| source.source_type == "local_path" && source.host_id == host_id)
}

pub fn provider_unavailable(provider: &SystemEnvironmentProvider) -> bool {
    matches!(
        provider.availability,
        Some(ProviderAvailability::Unavailable { .. })
    )
}

fn is_hostless(provider: &SystemEnvironmentProvider) -> bool {
    provider
        .machine_provider_id
        .as_deref()
        .is_some_and(|id| !id.is_empty())
}

pub fn resolve_host_environment_provider(
    current: Option<&SystemEnvironmentProvider>,
    providers: &[SystemEnvironmentProvider],
) -> Option<SystemEnvironmentProvider> {
    let candidates: Vec<&SystemEnvironmentProvider> = providers
        .iter()
        .filter(|provider| {
            provider.machine_provider_id.is_none() && !provider_unavailable(provider)
        })
        .collect();
    candidates
        .iter()
        .find(|provider| current.is_some_and(|current| current.id == provider.id))
        .or_else(|| candidates.first())
        .map(|provider| (*provider).clone())
        .or_else(|| {
            current
                .filter(|current| current.machine_provider_id.is_none())
                .cloned()
        })
}

pub fn resolve_new_thread_host_environment_provider(
    current_provider_id: Option<&str>,
    providers: &[SystemEnvironmentProvider],
) -> Option<SystemEnvironmentProvider> {
    let usable: Vec<&SystemEnvironmentProvider> = providers
        .iter()
        .filter(|provider| {
            provider.machine_provider_id.is_none() && !provider_unavailable(provider)
        })
        .collect();
    usable
        .iter()
        .find(|provider| Some(provider.id.as_str()) == current_provider_id)
        .or_else(|| usable.first())
        .map(|provider| (*provider).clone())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReuseThread {
    pub id: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReuseThreadOption {
    pub environment_id: String,
    pub branch_name: Option<String>,
    pub name: Option<String>,
    pub path: Option<String>,
    pub environment_provider_id: Option<String>,
    pub host_name: Option<String>,
    pub threads: Vec<ReuseThread>,
}

impl ReuseThreadOption {
    pub fn label(&self) -> &str {
        self.name
            .as_deref()
            .or(self.branch_name.as_deref())
            .filter(|label| !label.is_empty())
            .unwrap_or(&self.environment_id)
    }
}

fn compare_reuse_options(left: &ReuseThreadOption, right: &ReuseThreadOption) -> Ordering {
    let label = |option: &ReuseThreadOption| {
        option
            .name
            .clone()
            .or_else(|| option.branch_name.clone())
            .filter(|label| !label.is_empty())
    };
    match (label(left), label(right)) {
        (Some(a), Some(b)) => a
            .to_lowercase()
            .cmp(&b.to_lowercase())
            .then_with(|| a.cmp(&b))
            .then_with(|| left.environment_id.cmp(&right.environment_id)),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => left.environment_id.cmp(&right.environment_id),
    }
}

pub fn build_reuse_thread_options<'a>(
    threads: impl IntoIterator<Item = &'a ThreadListEntry>,
    host_names: Option<&HashMap<String, String>>,
) -> Vec<ReuseThreadOption> {
    let mut order: Vec<String> = Vec::new();
    let mut buckets: HashMap<String, (ReuseThreadOption, Vec<&ThreadListEntry>)> = HashMap::new();
    for entry in threads {
        let Some(environment_id) = entry.thread.environment_id.clone() else {
            continue;
        };
        let bucket = buckets.entry(environment_id.clone()).or_insert_with(|| {
            order.push(environment_id.clone());
            let host_name = match (host_names, entry.environment_host_id.as_deref()) {
                (Some(names), Some(host_id)) => names.get(host_id).cloned(),
                _ => None,
            };
            (
                ReuseThreadOption {
                    environment_id,
                    branch_name: entry.environment_branch_name.clone(),
                    name: entry.environment_name.clone(),
                    path: entry.environment_path.clone(),
                    environment_provider_id: entry.environment_provider_id.clone(),
                    host_name,
                    threads: Vec::new(),
                },
                Vec::new(),
            )
        });
        bucket.1.push(entry);
    }
    let mut options: Vec<ReuseThreadOption> = order
        .into_iter()
        .filter_map(|id| buckets.remove(&id))
        .map(|(mut option, mut entries)| {
            entries.sort_by(|a, b| {
                b.thread
                    .latest_attention_at
                    .cmp(&a.thread.latest_attention_at)
            });
            option.threads = entries
                .into_iter()
                .map(|entry| ReuseThread {
                    id: entry.thread.id.clone(),
                    title: thread_title(entry),
                })
                .collect();
            option
        })
        .collect();
    options.sort_by(compare_reuse_options);
    options
}

pub fn resolve_projectless_default_environment_provider(
    providers: &[&SystemEnvironmentProvider],
) -> Option<String> {
    providers
        .iter()
        .find(|provider| {
            provider.id == PERSONAL_WORKSPACE_PROVIDER_ID && provider.requires.projectless
        })
        .map(|provider| provider.id.clone())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeededReuseStatus {
    Pending,
    Available,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeededReuse {
    pub environment_id: String,
    pub status: SeededReuseStatus,
}

pub struct EnvironmentContext<'a> {
    pub selection: &'a str,
    pub providers: Option<&'a [SystemEnvironmentProvider]>,
    pub projectless: bool,
    pub known_host_ids: &'a HashSet<String>,
    pub primary_host_id: Option<&'a str>,
    pub sources: &'a [ProjectSource],
    pub reuse_options: &'a [ReuseThreadOption],
    pub reuse_loading: bool,
    pub seeded: Option<&'a SeededReuse>,
}

impl EnvironmentContext<'_> {
    fn reuse_survives(&self, environment_id: &str) -> bool {
        if self
            .reuse_options
            .iter()
            .any(|option| option.environment_id == environment_id)
        {
            return true;
        }
        if self.reuse_loading {
            return true;
        }
        match self.seeded {
            Some(seeded) if seeded.environment_id == environment_id => {
                seeded.status != SeededReuseStatus::Missing
            }
            _ => false,
        }
    }

    fn projectless_value(&self, parsed: Option<&EnvironmentValue>) -> String {
        if let Some(EnvironmentValue::Reuse(Some(environment_id))) = parsed
            && self.reuse_survives(environment_id)
        {
            return self.selection.to_owned();
        }
        let Some(providers) = self.providers else {
            return String::new();
        };
        let has_primary = self.primary_host_id.is_some();
        if let Some(EnvironmentValue::Provider(selected)) = parsed
            && providers
                .iter()
                .any(|provider| &provider.id == selected && provider.requires.projectless)
            && has_primary
        {
            return self.selection.to_owned();
        }
        let eligible: Vec<&SystemEnvironmentProvider> = providers
            .iter()
            .filter(|provider| provider.requires.projectless && has_primary)
            .collect();
        resolve_projectless_default_environment_provider(&eligible)
            .map(|id| encode_provider_value(&id))
            .unwrap_or_default()
    }

    pub fn effective_value(&self) -> String {
        let parsed = parse_environment_value(self.selection);
        if self.projectless {
            return self.projectless_value(parsed.as_ref());
        }
        let Some(providers) = self.providers else {
            return String::new();
        };
        let registered = |id: &str| providers.iter().any(|provider| provider.id == id);
        let fallback = match self.primary_host_id {
            Some(primary)
                if self.known_host_ids.contains(primary)
                    && find_local_path_source_for_host(self.sources, primary).is_some()
                    && registered(PROJECT_CHECKOUT_PROVIDER_ID) =>
            {
                encode_provider_value(PROJECT_CHECKOUT_PROVIDER_ID)
            }
            _ => String::new(),
        };
        match &parsed {
            Some(EnvironmentValue::Reuse(None)) => self.selection.to_owned(),
            Some(EnvironmentValue::Reuse(Some(environment_id))) => {
                if self.reuse_survives(environment_id) {
                    self.selection.to_owned()
                } else {
                    fallback
                }
            }
            Some(EnvironmentValue::Provider(id)) => {
                let selected = providers.iter().find(|provider| &provider.id == id);
                match selected {
                    Some(provider) if is_hostless(provider) || self.primary_host_id.is_some() => {
                        self.selection.to_owned()
                    }
                    _ => fallback,
                }
            }
            None => fallback,
        }
    }
}

pub fn resolve_thread_environment(
    environment_value: &str,
    project_id: &str,
    providers: Option<&[SystemEnvironmentProvider]>,
    machine: Option<MachineSelection>,
    inputs: Option<JsonValue>,
) -> Option<CreateThreadEnvironmentArgs> {
    if project_id.is_empty() {
        return None;
    }
    match parse_environment_value(environment_value)? {
        EnvironmentValue::Provider(id) => {
            let provider = providers?.iter().find(|provider| provider.id == id)?;
            if machine.is_none() && !is_hostless(provider) {
                return None;
            }
            let inputs = provider.inputs.as_ref().and(inputs);
            if provider.inputs.is_some() && inputs.is_none() {
                return None;
            }
            Some(CreateThreadEnvironmentArgs::Provider {
                environment_provider_id: provider.id.clone(),
                machine,
                inputs,
            })
        }
        EnvironmentValue::Reuse(environment_id) => Some(CreateThreadEnvironmentArgs::Reuse {
            environment_id: environment_id?,
        }),
    }
}

pub fn provider_inputs_control_required(provider: &SystemEnvironmentProvider) -> bool {
    provider.inputs.is_some() && !provider.accepts_empty_inputs
}

pub fn provider_disabled_reason(
    provider: &SystemEnvironmentProvider,
    controls: &HashSet<String>,
) -> Option<String> {
    if let Some(ProviderAvailability::Unavailable { message }) = &provider.availability {
        return Some(message.clone());
    }
    if !controls.contains(&provider.id) && provider_inputs_control_required(provider) {
        return Some(PROVIDER_INPUTS_CONTROL_MISSING_REASON.to_owned());
    }
    None
}

pub fn provider_description(
    provider: &SystemEnvironmentProvider,
    controls: &HashSet<String>,
) -> Option<String> {
    if let Some(ProviderAvailability::SetupRequired { message }) = &provider.availability {
        return Some(message.clone());
    }
    provider_disabled_reason(provider, controls)
}

pub fn inputs_control_provider_ids(providers: &[SystemEnvironmentProvider]) -> HashSet<String> {
    let mut ids: HashSet<String> = providers
        .iter()
        .filter(|provider| NATIVE_INPUT_CONTROLS.contains(&provider.id.as_str()))
        .map(|provider| provider.id.clone())
        .collect();
    for provider in providers {
        if let Some(inner) = &provider.environment_provider_id
            && ids.contains(inner)
        {
            ids.insert(provider.id.clone());
        }
    }
    ids
}

pub fn filter_environment_providers(
    registered: &[SystemEnvironmentProvider],
    projectless: bool,
) -> Vec<SystemEnvironmentProvider> {
    registered
        .iter()
        .filter(|provider| {
            if projectless {
                !provider.requires.project_checkout && !provider.requires.git_remote
            } else {
                !provider.requires.projectless
            }
        })
        .cloned()
        .collect()
}

pub fn persistent_hosts(hosts: &[Host]) -> Vec<&Host> {
    hosts
        .iter()
        .filter(|host| host.host_type != HostType::Ephemeral)
        .collect()
}

pub fn select_primary_host<'a>(
    hosts: &'a [Host],
    primary_host_id: Option<&str>,
) -> Option<&'a Host> {
    let available = persistent_hosts(hosts);
    if let Some(primary) = primary_host_id {
        return available.into_iter().find(|host| host.id == primary);
    }
    available
        .iter()
        .find(|host| host.status == HostStatus::Connected)
        .or_else(|| available.first())
        .copied()
}

pub struct HostProviderInputs<'a> {
    pub hosts: &'a [&'a Host],
    pub providers: &'a [SystemEnvironmentProvider],
    pub project_providers: Option<&'a [SystemEnvironmentProvider]>,
    pub sources: &'a [ProjectSource],
    pub has_git_remote: bool,
}

pub fn providers_by_host(
    inputs: &HostProviderInputs<'_>,
) -> HashMap<String, Vec<SystemEnvironmentProvider>> {
    inputs
        .hosts
        .iter()
        .map(|host| {
            let has_source = find_local_path_source_for_host(inputs.sources, &host.id).is_some();
            let providers = inputs
                .providers
                .iter()
                .filter(|provider| {
                    provider.machine_provider_id.is_none()
                        && (!(provider.requires.project_checkout || provider.requires.git_checkout)
                            || has_source)
                        && (!provider.requires.git_remote || inputs.has_git_remote)
                })
                .map(|provider| {
                    let availability = inputs
                        .project_providers
                        .and_then(|list| list.iter().find(|candidate| candidate.id == provider.id))
                        .and_then(|candidate| candidate.machine_availability.get(&host.id).cloned())
                        .flatten();
                    SystemEnvironmentProvider {
                        availability,
                        ..provider.clone()
                    }
                })
                .collect();
            (host.id.clone(), providers)
        })
        .collect()
}

pub fn merge_host_providers(
    providers: &[SystemEnvironmentProvider],
    resolved: &[SystemEnvironmentProvider],
) -> Vec<SystemEnvironmentProvider> {
    providers
        .iter()
        .filter_map(|provider| {
            resolved
                .iter()
                .find(|candidate| candidate.id == provider.id)
                .cloned()
        })
        .collect()
}

pub fn scoped_providers(
    providers: &[SystemEnvironmentProvider],
    by_host: Option<&HashMap<String, Vec<SystemEnvironmentProvider>>>,
    host_id: Option<&str>,
    value: &str,
    selected_host_id: Option<&str>,
) -> Vec<SystemEnvironmentProvider> {
    let host_providers = match (host_id, by_host) {
        (Some(host), Some(by_host)) => {
            merge_host_providers(providers, by_host.get(host).map_or(&[], Vec::as_slice))
        }
        _ => providers.to_vec(),
    };
    host_providers
        .into_iter()
        .filter(|provider| {
            !provider_unavailable(provider)
                || (value == encode_provider_value(&provider.id) && selected_host_id == host_id)
        })
        .collect()
}

pub fn host_selectable(host: &Host) -> bool {
    host.status == HostStatus::Connected
        && matches!(
            host.lifecycle.phase,
            MachinePhase::Active | MachinePhase::Unknown
        )
}

pub fn format_model_load_error_text(error: &ModelLoadError, provider_label: &str) -> String {
    let summary = format!("Could not load models for {provider_label}.");
    let reason = match error.code {
        ModelLoadErrorCode::ProviderUnavailable => {
            Some("Provider plugin failed to load".to_owned())
        }
        ModelLoadErrorCode::MissingExecutable => Some("CLI not found".to_owned()),
        ModelLoadErrorCode::AuthRequired => Some("Not signed in".to_owned()),
        ModelLoadErrorCode::Timeout => Some("Timed out".to_owned()),
        ModelLoadErrorCode::Failed | ModelLoadErrorCode::Unknown => error.detail.clone(),
    };
    match reason {
        Some(reason) => format!("{summary} {reason}."),
        None => summary,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectDefaultsStatus {
    Pending,
    Error,
    Resolved,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProjectDefaultsState {
    Pending,
    Error,
    Resolved(Option<ProjectExecutionDefaults>),
}

impl ProjectDefaultsState {
    pub fn status(&self) -> ProjectDefaultsStatus {
        match self {
            Self::Pending => ProjectDefaultsStatus::Pending,
            Self::Error => ProjectDefaultsStatus::Error,
            Self::Resolved(_) => ProjectDefaultsStatus::Resolved,
        }
    }

    pub fn defaults(&self) -> Option<&ProjectExecutionDefaults> {
        match self {
            Self::Resolved(defaults) => defaults.as_ref(),
            _ => None,
        }
    }
}

pub enum DefaultsQuery<'a> {
    Pending,
    Error,
    Success(Option<&'a ProjectExecutionDefaults>),
}

pub fn resolve_project_defaults_state(
    cached: Option<&ProjectExecutionDefaults>,
    project_found: bool,
    query: DefaultsQuery<'_>,
) -> ProjectDefaultsState {
    if let Some(cached) = cached {
        return ProjectDefaultsState::Resolved(Some(cached.clone()));
    }
    if !project_found {
        return ProjectDefaultsState::Pending;
    }
    match query {
        DefaultsQuery::Success(defaults) => ProjectDefaultsState::Resolved(defaults.cloned()),
        DefaultsQuery::Error => ProjectDefaultsState::Error,
        DefaultsQuery::Pending => ProjectDefaultsState::Pending,
    }
}

#[derive(Debug, Clone, Default)]
pub struct SubmitReadiness<'a> {
    pub environment_inputs_blocker: Option<&'a str>,
    pub environment_setup_required: Option<&'a str>,
    pub loading_models: bool,
    pub submitting: bool,
    pub model_load_error: Option<&'a ModelLoadError>,
    pub project_defaults_status: Option<ProjectDefaultsStatus>,
    pub project_defaults_unavailable: bool,
    pub prompt_empty: bool,
    pub provider_label: &'a str,
    pub provider_id: &'a str,
    pub model: &'a str,
    pub environment_unavailable: bool,
}

pub fn resolve_submit_disabled_reason(readiness: &SubmitReadiness<'_>) -> Option<String> {
    if readiness.submitting {
        return Some("Starting thread...".into());
    }
    if readiness.project_defaults_unavailable {
        return Some(
            if readiness.project_defaults_status == Some(ProjectDefaultsStatus::Error) {
                "Could not load the project's execution defaults."
            } else {
                "Loading the project's execution defaults..."
            }
            .into(),
        );
    }
    if readiness.provider_id.is_empty() {
        return Some("Select a provider.".into());
    }
    if readiness.loading_models {
        return Some("Loading models from the selected machine...".into());
    }
    if let Some(error) = readiness.model_load_error {
        let fatal = matches!(
            error.code,
            ModelLoadErrorCode::ProviderUnavailable
                | ModelLoadErrorCode::MissingExecutable
                | ModelLoadErrorCode::AuthRequired
        );
        if fatal || readiness.model.is_empty() {
            let label = if readiness.provider_label.is_empty() {
                readiness.provider_id
            } else {
                readiness.provider_label
            };
            return Some(format_model_load_error_text(error, label));
        }
    }
    if readiness.model.is_empty() {
        return Some("Select a model.".into());
    }
    if let Some(reason) = readiness.environment_setup_required {
        return Some(reason.to_owned());
    }
    if let Some(reason) = readiness.environment_inputs_blocker {
        return Some(reason.to_owned());
    }
    if readiness.environment_unavailable {
        return Some("Select an environment.".into());
    }
    if readiness.prompt_empty {
        return Some("Enter a prompt or attach a file.".into());
    }
    None
}

pub fn resolve_submitted_execution_sources(
    environment: &CreateThreadEnvironmentArgs,
    sources: CreateExecutionInputSources,
) -> CreateExecutionInputSources {
    match environment {
        CreateThreadEnvironmentArgs::Provider { machine, .. }
            if !matches!(machine, Some(MachineSelection::Existing { .. })) =>
        {
            CreateExecutionInputSources {
                model: Some(ExecutionInputSource::Explicit),
                ..sources
            }
        }
        _ => sources,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CreationField {
    Provider,
    Model,
    ServiceTier,
    Reasoning,
    Permission,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CreationPicks {
    pub provider_id: Option<String>,
    pub models: HashMap<String, String>,
    pub reasoning: HashMap<String, ReasoningLevel>,
    pub permission: Option<PermissionMode>,
    pub service_tier: Option<String>,
    pub touched: HashSet<CreationField>,
}

impl CreationPicks {
    pub fn select_provider(&mut self, provider_id: String, current: &CreationOptions) {
        self.touched.insert(CreationField::Provider);
        if !current.provider_id.is_empty() {
            self.models
                .insert(current.provider_id.clone(), current.selected_model.clone());
            self.reasoning
                .insert(current.provider_id.clone(), current.reasoning);
        }
        self.provider_id = Some(provider_id);
    }

    pub fn select_model(&mut self, model: String, current: &CreationOptions) {
        self.touched.insert(CreationField::Model);
        let next = current
            .catalog_models
            .iter()
            .find(|candidate| candidate.model == model);
        let supported: Vec<ReasoningLevel> = next
            .map(|model| {
                model
                    .supported_reasoning_efforts
                    .iter()
                    .map(|effort| effort.reasoning_effort)
                    .collect()
            })
            .unwrap_or_default();
        let reasoning = if supported.is_empty() {
            current.reasoning
        } else {
            crate::prompt::reconcile_reasoning_level(current.reasoning, &supported)
                .unwrap_or(current.reasoning)
        };
        self.models.insert(current.provider_id.clone(), model);
        self.reasoning
            .insert(current.provider_id.clone(), reasoning);
    }

    pub fn select_reasoning(&mut self, level: ReasoningLevel, current: &CreationOptions) {
        self.touched.insert(CreationField::Reasoning);
        self.reasoning.insert(current.provider_id.clone(), level);
    }

    pub fn select_permission(&mut self, mode: PermissionMode) {
        self.touched.insert(CreationField::Permission);
        self.permission = Some(mode);
    }
}

#[derive(Debug, Clone, Default)]
pub struct CreationInputs<'a> {
    pub options: Option<&'a SystemExecutionOptionsResponse>,
    pub loading: bool,
    pub failed: Option<&'a str>,
    pub defaults: Option<&'a ProjectExecutionDefaults>,
    pub picks: Option<&'a CreationPicks>,
    pub host_ceiling: Option<PermissionMode>,
    pub allow_fast_service_tier: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CreationOptions {
    pub provider_id: String,
    pub provider_label: String,
    pub provider_options: Vec<PickerOption<String>>,
    pub selected_model: String,
    pub model: String,
    pub model_label: String,
    pub model_options: Vec<PickerOption<String>>,
    pub catalog_models: Vec<AvailableModel>,
    pub reasoning: ReasoningLevel,
    pub reasoning_options: Vec<PickerOption<ReasoningLevel>>,
    pub permission: PermissionMode,
    pub permission_options: Vec<PickerOption<PermissionMode>>,
    pub service_tier: Option<String>,
    pub supports_service_tier: bool,
    pub unavailable_model_recovery: bool,
    pub loading_models: bool,
    pub model_load_error: Option<ModelLoadError>,
    pub load_failure: Option<String>,
    pub sources: CreateExecutionInputSources,
}

pub fn raw_provider_id(
    picks: Option<&CreationPicks>,
    defaults: Option<&ProjectExecutionDefaults>,
) -> String {
    picks
        .and_then(|picks| picks.provider_id.clone())
        .or_else(|| defaults.map(|defaults| defaults.provider_id.clone()))
        .unwrap_or_default()
}

fn effective_provider(raw: &str, providers: &[ProviderInfo]) -> String {
    if !raw.is_empty() && providers.iter().any(|provider| provider.id == raw) {
        return raw.to_owned();
    }
    providers
        .first()
        .map(|provider| provider.id.clone())
        .unwrap_or_default()
}

fn source(present: bool, touched: bool) -> Option<ExecutionInputSource> {
    (present && touched).then_some(ExecutionInputSource::Explicit)
}

pub fn resolve_creation_options(inputs: &CreationInputs<'_>) -> CreationOptions {
    let empty_picks = CreationPicks::default();
    let picks = inputs.picks.unwrap_or(&empty_picks);
    let providers: &[ProviderInfo] = inputs.options.map_or(&[], |options| &options.providers);
    let raw_provider = raw_provider_id(Some(picks), inputs.defaults);
    let provider_id = effective_provider(&raw_provider, providers);
    let provider = providers.iter().find(|provider| provider.id == provider_id);
    let matches_initial = !provider_id.is_empty()
        && inputs
            .defaults
            .is_some_and(|defaults| defaults.provider_id == provider_id);
    let raw_model = picks
        .models
        .get(&provider_id)
        .cloned()
        .or_else(|| {
            inputs
                .defaults
                .filter(|_| matches_initial)
                .map(|defaults| defaults.model.clone())
        })
        .unwrap_or_default();
    let preferred_reasoning = picks
        .reasoning
        .get(&provider_id)
        .copied()
        .or_else(|| {
            inputs
                .defaults
                .filter(|_| matches_initial)
                .map(|defaults| defaults.reasoning_level)
        })
        .filter(|level| level.is_known())
        .unwrap_or(ReasoningLevel::Medium);
    let verified = inputs
        .options
        .is_some_and(|options| options.model_load_error.is_none())
        && inputs.failed.is_none();
    let empty = Vec::new();
    let catalog = resolve_model_catalog_selection(
        inputs.options.map_or(&empty, |options| &options.models),
        inputs
            .options
            .map_or(&empty, |options| &options.selected_only_models),
        &raw_model,
        preferred_reasoning,
        provider,
        verified,
    );
    let model = catalog.active_model.as_ref().map_or_else(
        || catalog.selected_model.clone(),
        |active| active.model.clone(),
    );
    let model_label = catalog
        .model_options
        .iter()
        .find(|option| option.value == model)
        .map_or_else(|| format_model_label(&model), |option| option.label.clone());

    let modes: Vec<PermissionMode> = provider
        .map(|provider| provider.capabilities.permission_modes.clone())
        .filter(|modes| !modes.is_empty())
        .unwrap_or_else(|| vec![PermissionMode::Full]);
    let ceiling = inputs
        .options
        .map(|options| options.permission_ceiling)
        .filter(|ceiling| ceiling.is_known())
        .or(inputs.host_ceiling.filter(|ceiling| ceiling.is_known()))
        .unwrap_or(PermissionMode::Full);
    let ceiling_rank = permission_mode_rank(ceiling).unwrap_or(usize::MAX);
    let allowed: Vec<PermissionMode> = modes
        .iter()
        .copied()
        .filter(|mode| permission_mode_rank(*mode).is_some_and(|rank| rank <= ceiling_rank))
        .collect();
    let raw_permission = picks
        .permission
        .or_else(|| inputs.defaults.map(|defaults| defaults.permission_mode))
        .filter(|mode| mode.is_known())
        .unwrap_or(PermissionMode::Auto);
    let permission = resolve_permission_mode_selection(
        raw_permission,
        if allowed.is_empty() { &modes } else { &allowed },
    );
    let permission_options = PERMISSION_MODE_OPTIONS
        .iter()
        .filter(|option| modes.contains(&option.value))
        .map(|option| {
            let over = permission_mode_rank(option.value).is_some_and(|rank| rank > ceiling_rank);
            PickerOption {
                value: option.value,
                label: option.label.to_owned(),
                detail: Some(if over {
                    PERMISSION_CEILING_REASON.to_owned()
                } else {
                    option.description.to_owned()
                }),
                disabled: over,
            }
        })
        .collect();

    let provider_supports_tier =
        provider.is_some_and(|provider| provider.capabilities.supports_service_tier);
    let supports_service_tier = inputs.allow_fast_service_tier && provider_supports_tier;
    let tiers = if inputs.allow_fast_service_tier {
        service_tier_options(provider, catalog.active_model.as_ref())
    } else {
        Vec::new()
    };
    let raw_tier = picks
        .service_tier
        .clone()
        .or_else(|| {
            inputs
                .defaults
                .map(|defaults| defaults.service_tier.clone())
        })
        .filter(|tier| !tier.is_empty());
    let service_tier = if !provider_supports_tier {
        None
    } else if tiers.is_empty() {
        Some(DEFAULT_SERVICE_TIER.to_owned())
    } else {
        raw_tier.map(|tier| {
            if tiers.contains(&tier) {
                tier
            } else {
                DEFAULT_SERVICE_TIER.to_owned()
            }
        })
    };

    let touched = |field| picks.touched.contains(&field);
    let sources = CreateExecutionInputSources {
        provider_id: source(!provider_id.is_empty(), touched(CreationField::Provider)),
        model: source(
            !catalog.selected_model.is_empty(),
            catalog.unavailable_model_recovery || touched(CreationField::Model),
        ),
        service_tier: source(service_tier.is_some(), touched(CreationField::ServiceTier)),
        reasoning_level: source(true, touched(CreationField::Reasoning)),
        permission_mode: source(true, touched(CreationField::Permission)),
    };
    let model_load_error = inputs
        .options
        .and_then(|options| options.model_load_error.clone());
    let loading_models = inputs.loading || (inputs.options.is_none() && inputs.failed.is_none());

    CreationOptions {
        provider_label: provider
            .map(|provider| provider.display_name.clone())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| provider_id.clone()),
        provider_options: providers
            .iter()
            .map(|provider| PickerOption {
                value: provider.id.clone(),
                label: if provider.display_name.is_empty() {
                    provider.id.clone()
                } else {
                    provider.display_name.clone()
                },
                detail: None,
                disabled: false,
            })
            .collect(),
        provider_id,
        selected_model: catalog.selected_model.clone(),
        model,
        model_label,
        model_options: catalog.model_options,
        catalog_models: inputs
            .options
            .map(|options| {
                options
                    .models
                    .iter()
                    .chain(options.selected_only_models.iter())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default(),
        reasoning: catalog.reasoning,
        reasoning_options: catalog.reasoning_options,
        permission,
        permission_options,
        service_tier,
        supports_service_tier,
        unavailable_model_recovery: catalog.unavailable_model_recovery,
        loading_models,
        model_load_error,
        load_failure: inputs.failed.map(str::to_owned),
        sources,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CheckoutState {
    pub is_git: Option<bool>,
    pub dirty: bool,
    pub detached: bool,
    pub unborn: bool,
    pub operation: Option<(WorkspaceGitOperationKind, bool)>,
    pub current_branch: Option<String>,
}

impl CheckoutState {
    pub fn checking() -> Self {
        Self::default()
    }

    pub fn unknown() -> Self {
        Self {
            is_git: Some(false),
            ..Self::default()
        }
    }

    pub fn from_branches(response: &ProjectBranchesResponse) -> Self {
        let (current_branch, detached, unborn, resolved) = match &response.checkout {
            GitCheckoutRef::Branch { branch_name, .. } => {
                (Some(branch_name.clone()), false, false, true)
            }
            GitCheckoutRef::Detached { .. } => (None, true, false, true),
            GitCheckoutRef::Unborn { branch_name } => (branch_name.clone(), false, true, true),
            GitCheckoutRef::Unresolved { .. } | GitCheckoutRef::Unknown(_) => {
                (None, false, false, false)
            }
        };
        let operation = match response.operation.kind {
            WorkspaceGitOperationKind::None => None,
            WorkspaceGitOperationKind::Unknown if !response.operation.has_conflicts => None,
            kind => Some((kind, response.operation.has_conflicts)),
        };
        Self {
            is_git: Some(resolved),
            dirty: response.has_uncommitted_changes,
            detached,
            unborn,
            operation,
            current_branch: current_branch.filter(|_| !unborn),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckoutBlocker {
    pub label: String,
    pub reason: String,
    pub guidance: String,
}

fn blocker(label: &str, reason: &str, guidance: &str) -> Option<CheckoutBlocker> {
    Some(CheckoutBlocker {
        label: label.into(),
        reason: reason.into(),
        guidance: guidance.into(),
    })
}

fn operation_name(kind: WorkspaceGitOperationKind) -> &'static str {
    match kind {
        WorkspaceGitOperationKind::Merge => "Merge",
        WorkspaceGitOperationKind::Rebase => "Rebase",
        WorkspaceGitOperationKind::CherryPick => "Cherry-pick",
        WorkspaceGitOperationKind::Revert => "Revert",
        WorkspaceGitOperationKind::None => "",
        WorkspaceGitOperationKind::Unknown => "Operation",
    }
}

pub fn checkout_blocker(state: &CheckoutState) -> Option<CheckoutBlocker> {
    match state.is_git {
        None => {
            return blocker(
                "Checking",
                "Checking checkout state",
                "Checking whether branch changes are available…",
            );
        }
        Some(false) => {
            return blocker(
                "Unknown",
                "Checkout state is unavailable",
                "Branch changes are unavailable because this checkout could not be inspected.",
            );
        }
        Some(true) => {}
    }
    if let Some((kind, conflicts)) = state.operation {
        if conflicts {
            return blocker(
                "Conflicts",
                "Checkout blocked by unresolved conflicts",
                "Resolve the conflicts and finish or abort the Git operation to change branches.",
            );
        }
        let name = operation_name(kind);
        let lower = name.to_lowercase();
        return Some(CheckoutBlocker {
            label: name.into(),
            reason: format!("Checkout blocked by an in-progress {lower}"),
            guidance: format!("Finish or abort the {lower} to change branches."),
        });
    }
    if state.dirty {
        return blocker(
            "Dirty",
            "Checkout blocked by uncommitted changes",
            "Commit or stash the uncommitted changes in this checkout to create or switch branches.",
        );
    }
    if state.detached {
        return blocker(
            "Detached",
            "Checkout blocked while HEAD is detached",
            "Attach HEAD to a branch in this checkout to create or switch branches here.",
        );
    }
    if state.unborn {
        return blocker(
            "Empty repo",
            "Checkout blocked before the first commit",
            "Create the first commit in this checkout to create or switch branches.",
        );
    }
    None
}

pub fn current_checkout_label(state: &CheckoutState) -> String {
    if let Some(branch) = &state.current_branch {
        return format!("Current: {branch}");
    }
    if state.detached {
        return "Current (detached)".into();
    }
    if state.unborn {
        return "Current (empty repo)".into();
    }
    if state.is_git.is_none() {
        return "Checking checkout".into();
    }
    "Unknown checkout".into()
}

pub fn checkout_trigger_label(
    branch: Option<&UnmanagedBranchSpec>,
    state: &CheckoutState,
) -> String {
    match branch {
        Some(UnmanagedBranchSpec::Existing { name }) => format!("Checkout: {name}"),
        Some(UnmanagedBranchSpec::New { base_branch }) => format!("New branch from: {base_branch}"),
        None => match &state.current_branch {
            Some(branch) => format!("Current ({branch})"),
            None => current_checkout_label(state),
        },
    }
}

pub fn filter_branches(branches: &[String], query: &str) -> Vec<String> {
    let query = query.trim().to_lowercase();
    branches
        .iter()
        .filter(|branch| query.is_empty() || branch.to_lowercase().contains(&query))
        .cloned()
        .collect()
}

fn dedup_union(first: &[String], second: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for branch in first.iter().chain(second) {
        if !out.contains(branch) {
            out.push(branch.clone());
        }
    }
    out
}

pub fn worktree_branch_options(response: &ProjectBranchesResponse, query: &str) -> Vec<String> {
    filter_branches(
        &dedup_union(&response.branches, &response.remote_branches),
        query,
    )
}

pub fn checkout_branch_options(
    response: &ProjectBranchesResponse,
    new_branch: bool,
    query: &str,
    selected: Option<&str>,
) -> Vec<String> {
    let branches = if new_branch {
        dedup_union(&response.branches, &response.remote_branches)
    } else {
        response.branches.clone()
    };
    let filtered = filter_branches(&branches, query);
    let selected = selected.filter(|_| query.trim().is_empty());
    match selected {
        Some(selected) if filtered.iter().any(|branch| branch == selected) => {
            let mut ordered = vec![selected.to_owned()];
            ordered.extend(filtered.into_iter().filter(|branch| branch != selected));
            ordered
        }
        _ => filtered,
    }
}

pub fn worktree_inputs(branch: &GitBranchSelection) -> JsonValue {
    serde_json::json!({ "branch": branch })
}

pub fn checkout_inputs(path: Option<&str>, branch: Option<&UnmanagedBranchSpec>) -> JsonValue {
    let mut object = serde_json::Map::new();
    if let Some(path) = path {
        object.insert("path".into(), JsonValue::String(path.to_owned()));
    }
    if let Some(branch) = branch {
        object.insert(
            "branch".into(),
            serde_json::to_value(branch).unwrap_or(JsonValue::Null),
        );
    }
    JsonValue::Object(object)
}

pub fn worktree_trigger_label(branch: &GitBranchSelection, default_base: Option<&str>) -> String {
    match branch {
        GitBranchSelection::Named { name } => format!("Branch from: {name}"),
        GitBranchSelection::Default => {
            format!("Branch from: {}", default_base.unwrap_or("default"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bb_wire::environment::MachineLifecycle;
    use bb_wire::project::WorkspaceGitOperation;
    use bb_wire::system::{
        EnvironmentProviderRequirements, ModelReasoningEffort, ProviderCapabilities,
    };
    use std::collections::BTreeMap;

    fn project_provider(id: &str) -> SystemEnvironmentProvider {
        SystemEnvironmentProvider {
            machine_provider_id: None,
            id: id.into(),
            display_name: id.into(),
            plugin_id: id.into(),
            accepts_empty_inputs: true,
            requires: EnvironmentProviderRequirements {
                project_checkout: true,
                git_checkout: id == GIT_WORKTREE_PROVIDER_ID,
                git_remote: false,
                projectless: false,
            },
            ..SystemEnvironmentProvider::default()
        }
    }

    fn projectless_provider(id: &str, projectless: bool) -> SystemEnvironmentProvider {
        SystemEnvironmentProvider {
            requires: EnvironmentProviderRequirements {
                projectless,
                ..EnvironmentProviderRequirements::default()
            },
            ..project_provider(id)
        }
    }

    fn unavailable(provider: &SystemEnvironmentProvider) -> SystemEnvironmentProvider {
        SystemEnvironmentProvider {
            availability: Some(ProviderAvailability::Unavailable {
                message: "Not installed".into(),
            }),
            ..provider.clone()
        }
    }

    fn source(host: &str) -> ProjectSource {
        ProjectSource {
            id: "src_1".into(),
            project_id: "proj_app".into(),
            source_type: "local_path".into(),
            host_id: host.into(),
            path: "/repo".into(),
            is_default: true,
            ..ProjectSource::default()
        }
    }

    fn reuse_option(environment_id: &str) -> ReuseThreadOption {
        ReuseThreadOption {
            environment_id: environment_id.into(),
            branch_name: Some("feature".into()),
            name: None,
            path: None,
            environment_provider_id: Some(GIT_WORKTREE_PROVIDER_ID.into()),
            host_name: None,
            threads: vec![ReuseThread {
                id: "thr_1".into(),
                title: "Thread".into(),
            }],
        }
    }

    struct Ctx {
        hosts: HashSet<String>,
        providers: Option<Vec<SystemEnvironmentProvider>>,
        projectless: bool,
        sources: Vec<ProjectSource>,
        reuse: Vec<ReuseThreadOption>,
        reuse_loading: bool,
        seeded: Option<SeededReuse>,
    }

    impl Ctx {
        fn project(providers: Vec<SystemEnvironmentProvider>) -> Self {
            Self {
                hosts: HashSet::from(["host_1".to_owned()]),
                providers: Some(providers),
                projectless: false,
                sources: vec![source("host_1")],
                reuse: Vec::new(),
                reuse_loading: false,
                seeded: None,
            }
        }

        fn effective(&self, selection: &str) -> String {
            EnvironmentContext {
                selection,
                providers: self.providers.as_deref(),
                projectless: self.projectless,
                known_host_ids: &self.hosts,
                primary_host_id: Some("host_1"),
                sources: &self.sources,
                reuse_options: &self.reuse,
                reuse_loading: self.reuse_loading,
                seeded: self.seeded.as_ref(),
            }
            .effective_value()
        }
    }

    #[test]
    fn provider_values_round_trip_and_reject_malformed_ids() {
        assert_eq!(
            encode_provider_value("docker-sandbox"),
            "provider:docker-sandbox"
        );
        assert_eq!(
            parse_environment_value("provider:docker-sandbox"),
            Some(EnvironmentValue::Provider("docker-sandbox".into()))
        );
        assert_eq!(
            parse_environment_value(&encode_provider_value("wt_2")),
            Some(EnvironmentValue::Provider("wt_2".into()))
        );
        assert_eq!(parse_environment_value("provider:"), None);
        assert_eq!(parse_environment_value("provider:a/b"), None);
        assert_eq!(parse_environment_value("provider:bad*id"), None);
        assert_eq!(
            parse_environment_value(&format!("provider:{}", "a".repeat(65))),
            None
        );
        assert_eq!(
            parse_environment_value("reuse"),
            Some(EnvironmentValue::Reuse(None))
        );
        assert_eq!(parse_environment_value("reuse:"), None);
        assert_eq!(
            parse_environment_value(&encode_reuse_value("env_1")),
            Some(EnvironmentValue::Reuse(Some("env_1".into())))
        );
    }

    #[test]
    fn host_environment_provider_keeps_current_or_falls_back() {
        let checkout = project_provider(PROJECT_CHECKOUT_PROVIDER_ID);
        let worktree = project_provider(GIT_WORKTREE_PROVIDER_ID);
        assert_eq!(
            resolve_host_environment_provider(
                Some(&worktree),
                &[checkout.clone(), worktree.clone()]
            ),
            Some(worktree.clone())
        );
        let gone = unavailable(&worktree);
        assert_eq!(
            resolve_host_environment_provider(Some(&gone), &[checkout.clone(), gone.clone()]),
            Some(checkout.clone())
        );
        assert_eq!(
            resolve_host_environment_provider(Some(&worktree), &[]),
            Some(worktree.clone())
        );
        assert_eq!(resolve_host_environment_provider(None, &[]), None);
    }

    #[test]
    fn new_thread_host_provider_rejects_a_machine_without_usable_providers() {
        let checkout = project_provider(PROJECT_CHECKOUT_PROVIDER_ID);
        let worktree = project_provider(GIT_WORKTREE_PROVIDER_ID);
        assert_eq!(
            resolve_new_thread_host_environment_provider(
                Some(GIT_WORKTREE_PROVIDER_ID),
                &[checkout.clone(), worktree.clone()]
            ),
            Some(worktree.clone())
        );
        let gone = unavailable(&worktree);
        assert_eq!(
            resolve_new_thread_host_environment_provider(
                Some(GIT_WORKTREE_PROVIDER_ID),
                &[checkout.clone(), gone.clone()]
            ),
            Some(checkout)
        );
        assert_eq!(
            resolve_new_thread_host_environment_provider(Some(GIT_WORKTREE_PROVIDER_ID), &[gone]),
            None
        );
    }

    #[test]
    fn effective_value_falls_back_to_checkout_only_with_a_primary_source() {
        let checkout = project_provider(PROJECT_CHECKOUT_PROVIDER_ID);
        let worktree = project_provider(GIT_WORKTREE_PROVIDER_ID);
        let ctx = Ctx::project(vec![checkout.clone(), worktree]);
        assert_eq!(ctx.effective(""), "provider:project-checkout");

        let mut elsewhere = Ctx::project(vec![checkout]);
        elsewhere.hosts.insert("host_2".into());
        elsewhere.sources = vec![source("host_2")];
        assert_eq!(elsewhere.effective(""), "");

        let mut loading = Ctx::project(Vec::new());
        loading.providers = None;
        assert_eq!(loading.effective("provider:git-worktree"), "");
    }

    #[test]
    fn effective_value_keeps_registered_picks_and_drops_gone_ones() {
        let ctx = Ctx::project(vec![
            project_provider(PROJECT_CHECKOUT_PROVIDER_ID),
            project_provider(GIT_WORKTREE_PROVIDER_ID),
        ]);
        assert_eq!(
            ctx.effective("provider:git-worktree"),
            "provider:git-worktree"
        );
        assert_eq!(ctx.effective("provider:gone"), "provider:project-checkout");
    }

    #[test]
    fn effective_value_reuse_rules() {
        let mut ctx = Ctx::project(vec![project_provider(PROJECT_CHECKOUT_PROVIDER_ID)]);
        ctx.reuse = vec![reuse_option("env_current")];
        assert_eq!(ctx.effective("reuse:env_current"), "reuse:env_current");
        assert_eq!(
            ctx.effective("reuse:env_stale"),
            "provider:project-checkout"
        );
        assert_eq!(ctx.effective("reuse"), "reuse");
        ctx.reuse.clear();
        assert_eq!(ctx.effective("reuse"), "reuse");

        ctx.seeded = Some(SeededReuse {
            environment_id: "env_seeded".into(),
            status: SeededReuseStatus::Available,
        });
        assert_eq!(ctx.effective("reuse:env_seeded"), "reuse:env_seeded");
        ctx.seeded = Some(SeededReuse {
            environment_id: "env_seeded".into(),
            status: SeededReuseStatus::Pending,
        });
        assert_eq!(ctx.effective("reuse:env_seeded"), "reuse:env_seeded");
        ctx.seeded = Some(SeededReuse {
            environment_id: "env_seeded".into(),
            status: SeededReuseStatus::Missing,
        });
        assert_eq!(
            ctx.effective("reuse:env_seeded"),
            "provider:project-checkout"
        );
        ctx.seeded = Some(SeededReuse {
            environment_id: "env_other".into(),
            status: SeededReuseStatus::Available,
        });
        assert_eq!(
            ctx.effective("reuse:env_stale"),
            "provider:project-checkout"
        );

        ctx.seeded = None;
        ctx.reuse_loading = true;
        assert_eq!(ctx.effective("reuse:env_pending"), "reuse:env_pending");
    }

    #[test]
    fn effective_value_projectless_rules() {
        let mut ctx = Ctx::project(vec![projectless_provider(
            PERSONAL_WORKSPACE_PROVIDER_ID,
            true,
        )]);
        ctx.projectless = true;
        ctx.sources.clear();
        ctx.reuse = vec![reuse_option("env_personal")];
        assert_eq!(ctx.effective("reuse:env_personal"), "reuse:env_personal");
        assert_eq!(
            ctx.effective("reuse:env_gone"),
            "provider:personal-workspace"
        );

        ctx.reuse.clear();
        ctx.providers = Some(vec![
            projectless_provider("alpha-sandbox", true),
            projectless_provider(PERSONAL_WORKSPACE_PROVIDER_ID, true),
        ]);
        assert_eq!(ctx.effective(""), "provider:personal-workspace");

        ctx.providers = Some(vec![
            projectless_provider("modal-sandbox", false),
            projectless_provider(PERSONAL_WORKSPACE_PROVIDER_ID, true),
        ]);
        assert_eq!(
            ctx.effective("provider:modal-sandbox"),
            "provider:personal-workspace"
        );

        ctx.providers = None;
        assert_eq!(ctx.effective("provider:personal-workspace"), "");
    }

    fn branchy() -> SystemEnvironmentProvider {
        SystemEnvironmentProvider {
            id: "branchy".into(),
            accepts_empty_inputs: false,
            inputs: Some(serde_json::json!({"type": "object", "required": ["branch"]})),
            ..project_provider("branchy")
        }
    }

    fn hosted() -> SystemEnvironmentProvider {
        SystemEnvironmentProvider {
            requires: EnvironmentProviderRequirements::default(),
            ..project_provider("hosted")
        }
    }

    fn existing(host: &str) -> Option<MachineSelection> {
        Some(MachineSelection::Existing {
            host_id: host.into(),
        })
    }

    #[test]
    fn thread_environment_resolution() {
        let providers = vec![branchy(), hosted()];
        let modal = SystemEnvironmentProvider {
            id: "modal-sandbox".into(),
            machine_provider_id: Some("modal-sandbox".into()),
            ..hosted()
        };
        let json = |args: Option<CreateThreadEnvironmentArgs>| serde_json::to_value(args).unwrap();
        assert_eq!(
            json(resolve_thread_environment(
                "provider:modal-sandbox",
                "proj_123",
                Some(std::slice::from_ref(&modal)),
                None,
                None
            )),
            serde_json::json!({"type": "provider", "environmentProviderId": "modal-sandbox", "inputs": null})
        );
        assert_eq!(
            json(resolve_thread_environment(
                "provider:branchy",
                "proj_123",
                Some(&providers),
                existing("host_123"),
                Some(serde_json::json!({"branch": {"kind": "named", "name": "release"}}))
            )),
            serde_json::json!({
                "type": "provider",
                "environmentProviderId": "branchy",
                "machine": {"type": "existing", "hostId": "host_123"},
                "inputs": {"branch": {"kind": "named", "name": "release"}}
            })
        );
        assert_eq!(
            resolve_thread_environment(
                "provider:branchy",
                "proj_123",
                Some(&providers),
                existing("host_123"),
                None
            ),
            None
        );
        assert_eq!(
            resolve_thread_environment("provider:hosted", "proj_123", Some(&providers), None, None),
            None
        );
        assert_eq!(
            json(resolve_thread_environment(
                "provider:hosted",
                "proj_123",
                Some(&providers),
                existing("host_123"),
                Some(serde_json::json!({"image": "stale"}))
            )),
            serde_json::json!({
                "type": "provider",
                "environmentProviderId": "hosted",
                "machine": {"type": "existing", "hostId": "host_123"},
                "inputs": null
            })
        );
        assert_eq!(
            resolve_thread_environment("reuse:env_1", "proj_123", None, None, None),
            Some(CreateThreadEnvironmentArgs::Reuse {
                environment_id: "env_1".into()
            })
        );
        assert_eq!(
            resolve_thread_environment("reuse", "proj_123", None, None, None),
            None
        );
    }

    #[test]
    fn picker_disables_providers_with_reasons() {
        let controls = inputs_control_provider_ids(&[
            project_provider(GIT_WORKTREE_PROVIDER_ID),
            branchy(),
            SystemEnvironmentProvider {
                id: "composed".into(),
                environment_provider_id: Some(GIT_WORKTREE_PROVIDER_ID.into()),
                ..project_provider("composed")
            },
        ]);
        assert!(controls.contains(GIT_WORKTREE_PROVIDER_ID));
        assert!(controls.contains("composed"));
        assert!(!controls.contains("branchy"));
        assert_eq!(
            provider_disabled_reason(&branchy(), &controls).as_deref(),
            Some(PROVIDER_INPUTS_CONTROL_MISSING_REASON)
        );
        let empty_ok = SystemEnvironmentProvider {
            accepts_empty_inputs: true,
            ..branchy()
        };
        assert_eq!(provider_disabled_reason(&empty_ok, &controls), None);
        let gone = unavailable(&hosted());
        assert_eq!(
            provider_disabled_reason(&gone, &controls).as_deref(),
            Some("Not installed")
        );
        let setup = SystemEnvironmentProvider {
            availability: Some(ProviderAvailability::SetupRequired {
                message: "Configure the token".into(),
            }),
            ..hosted()
        };
        assert_eq!(provider_disabled_reason(&setup, &controls), None);
        assert_eq!(
            provider_description(&setup, &controls).as_deref(),
            Some("Configure the token")
        );
    }

    #[test]
    fn scoped_providers_hide_unavailable_unless_selected_here() {
        let checkout = project_provider(PROJECT_CHECKOUT_PROVIDER_ID);
        let worktree = project_provider(GIT_WORKTREE_PROVIDER_ID);
        let by_host = HashMap::from([(
            "host_1".to_owned(),
            vec![checkout.clone(), unavailable(&worktree)],
        )]);
        let all = vec![checkout.clone(), worktree.clone(), hosted()];
        let ids = |list: Vec<SystemEnvironmentProvider>| {
            list.into_iter().map(|p| p.id).collect::<Vec<_>>()
        };
        assert_eq!(
            ids(scoped_providers(
                &all,
                Some(&by_host),
                Some("host_1"),
                "",
                None
            )),
            vec![PROJECT_CHECKOUT_PROVIDER_ID]
        );
        assert_eq!(
            ids(scoped_providers(
                &all,
                Some(&by_host),
                Some("host_1"),
                "provider:git-worktree",
                Some("host_1")
            )),
            vec![PROJECT_CHECKOUT_PROVIDER_ID, GIT_WORKTREE_PROVIDER_ID]
        );
    }

    fn host(id: &str, connected: bool) -> Host {
        Host {
            id: id.into(),
            name: id.into(),
            host_type: HostType::Persistent,
            status: if connected {
                HostStatus::Connected
            } else {
                HostStatus::Disconnected
            },
            lifecycle: MachineLifecycle {
                phase: MachinePhase::Active,
                ..MachineLifecycle::default()
            },
            max_permission_mode: PermissionMode::Full,
            ..Host::default()
        }
    }

    #[test]
    fn providers_by_host_require_a_source_and_take_machine_availability() {
        let hosts = [host("host_1", true), host("host_2", true)];
        let host_refs: Vec<&Host> = hosts.iter().collect();
        let checkout = project_provider(PROJECT_CHECKOUT_PROVIDER_ID);
        let remote_only = SystemEnvironmentProvider {
            requires: EnvironmentProviderRequirements {
                git_remote: true,
                ..EnvironmentProviderRequirements::default()
            },
            ..project_provider("remote")
        };
        let mut project_view = checkout.clone();
        project_view.machine_availability = BTreeMap::from([(
            "host_1".to_owned(),
            Some(ProviderAvailability::Unavailable {
                message: "git missing".into(),
            }),
        )]);
        let sources = [source("host_1")];
        let providers = [checkout.clone(), remote_only.clone(), hosted()];
        let by_host = providers_by_host(&HostProviderInputs {
            hosts: &host_refs,
            providers: &providers,
            project_providers: Some(std::slice::from_ref(&project_view)),
            sources: &sources,
            has_git_remote: false,
        });
        let host_1: Vec<&str> = by_host["host_1"].iter().map(|p| p.id.as_str()).collect();
        let host_2: Vec<&str> = by_host["host_2"].iter().map(|p| p.id.as_str()).collect();
        assert_eq!(host_1, vec![PROJECT_CHECKOUT_PROVIDER_ID, "hosted"]);
        assert!(provider_unavailable(&by_host["host_1"][0]));
        assert_eq!(host_2, vec!["hosted"]);
    }

    #[test]
    fn primary_host_selection_skips_ephemeral_hosts() {
        let mut ephemeral = host("eph", true);
        ephemeral.host_type = HostType::Ephemeral;
        let hosts = vec![ephemeral, host("off", false), host("on", true)];
        assert_eq!(
            select_primary_host(&hosts, None).map(|h| h.id.as_str()),
            Some("on")
        );
        assert_eq!(
            select_primary_host(&hosts, Some("off")).map(|h| h.id.as_str()),
            Some("off")
        );
        assert_eq!(select_primary_host(&hosts, Some("eph")), None);
    }

    fn defaults() -> ProjectExecutionDefaults {
        ProjectExecutionDefaults {
            provider_id: "codex".into(),
            model: "gpt-5.6-sol".into(),
            service_tier: "default".into(),
            reasoning_level: ReasoningLevel::Medium,
            permission_mode: PermissionMode::Auto,
        }
    }

    #[test]
    fn project_defaults_state() {
        let stored = defaults();
        assert_eq!(
            resolve_project_defaults_state(None, true, DefaultsQuery::Pending),
            ProjectDefaultsState::Pending
        );
        assert_eq!(
            resolve_project_defaults_state(None, true, DefaultsQuery::Success(Some(&stored))),
            ProjectDefaultsState::Resolved(Some(stored.clone()))
        );
        assert_eq!(
            resolve_project_defaults_state(None, true, DefaultsQuery::Success(None)),
            ProjectDefaultsState::Resolved(None)
        );
        assert_eq!(
            resolve_project_defaults_state(None, true, DefaultsQuery::Error),
            ProjectDefaultsState::Error
        );
        assert_eq!(
            resolve_project_defaults_state(Some(&stored), false, DefaultsQuery::Error),
            ProjectDefaultsState::Resolved(Some(stored.clone()))
        );
        assert_eq!(
            resolve_project_defaults_state(None, false, DefaultsQuery::Success(None)),
            ProjectDefaultsState::Pending
        );
    }

    fn ready() -> SubmitReadiness<'static> {
        SubmitReadiness {
            project_defaults_status: Some(ProjectDefaultsStatus::Resolved),
            provider_label: "Codex",
            provider_id: "codex",
            model: "gpt-5.6-sol",
            ..SubmitReadiness::default()
        }
    }

    #[test]
    fn submit_disabled_reasons() {
        assert_eq!(resolve_submit_disabled_reason(&ready()), None);
        let setup =
            "Modal Sandbox is not configured: set tokenId, tokenSecret in the plugin's settings.";
        assert_eq!(
            resolve_submit_disabled_reason(&SubmitReadiness {
                environment_setup_required: Some(setup),
                ..ready()
            })
            .as_deref(),
            Some(setup)
        );
        let auth = ModelLoadError {
            provider_id: "codex".into(),
            code: ModelLoadErrorCode::AuthRequired,
            detail: None,
        };
        let timeout = ModelLoadError {
            provider_id: "claude-code".into(),
            code: ModelLoadErrorCode::Timeout,
            detail: None,
        };
        let cases: Vec<(SubmitReadiness, &str)> = vec![
            (
                SubmitReadiness {
                    loading_models: true,
                    ..ready()
                },
                "Loading models from the selected machine...",
            ),
            (
                SubmitReadiness {
                    model_load_error: Some(&auth),
                    ..ready()
                },
                "Could not load models for Codex. Not signed in.",
            ),
            (
                SubmitReadiness {
                    project_defaults_status: Some(ProjectDefaultsStatus::Error),
                    project_defaults_unavailable: true,
                    ..ready()
                },
                "Could not load the project's execution defaults.",
            ),
            (
                SubmitReadiness {
                    environment_unavailable: true,
                    ..ready()
                },
                "Select an environment.",
            ),
            (
                SubmitReadiness {
                    environment_inputs_blocker: Some("Configure Docker container"),
                    environment_unavailable: true,
                    ..ready()
                },
                "Configure Docker container",
            ),
            (
                SubmitReadiness {
                    prompt_empty: true,
                    ..ready()
                },
                "Enter a prompt or attach a file.",
            ),
        ];
        for (readiness, reason) in cases {
            assert_eq!(
                resolve_submit_disabled_reason(&readiness).as_deref(),
                Some(reason)
            );
        }
        assert_eq!(
            resolve_submit_disabled_reason(&SubmitReadiness {
                model_load_error: Some(&timeout),
                ..ready()
            }),
            None
        );
    }

    #[test]
    fn submitted_sources_force_an_explicit_model_for_new_machines() {
        let base = CreateExecutionInputSources::default();
        let reuse = CreateThreadEnvironmentArgs::Reuse {
            environment_id: "env".into(),
        };
        assert_eq!(
            resolve_submitted_execution_sources(&reuse, base.clone()),
            base
        );
        let on_host = CreateThreadEnvironmentArgs::Provider {
            environment_provider_id: "x".into(),
            machine: existing("h"),
            inputs: None,
        };
        assert_eq!(
            resolve_submitted_execution_sources(&on_host, base.clone()),
            base
        );
        let composed = CreateThreadEnvironmentArgs::Provider {
            environment_provider_id: "x".into(),
            machine: None,
            inputs: None,
        };
        assert_eq!(
            resolve_submitted_execution_sources(&composed, base).model,
            Some(ExecutionInputSource::Explicit)
        );
    }

    fn model(id: &str, default: bool, efforts: &[ReasoningLevel]) -> AvailableModel {
        AvailableModel {
            id: id.into(),
            model: id.into(),
            display_name: id.into(),
            supported_reasoning_efforts: efforts
                .iter()
                .map(|level| ModelReasoningEffort {
                    reasoning_effort: *level,
                    description: String::new(),
                })
                .collect(),
            is_default: default,
            ..AvailableModel::default()
        }
    }

    fn provider_info(id: &str, modes: &[PermissionMode]) -> ProviderInfo {
        ProviderInfo {
            id: id.into(),
            display_name: id.to_uppercase(),
            capabilities: ProviderCapabilities {
                permission_modes: modes.to_vec(),
                ..ProviderCapabilities::default()
            },
            ..ProviderInfo::default()
        }
    }

    fn catalog() -> SystemExecutionOptionsResponse {
        use ReasoningLevel as R;
        SystemExecutionOptionsResponse {
            providers: vec![
                provider_info(
                    "codex",
                    &[
                        PermissionMode::AcceptEdits,
                        PermissionMode::Auto,
                        PermissionMode::Full,
                    ],
                ),
                provider_info("claude", &[]),
            ],
            permission_ceiling: PermissionMode::Unknown,
            models: vec![
                model("gpt-5.6-sol", true, &[R::Low, R::Medium, R::High]),
                model("gpt-5-mini", false, &[R::Low]),
            ],
            selected_only_models: Vec::new(),
            model_load_error: None,
        }
    }

    #[test]
    fn creation_options_follow_project_defaults_and_mark_only_touched_fields() {
        let catalog = catalog();
        let defaults = defaults();
        let mut picks = CreationPicks::default();
        let inputs = |picks: &CreationPicks| -> CreationOptions {
            resolve_creation_options(&CreationInputs {
                options: Some(&catalog),
                defaults: Some(&defaults),
                picks: Some(picks),
                host_ceiling: Some(PermissionMode::Auto),
                allow_fast_service_tier: true,
                ..CreationInputs::default()
            })
        };
        let resolved = inputs(&picks);
        assert_eq!(resolved.provider_id, "codex");
        assert_eq!(resolved.model, "gpt-5.6-sol");
        assert_eq!(resolved.permission, PermissionMode::Auto);
        assert!(resolved.permission_options[2].disabled);
        assert_eq!(
            resolved.permission_options[2].detail.as_deref(),
            Some(PERMISSION_CEILING_REASON)
        );
        assert_eq!(resolved.sources, CreateExecutionInputSources::default());

        picks.select_model("gpt-5-mini".into(), &resolved);
        let resolved = inputs(&picks);
        assert_eq!(resolved.model, "gpt-5-mini");
        assert_eq!(resolved.reasoning, ReasoningLevel::Low);
        assert_eq!(resolved.sources.model, Some(ExecutionInputSource::Explicit));
        assert_eq!(resolved.sources.provider_id, None);
        assert_eq!(resolved.sources.permission_mode, None);

        picks.select_provider("claude".into(), &resolved);
        let switched = inputs(&picks);
        assert_eq!(switched.provider_id, "claude");
        assert_eq!(switched.model, "gpt-5.6-sol");
        assert_eq!(switched.reasoning, ReasoningLevel::Medium);
        assert_eq!(switched.permission, PermissionMode::Full);
        assert_eq!(
            switched.sources.provider_id,
            Some(ExecutionInputSource::Explicit)
        );
        picks.select_provider("codex".into(), &switched);
        let back = inputs(&picks);
        assert_eq!(back.model, "gpt-5-mini");
        assert_eq!(back.reasoning, ReasoningLevel::Low);
    }

    #[test]
    fn creation_options_recover_from_an_unavailable_default_model() {
        let catalog = catalog();
        let mut stale = defaults();
        stale.model = "retired".into();
        let resolved = resolve_creation_options(&CreationInputs {
            options: Some(&catalog),
            defaults: Some(&stale),
            allow_fast_service_tier: true,
            ..CreationInputs::default()
        });
        assert_eq!(resolved.model, "gpt-5.6-sol");
        assert!(resolved.unavailable_model_recovery);
        assert_eq!(resolved.sources.model, Some(ExecutionInputSource::Explicit));
    }

    #[test]
    fn creation_options_fall_back_to_the_first_provider_and_report_loading() {
        let loading = resolve_creation_options(&CreationInputs {
            loading: true,
            ..CreationInputs::default()
        });
        assert!(loading.loading_models);
        assert_eq!(loading.provider_id, "");
        let catalog = catalog();
        let mut gone = defaults();
        gone.provider_id = "removed".into();
        let resolved = resolve_creation_options(&CreationInputs {
            options: Some(&catalog),
            defaults: Some(&gone),
            ..CreationInputs::default()
        });
        assert_eq!(resolved.provider_id, "codex");
        assert_eq!(resolved.model, "gpt-5.6-sol");
        assert!(!resolved.unavailable_model_recovery);
    }

    fn branches(checkout: GitCheckoutRef) -> ProjectBranchesResponse {
        ProjectBranchesResponse {
            checkout,
            branches: vec!["main".into(), "feature".into()],
            remote_branches: vec!["origin/main".into(), "feature".into()],
            ..ProjectBranchesResponse::default()
        }
    }

    #[test]
    fn checkout_blockers_follow_plugin_priority() {
        let clean = CheckoutState::from_branches(&branches(GitCheckoutRef::Branch {
            branch_name: "main".into(),
            head_sha: None,
        }));
        assert_eq!(checkout_blocker(&clean), None);
        assert_eq!(current_checkout_label(&clean), "Current: main");
        assert_eq!(checkout_trigger_label(None, &clean), "Current (main)");
        assert_eq!(
            checkout_blocker(&CheckoutState::checking()).unwrap().reason,
            "Checking checkout state"
        );
        assert_eq!(
            checkout_blocker(&CheckoutState::unknown()).unwrap().label,
            "Unknown"
        );
        let mut dirty = branches(GitCheckoutRef::Detached { head_sha: None });
        dirty.has_uncommitted_changes = true;
        let state = CheckoutState::from_branches(&dirty);
        assert_eq!(
            checkout_blocker(&state).unwrap().reason,
            "Checkout blocked by uncommitted changes"
        );
        dirty.operation = WorkspaceGitOperation {
            kind: WorkspaceGitOperationKind::Rebase,
            has_conflicts: false,
            reason: None,
        };
        assert_eq!(
            checkout_blocker(&CheckoutState::from_branches(&dirty))
                .unwrap()
                .reason,
            "Checkout blocked by an in-progress rebase"
        );
        dirty.operation.has_conflicts = true;
        assert_eq!(
            checkout_blocker(&CheckoutState::from_branches(&dirty))
                .unwrap()
                .label,
            "Conflicts"
        );
        let detached =
            CheckoutState::from_branches(&branches(GitCheckoutRef::Detached { head_sha: None }));
        assert_eq!(checkout_blocker(&detached).unwrap().label, "Detached");
        assert_eq!(current_checkout_label(&detached), "Current (detached)");
        let unborn = CheckoutState::from_branches(&branches(GitCheckoutRef::Unborn {
            branch_name: Some("main".into()),
        }));
        assert_eq!(checkout_blocker(&unborn).unwrap().label, "Empty repo");
    }

    #[test]
    fn branch_option_lists_and_inputs() {
        let response = branches(GitCheckoutRef::default());
        assert_eq!(
            worktree_branch_options(&response, ""),
            vec!["main", "feature", "origin/main"]
        );
        assert_eq!(
            worktree_branch_options(&response, " MAI "),
            vec!["main", "origin/main"]
        );
        assert_eq!(
            checkout_branch_options(&response, false, "", Some("feature")),
            vec!["feature", "main"]
        );
        assert_eq!(
            checkout_branch_options(&response, true, "", None),
            vec!["main", "feature", "origin/main"]
        );
        assert_eq!(
            checkout_branch_options(&response, true, "fea", Some("main")),
            vec!["feature"]
        );
        assert_eq!(
            worktree_inputs(&GitBranchSelection::Default),
            serde_json::json!({"branch": {"kind": "default"}})
        );
        assert_eq!(
            worktree_inputs(&GitBranchSelection::Named {
                name: "release".into()
            }),
            serde_json::json!({"branch": {"kind": "named", "name": "release"}})
        );
        assert_eq!(checkout_inputs(None, None), serde_json::json!({}));
        assert_eq!(
            checkout_inputs(
                Some("/repo"),
                Some(&UnmanagedBranchSpec::New {
                    base_branch: "main".into()
                })
            ),
            serde_json::json!({"path": "/repo", "branch": {"kind": "new", "baseBranch": "main"}})
        );
        assert_eq!(
            worktree_trigger_label(&GitBranchSelection::Default, Some("main")),
            "Branch from: main"
        );
    }

    #[test]
    fn reuse_options_group_threads_by_environment() {
        let mut first = ThreadListEntry::default();
        first.thread.id = "thr_a".into();
        first.thread.title = Some("Older".into());
        first.thread.environment_id = Some("env_2".into());
        first.thread.latest_attention_at = 1;
        first.environment_branch_name = Some("zeta".into());
        first.environment_host_id = Some("host_1".into());
        let mut second = first.clone();
        second.thread.id = "thr_b".into();
        second.thread.title = Some("Newer".into());
        second.thread.latest_attention_at = 5;
        let mut third = ThreadListEntry::default();
        third.thread.id = "thr_c".into();
        third.thread.environment_id = Some("env_1".into());
        third.environment_name = Some("Alpha".into());
        let mut none = ThreadListEntry::default();
        none.thread.id = "thr_d".into();
        let names = HashMap::from([("host_1".to_owned(), "Laptop".to_owned())]);
        let options = build_reuse_thread_options([&first, &second, &third, &none], Some(&names));
        assert_eq!(options.len(), 2);
        assert_eq!(options[0].environment_id, "env_1");
        assert_eq!(options[0].label(), "Alpha");
        assert_eq!(options[1].label(), "zeta");
        assert_eq!(options[1].host_name.as_deref(), Some("Laptop"));
        let titles: Vec<&str> = options[1]
            .threads
            .iter()
            .map(|t| t.title.as_str())
            .collect();
        assert_eq!(titles, vec!["Newer", "Older"]);
    }
}
