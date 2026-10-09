use std::collections::{HashMap, HashSet};

use bb_store::StoreSnapshot;
use bb_wire::common::JsonValue;
use bb_wire::environment::{
    CreateThreadEnvironmentArgs, EnvironmentStatus, GitBranchSelection, Host, HostStatus,
    MachineSelection, UnmanagedBranchSpec,
};
use bb_wire::project::{ProjectBranchesQuery, ProjectBranchesResponse, ProjectWithThreadsResponse};
use bb_wire::prompt::{PermissionMode, ProjectExecutionDefaults, ReasoningLevel};
use bb_wire::system::{
    SystemEnvironmentProvider, SystemExecutionOptionsQuery, SystemExecutionOptionsResponse,
};
use bb_wire::thread::{CreateThreadRequest, ThreadResponse};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{AppState, Effect, Focus, ToastLevel};
use crate::compose::PickerItem;
use crate::composer::{Composer, ComposerKey};
use crate::creation::{
    CheckoutState, CreationInputs, CreationOptions, CreationPicks, DefaultsQuery,
    EnvironmentContext, EnvironmentValue, GIT_WORKTREE_PROVIDER_ID, HostProviderInputs,
    PROJECT_CHECKOUT_PROVIDER_ID, ProjectDefaultsState, ReuseThreadOption, SeededReuse,
    SeededReuseStatus, SubmitReadiness, build_reuse_thread_options, checkout_blocker,
    checkout_branch_options, checkout_inputs, checkout_trigger_label, current_checkout_label,
    encode_provider_value, encode_reuse_value, filter_environment_providers,
    find_local_path_source_for_host, host_selectable, inputs_control_provider_ids,
    is_projectless_project_id, parse_environment_value, persistent_hosts, provider_description,
    provider_disabled_reason, provider_inputs_control_required, provider_unavailable,
    providers_by_host, raw_provider_id, resolve_creation_options, resolve_project_defaults_state,
    resolve_submit_disabled_reason, resolve_submitted_execution_sources,
    resolve_thread_environment, scoped_providers, select_primary_host, worktree_branch_options,
    worktree_inputs, worktree_trigger_label,
};
use crate::execution::{Load, PickerOption};
use crate::input::{InputOutcome, LineInput};
use crate::keymap::Command;
use crate::prompt::PromptDraft;
use crate::sidebar::project_label;

pub const NEW_THREAD_COMPOSER_ID: &str = "new-thread";
const PROMPT_PLACEHOLDER: &str = "Describe the task for the new thread";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormField {
    Project,
    Environment,
    Branch,
    Provider,
    Model,
    Reasoning,
    Permission,
    Prompt,
}

impl FormField {
    pub const ALL: [FormField; 8] = [
        FormField::Project,
        FormField::Environment,
        FormField::Branch,
        FormField::Provider,
        FormField::Model,
        FormField::Reasoning,
        FormField::Permission,
        FormField::Prompt,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Project => "Project",
            Self::Environment => "Environment",
            Self::Branch => "Branch",
            Self::Provider => "Provider",
            Self::Model => "Model",
            Self::Reasoning => "Reasoning",
            Self::Permission => "Permissions",
            Self::Prompt => "Prompt",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Placement {
    pub section_id: Option<String>,
    pub pinned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CreationConfig {
    pub primary_host_id: Option<String>,
    pub allow_fast_service_tier: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FormPick {
    Nothing,
    Project(String),
    Environment {
        provider_id: String,
        host_id: Option<String>,
    },
    ReuseList,
    Reuse(String),
    Worktree(GitBranchSelection),
    Checkout(Option<UnmanagedBranchSpec>),
    Provider(String),
    Model(String),
    Reasoning(ReasoningLevel),
    Permission(PermissionMode),
}

#[derive(Debug, Clone, PartialEq)]
pub struct FormPicker {
    pub field: FormField,
    pub title: String,
    pub items: Vec<PickerItem>,
    pub values: Vec<FormPick>,
    pub selected: usize,
    pub filter: LineInput,
}

impl FormPicker {
    fn new(
        field: FormField,
        title: impl Into<String>,
        entries: Vec<(PickerItem, FormPick)>,
    ) -> Self {
        let (items, values): (Vec<_>, Vec<_>) = entries.into_iter().unzip();
        let mut picker = Self {
            field,
            title: title.into(),
            items,
            values,
            selected: 0,
            filter: LineInput::default(),
        };
        picker.selected = picker
            .visible()
            .iter()
            .position(|index| picker.items[*index].current)
            .unwrap_or(0);
        picker
    }

    pub fn visible(&self) -> Vec<usize> {
        let query = self.filter.text().trim().to_lowercase();
        self.items
            .iter()
            .enumerate()
            .filter(|(_, item)| query.is_empty() || item.label.to_lowercase().contains(&query))
            .map(|(index, _)| index)
            .collect()
    }

    fn step(&mut self, delta: isize) {
        let count = self.visible().len();
        if count == 0 {
            self.selected = 0;
            return;
        }
        self.selected = (self.selected as isize + delta).rem_euclid(count as isize) as usize;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BranchKey {
    pub project_id: String,
    pub host_id: String,
}

#[derive(Debug, Clone)]
pub struct NewThreadForm {
    pub generation: u64,
    pub project_id: String,
    pub field: FormField,
    pub prompt_active: bool,
    pub environment_value: String,
    pub picked_machine: Option<(String, MachineSelection)>,
    pub seeded_reuse: Option<SeededReuse>,
    pub placement: Placement,
    pub registered: Load<Vec<SystemEnvironmentProvider>>,
    pub project_providers: Option<(String, Load<Vec<SystemEnvironmentProvider>>)>,
    pub config: Load<CreationConfig>,
    pub defaults: Option<(String, Load<Option<ProjectExecutionDefaults>>)>,
    pub branches: Option<(BranchKey, Load<ProjectBranchesResponse>)>,
    pub worktree_branch: GitBranchSelection,
    pub checkout_branch: Option<UnmanagedBranchSpec>,
    pub catalog: Option<(
        SystemExecutionOptionsQuery,
        Load<SystemExecutionOptionsResponse>,
    )>,
    pub picks: CreationPicks,
    pub composer: Composer,
    pub picker: Option<FormPicker>,
    pub submitting: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NewThreadRequest {
    Providers {
        project_id: Option<String>,
    },
    Config,
    ProjectDefaults {
        project_id: String,
    },
    Branches {
        project_id: String,
        query: ProjectBranchesQuery,
    },
    Catalog {
        query: SystemExecutionOptionsQuery,
    },
    Create(Box<CreateThreadRequest>),
}

#[derive(Debug, Clone)]
pub enum NewThreadLoaded {
    Providers {
        project_id: Option<String>,
        result: Result<Vec<SystemEnvironmentProvider>, String>,
    },
    Config(Result<CreationConfig, String>),
    ProjectDefaults {
        project_id: String,
        result: Result<Option<ProjectExecutionDefaults>, String>,
    },
    Branches {
        project_id: String,
        host_id: String,
        result: Result<Box<ProjectBranchesResponse>, String>,
    },
    Catalog {
        query: SystemExecutionOptionsQuery,
        result: Result<Box<SystemExecutionOptionsResponse>, String>,
    },
    Created(Result<Box<ThreadResponse>, String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchControl {
    Worktree,
    Checkout,
}

#[derive(Debug, Clone)]
pub struct Resolved {
    pub project_label: String,
    pub project_found: bool,
    pub projectless: bool,
    pub hosts: Vec<Host>,
    pub source_host_ids: HashSet<String>,
    pub primary_host_id: Option<String>,
    pub environment_providers: Option<Vec<SystemEnvironmentProvider>>,
    pub by_host: HashMap<String, Vec<SystemEnvironmentProvider>>,
    pub controls: HashSet<String>,
    pub reuse_options: Vec<ReuseThreadOption>,
    pub effective_value: String,
    pub provider: Option<SystemEnvironmentProvider>,
    pub machine: Option<MachineSelection>,
    pub machine_unavailable: bool,
    pub input_provider: Option<SystemEnvironmentProvider>,
    pub branch_control: Option<BranchControl>,
    pub checkout_state: Option<CheckoutState>,
    pub inputs: Option<JsonValue>,
    pub inputs_blocker: Option<String>,
    pub environment: Option<CreateThreadEnvironmentArgs>,
    pub routing: SystemExecutionOptionsQuery,
    pub defaults_state: ProjectDefaultsState,
    pub options: CreationOptions,
    pub submit_reason: Option<String>,
}

impl Resolved {
    pub fn host_name(&self, host_id: &str) -> Option<&str> {
        self.hosts
            .iter()
            .find(|host| host.id == host_id)
            .map(|host| host.name.as_str())
            .filter(|name| !name.is_empty())
    }

    pub fn environment_label(&self) -> String {
        match parse_environment_value(&self.effective_value) {
            Some(EnvironmentValue::Provider(_)) => {
                let name = self
                    .provider
                    .as_ref()
                    .map_or("Environment".to_owned(), |provider| {
                        provider.display_name.clone()
                    });
                match &self.machine {
                    Some(MachineSelection::Existing { host_id }) if self.hosts.len() > 1 => {
                        let host = self.host_name(host_id).unwrap_or(host_id);
                        format!("{host} · {name}")
                    }
                    _ => name,
                }
            }
            Some(EnvironmentValue::Reuse(Some(environment_id))) => {
                let label = self
                    .reuse_options
                    .iter()
                    .find(|option| option.environment_id == environment_id)
                    .map_or(environment_id.as_str(), ReuseThreadOption::label);
                format!("Reuse · {label}")
            }
            Some(EnvironmentValue::Reuse(None)) => "Reuse existing (pick one)".into(),
            None => "Select an environment".into(),
        }
    }

    pub fn branch_label(&self, form: &NewThreadForm) -> Option<String> {
        let control = self.branch_control?;
        let response = form.branches.as_ref().and_then(|(_, load)| load.loaded());
        Some(match control {
            BranchControl::Worktree => {
                let base = response.and_then(|response| {
                    response
                        .default_worktree_base_branch
                        .as_deref()
                        .or(response.default_branch.as_deref())
                });
                worktree_trigger_label(&form.worktree_branch, base)
            }
            BranchControl::Checkout => checkout_trigger_label(
                form.checkout_branch.as_ref(),
                self.checkout_state
                    .as_ref()
                    .unwrap_or(&CheckoutState::checking()),
            ),
        })
    }
}

fn find_project<'a>(
    snapshot: &'a StoreSnapshot,
    project_id: &str,
) -> Option<&'a ProjectWithThreadsResponse> {
    let sidebar = snapshot.sidebar.data.as_deref()?;
    sidebar
        .projects
        .iter()
        .chain(std::iter::once(&sidebar.personal_project))
        .find(|project| project.project.id == project_id)
}

impl NewThreadForm {
    pub fn new(generation: u64, project_id: String, draft: Option<PromptDraft>) -> Self {
        let mut composer = Composer::new(NEW_THREAD_COMPOSER_ID);
        composer.set_placeholder(PROMPT_PLACEHOLDER);
        composer.draft_loaded = true;
        if let Some(draft) = draft {
            composer.restore_if_empty(draft, std::time::Instant::now());
            composer.dirty_since = None;
        }
        Self {
            generation,
            project_id,
            field: FormField::Project,
            prompt_active: false,
            environment_value: String::new(),
            picked_machine: None,
            seeded_reuse: None,
            placement: Placement::default(),
            registered: Load::Idle,
            project_providers: None,
            config: Load::Idle,
            defaults: None,
            branches: None,
            worktree_branch: GitBranchSelection::Default,
            checkout_branch: None,
            catalog: None,
            picks: CreationPicks::default(),
            composer,
            picker: None,
            submitting: false,
            error: None,
        }
    }

    pub fn activate_prompt(&mut self) {
        self.field = FormField::Prompt;
        self.prompt_active = true;
    }

    fn config(&self) -> CreationConfig {
        self.config.loaded().cloned().unwrap_or(CreationConfig {
            primary_host_id: None,
            allow_fast_service_tier: true,
        })
    }

    fn defaults_state(&self, project: Option<&ProjectWithThreadsResponse>) -> ProjectDefaultsState {
        let query = match self
            .defaults
            .as_ref()
            .filter(|(project_id, _)| *project_id == self.project_id)
            .map(|(_, load)| load)
        {
            Some(Load::Loaded(defaults)) => DefaultsQuery::Success(defaults.as_ref()),
            Some(Load::Failed(_)) => DefaultsQuery::Error,
            _ => DefaultsQuery::Pending,
        };
        resolve_project_defaults_state(
            project.and_then(|project| project.default_execution_options.as_ref()),
            project.is_some(),
            query,
        )
    }

    pub fn resolve(&self, snapshot: &StoreSnapshot) -> Resolved {
        let project = find_project(snapshot, &self.project_id);
        let projectless = is_projectless_project_id(&self.project_id);
        let all_hosts: &[Host] = snapshot.hosts.data.as_deref().map_or(&[], Vec::as_slice);
        let hosts: Vec<Host> = persistent_hosts(all_hosts).into_iter().cloned().collect();
        let host_refs: Vec<&Host> = hosts.iter().collect();
        let config = self.config();
        let primary_host_id = select_primary_host(all_hosts, config.primary_host_id.as_deref())
            .map(|host| host.id.clone());
        let known_host_ids: HashSet<String> = hosts.iter().map(|host| host.id.clone()).collect();
        let sources = project.map_or(&[][..], |project| project.sources.as_slice());
        let environment_providers = self
            .registered
            .loaded()
            .map(|registered| filter_environment_providers(registered, projectless));
        let project_providers = self
            .project_providers
            .as_ref()
            .filter(|(project_id, _)| *project_id == self.project_id)
            .and_then(|(_, load)| load.loaded())
            .map(Vec::as_slice);
        let by_host = providers_by_host(&HostProviderInputs {
            hosts: &host_refs,
            providers: environment_providers.as_deref().unwrap_or(&[]),
            project_providers,
            sources,
            has_git_remote: project.is_some_and(|project| project.project.git_remote_url.is_some()),
        });
        let controls = inputs_control_provider_ids(environment_providers.as_deref().unwrap_or(&[]));
        let host_names: Option<HashMap<String, String>> = (hosts.len() > 1).then(|| {
            hosts
                .iter()
                .map(|host| (host.id.clone(), host.name.clone()))
                .collect()
        });
        let reuse_options = build_reuse_thread_options(
            project.map_or(&[][..], |project| project.threads.as_slice()),
            host_names.as_ref(),
        );
        let effective_value = EnvironmentContext {
            selection: &self.environment_value,
            providers: environment_providers.as_deref(),
            projectless,
            known_host_ids: &known_host_ids,
            primary_host_id: primary_host_id.as_deref(),
            sources,
            reuse_options: &reuse_options,
            reuse_loading: snapshot.sidebar.data.is_none(),
            seeded: self.seeded_reuse.as_ref(),
        }
        .effective_value();

        let provider = match parse_environment_value(&effective_value) {
            Some(EnvironmentValue::Provider(id)) => environment_providers
                .as_ref()
                .and_then(|list| list.iter().find(|provider| provider.id == id))
                .cloned(),
            _ => None,
        };
        let usable = |host_id: &str, provider_id: &str| {
            known_host_ids.contains(host_id)
                && by_host.get(host_id).is_some_and(|list| {
                    list.iter().any(|candidate| {
                        candidate.id == provider_id && !provider_unavailable(candidate)
                    })
                })
        };
        let machine = provider.as_ref().and_then(|provider| {
            if provider.machine_provider_id.is_some() {
                return None;
            }
            if let Some((value, machine)) = &self.picked_machine
                && *value == effective_value
            {
                return Some(machine.clone());
            }
            primary_host_id
                .as_deref()
                .filter(|primary| usable(primary, &provider.id))
                .map(|primary| MachineSelection::Existing {
                    host_id: primary.to_owned(),
                })
        });
        let provider_host_id = match &machine {
            Some(MachineSelection::Existing { host_id }) => Some(host_id.clone()),
            _ => None,
        };
        let machine_unavailable = match (&provider_host_id, &provider) {
            (Some(host_id), Some(provider)) => !by_host.get(host_id).is_some_and(|list| {
                list.iter().any(|candidate| {
                    candidate.id == provider.id && !provider_unavailable(candidate)
                })
            }),
            _ => false,
        };
        let input_provider =
            provider
                .as_ref()
                .and_then(|provider| match &provider.environment_provider_id {
                    Some(inner) => environment_providers
                        .as_ref()
                        .and_then(|list| list.iter().find(|candidate| &candidate.id == inner))
                        .cloned(),
                    None => Some(provider.clone()),
                });
        let branch_control = input_provider.as_ref().and_then(|provider| {
            provider_host_id.as_ref()?;
            match provider.id.as_str() {
                GIT_WORKTREE_PROVIDER_ID => Some(BranchControl::Worktree),
                PROJECT_CHECKOUT_PROVIDER_ID => Some(BranchControl::Checkout),
                _ => None,
            }
        });
        let branch_load = match (&self.branches, &provider_host_id) {
            (Some((key, load)), Some(host_id))
                if key.host_id == *host_id && key.project_id == self.project_id =>
            {
                Some(load)
            }
            _ => None,
        };
        let checkout_state =
            (branch_control == Some(BranchControl::Checkout)).then(|| match branch_load {
                Some(Load::Loaded(response)) => CheckoutState::from_branches(response),
                Some(Load::Failed(_)) => CheckoutState::unknown(),
                _ => CheckoutState::checking(),
            });
        let (inputs, inputs_blocker) = match &input_provider {
            Some(provider) if provider.inputs.is_some() => match branch_control {
                Some(BranchControl::Worktree) => {
                    (Some(worktree_inputs(&self.worktree_branch)), None)
                }
                Some(BranchControl::Checkout) => {
                    let blocked = self
                        .checkout_branch
                        .as_ref()
                        .and_then(|_| checkout_state.as_ref().and_then(checkout_blocker));
                    match blocked {
                        Some(blocker) => (None, Some(blocker.reason)),
                        None => (
                            Some(checkout_inputs(None, self.checkout_branch.as_ref())),
                            None,
                        ),
                    }
                }
                None if controls.contains(&provider.id) => {
                    (Some(JsonValue::Object(serde_json::Map::new())), None)
                }
                None if provider_inputs_control_required(provider) => (
                    None,
                    Some(format!(
                        "{} needs its plugin's control",
                        provider.display_name
                    )),
                ),
                None => (Some(JsonValue::Object(serde_json::Map::new())), None),
            },
            _ => (None, None),
        };
        let environment = if machine_unavailable {
            None
        } else {
            resolve_thread_environment(
                &effective_value,
                &self.project_id,
                environment_providers.as_deref(),
                machine.clone(),
                inputs.clone(),
            )
        };
        let routing = match (&provider, &machine) {
            (Some(_), Some(MachineSelection::Existing { host_id })) => {
                SystemExecutionOptionsQuery {
                    host_id: Some(host_id.clone()),
                    ..SystemExecutionOptionsQuery::default()
                }
            }
            (Some(_), _) => SystemExecutionOptionsQuery::default(),
            (None, _) => match parse_environment_value(&effective_value) {
                Some(EnvironmentValue::Reuse(Some(environment_id))) => {
                    SystemExecutionOptionsQuery {
                        environment_id: Some(environment_id),
                        ..SystemExecutionOptionsQuery::default()
                    }
                }
                _ => SystemExecutionOptionsQuery::default(),
            },
        };
        let defaults_state = self.defaults_state(project);
        let defaults = defaults_state.defaults();
        let catalog = self
            .catalog
            .as_ref()
            .filter(|(query, _)| *query == self.catalog_query(&routing, defaults))
            .map(|(_, load)| load);
        let routed_host = routing.host_id.clone().or_else(|| primary_host_id.clone());
        let host_ceiling = routed_host.and_then(|host_id| {
            hosts
                .iter()
                .find(|host| host.id == host_id)
                .map(|host| host.max_permission_mode)
        });
        let options = resolve_creation_options(&CreationInputs {
            options: catalog.and_then(Load::loaded),
            loading: catalog.is_none_or(|load| matches!(load, Load::Loading | Load::Idle)),
            failed: match catalog {
                Some(Load::Failed(error)) => Some(error.as_str()),
                _ => None,
            },
            defaults,
            picks: Some(&self.picks),
            host_ceiling,
            allow_fast_service_tier: config.allow_fast_service_tier,
        });
        let prompt_empty = self.composer.draft().to_input().is_empty();
        let submit_reason = resolve_submit_disabled_reason(&SubmitReadiness {
            environment_inputs_blocker: inputs_blocker.as_deref(),
            environment_setup_required: None,
            loading_models: options.loading_models,
            submitting: self.submitting,
            model_load_error: options.model_load_error.as_ref(),
            project_defaults_status: Some(defaults_state.status()),
            project_defaults_unavailable: !matches!(
                defaults_state,
                ProjectDefaultsState::Resolved(_)
            ),
            prompt_empty,
            provider_label: &options.provider_label,
            provider_id: &options.provider_id,
            model: &options.model,
            environment_unavailable: environment.is_none(),
        });
        Resolved {
            project_label: project.map_or_else(|| self.project_id.clone(), project_label),
            project_found: project.is_some(),
            projectless,
            source_host_ids: hosts
                .iter()
                .filter(|host| find_local_path_source_for_host(sources, &host.id).is_some())
                .map(|host| host.id.clone())
                .collect(),
            hosts,
            primary_host_id,
            environment_providers,
            by_host,
            controls,
            reuse_options,
            effective_value,
            provider,
            machine,
            machine_unavailable,
            input_provider,
            branch_control,
            checkout_state,
            inputs,
            inputs_blocker,
            environment,
            routing,
            defaults_state,
            options,
            submit_reason,
        }
    }

    fn catalog_query(
        &self,
        routing: &SystemExecutionOptionsQuery,
        defaults: Option<&ProjectExecutionDefaults>,
    ) -> SystemExecutionOptionsQuery {
        let provider = raw_provider_id(Some(&self.picks), defaults);
        SystemExecutionOptionsQuery {
            provider_id: (!provider.is_empty()).then_some(provider),
            ..routing.clone()
        }
    }

    pub fn sync(&mut self, snapshot: &StoreSnapshot) -> Vec<NewThreadRequest> {
        let mut requests = Vec::new();
        if self.registered.is_idle() {
            self.registered = Load::Loading;
            requests.push(NewThreadRequest::Providers { project_id: None });
        }
        if self.config.is_idle() {
            self.config = Load::Loading;
            requests.push(NewThreadRequest::Config);
        }
        if self
            .project_providers
            .as_ref()
            .is_none_or(|(project_id, _)| *project_id != self.project_id)
        {
            self.project_providers = Some((self.project_id.clone(), Load::Loading));
            requests.push(NewThreadRequest::Providers {
                project_id: Some(self.project_id.clone()),
            });
        }
        let project = find_project(snapshot, &self.project_id);
        let cached = project.is_some_and(|project| project.default_execution_options.is_some());
        if project.is_some()
            && !cached
            && self
                .defaults
                .as_ref()
                .is_none_or(|(project_id, _)| *project_id != self.project_id)
        {
            self.defaults = Some((self.project_id.clone(), Load::Loading));
            requests.push(NewThreadRequest::ProjectDefaults {
                project_id: self.project_id.clone(),
            });
        }
        let resolved = self.resolve(snapshot);
        if !matches!(resolved.defaults_state, ProjectDefaultsState::Pending) {
            let query = self.catalog_query(&resolved.routing, resolved.defaults_state.defaults());
            if self
                .catalog
                .as_ref()
                .is_none_or(|(current, _)| *current != query)
            {
                self.catalog = Some((query.clone(), Load::Loading));
                requests.push(NewThreadRequest::Catalog { query });
            }
        }
        if resolved.branch_control.is_some()
            && let Some(MachineSelection::Existing { host_id }) = &resolved.machine
        {
            let key = BranchKey {
                project_id: self.project_id.clone(),
                host_id: host_id.clone(),
            };
            if self
                .branches
                .as_ref()
                .is_none_or(|(current, _)| *current != key)
            {
                self.branches = Some((key, Load::Loading));
                requests.push(NewThreadRequest::Branches {
                    project_id: self.project_id.clone(),
                    query: ProjectBranchesQuery {
                        host_id: host_id.clone(),
                        ..ProjectBranchesQuery::default()
                    },
                });
            }
        }
        requests
    }

    pub fn apply_loaded(&mut self, loaded: NewThreadLoaded) {
        match loaded {
            NewThreadLoaded::Providers {
                project_id: None,
                result,
            } => self.registered = Load::from_result(result),
            NewThreadLoaded::Providers {
                project_id: Some(project_id),
                result,
            } => {
                if let Some((current, load)) = &mut self.project_providers
                    && *current == project_id
                {
                    *load = Load::from_result(result);
                }
            }
            NewThreadLoaded::Config(result) => self.config = Load::from_result(result),
            NewThreadLoaded::ProjectDefaults { project_id, result } => {
                if let Some((current, load)) = &mut self.defaults
                    && *current == project_id
                {
                    *load = Load::from_result(result);
                }
            }
            NewThreadLoaded::Branches {
                project_id,
                host_id,
                result,
            } => {
                if let Some((key, load)) = &mut self.branches
                    && key.project_id == project_id
                    && key.host_id == host_id
                {
                    *load = Load::from_result(result.map(|response| *response));
                }
            }
            NewThreadLoaded::Catalog { query, result } => {
                if let Some((current, load)) = &mut self.catalog
                    && *current == query
                {
                    *load = Load::from_result(result.map(|response| *response));
                }
            }
            NewThreadLoaded::Created(_) => {}
        }
    }

    pub fn field_visible(&self, resolved: &Resolved, field: FormField) -> bool {
        field != FormField::Branch || resolved.branch_control.is_some()
    }

    fn move_field(&mut self, resolved: &Resolved, forward: bool) {
        let fields: Vec<FormField> = FormField::ALL
            .into_iter()
            .filter(|field| self.field_visible(resolved, *field))
            .collect();
        let index = fields
            .iter()
            .position(|field| *field == self.field)
            .unwrap_or(0);
        let next = if forward {
            (index + 1) % fields.len()
        } else {
            (index + fields.len() - 1) % fields.len()
        };
        self.field = fields[next];
        self.prompt_active = self.field == FormField::Prompt;
    }

    fn reset_environment(&mut self) {
        self.environment_value.clear();
        self.picked_machine = None;
        self.reset_inputs();
    }

    fn reset_inputs(&mut self) {
        self.worktree_branch = GitBranchSelection::Default;
        self.checkout_branch = None;
    }

    fn set_project(&mut self, project_id: String) {
        if project_id == self.project_id {
            return;
        }
        self.project_id = project_id;
        self.reset_environment();
        self.seeded_reuse = None;
        self.placement = Placement::default();
    }

    pub fn seed_reuse(&mut self, environment_id: String, placement: Placement) {
        self.environment_value = encode_reuse_value(&environment_id);
        self.seeded_reuse = Some(SeededReuse {
            environment_id,
            status: SeededReuseStatus::Available,
        });
        self.placement = placement;
    }

    fn apply_pick(&mut self, pick: FormPick, options: &CreationOptions) -> bool {
        match pick {
            FormPick::Nothing => return false,
            FormPick::Project(project_id) => self.set_project(project_id),
            FormPick::Environment {
                provider_id,
                host_id,
            } => {
                let value = encode_provider_value(&provider_id);
                self.environment_value = value.clone();
                self.picked_machine =
                    host_id.map(|host_id| (value, MachineSelection::Existing { host_id }));
                self.reset_inputs();
            }
            FormPick::ReuseList => return false,
            FormPick::Reuse(environment_id) => {
                self.environment_value = encode_reuse_value(&environment_id);
                self.picked_machine = None;
                self.reset_inputs();
            }
            FormPick::Worktree(branch) => self.worktree_branch = branch,
            FormPick::Checkout(branch) => self.checkout_branch = branch,
            FormPick::Provider(provider_id) => self.picks.select_provider(provider_id, options),
            FormPick::Model(model) => self.picks.select_model(model, options),
            FormPick::Reasoning(level) => self.picks.select_reasoning(level, options),
            FormPick::Permission(mode) => self.picks.select_permission(mode),
        }
        true
    }
}

fn item(
    label: impl Into<String>,
    detail: Option<String>,
    disabled: bool,
    current: bool,
) -> PickerItem {
    PickerItem {
        label: label.into(),
        detail,
        disabled,
        current,
    }
}

fn option_entries<T: Clone + PartialEq>(
    options: &[PickerOption<T>],
    current: &T,
    wrap: impl Fn(T) -> FormPick,
) -> Vec<(PickerItem, FormPick)> {
    options
        .iter()
        .map(|option| {
            (
                item(
                    option.label.clone(),
                    option.detail.clone(),
                    option.disabled,
                    option.value == *current,
                ),
                wrap(option.value.clone()),
            )
        })
        .collect()
}

fn project_entries(snapshot: &StoreSnapshot, current: &str) -> Vec<(PickerItem, FormPick)> {
    let Some(sidebar) = snapshot.sidebar.data.as_deref() else {
        return Vec::new();
    };
    sidebar
        .projects
        .iter()
        .chain(std::iter::once(&sidebar.personal_project))
        .filter(|project| !project.project.id.is_empty())
        .map(|project| {
            (
                item(
                    project_label(project),
                    None,
                    false,
                    project.project.id == current,
                ),
                FormPick::Project(project.project.id.clone()),
            )
        })
        .collect()
}

fn environment_entries(resolved: &Resolved) -> Vec<(PickerItem, FormPick)> {
    let mut entries = Vec::new();
    let Some(providers) = &resolved.environment_providers else {
        return entries;
    };
    let hosted: Vec<SystemEnvironmentProvider> = providers
        .iter()
        .filter(|provider| {
            provider.machine_provider_id.is_none()
                && provider.requires.projectless == resolved.projectless
        })
        .cloned()
        .collect();
    let selected_host = match &resolved.machine {
        Some(MachineSelection::Existing { host_id }) => Some(host_id.as_str()),
        _ => None,
    };
    let mut hosts: Vec<&Host> = resolved.hosts.iter().collect();
    hosts.sort_by_key(|host| Some(host.id.as_str()) != resolved.primary_host_id.as_deref());
    let multiple = hosts.len() > 1;
    for host in hosts {
        let connected = host.status == HostStatus::Connected;
        let selectable = host_selectable(host);
        let scoped = scoped_providers(
            &hosted,
            Some(&resolved.by_host),
            Some(&host.id),
            &resolved.effective_value,
            selected_host,
        );
        let host_label = if host.name.is_empty() {
            &host.id
        } else {
            &host.name
        };
        for provider in &scoped {
            let reason = provider_disabled_reason(provider, &resolved.controls);
            let detail = if !connected {
                Some("Machine is offline".to_owned())
            } else if !selectable {
                Some("Machine is not ready".to_owned())
            } else {
                provider_description(provider, &resolved.controls)
            };
            let label = if multiple {
                format!("{host_label} · {}", provider.display_name)
            } else {
                provider.display_name.clone()
            };
            let current = resolved.effective_value == encode_provider_value(&provider.id)
                && selected_host == Some(host.id.as_str());
            entries.push((
                item(label, detail, !selectable || reason.is_some(), current),
                FormPick::Environment {
                    provider_id: provider.id.clone(),
                    host_id: Some(host.id.clone()),
                },
            ));
        }
        if scoped.is_empty() {
            let detail = if resolved.source_host_ids.contains(&host.id) {
                "No environment options on this machine"
            } else {
                "Not set up for this project"
            };
            entries.push((
                item(host_label.clone(), Some(detail.to_owned()), true, false),
                FormPick::Nothing,
            ));
        }
    }
    for provider in providers.iter().filter(|provider| {
        provider.machine_provider_id.is_some()
            && provider.requires.projectless == resolved.projectless
    }) {
        let reason = provider_disabled_reason(provider, &resolved.controls);
        entries.push((
            item(
                provider.display_name.clone(),
                provider_description(provider, &resolved.controls).or(provider.description.clone()),
                reason.is_some(),
                resolved.effective_value == encode_provider_value(&provider.id),
            ),
            FormPick::Environment {
                provider_id: provider.id.clone(),
                host_id: None,
            },
        ));
    }
    let reuse_current = matches!(
        parse_environment_value(&resolved.effective_value),
        Some(EnvironmentValue::Reuse(_))
    );
    let empty = resolved.reuse_options.is_empty();
    entries.push((
        item(
            "Reuse existing environment…",
            Some(if empty {
                "No environments to reuse in this project".to_owned()
            } else {
                format!("{} available", resolved.reuse_options.len())
            }),
            empty,
            reuse_current,
        ),
        FormPick::ReuseList,
    ));
    entries
}

fn reuse_entries(resolved: &Resolved) -> Vec<(PickerItem, FormPick)> {
    resolved
        .reuse_options
        .iter()
        .map(|option| {
            let mut detail = Vec::new();
            if let Some(host) = &option.host_name {
                detail.push(host.clone());
            }
            if let Some(first) = option.threads.first() {
                let more = option.threads.len() - 1;
                detail.push(if more > 0 {
                    format!("{} (+{more})", first.title)
                } else {
                    first.title.clone()
                });
            }
            (
                item(
                    option.label().to_owned(),
                    (!detail.is_empty()).then(|| detail.join(" · ")),
                    false,
                    resolved.effective_value == encode_reuse_value(&option.environment_id),
                ),
                FormPick::Reuse(option.environment_id.clone()),
            )
        })
        .collect()
}

fn branch_entries(form: &NewThreadForm, resolved: &Resolved) -> Vec<(PickerItem, FormPick)> {
    let load = form.branches.as_ref().map(|(_, load)| load);
    let response = load.and_then(Load::loaded);
    let status_entry = match load {
        Some(Load::Failed(error)) => Some(format!("Could not load branches: {error}")),
        Some(Load::Loaded(_)) => None,
        _ => Some("Loading branches…".to_owned()),
    }
    .map(|detail| {
        (
            item("Branches", Some(detail), true, false),
            FormPick::Nothing,
        )
    });
    let mut entries = Vec::new();
    match resolved.branch_control {
        Some(BranchControl::Worktree) => {
            let base = response.and_then(|response| {
                response
                    .default_worktree_base_branch
                    .clone()
                    .or(response.default_branch.clone())
            });
            entries.push((
                item(
                    format!("Default branch ({})", base.as_deref().unwrap_or("default")),
                    Some("Create the worktree from the project's base branch".into()),
                    false,
                    form.worktree_branch == GitBranchSelection::Default,
                ),
                FormPick::Worktree(GitBranchSelection::Default),
            ));
            for branch in response
                .map(|response| worktree_branch_options(response, ""))
                .unwrap_or_default()
            {
                let selection = GitBranchSelection::Named {
                    name: branch.clone(),
                };
                entries.push((
                    item(
                        format!("Branch from: {branch}"),
                        None,
                        false,
                        form.worktree_branch == selection,
                    ),
                    FormPick::Worktree(selection),
                ));
            }
        }
        Some(BranchControl::Checkout) => {
            let state = resolved.checkout_state.clone().unwrap_or_default();
            let blocker = checkout_blocker(&state);
            entries.push((
                item(
                    current_checkout_label(&state),
                    Some("Keep the checkout as it is".into()),
                    false,
                    form.checkout_branch.is_none(),
                ),
                FormPick::Checkout(None),
            ));
            let selected = match &form.checkout_branch {
                Some(UnmanagedBranchSpec::Existing { name }) => Some(name.as_str()),
                Some(UnmanagedBranchSpec::New { base_branch }) => Some(base_branch.as_str()),
                None => None,
            };
            let detail = blocker.as_ref().map(|blocker| blocker.reason.clone());
            if let Some(response) = response {
                for branch in checkout_branch_options(response, false, "", selected) {
                    let spec = UnmanagedBranchSpec::Existing {
                        name: branch.clone(),
                    };
                    entries.push((
                        item(
                            format!("Checkout: {branch}"),
                            detail.clone(),
                            blocker.is_some(),
                            form.checkout_branch.as_ref() == Some(&spec),
                        ),
                        FormPick::Checkout(Some(spec)),
                    ));
                }
                for branch in checkout_branch_options(response, true, "", selected) {
                    let spec = UnmanagedBranchSpec::New {
                        base_branch: branch.clone(),
                    };
                    entries.push((
                        item(
                            format!("New branch from: {branch}"),
                            detail.clone(),
                            blocker.is_some(),
                            form.checkout_branch.as_ref() == Some(&spec),
                        ),
                        FormPick::Checkout(Some(spec)),
                    ));
                }
            }
        }
        None => {}
    }
    entries.extend(status_entry);
    entries
}

fn field_picker(
    form: &NewThreadForm,
    snapshot: &StoreSnapshot,
    resolved: &Resolved,
    field: FormField,
) -> Result<FormPicker, String> {
    let options = &resolved.options;
    let entries = match field {
        FormField::Project => project_entries(snapshot, &form.project_id),
        FormField::Environment => {
            if resolved.environment_providers.is_none() {
                return Err(match &form.registered {
                    Load::Failed(error) => format!("Environment options failed to load: {error}"),
                    _ => "Environment options are still loading".into(),
                });
            }
            environment_entries(resolved)
        }
        FormField::Branch => branch_entries(form, resolved),
        FormField::Provider => option_entries(
            &options.provider_options,
            &options.provider_id,
            FormPick::Provider,
        ),
        FormField::Model => option_entries(&options.model_options, &options.model, FormPick::Model),
        FormField::Reasoning => option_entries(
            &options.reasoning_options,
            &options.reasoning,
            FormPick::Reasoning,
        ),
        FormField::Permission => option_entries(
            &options.permission_options,
            &options.permission,
            FormPick::Permission,
        ),
        FormField::Prompt => return Err(String::new()),
    };
    if entries.is_empty() {
        let message = match field {
            FormField::Provider | FormField::Model | FormField::Reasoning
                if options.loading_models =>
            {
                "Model options are still loading".to_owned()
            }
            FormField::Project => "Projects are still loading".to_owned(),
            _ => format!("No {} options available", field.label().to_lowercase()),
        };
        return Err(message);
    }
    Ok(FormPicker::new(field, field.label(), entries))
}

impl AppState {
    pub fn new_thread_form(&self) -> Option<&NewThreadForm> {
        self.new_thread.as_ref()
    }

    pub fn resolved_new_thread(&self) -> Option<Resolved> {
        Some(self.new_thread.as_ref()?.resolve(&self.snapshot))
    }

    pub(crate) fn sync_new_thread(&mut self) -> Vec<Effect> {
        let snapshot = self.snapshot.clone();
        let Some(form) = self.new_thread.as_mut() else {
            return Vec::new();
        };
        let generation = form.generation;
        form.sync(&snapshot)
            .into_iter()
            .map(|request| Effect::NewThread {
                generation,
                request,
            })
            .collect()
    }

    fn default_new_thread_project(&self) -> Option<String> {
        if let Some(row) = self.selected_row() {
            match row {
                crate::sidebar::SidebarRow::Thread(thread) => {
                    return Some(thread.project_id.clone());
                }
                crate::sidebar::SidebarRow::Group(group) => {
                    if find_project(&self.snapshot, &group.key).is_some() {
                        return Some(group.key.clone());
                    }
                }
            }
        }
        if let Some(thread_id) = &self.open_thread
            && let Some(project_id) = self
                .sidebar_entry(thread_id)
                .map(|entry| entry.thread.project_id.clone())
                .or_else(|| {
                    self.snapshot
                        .thread(thread_id)
                        .map(|thread| thread.thread.project_id.clone())
                })
        {
            return Some(project_id);
        }
        let sidebar = self.snapshot.sidebar.data.as_deref()?;
        sidebar
            .projects
            .first()
            .map(|project| project.project.id.clone())
            .or_else(|| Some(sidebar.personal_project.project.id.clone()))
            .filter(|id| !id.is_empty())
    }

    fn start_new_thread(&mut self, project_id: String) -> &mut NewThreadForm {
        if self.new_thread.is_some() {
            self.close_new_thread();
        }
        self.new_thread_generation += 1;
        let draft = self.new_thread_draft.take();
        let form = NewThreadForm::new(self.new_thread_generation, project_id, draft);
        self.mode = crate::app::Mode::Normal;
        self.focus = Focus::NewThread;
        self.new_thread.insert(form)
    }

    pub(crate) fn open_new_thread(&mut self) -> Vec<Effect> {
        let Some(project_id) = self.default_new_thread_project() else {
            self.toast("Projects are still loading", ToastLevel::Info);
            return Vec::new();
        };
        self.start_new_thread(project_id);
        self.sync_new_thread()
    }

    pub(crate) fn open_new_thread_in_environment(&mut self) -> Vec<Effect> {
        let Some(thread_id) = self.open_thread.clone() else {
            self.toast("Open a thread first", ToastLevel::Info);
            return Vec::new();
        };
        let Some(thread) = self.snapshot.thread(&thread_id).cloned() else {
            self.toast("The thread is still loading", ToastLevel::Info);
            return Vec::new();
        };
        let reusable = thread.environment.as_ref().filter(|environment| {
            environment.status == EnvironmentStatus::Ready && environment.path.is_some()
        });
        let (Some(environment_id), Some(_)) = (thread.thread.environment_id.clone(), reusable)
        else {
            self.toast(
                "This thread has no ready environment to reuse",
                ToastLevel::Info,
            );
            return Vec::new();
        };
        let placement = Placement {
            section_id: thread.thread.section_id.clone(),
            pinned: thread.thread.is_pinned(),
        };
        let form = self.start_new_thread(thread.thread.project_id.clone());
        form.seed_reuse(environment_id, placement);
        form.activate_prompt();
        self.sync_new_thread()
    }

    pub(crate) fn close_new_thread(&mut self) {
        if let Some(form) = self.new_thread.take() {
            let draft = form.composer.draft().clone();
            self.new_thread_draft = (!draft.is_empty()).then_some(draft);
        }
        if self.focus == Focus::NewThread {
            self.focus = if self.sidebar_visible {
                Focus::Sidebar
            } else {
                Focus::Main
            };
        }
    }

    fn open_form_picker(&mut self, field: FormField) {
        let Some(form) = self.new_thread.as_ref() else {
            return;
        };
        if field == FormField::Prompt {
            if let Some(form) = self.new_thread.as_mut() {
                form.activate_prompt();
            }
            return;
        }
        let resolved = form.resolve(&self.snapshot);
        match field_picker(form, &self.snapshot, &resolved, field) {
            Ok(picker) => {
                if let Some(form) = self.new_thread.as_mut() {
                    form.field = field;
                    form.prompt_active = false;
                    form.picker = Some(picker);
                }
            }
            Err(message) => {
                if !message.is_empty() {
                    self.toast(message, ToastLevel::Info);
                }
            }
        }
    }

    fn submit_new_thread(&mut self) -> Vec<Effect> {
        let Some(form) = self.new_thread.as_ref() else {
            return Vec::new();
        };
        let resolved = form.resolve(&self.snapshot);
        if let Some(reason) = resolved.submit_reason {
            self.toast(reason, ToastLevel::Info);
            return Vec::new();
        }
        if self.read_only {
            self.toast("Read-only mode: cannot create thread", ToastLevel::Error);
            return Vec::new();
        }
        let Some(environment) = resolved.environment else {
            return Vec::new();
        };
        let options = resolved.options;
        let mut request = CreateThreadRequest::new(
            form.project_id.clone(),
            environment.clone(),
            form.composer.draft().to_input(),
        );
        request.provider_id = Some(options.provider_id.clone());
        request.model = Some(options.model.clone());
        request.reasoning_level = Some(options.reasoning);
        request.permission_mode = Some(options.permission);
        if options.supports_service_tier {
            request.service_tier = options.service_tier.clone();
        }
        request.execution_input_sources = Some(resolve_submitted_execution_sources(
            &environment,
            options.sources.clone(),
        ));
        request.section_id = Some(form.placement.section_id.clone());
        request.pinned = Some(form.placement.pinned);
        let generation = form.generation;
        if let Some(form) = self.new_thread.as_mut() {
            form.submitting = true;
            form.error = None;
        }
        vec![Effect::NewThread {
            generation,
            request: NewThreadRequest::Create(Box::new(request)),
        }]
    }

    pub(crate) fn new_thread_loaded(
        &mut self,
        generation: u64,
        loaded: NewThreadLoaded,
    ) -> Vec<Effect> {
        let Some(form) = self.new_thread.as_mut() else {
            return Vec::new();
        };
        if form.generation != generation {
            return Vec::new();
        }
        if let NewThreadLoaded::Created(result) = loaded {
            form.submitting = false;
            return match result {
                Ok(thread) => {
                    self.new_thread = None;
                    self.new_thread_draft = None;
                    let title = thread
                        .thread
                        .title_or_fallback()
                        .unwrap_or("new thread")
                        .to_owned();
                    self.toast(format!("Created: {title}"), ToastLevel::Info);
                    let mut effects = vec![Effect::RefreshSidebar];
                    effects.extend(self.open(thread.thread.id.clone()));
                    self.focus = Focus::Main;
                    effects
                }
                Err(error) => {
                    form.error = Some(error.clone());
                    self.toast(
                        format!("Failed to create thread: {error}"),
                        ToastLevel::Error,
                    );
                    Vec::new()
                }
            };
        }
        form.apply_loaded(loaded);
        self.sync_new_thread()
    }
}

fn picker_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    let Some(form) = state.new_thread.as_mut() else {
        return Vec::new();
    };
    let Some(mut picker) = form.picker.take() else {
        return Vec::new();
    };
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc => return Vec::new(),
        KeyCode::Down | KeyCode::Tab => picker.step(1),
        KeyCode::Char('n') if ctrl => picker.step(1),
        KeyCode::Up | KeyCode::BackTab => picker.step(-1),
        KeyCode::Char('p') if ctrl => picker.step(-1),
        KeyCode::PageDown => picker.step(8),
        KeyCode::PageUp => picker.step(-8),
        KeyCode::Char('c') if ctrl => return Vec::new(),
        KeyCode::Enter => {
            let visible = picker.visible();
            let Some(index) = visible.get(picker.selected).copied() else {
                form.picker = Some(picker);
                return Vec::new();
            };
            let entry = &picker.items[index];
            if entry.disabled {
                let detail = entry
                    .detail
                    .clone()
                    .unwrap_or_else(|| "Not available".into());
                form.picker = Some(picker);
                state.toast(detail, ToastLevel::Info);
                return Vec::new();
            }
            let pick = picker.values[index].clone();
            let field = picker.field;
            if pick == FormPick::ReuseList {
                let resolved = form.resolve(&state.snapshot);
                form.picker = Some(FormPicker::new(
                    FormField::Environment,
                    "Reuse environment",
                    reuse_entries(&resolved),
                ));
                return Vec::new();
            }
            let options = form.resolve(&state.snapshot).options;
            if form.apply_pick(pick, &options) {
                let resolved = form.resolve(&state.snapshot);
                form.field = field;
                form.move_field(&resolved, true);
            }
            return state.sync_new_thread();
        }
        _ => {
            if picker.filter.handle_key(key) == InputOutcome::Edited {
                picker.selected = 0;
            }
        }
    }
    form.picker = Some(picker);
    Vec::new()
}

fn form_command(state: &mut AppState, command: Command) -> Option<Vec<Effect>> {
    let field = match command {
        Command::ModelPicker => FormField::Model,
        Command::ReasoningPicker => FormField::Reasoning,
        Command::PermissionPicker => FormField::Permission,
        _ => return None,
    };
    state.open_form_picker(field);
    Some(Vec::new())
}

fn prompt_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    let now = state.clock;
    let submit_key = state.submit_key;
    let Some(form) = state.new_thread.as_mut() else {
        return Vec::new();
    };
    match form.composer.handle_key(key, submit_key, now) {
        ComposerKey::Submit | ComposerKey::Steer => state.submit_new_thread(),
        ComposerKey::Escape => {
            form.prompt_active = false;
            Vec::new()
        }
        ComposerKey::Interrupt => {
            if form.composer.is_empty() {
                state.close_new_thread();
            } else {
                form.composer.clear(now);
            }
            Vec::new()
        }
        ComposerKey::Edited | ComposerKey::Handled => Vec::new(),
        ComposerKey::Unhandled => match key.code {
            KeyCode::Tab | KeyCode::BackTab => {
                let resolved = form.resolve(&state.snapshot);
                form.move_field(&resolved, key.code == KeyCode::Tab);
                Vec::new()
            }
            _ => match state.keymap.lookup(&key) {
                Some(command) => form_command(state, command).unwrap_or_default(),
                None => Vec::new(),
            },
        },
    }
}

pub(crate) fn handle_new_thread_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    let Some(form) = state.new_thread.as_ref() else {
        state.focus = Focus::Main;
        return Vec::new();
    };
    if form.picker.is_some() {
        return picker_key(state, key);
    }
    if form.field == FormField::Prompt && form.prompt_active {
        return prompt_key(state, key);
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let field = form.field;
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') if !ctrl => {
            state.close_new_thread();
            Vec::new()
        }
        KeyCode::Char('c') if ctrl => {
            state.close_new_thread();
            Vec::new()
        }
        KeyCode::Down | KeyCode::Tab | KeyCode::Char('j') => {
            if let Some(form) = state.new_thread.as_mut() {
                let resolved = form.resolve(&state.snapshot);
                form.move_field(&resolved, true);
            }
            Vec::new()
        }
        KeyCode::Up | KeyCode::BackTab | KeyCode::Char('k') => {
            if let Some(form) = state.new_thread.as_mut() {
                let resolved = form.resolve(&state.snapshot);
                form.move_field(&resolved, false);
            }
            Vec::new()
        }
        KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Char('l') | KeyCode::Right => {
            state.open_form_picker(field);
            Vec::new()
        }
        KeyCode::Char('i') | KeyCode::Char('c') => {
            if let Some(form) = state.new_thread.as_mut() {
                form.activate_prompt();
            }
            Vec::new()
        }
        _ => match state.keymap.lookup(&key) {
            Some(command) => form_command(state, command).unwrap_or_else(|| match command {
                Command::SidebarToggle | Command::Help | Command::PanelToggle => {
                    crate::app::run_command(state, command)
                }
                _ => Vec::new(),
            }),
            None => Vec::new(),
        },
    }
}

pub(crate) fn new_thread_paste(state: &mut AppState, text: &str) {
    let now = state.clock;
    if let Some(form) = state.new_thread.as_mut() {
        if let Some(picker) = form.picker.as_mut() {
            for ch in text.chars().filter(|ch| !ch.is_control()) {
                picker.filter.insert(ch);
            }
            picker.selected = 0;
            return;
        }
        form.activate_prompt();
        form.composer.insert_text(text, now);
    }
}
