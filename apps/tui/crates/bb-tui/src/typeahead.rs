use std::collections::HashMap;
use std::time::Duration;

use bb_wire::plugin::{
    PluginMentionProviderContribution, PluginMentionSearchGroup, PluginMentionSearchQuery,
};
use bb_wire::project::{
    PERSONAL_PROJECT_ID, ProjectCommandsQuery, ProviderCommand, WorkspacePathEntry,
    WorkspacePathListResponse,
};
use bb_wire::system::ProviderComposerAction;
use bb_wire::thread::Thread;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{AppState, Effect, Focus};
use crate::composer::Composer;
use crate::execution::Load;
use crate::mentions::{
    ActiveTrigger, CommandScope, CommandSuggestion, DEFAULT_MENTION_TRIGGER, MENTION_SOURCE_LIMIT,
    MentionSources, MentionSuggestion, NamedCandidate, OrderedSuggestions, PATH_OVERSAMPLE,
    ThreadMentionArgs, TriggerKind, build_command_suggestions, build_path_suggestions,
    build_plugin_mention_suggestions, build_project_mention_suggestions,
    build_prompt_mention_results, build_section_mention_suggestions,
    build_thread_mention_suggestions, command_trigger_of, mention_triggers,
    order_command_suggestions, prompt_action_commands, skills_triggers, typeahead_triggers,
};
use crate::new_thread::NEW_THREAD_COMPOSER_ID;
use crate::palette::FileSource;

pub const MENTION_DEBOUNCE: Duration = Duration::from_millis(120);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ComposerTarget {
    Thread(String),
    NewThread,
}

impl ComposerTarget {
    pub fn id(&self) -> &str {
        match self {
            Self::Thread(thread_id) => thread_id,
            Self::NewThread => NEW_THREAD_COMPOSER_ID,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MentionContext {
    pub project_id: Option<String>,
    pub thread_id: Option<String>,
    pub environment_id: Option<String>,
    pub host_id: Option<String>,
    pub provider_id: Option<String>,
    pub composer_actions: Vec<ProviderComposerAction>,
    pub scope: Option<CommandScope>,
}

impl MentionContext {
    pub fn path_source(&self) -> Option<FileSource> {
        if let Some(environment_id) = &self.environment_id {
            return Some(FileSource::Environment {
                environment_id: environment_id.clone(),
                root: None,
            });
        }
        self.project_id
            .as_ref()
            .filter(|project_id| project_id.as_str() != PERSONAL_PROJECT_ID)
            .map(|project_id| FileSource::Project {
                project_id: project_id.clone(),
                root: None,
            })
    }

    pub fn catalog_key(&self) -> Option<CommandCatalogKey> {
        Some(CommandCatalogKey {
            project_id: self.project_id.clone()?,
            provider_id: self.provider_id.clone().filter(|id| !id.is_empty())?,
            environment_id: self.environment_id.clone(),
            host_id: if self.environment_id.is_some() {
                None
            } else {
                self.host_id.clone()
            },
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CommandCatalogKey {
    pub project_id: String,
    pub provider_id: String,
    pub environment_id: Option<String>,
    pub host_id: Option<String>,
}

impl CommandCatalogKey {
    pub fn query(&self) -> ProjectCommandsQuery {
        ProjectCommandsQuery {
            host_id: self.host_id.clone(),
            environment_id: self.environment_id.clone(),
            provider: self.provider_id.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeaheadRequest {
    MentionProviders,
    WorkspacePaths { source: FileSource, query: String },
    StoragePaths { thread_id: String, query: String },
    PluginMentions(PluginMentionSearchQuery),
    Commands(CommandCatalogKey),
}

impl TypeaheadRequest {
    pub fn debounce(&self) -> Duration {
        match self {
            Self::WorkspacePaths { .. } | Self::StoragePaths { .. } | Self::PluginMentions(_) => {
                MENTION_DEBOUNCE
            }
            _ => Duration::ZERO,
        }
    }

    pub fn slot(&self) -> Option<&'static str> {
        match self {
            Self::WorkspacePaths { .. } => Some("mention-workspace"),
            Self::StoragePaths { .. } => Some("mention-storage"),
            Self::PluginMentions(_) => Some("mention-plugins"),
            _ => None,
        }
    }

    pub fn path_limit() -> u32 {
        (MENTION_SOURCE_LIMIT * PATH_OVERSAMPLE) as u32
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeaheadLoaded {
    MentionProviders(Result<Vec<PluginMentionProviderContribution>, String>),
    WorkspacePaths {
        query: String,
        result: Result<WorkspacePathListResponse, String>,
    },
    StoragePaths {
        query: String,
        result: Result<WorkspacePathListResponse, String>,
    },
    PluginMentions {
        trigger: String,
        query: String,
        result: Result<Vec<PluginMentionSearchGroup>, String>,
    },
    Commands {
        key: CommandCatalogKey,
        result: Result<Vec<ProviderCommand>, String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct SourceResult<T> {
    pub query: String,
    pub result: Result<T, String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MenuState {
    Hidden,
    Hint(&'static str),
    Loading(&'static str),
    Error(String),
    Mentions(OrderedSuggestions<MentionSuggestion>),
    Commands(OrderedSuggestions<CommandSuggestion>),
}

impl MenuState {
    pub fn is_visible(&self) -> bool {
        !matches!(self, Self::Hidden)
    }

    pub fn len(&self) -> usize {
        match self {
            Self::Mentions(ordered) => ordered.len(),
            Self::Commands(ordered) => ordered.len(),
            _ => 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Typeahead {
    pub target: ComposerTarget,
    pub trigger: ActiveTrigger,
    pub context: MentionContext,
    pub selected: usize,
    pub requested: Option<String>,
    pub workspace: Option<SourceResult<Vec<WorkspacePathEntry>>>,
    pub storage: Option<SourceResult<Vec<WorkspacePathEntry>>>,
    pub plugins: Option<SourceResult<Vec<PluginMentionSearchGroup>>>,
    pub menu: MenuState,
}

impl Typeahead {
    fn new(target: ComposerTarget, trigger: ActiveTrigger, context: MentionContext) -> Self {
        Self {
            target,
            trigger,
            context,
            selected: 0,
            requested: None,
            workspace: None,
            storage: None,
            plugins: None,
            menu: MenuState::Hidden,
        }
    }

    fn trimmed_query(&self) -> String {
        self.trigger.query.trim().to_owned()
    }
}

fn source_pending<T>(source: &Option<SourceResult<T>>, wanted: bool, query: &str) -> bool {
    wanted && source.as_ref().is_none_or(|source| source.query != query)
}

fn source_items<T: Clone>(source: &Option<SourceResult<Vec<T>>>) -> Vec<T> {
    source
        .as_ref()
        .and_then(|source| source.result.as_ref().ok())
        .cloned()
        .unwrap_or_default()
}

fn source_error<T>(source: &Option<SourceResult<T>>, query: &str) -> Option<String> {
    source
        .as_ref()
        .filter(|source| source.query == query)
        .and_then(|source| source.result.as_ref().err())
        .cloned()
}

impl AppState {
    pub fn composer_for(&self, target: &ComposerTarget) -> Option<&Composer> {
        match target {
            ComposerTarget::Thread(thread_id) => self.composers.get(thread_id),
            ComposerTarget::NewThread => self.new_thread.as_ref().map(|form| &form.composer),
        }
    }

    pub fn composer_for_mut(&mut self, target: &ComposerTarget) -> Option<&mut Composer> {
        match target {
            ComposerTarget::Thread(thread_id) => self.composers.get_mut(thread_id),
            ComposerTarget::NewThread => self.new_thread.as_mut().map(|form| &mut form.composer),
        }
    }

    pub fn active_composer_target(&self) -> Option<ComposerTarget> {
        match self.focus {
            Focus::Composer => self.open_thread.clone().map(ComposerTarget::Thread),
            Focus::NewThread => self
                .new_thread
                .as_ref()
                .filter(|form| form.prompt_active)
                .map(|_| ComposerTarget::NewThread),
            _ => None,
        }
    }

    pub fn attachment_target(&self) -> Option<ComposerTarget> {
        if self.focus == Focus::NewThread && self.new_thread.is_some() {
            return Some(ComposerTarget::NewThread);
        }
        if self.new_thread.is_some() && self.open_thread.is_none() {
            return Some(ComposerTarget::NewThread);
        }
        self.open_thread
            .clone()
            .filter(|_| self.composer_available())
            .map(ComposerTarget::Thread)
    }

    pub fn mention_context(&self, target: &ComposerTarget) -> MentionContext {
        match target {
            ComposerTarget::Thread(thread_id) => {
                let entry = self.sidebar_entry(thread_id).map(|entry| &entry.thread);
                let detail = self.snapshot.thread(thread_id).map(|detail| &detail.thread);
                let thread = entry.or(detail);
                let execution = self.executions.get(thread_id);
                MentionContext {
                    project_id: thread.map(|thread| thread.project_id.clone()),
                    thread_id: Some(thread_id.clone()),
                    environment_id: thread
                        .and_then(|thread| thread.environment_id.clone())
                        .filter(|id| !id.is_empty()),
                    host_id: None,
                    provider_id: execution
                        .map(|execution| execution.provider_id.clone())
                        .filter(|id| !id.is_empty())
                        .or_else(|| thread.map(|thread| thread.provider_id.clone())),
                    composer_actions: execution
                        .and_then(|execution| execution.provider())
                        .map(|provider| provider.composer_actions.clone())
                        .unwrap_or_default(),
                    scope: Some(CommandScope::Thread),
                }
            }
            ComposerTarget::NewThread => {
                let Some(form) = self.new_thread.as_ref() else {
                    return MentionContext::default();
                };
                let resolved = form.resolve(&self.snapshot);
                let provider_id = resolved.options.provider_id.clone();
                let composer_actions = form
                    .catalog
                    .as_ref()
                    .and_then(|(_, load)| load.loaded())
                    .and_then(|options| {
                        options
                            .providers
                            .iter()
                            .find(|provider| provider.id == provider_id)
                    })
                    .map(|provider| provider.composer_actions.clone())
                    .unwrap_or_default();
                MentionContext {
                    project_id: Some(form.project_id.clone()),
                    thread_id: None,
                    environment_id: resolved.routing.environment_id.clone(),
                    host_id: resolved.routing.host_id.clone(),
                    provider_id: Some(provider_id).filter(|id| !id.is_empty()),
                    composer_actions,
                    scope: Some(CommandScope::NewThread),
                }
            }
        }
    }

    fn mention_provider_list(&self) -> &[PluginMentionProviderContribution] {
        self.mention_providers.loaded().map_or(&[], Vec::as_slice)
    }

    pub fn typeahead_visible(&self) -> bool {
        self.typeahead
            .as_ref()
            .is_some_and(|typeahead| typeahead.menu.is_visible())
    }

    pub(crate) fn sync_typeahead(&mut self, target: &ComposerTarget) -> Vec<Effect> {
        let mut effects = Vec::new();
        if self.mention_providers.is_idle() {
            self.mention_providers = Load::Loading;
            effects.push(Effect::Typeahead(TypeaheadRequest::MentionProviders));
        }
        let context = self.mention_context(target);
        let mention_chars = mention_triggers(self.mention_provider_list());
        let command_chars = if context.catalog_key().is_some() {
            skills_triggers(&context.composer_actions)
        } else {
            Vec::new()
        };
        let triggers = typeahead_triggers(&mention_chars, &command_chars);
        let Some(composer) = self.composer_for_mut(target) else {
            self.typeahead = None;
            return effects;
        };
        let Some(trigger) = composer.sync_trigger(&triggers) else {
            if self
                .typeahead
                .as_ref()
                .is_some_and(|typeahead| typeahead.target == *target)
            {
                self.typeahead = None;
            }
            return effects;
        };
        let mut typeahead = match self.typeahead.take() {
            Some(previous)
                if previous.target == *target
                    && previous.trigger.ch == trigger.ch
                    && previous.trigger.kind == trigger.kind =>
            {
                let changed = previous.trigger.query != trigger.query;
                Typeahead {
                    trigger,
                    context,
                    selected: if changed { 0 } else { previous.selected },
                    ..previous
                }
            }
            _ => Typeahead::new(target.clone(), trigger, context),
        };
        effects.extend(self.typeahead_requests(&mut typeahead));
        self.typeahead = Some(typeahead);
        self.refresh_typeahead();
        effects
    }

    fn typeahead_requests(&mut self, typeahead: &mut Typeahead) -> Vec<Effect> {
        let mut effects = Vec::new();
        match typeahead.trigger.kind {
            TriggerKind::Mention => {
                let query = typeahead.trimmed_query();
                if query.is_empty() || typeahead.requested.as_deref() == Some(query.as_str()) {
                    return effects;
                }
                typeahead.requested = Some(query.clone());
                if typeahead.trigger.ch == DEFAULT_MENTION_TRIGGER {
                    if let Some(source) = typeahead.context.path_source() {
                        effects.push(Effect::Typeahead(TypeaheadRequest::WorkspacePaths {
                            source,
                            query: query.clone(),
                        }));
                    }
                    if let Some(thread_id) = typeahead.context.thread_id.clone() {
                        effects.push(Effect::Typeahead(TypeaheadRequest::StoragePaths {
                            thread_id,
                            query: query.clone(),
                        }));
                    }
                }
                if self.has_mention_provider(typeahead.trigger.ch) {
                    effects.push(Effect::Typeahead(TypeaheadRequest::PluginMentions(
                        PluginMentionSearchQuery {
                            q: query,
                            trigger: typeahead.trigger.ch.to_string(),
                            project_id: typeahead.context.project_id.clone(),
                            thread_id: typeahead.context.thread_id.clone(),
                        },
                    )));
                }
            }
            TriggerKind::Command => {
                if let Some(key) = typeahead.context.catalog_key() {
                    let load = self.command_catalogs.entry(key.clone()).or_default();
                    if load.is_idle() || load.is_failed() && typeahead.requested.is_none() {
                        *load = Load::Loading;
                        typeahead.requested = Some(String::new());
                        effects.push(Effect::Typeahead(TypeaheadRequest::Commands(key)));
                    }
                }
            }
        }
        effects
    }

    fn has_mention_provider(&self, trigger: char) -> bool {
        let trigger = trigger.to_string();
        self.mention_provider_list()
            .iter()
            .any(|provider| provider.triggers.contains(&trigger))
    }

    fn sidebar_threads(&self) -> Vec<&Thread> {
        let Some(sidebar) = self.snapshot.sidebar.data.as_deref() else {
            return Vec::new();
        };
        sidebar
            .projects
            .iter()
            .chain(std::iter::once(&sidebar.personal_project))
            .flat_map(|project| project.threads.iter().map(|entry| &entry.thread))
            .collect()
    }

    fn mention_menu(&self, typeahead: &Typeahead) -> MenuState {
        let query = typeahead.trimmed_query();
        let builtin = typeahead.trigger.ch == DEFAULT_MENTION_TRIGGER;
        if query.is_empty() {
            return if builtin {
                MenuState::Hint("Type to search files, threads, projects and sections")
            } else {
                MenuState::Hidden
            };
        }
        let plugins_wanted = self.has_mention_provider(typeahead.trigger.ch);
        let mut sources = MentionSources::default();
        if builtin {
            sources.paths = build_path_suggestions(
                &source_items(&typeahead.workspace),
                &source_items(&typeahead.storage),
                MENTION_SOURCE_LIMIT,
            );
            if let Some(sidebar) = self.snapshot.sidebar.data.as_deref() {
                let projects: Vec<NamedCandidate> = sidebar
                    .projects
                    .iter()
                    .chain(std::iter::once(&sidebar.personal_project))
                    .map(|project| NamedCandidate {
                        id: project.project.id.clone(),
                        name: project.project.name.clone(),
                    })
                    .collect();
                let project_names: HashMap<String, String> = projects
                    .iter()
                    .map(|project| (project.id.clone(), project.name.clone()))
                    .collect();
                let sections: Vec<NamedCandidate> = sidebar
                    .sections
                    .iter()
                    .map(|section| NamedCandidate {
                        id: section.id.clone(),
                        name: section.name.clone(),
                    })
                    .collect();
                sources.threads = build_thread_mention_suggestions(&ThreadMentionArgs {
                    threads: self.sidebar_threads(),
                    query: &query,
                    current_project_id: typeahead.context.project_id.as_deref(),
                    current_thread_id: typeahead.context.thread_id.as_deref(),
                    current_environment_id: typeahead.context.environment_id.as_deref(),
                    project_names,
                    limit: MENTION_SOURCE_LIMIT,
                });
                sources.projects =
                    build_project_mention_suggestions(&projects, &query, MENTION_SOURCE_LIMIT);
                sources.sections =
                    build_section_mention_suggestions(&sections, &query, MENTION_SOURCE_LIMIT);
            }
        }
        if plugins_wanted
            && let Some(plugins) = typeahead
                .plugins
                .as_ref()
                .filter(|plugins| plugins.query == query)
            && let Ok(groups) = &plugins.result
        {
            sources.plugins = build_plugin_mention_suggestions(groups);
        }
        let results = build_prompt_mention_results(&query, sources);
        if !results.is_empty() {
            return MenuState::Mentions(results);
        }
        let paths_wanted = builtin && typeahead.context.path_source().is_some();
        let storage_wanted = builtin && typeahead.context.thread_id.is_some();
        let pending = source_pending(&typeahead.workspace, paths_wanted, &query)
            || source_pending(&typeahead.storage, storage_wanted, &query)
            || source_pending(&typeahead.plugins, plugins_wanted, &query);
        if pending {
            return MenuState::Loading("Searching…");
        }
        if let Some(error) = (paths_wanted)
            .then(|| source_error(&typeahead.workspace, &query))
            .flatten()
        {
            return MenuState::Error(error);
        }
        MenuState::Mentions(results)
    }

    fn command_menu(&self, typeahead: &Typeahead) -> MenuState {
        let Some(key) = typeahead.context.catalog_key() else {
            return MenuState::Hidden;
        };
        let scope = typeahead.context.scope.unwrap_or(CommandScope::Thread);
        let actions = prompt_action_commands(&typeahead.context.composer_actions);
        let load = self.command_catalogs.get(&key);
        let commands: &[ProviderCommand] = load.and_then(Load::loaded).map_or(&[], Vec::as_slice);
        let suggestions = build_command_suggestions(
            commands,
            &actions,
            typeahead.trigger.ch,
            scope,
            &typeahead.trigger.query,
        );
        match load {
            Some(Load::Loading) | None | Some(Load::Idle) if suggestions.is_empty() => {
                MenuState::Loading("Loading commands…")
            }
            Some(Load::Failed(error)) if suggestions.is_empty() => MenuState::Error(error.clone()),
            _ if suggestions.is_empty() => MenuState::Hidden,
            _ => MenuState::Commands(order_command_suggestions(
                suggestions,
                &typeahead.trigger.query,
            )),
        }
    }

    pub(crate) fn refresh_typeahead(&mut self) {
        let Some(typeahead) = self.typeahead.as_ref() else {
            return;
        };
        let menu = match typeahead.trigger.kind {
            TriggerKind::Mention => self.mention_menu(typeahead),
            TriggerKind::Command => self.command_menu(typeahead),
        };
        if let Some(typeahead) = self.typeahead.as_mut() {
            typeahead.selected = typeahead.selected.min(menu.len().saturating_sub(1));
            typeahead.menu = menu;
        }
    }

    pub(crate) fn typeahead_loaded(&mut self, loaded: TypeaheadLoaded) -> Vec<Effect> {
        match loaded {
            TypeaheadLoaded::MentionProviders(result) => {
                if let Err(error) = &result {
                    tracing::debug!(%error, "plugin mention providers unavailable");
                }
                self.mention_providers = Load::Loaded(result.unwrap_or_default());
                if let Some(target) = self.active_composer_target() {
                    return self.sync_typeahead(&target);
                }
            }
            TypeaheadLoaded::Commands { key, result } => {
                self.command_catalogs.insert(key, Load::from_result(result));
            }
            TypeaheadLoaded::WorkspacePaths { query, result } => {
                if let Some(typeahead) = self.typeahead.as_mut() {
                    typeahead.workspace = Some(SourceResult {
                        query,
                        result: result.map(|response| response.paths),
                    });
                }
            }
            TypeaheadLoaded::StoragePaths { query, result } => {
                if let Some(typeahead) = self.typeahead.as_mut() {
                    typeahead.storage = Some(SourceResult {
                        query,
                        result: result.map(|response| response.paths),
                    });
                }
            }
            TypeaheadLoaded::PluginMentions {
                trigger,
                query,
                result,
            } => {
                if let Some(typeahead) = self
                    .typeahead
                    .as_mut()
                    .filter(|typeahead| typeahead.trigger.ch.to_string() == trigger)
                {
                    typeahead.plugins = Some(SourceResult { query, result });
                }
            }
        }
        self.refresh_typeahead();
        Vec::new()
    }

    pub(crate) fn apply_typeahead(&mut self) -> bool {
        let Some(typeahead) = self.typeahead.take() else {
            return false;
        };
        let now = self.clock;
        let trigger = typeahead.trigger.clone();
        let applied = match &typeahead.menu {
            MenuState::Mentions(ordered) => ordered.get(typeahead.selected).map(|suggestion| {
                (
                    suggestion.serialized_text(trigger.ch),
                    suggestion.resource(),
                    true,
                )
            }),
            MenuState::Commands(ordered) => ordered
                .get(typeahead.selected)
                .zip(command_trigger_of(trigger.ch))
                .map(|(suggestion, command_trigger)| {
                    (
                        format!("{}{}", trigger.ch, suggestion.name),
                        suggestion.resource(command_trigger),
                        false,
                    )
                }),
            _ => None,
        };
        let Some((serialized, resource, mention)) = applied else {
            self.typeahead = Some(typeahead);
            return false;
        };
        let Some(composer) = self.composer_for_mut(&typeahead.target) else {
            return false;
        };
        composer.apply_mention(
            &trigger,
            &serialized,
            resource,
            |trailing| {
                if mention {
                    trigger.from + 2
                } else {
                    trigger.from + 1 + trailing.chars().count()
                }
            },
            now,
        );
        true
    }

    pub(crate) fn dismiss_typeahead(&mut self) {
        let Some(typeahead) = self.typeahead.take() else {
            return;
        };
        if let Some(composer) = self.composer_for_mut(&typeahead.target) {
            composer.dismiss_trigger(&typeahead.trigger);
        }
    }

    fn move_typeahead(&mut self, delta: isize) {
        if let Some(typeahead) = self.typeahead.as_mut() {
            let len = typeahead.menu.len();
            if len > 0 {
                typeahead.selected =
                    (typeahead.selected as isize + delta).rem_euclid(len as isize) as usize;
            }
        }
    }
}

pub(crate) fn handle_typeahead_key(
    state: &mut AppState,
    target: &ComposerTarget,
    key: &KeyEvent,
) -> Option<Vec<Effect>> {
    let typeahead = state.typeahead.as_ref()?;
    if typeahead.target != *target || !typeahead.menu.is_visible() {
        return None;
    }
    let has_items = !typeahead.menu.is_empty();
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let plain = !ctrl && !key.modifiers.contains(KeyModifiers::ALT);
    match key.code {
        KeyCode::Esc if plain => {
            state.dismiss_typeahead();
            Some(Vec::new())
        }
        KeyCode::Down if plain && has_items => {
            state.move_typeahead(1);
            Some(Vec::new())
        }
        KeyCode::Up if plain && has_items => {
            state.move_typeahead(-1);
            Some(Vec::new())
        }
        KeyCode::Char('n') if ctrl && has_items => {
            state.move_typeahead(1);
            Some(Vec::new())
        }
        KeyCode::Char('p') if ctrl && has_items => {
            state.move_typeahead(-1);
            Some(Vec::new())
        }
        KeyCode::BackTab if has_items => {
            state.move_typeahead(-1);
            Some(Vec::new())
        }
        KeyCode::Enter | KeyCode::Tab
            if has_items && !key.modifiers.contains(KeyModifiers::SHIFT) =>
        {
            state.apply_typeahead();
            Some(state.sync_typeahead(target))
        }
        _ => None,
    }
}
