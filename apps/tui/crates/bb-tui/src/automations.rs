use std::cell::Cell;
use std::collections::{HashMap, HashSet};

use bb_wire::common::JsonValue;
use bb_wire::environment::HostStatus;
use bb_wire::prompt::{PermissionMode, ProjectExecutionDefaults, ReasoningLevel};
use bb_wire::system::{AvailableModel, SystemExecutionOptionsResponse, SystemProviderCatalogEntry};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::json;

use crate::app::{AppState, Effect, Focus, ToastLevel};
use crate::automation_data::{
    Automation, Execution, NAME_MAX_LENGTH, OverviewEntry, PERSONAL_PROJECT_ID, Route, Run,
    RunPage, RunStatus, Trigger, describe_cron, environment_label, format_run_at_input,
    format_run_time, local_timezone, parse_run_at, validate_cron, working_directory_label,
};
use crate::execution::Load;
use crate::input::{InputOutcome, LineInput};
use crate::keymap::Command;
use crate::settings_form::{escape_multiline, unescape_multiline};

const DEFAULT_CRON: &str = "0 9 * * 1-5";
const PAGE: isize = 10;

#[derive(Debug, Clone, PartialEq)]
pub enum AutomationMutation {
    Create { input: JsonValue },
    Update { route: Route, input: JsonValue },
    Delete(Route),
    Pause(Route),
    Resume(Route),
    Run(Route),
}

fn route_input(route: &Route) -> JsonValue {
    json!({"projectId": route.project_id, "automationId": route.automation_id})
}

impl AutomationMutation {
    pub fn method(&self) -> &'static str {
        match self {
            Self::Create { .. } => "automations_create",
            Self::Update { .. } => "automations_update",
            Self::Delete(_) => "automations_delete",
            Self::Pause(_) => "automations_pause",
            Self::Resume(_) => "automations_resume",
            Self::Run(_) => "automations_run",
        }
    }

    pub fn input(&self) -> JsonValue {
        match self {
            Self::Create { input } | Self::Update { input, .. } => input.clone(),
            Self::Delete(route) | Self::Pause(route) | Self::Resume(route) | Self::Run(route) => {
                route_input(route)
            }
        }
    }

    pub fn route(&self) -> Option<&Route> {
        match self {
            Self::Create { .. } => None,
            Self::Update { route, .. }
            | Self::Delete(route)
            | Self::Pause(route)
            | Self::Resume(route)
            | Self::Run(route) => Some(route),
        }
    }

    pub fn key(&self) -> String {
        match self.route() {
            Some(route) => format!("{}:{}", route.project_id, route.automation_id),
            None => "create".into(),
        }
    }

    pub fn failure_verb(&self) -> &'static str {
        match self {
            Self::Create { .. } => "create the automation",
            Self::Update { .. } => "update the automation",
            Self::Delete(_) => "delete the automation",
            Self::Pause(_) => "pause the automation",
            Self::Resume(_) => "resume the automation",
            Self::Run(_) => "run the automation",
        }
    }

    fn success_message(&self) -> &'static str {
        match self {
            Self::Create { .. } => "Automation created",
            Self::Update { .. } => "Automation updated",
            Self::Delete(_) => "Automation deleted",
            Self::Pause(_) => "Automation paused",
            Self::Resume(_) => "Automation resumed",
            Self::Run(_) => "Run started",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AutomationRequest {
    Overview,
    Detail(Route),
    Runs {
        route: Route,
        cursor: Option<String>,
    },
    Providers,
    Models {
        provider_id: String,
    },
    ProjectDefaults {
        project_id: String,
    },
    Mutate(AutomationMutation),
}

#[derive(Debug, Clone, PartialEq)]
pub enum AutomationLoaded {
    Overview(Result<Vec<OverviewEntry>, String>),
    Detail {
        route: Route,
        result: Result<Box<Automation>, String>,
    },
    Runs {
        route: Route,
        cursor: Option<String>,
        result: Result<RunPage, String>,
    },
    Providers(Result<Vec<SystemProviderCatalogEntry>, String>),
    Models {
        provider_id: String,
        result: Result<SystemExecutionOptionsResponse, String>,
    },
    ProjectDefaults {
        project_id: String,
        result: Result<Option<ProjectExecutionDefaults>, String>,
    },
    Mutated {
        mutation: AutomationMutation,
        result: Result<JsonValue, String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct DetailState {
    pub route: Route,
    pub automation: Load<Automation>,
    pub runs: Load<Vec<Run>>,
    pub next_cursor: Option<String>,
    pub loading_more: bool,
    pub cursor: usize,
    pub offset: Cell<usize>,
    inflight: bool,
    stale: bool,
    runs_inflight: bool,
    runs_stale: bool,
}

impl DetailState {
    fn new(route: Route, automation: Option<Automation>) -> Self {
        Self {
            route,
            automation: automation.map_or(Load::Loading, Load::Loaded),
            runs: Load::Loading,
            next_cursor: None,
            loading_more: false,
            cursor: 0,
            offset: Cell::new(0),
            inflight: false,
            stale: false,
            runs_inflight: false,
            runs_stale: false,
        }
    }

    fn fetch(&mut self) -> Vec<Effect> {
        if self.inflight {
            self.stale = true;
            return Vec::new();
        }
        self.inflight = true;
        vec![Effect::Automations(AutomationRequest::Detail(
            self.route.clone(),
        ))]
    }

    fn fetch_runs(&mut self) -> Vec<Effect> {
        if self.runs_inflight {
            self.runs_stale = true;
            return Vec::new();
        }
        self.runs_inflight = true;
        vec![Effect::Automations(AutomationRequest::Runs {
            route: self.route.clone(),
            cursor: None,
        })]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormField {
    Name,
    Project,
    Schedule,
    Cron,
    Timezone,
    RunAt,
    Prompt,
    Provider,
    Model,
    Reasoning,
    Permission,
    Environment,
    Paused,
    Save,
}

impl FormField {
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Project => "Project",
            Self::Schedule => "Schedule",
            Self::Cron => "Cron",
            Self::Timezone => "Timezone",
            Self::RunAt => "Run at",
            Self::Prompt => "Prompt",
            Self::Provider => "Provider",
            Self::Model => "Model",
            Self::Reasoning => "Reasoning",
            Self::Permission => "Permission mode",
            Self::Environment => "Environment",
            Self::Paused => "Start paused",
            Self::Save => "",
        }
    }

    fn is_text(self) -> bool {
        matches!(
            self,
            Self::Name | Self::Cron | Self::Timezone | Self::RunAt | Self::Prompt
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum EnvChoice {
    ProjectDefault,
    NewWorktree,
    Reuse {
        environment_id: String,
        label: String,
    },
    Keep {
        value: JsonValue,
        target_thread: Option<String>,
        label: String,
    },
}

impl EnvChoice {
    fn label(&self) -> String {
        match self {
            Self::ProjectDefault => "Project default".into(),
            Self::NewWorktree => "New worktree each run".into(),
            Self::Reuse { label, .. } => format!("Reuse {label}"),
            Self::Keep { label, .. } => label.clone(),
        }
    }

    fn value(&self) -> String {
        match self {
            Self::ProjectDefault => "default".into(),
            Self::NewWorktree => "worktree".into(),
            Self::Reuse { environment_id, .. } => format!("reuse:{environment_id}"),
            Self::Keep { .. } => "keep".into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleKind {
    Recurring,
    Once,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FormEdit {
    pub field: FormField,
    pub input: LineInput,
    pub error: Option<String>,
    pub hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FormChoice {
    pub field: FormField,
    pub title: String,
    pub options: Vec<(String, String)>,
    pub selected: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AutomationForm {
    pub editing: Option<Automation>,
    pub name: String,
    pub project_id: String,
    pub schedule: ScheduleKind,
    pub cron: String,
    pub timezone: String,
    pub run_at: String,
    pub agent: bool,
    pub prompt: String,
    pub provider_id: String,
    pub model: String,
    pub reasoning: String,
    pub permission: String,
    pub environment: EnvChoice,
    pub start_paused: bool,
    pub cursor: usize,
    pub edit: Option<FormEdit>,
    pub choice: Option<FormChoice>,
    pub error: Option<String>,
    pub saving: bool,
    provider_touched: bool,
}

impl AutomationForm {
    pub fn is_create(&self) -> bool {
        self.editing.is_none()
    }

    pub fn fields(&self) -> Vec<FormField> {
        let mut fields = vec![FormField::Name, FormField::Project, FormField::Schedule];
        match self.schedule {
            ScheduleKind::Recurring => fields.extend([FormField::Cron, FormField::Timezone]),
            ScheduleKind::Once => fields.push(FormField::RunAt),
        }
        if self.agent {
            fields.extend([
                FormField::Prompt,
                FormField::Provider,
                FormField::Model,
                FormField::Reasoning,
                FormField::Permission,
                FormField::Environment,
            ]);
        }
        if self.is_create() {
            fields.push(FormField::Paused);
        }
        fields.push(FormField::Save);
        fields
    }

    pub fn current_field(&self) -> FormField {
        let fields = self.fields();
        fields[self.cursor.min(fields.len() - 1)]
    }

    fn from_automation(automation: &Automation) -> Self {
        let (schedule, cron, timezone, run_at) = match &automation.trigger {
            Some(Trigger::Schedule { cron, timezone }) => (
                ScheduleKind::Recurring,
                cron.clone(),
                timezone.clone(),
                String::new(),
            ),
            Some(Trigger::Once { run_at }) => (
                ScheduleKind::Once,
                DEFAULT_CRON.into(),
                local_timezone(),
                format_run_at_input(*run_at),
            ),
            None => (
                ScheduleKind::Recurring,
                DEFAULT_CRON.into(),
                local_timezone(),
                String::new(),
            ),
        };
        let mut form = Self {
            editing: Some(automation.clone()),
            name: automation.name.clone(),
            project_id: automation.project_id.clone(),
            schedule,
            cron,
            timezone,
            run_at,
            agent: false,
            prompt: String::new(),
            provider_id: String::new(),
            model: String::new(),
            reasoning: String::new(),
            permission: String::new(),
            environment: EnvChoice::ProjectDefault,
            start_paused: false,
            cursor: 0,
            edit: None,
            choice: None,
            error: None,
            saving: false,
            provider_touched: true,
        };
        if let Some(Execution::Agent {
            prompt,
            provider_id,
            model,
            reasoning_level,
            permission_mode,
            environment,
            target_thread_id,
            ..
        }) = &automation.execution
        {
            form.agent = true;
            form.prompt = prompt.clone();
            form.provider_id = provider_id.clone();
            form.model = model.clone();
            form.reasoning = if reasoning_level.is_empty() {
                ReasoningLevel::Medium.as_str().into()
            } else {
                reasoning_level.clone()
            };
            form.permission = permission_mode.clone();
            form.environment = EnvChoice::Keep {
                value: environment.clone(),
                target_thread: target_thread_id.clone(),
                label: environment_label(environment, target_thread_id.as_deref()),
            };
        }
        form
    }

    fn create(project_id: String) -> Self {
        Self {
            editing: None,
            name: String::new(),
            project_id,
            schedule: ScheduleKind::Recurring,
            cron: DEFAULT_CRON.into(),
            timezone: local_timezone(),
            run_at: String::new(),
            agent: true,
            prompt: String::new(),
            provider_id: String::new(),
            model: String::new(),
            reasoning: ReasoningLevel::Medium.as_str().into(),
            permission: String::new(),
            environment: EnvChoice::ProjectDefault,
            start_paused: false,
            cursor: 0,
            edit: None,
            choice: None,
            error: None,
            saving: false,
            provider_touched: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteConfirm {
    pub route: Route,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AutomationsView {
    pub overview: Load<Vec<OverviewEntry>>,
    pub cursor: usize,
    pub offset: Cell<usize>,
    pub filter: String,
    pub filter_input: Option<LineInput>,
    pub detail: Option<DetailState>,
    pub form: Option<AutomationForm>,
    pub confirm: Option<DeleteConfirm>,
    pub pending: HashSet<String>,
    pub providers: Load<Vec<SystemProviderCatalogEntry>>,
    pub models: HashMap<String, Load<SystemExecutionOptionsResponse>>,
    pub project_defaults: HashMap<String, Load<Option<ProjectExecutionDefaults>>>,
    overview_inflight: bool,
    overview_stale: bool,
}

impl Default for AutomationsView {
    fn default() -> Self {
        Self {
            overview: Load::Loading,
            cursor: 0,
            offset: Cell::new(0),
            filter: String::new(),
            filter_input: None,
            detail: None,
            form: None,
            confirm: None,
            pending: HashSet::new(),
            providers: Load::Idle,
            models: HashMap::new(),
            project_defaults: HashMap::new(),
            overview_inflight: false,
            overview_stale: false,
        }
    }
}

impl AutomationsView {
    pub fn captures_keys(&self) -> bool {
        self.form.is_some() || self.confirm.is_some() || self.filter_input.is_some()
    }

    pub fn filter_text(&self) -> &str {
        match &self.filter_input {
            Some(input) => input.text(),
            None => &self.filter,
        }
    }

    fn fetch_overview(&mut self) -> Vec<Effect> {
        if self.overview_inflight {
            self.overview_stale = true;
            return Vec::new();
        }
        self.overview_inflight = true;
        vec![Effect::Automations(AutomationRequest::Overview)]
    }

    pub fn find(&self, route: &Route) -> Option<&Automation> {
        if let Some(detail) = &self.detail
            && &detail.route == route
            && let Some(automation) = detail.automation.loaded()
        {
            return Some(automation);
        }
        self.overview
            .loaded()?
            .iter()
            .map(|entry| &entry.automation)
            .find(|automation| &automation.route() == route)
    }

    fn project_name(&self, project_id: &str) -> Option<String> {
        self.overview
            .loaded()?
            .iter()
            .find(|entry| entry.project.id == project_id)
            .map(|entry| entry.project.name.clone())
    }

    fn model_options(&self, provider_id: &str) -> Option<&SystemExecutionOptionsResponse> {
        self.models.get(provider_id)?.loaded()
    }

    fn sorted_entries(&self) -> Vec<&OverviewEntry> {
        let filter = self.filter_text().to_lowercase();
        let mut entries: Vec<&OverviewEntry> = self
            .overview
            .loaded()
            .map(|entries| entries.iter().collect())
            .unwrap_or_default();
        entries.retain(|entry| {
            filter.is_empty()
                || entry.automation.name.to_lowercase().contains(&filter)
                || entry.project.name.to_lowercase().contains(&filter)
                || entry.automation.cadence().to_lowercase().contains(&filter)
        });
        entries.sort_by(|left, right| {
            left.automation
                .name
                .to_lowercase()
                .cmp(&right.automation.name.to_lowercase())
                .then_with(|| left.automation.id.cmp(&right.automation.id))
        });
        entries
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Plain,
    Muted,
    Good,
    Warn,
    Bad,
    Accent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Header,
    Field,
    Text,
    Item,
    Note,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowTarget {
    Automation(Route),
    Run(usize),
    LoadMore,
    Thread(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomationRow {
    pub kind: RowKind,
    pub glyph: Option<(String, Tone)>,
    pub label: String,
    pub value: String,
    pub badges: Vec<(String, Tone)>,
    pub target: Option<RowTarget>,
}

impl AutomationRow {
    fn new(kind: RowKind, label: impl Into<String>) -> Self {
        Self {
            kind,
            glyph: None,
            label: label.into(),
            value: String::new(),
            badges: Vec::new(),
            target: None,
        }
    }

    fn field(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            ..Self::new(RowKind::Field, label)
        }
    }

    pub fn selectable(&self) -> bool {
        self.target.is_some()
            || matches!(self.kind, RowKind::Field | RowKind::Text | RowKind::Error)
    }
}

fn status_glyph(automation: &Automation, now_ms: i64) -> (String, Tone) {
    if automation.is_unreadable() || automation.needs_prompt() {
        return ("!".into(), Tone::Bad);
    }
    match automation.last_run_status {
        Some(RunStatus::Running) => return ("◐".into(), Tone::Accent),
        Some(RunStatus::Failed) => return ("✗".into(), Tone::Bad),
        _ => {}
    }
    if automation.allows_toggle(now_ms) && automation.enabled {
        ("●".into(), Tone::Good)
    } else {
        ("○".into(), Tone::Muted)
    }
}

fn automation_badges(
    view: &AutomationsView,
    automation: &Automation,
    now_ms: i64,
) -> Vec<(String, Tone)> {
    let mut badges = Vec::new();
    if automation.is_unreadable() {
        badges.push(("invalid".into(), Tone::Bad));
        return badges;
    }
    if automation.needs_prompt() {
        badges.push(("needs prompt".into(), Tone::Warn));
    }
    if matches!(automation.execution, Some(Execution::Script { .. })) {
        badges.push(("script".into(), Tone::Muted));
    }
    if !automation.enabled && automation.allows_toggle(now_ms) {
        badges.push(("paused".into(), Tone::Warn));
    }
    if view
        .pending
        .contains(&format!("{}:{}", automation.project_id, automation.id))
    {
        badges.push(("…".into(), Tone::Accent));
    }
    badges
}

fn project_label(name: &str, id: &str) -> String {
    if id == PERSONAL_PROJECT_ID {
        "Personal".into()
    } else {
        name.into()
    }
}

fn load_rows<T>(load: &Load<T>, what: &str, rows: &mut Vec<AutomationRow>) -> bool {
    match load {
        Load::Loaded(_) => false,
        Load::Failed(error) => {
            rows.push(AutomationRow::new(
                RowKind::Error,
                format!("Couldn't load {what}: {error}"),
            ));
            true
        }
        Load::Idle | Load::Loading => {
            rows.push(AutomationRow::new(
                RowKind::Note,
                format!("Loading {what}…"),
            ));
            true
        }
    }
}

pub fn automation_rows(state: &AppState) -> Vec<AutomationRow> {
    let Some(view) = state.automations.as_ref() else {
        return Vec::new();
    };
    match &view.detail {
        Some(detail) => detail_rows(state, view, detail),
        None => list_rows(view, state.now_ms),
    }
}

fn list_rows(view: &AutomationsView, now_ms: i64) -> Vec<AutomationRow> {
    let mut rows = Vec::new();
    if load_rows(&view.overview, "automations", &mut rows) {
        return rows;
    }
    let entries = view.sorted_entries();
    if entries.is_empty() {
        rows.push(AutomationRow::new(
            RowKind::Note,
            if view.filter_text().is_empty() {
                "No automations yet · n creates one"
            } else {
                "No automations match the filter"
            },
        ));
        return rows;
    }
    for entry in entries {
        let automation = &entry.automation;
        let mut parts = vec![project_label(&entry.project.name, &entry.project.id)];
        if !automation.is_unreadable() {
            parts.push(automation.cadence());
            parts.push(automation.status_label(now_ms));
        }
        rows.push(AutomationRow {
            kind: RowKind::Item,
            glyph: Some(status_glyph(automation, now_ms)),
            label: automation.name.clone(),
            value: parts.join(" · "),
            badges: automation_badges(view, automation, now_ms),
            target: Some(RowTarget::Automation(automation.route())),
        });
    }
    rows
}

fn thread_label(state: &AppState, thread_id: &str) -> String {
    if state.sidebar_entry(thread_id).is_some() || state.snapshot.thread(thread_id).is_some() {
        format!("{} ({thread_id})", state.thread_title(thread_id))
    } else {
        thread_id.to_owned()
    }
}

fn detail_rows(
    state: &AppState,
    view: &AutomationsView,
    detail: &DetailState,
) -> Vec<AutomationRow> {
    let now_ms = state.now_ms;
    let mut rows = Vec::new();
    if load_rows(&detail.automation, "the automation", &mut rows) {
        return rows;
    }
    let Some(automation) = detail.automation.loaded() else {
        return rows;
    };
    let mut header = AutomationRow::new(RowKind::Header, automation.name.clone());
    header.glyph = Some(status_glyph(automation, now_ms));
    header.badges = automation_badges(view, automation, now_ms);
    rows.push(header);
    let project = view
        .project_name(&automation.project_id)
        .map(|name| project_label(&name, &automation.project_id))
        .unwrap_or_else(|| project_label(&automation.project_id, &automation.project_id));
    if automation.is_unreadable() {
        rows.push(AutomationRow::new(
            RowKind::Error,
            "The stored automation configuration cannot be read. d deletes it.",
        ));
        rows.push(AutomationRow::field("Project", project));
        return rows;
    }
    if automation.needs_prompt() {
        rows.push(AutomationRow::new(
            RowKind::Error,
            "This automation has no prompt and will not run · e to add one",
        ));
    }
    rows.push(AutomationRow::field(
        "Status",
        automation.status_label(now_ms),
    ));
    rows.push(AutomationRow::field("Project", project));
    rows.push(AutomationRow::field("Schedule", automation.cadence()));
    match &automation.trigger {
        Some(Trigger::Schedule { cron, timezone }) => {
            rows.push(AutomationRow::field("Cron", format!("{cron} ({timezone})")));
        }
        Some(Trigger::Once { run_at }) => {
            rows.push(AutomationRow::field("Runs at", format_run_time(*run_at)));
        }
        None => {}
    }
    let last_run = match (automation.last_run_at, automation.last_run_status) {
        (Some(at), Some(status)) => format!("{} · {}", format_run_time(at), status.label()),
        (Some(at), None) => format_run_time(at),
        _ => "Never".into(),
    };
    rows.push(AutomationRow::field("Last run", last_run));
    rows.push(AutomationRow::field(
        "Run count",
        automation.run_count.to_string(),
    ));
    if let Some(thread_id) = &automation.last_run_thread_id {
        let mut row = AutomationRow::field("Last run thread", thread_label(state, thread_id));
        row.target = Some(RowTarget::Thread(thread_id.clone()));
        rows.push(row);
    }
    match &automation.execution {
        Some(Execution::Agent {
            prompt,
            provider_id,
            model,
            reasoning_level,
            service_tier,
            permission_mode,
            environment,
            target_thread_id,
        }) => {
            rows.push(AutomationRow::field("Mode", "Agent"));
            rows.push(AutomationRow::field("Provider", provider_id.clone()));
            rows.push(AutomationRow::field("Model", model.clone()));
            rows.push(AutomationRow::field(
                "Reasoning",
                if reasoning_level.is_empty() {
                    "medium".to_owned()
                } else {
                    reasoning_level.clone()
                },
            ));
            if let Some(tier) = service_tier {
                rows.push(AutomationRow::field("Service tier", tier.clone()));
            }
            rows.push(AutomationRow::field(
                "Permission mode",
                permission_mode.clone(),
            ));
            rows.push(AutomationRow::field(
                "Environment",
                environment_label(environment, target_thread_id.as_deref()),
            ));
            if let Some(thread_id) = target_thread_id {
                let mut row = AutomationRow::field("Target thread", thread_label(state, thread_id));
                row.target = Some(RowTarget::Thread(thread_id.clone()));
                rows.push(row);
            }
            push_origin(state, automation, &mut rows);
            rows.push(AutomationRow::new(RowKind::Header, "Prompt"));
            if prompt.is_empty() {
                rows.push(AutomationRow::new(RowKind::Note, "(empty)"));
            }
            for line in prompt.lines() {
                rows.push(AutomationRow::new(RowKind::Text, line));
            }
        }
        Some(Execution::Script {
            script,
            script_file,
            interpreter,
            timeout_ms,
            env,
            working_directory,
            stored_script_path,
            resolved_working_directory,
        }) => {
            rows.push(AutomationRow::field("Mode", "Script"));
            if let Some(interpreter) = interpreter {
                rows.push(AutomationRow::field("Interpreter", interpreter.clone()));
            }
            if let Some(timeout) = timeout_ms {
                rows.push(AutomationRow::field(
                    "Timeout",
                    format!("{}s", timeout / 1000),
                ));
            }
            rows.push(AutomationRow::field(
                "Working directory",
                working_directory_label(
                    working_directory.as_ref(),
                    resolved_working_directory.as_deref(),
                ),
            ));
            if let Some(env) = env.as_ref().filter(|env| !env.is_empty()) {
                rows.push(AutomationRow::field(
                    "Environment variables",
                    env.keys().cloned().collect::<Vec<_>>().join(", "),
                ));
            }
            if let Some(path) = stored_script_path.as_ref().or(script_file.as_ref()) {
                rows.push(AutomationRow::field("Script file", path.clone()));
            }
            push_origin(state, automation, &mut rows);
            if let Some(script) = script {
                rows.push(AutomationRow::new(RowKind::Header, "Script"));
                for line in script.lines() {
                    rows.push(AutomationRow::new(RowKind::Text, line));
                }
            }
        }
        None => {}
    }
    if let Some(error) = &automation.last_error {
        rows.push(AutomationRow::new(RowKind::Header, "Last error"));
        for line in error.lines() {
            rows.push(AutomationRow::new(RowKind::Error, line));
        }
    }
    rows.push(AutomationRow::new(RowKind::Header, "Run history"));
    if load_rows(&detail.runs, "runs", &mut rows) {
        return rows;
    }
    let runs = detail.runs.loaded().map(Vec::as_slice).unwrap_or_default();
    if runs.is_empty() {
        rows.push(AutomationRow::new(
            RowKind::Note,
            "No runs yet · r runs it now",
        ));
    }
    for (index, run) in runs.iter().enumerate() {
        rows.push(run_row(run, index));
    }
    if detail.next_cursor.is_some() {
        let mut more = AutomationRow::new(
            RowKind::Item,
            if detail.loading_more {
                "Loading more runs…"
            } else {
                "Load more runs"
            },
        );
        more.target = Some(RowTarget::LoadMore);
        rows.push(more);
    }
    rows
}

fn push_origin(state: &AppState, automation: &Automation, rows: &mut Vec<AutomationRow>) {
    let origin = match automation.created_by_thread_id.as_deref() {
        Some(thread_id) => format!("{} · {}", automation.origin, thread_label(state, thread_id)),
        None => automation.origin.clone(),
    };
    if !origin.is_empty() {
        rows.push(AutomationRow::field("Created by", origin));
    }
    if automation.created_at > 0 {
        rows.push(AutomationRow::field(
            "Created",
            format_run_time(automation.created_at),
        ));
    }
}

fn run_row(run: &Run, index: usize) -> AutomationRow {
    let tone = match run.status {
        RunStatus::Running => Tone::Accent,
        RunStatus::Succeeded => Tone::Good,
        RunStatus::Failed => Tone::Bad,
        RunStatus::Skipped | RunStatus::Unknown => Tone::Muted,
    };
    let mut parts = Vec::new();
    if run.status == RunStatus::Running {
        parts.push("Running…".to_owned());
    } else if let Some(duration) = run.duration() {
        parts.push(duration);
    }
    parts.push(run.status.label().to_owned());
    if run.trigger == "manual" {
        parts.push("manual".into());
    }
    if let Some(reason) = &run.skip_reason {
        parts.push(reason.clone());
    }
    if let Some(error) = &run.error {
        parts.push(error.lines().next().unwrap_or_default().to_owned());
    }
    if let Some(code) = run.exit_code {
        parts.push(format!("exit {code}"));
    }
    if let Some(output) = run
        .output
        .as_deref()
        .map(str::trim)
        .filter(|out| !out.is_empty())
    {
        parts.push(output.lines().last().unwrap_or_default().to_owned());
    }
    let mut badges = Vec::new();
    if run.openable_thread().is_some() {
        badges.push(("thread".to_owned(), Tone::Accent));
    }
    AutomationRow {
        kind: RowKind::Item,
        glyph: Some((run.status.glyph().to_owned(), tone)),
        label: format_run_time(run.started_at),
        value: parts.join(" · "),
        badges,
        target: Some(RowTarget::Run(index)),
    }
}

pub fn clamp_cursor(rows: &[AutomationRow], cursor: usize) -> Option<usize> {
    if rows.is_empty() {
        return None;
    }
    let cursor = cursor.min(rows.len() - 1);
    if rows[cursor].selectable() {
        return Some(cursor);
    }
    rows[cursor..]
        .iter()
        .position(AutomationRow::selectable)
        .map(|offset| cursor + offset)
        .or_else(|| rows[..cursor].iter().rposition(AutomationRow::selectable))
}

fn step_cursor(rows: &[AutomationRow], cursor: usize, delta: isize) -> usize {
    let selectable: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.selectable())
        .map(|(index, _)| index)
        .collect();
    if selectable.is_empty() {
        return cursor;
    }
    let position = selectable
        .iter()
        .position(|index| *index >= cursor)
        .unwrap_or(selectable.len() - 1);
    let next = position
        .saturating_add_signed(delta)
        .min(selectable.len() - 1);
    selectable[next]
}

pub fn form_value(state: &AppState, form: &AutomationForm, field: FormField) -> String {
    let view = state.automations.as_ref();
    match field {
        FormField::Name => form.name.clone(),
        FormField::Project => project_display(state, &form.project_id),
        FormField::Schedule => match form.schedule {
            ScheduleKind::Recurring => "Recurring (cron)".into(),
            ScheduleKind::Once => "One time".into(),
        },
        FormField::Cron => format!("{}  → {}", form.cron, describe_cron(&form.cron)),
        FormField::Timezone => form.timezone.clone(),
        FormField::RunAt => form.run_at.clone(),
        FormField::Prompt => {
            let mut lines = form.prompt.lines();
            let first = lines.next().unwrap_or_default().to_owned();
            let more = lines.count();
            if more > 0 {
                format!("{first} … (+{more} lines)")
            } else {
                first
            }
        }
        FormField::Provider => view
            .and_then(|view| view.providers.loaded())
            .and_then(|providers| providers.iter().find(|entry| entry.id == form.provider_id))
            .map_or_else(
                || form.provider_id.clone(),
                |entry| entry.display_name.clone(),
            ),
        FormField::Model => view
            .and_then(|view| view.model_options(&form.provider_id))
            .and_then(|options| {
                options
                    .models
                    .iter()
                    .find(|model| model.model == form.model)
            })
            .map_or_else(
                || form.model.clone(),
                |model| format!("{} ({})", model.display_name, model.model),
            ),
        FormField::Reasoning => form.reasoning.clone(),
        FormField::Permission => form.permission.clone(),
        FormField::Environment => form.environment.label(),
        FormField::Paused => if form.start_paused { "yes" } else { "no" }.into(),
        FormField::Save => {
            if form.saving {
                "Saving…".into()
            } else if form.is_create() {
                "[ Create automation ]".into()
            } else {
                "[ Save changes ]".into()
            }
        }
    }
}

fn project_display(state: &AppState, project_id: &str) -> String {
    state
        .snapshot
        .sidebar
        .data
        .as_deref()
        .and_then(|sidebar| {
            sidebar
                .projects
                .iter()
                .chain(std::iter::once(&sidebar.personal_project))
                .find(|project| project.project.id == project_id)
        })
        .map(|project| project_label(&project.project.name, project_id))
        .or_else(|| {
            state
                .automations
                .as_ref()
                .and_then(|view| view.project_name(project_id))
        })
        .unwrap_or_else(|| project_id.to_owned())
}

fn project_choices(state: &AppState) -> Vec<(String, String)> {
    let Some(sidebar) = state.snapshot.sidebar.data.as_deref() else {
        return Vec::new();
    };
    let labels = crate::sidebar::project_labels(sidebar);
    sidebar
        .projects
        .iter()
        .chain(std::iter::once(&sidebar.personal_project))
        .filter(|project| !project.project.id.is_empty())
        .map(|project| {
            (
                project.project.id.clone(),
                labels
                    .get(&project.project.id)
                    .cloned()
                    .unwrap_or_else(|| project_label(&project.project.name, &project.project.id)),
            )
        })
        .collect()
}

fn project_environments(state: &AppState, project_id: &str) -> Vec<(String, String)> {
    let mut seen = HashSet::new();
    let mut environments = Vec::new();
    for entry in state.snapshot.sidebar_threads() {
        if entry.thread.project_id != project_id || entry.thread.is_archived() {
            continue;
        }
        let Some(environment_id) = entry.thread.environment_id.clone() else {
            continue;
        };
        if !seen.insert(environment_id.clone()) {
            continue;
        }
        let label = entry
            .environment_branch_name
            .clone()
            .or_else(|| entry.environment_name.clone())
            .or_else(|| entry.environment_path.clone())
            .unwrap_or_else(|| environment_id.clone());
        environments.push((environment_id, label));
    }
    environments
}

fn worktree_host(state: &AppState) -> Option<String> {
    let hosts = state.snapshot.hosts.data.as_deref()?;
    hosts
        .iter()
        .find(|host| host.status == HostStatus::Connected)
        .or_else(|| hosts.first())
        .map(|host| host.id.clone())
}

fn reasoning_choices(model: Option<&AvailableModel>) -> Vec<String> {
    match model {
        Some(model) if !model.supported_reasoning_efforts.is_empty() => model
            .supported_reasoning_efforts
            .iter()
            .map(|effort| effort.reasoning_effort.as_str().to_owned())
            .collect(),
        _ => ReasoningLevel::KNOWN
            .iter()
            .map(|level| level.as_str().to_owned())
            .collect(),
    }
}

fn permission_choices(
    options: Option<&SystemExecutionOptionsResponse>,
    provider_id: &str,
) -> Vec<String> {
    let modes: Vec<PermissionMode> = options
        .and_then(|options| options.providers.iter().find(|info| info.id == provider_id))
        .map(|info| info.capabilities.permission_modes.clone())
        .filter(|modes| !modes.is_empty())
        .unwrap_or_else(|| PermissionMode::KNOWN.to_vec());
    modes
        .into_iter()
        .filter(|mode| mode.is_known())
        .map(|mode| mode.as_str().to_owned())
        .collect()
}

fn preferred_permission(choices: &[String]) -> String {
    ["auto", "full"]
        .iter()
        .find(|mode| choices.iter().any(|choice| choice == *mode))
        .map(|mode| (*mode).to_owned())
        .or_else(|| choices.first().cloned())
        .unwrap_or_default()
}

fn reconcile_with_models(form: &mut AutomationForm, options: &SystemExecutionOptionsResponse) {
    let model = options
        .models
        .iter()
        .find(|model| model.model == form.model)
        .or_else(|| {
            if !form.model.is_empty() && !form.is_create() {
                return None;
            }
            options
                .models
                .iter()
                .find(|model| model.is_default)
                .or_else(|| options.models.first())
        });
    if let Some(model) = model {
        if form.model != model.model {
            form.model = model.model.clone();
        }
        let levels = reasoning_choices(Some(model));
        if !levels.contains(&form.reasoning) {
            form.reasoning = model.default_reasoning_effort.as_str().to_owned();
        }
    }
    let permissions = permission_choices(Some(options), &form.provider_id);
    if form.permission.is_empty() || !permissions.contains(&form.permission) {
        form.permission = preferred_permission(&permissions);
    }
}

impl AppState {
    pub fn open_automations(&mut self) -> Vec<Effect> {
        if self.automations.is_some() {
            self.automations = None;
            return Vec::new();
        }
        self.close_settings();
        let mut view = AutomationsView::default();
        let effects = view.fetch_overview();
        self.automations = Some(view);
        effects
    }

    pub fn new_automation(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();
        if self.automations.is_none() {
            effects.extend(self.open_automations());
        }
        effects.extend(self.open_form(None));
        effects
    }

    fn default_project(&self) -> Option<String> {
        self.open_thread
            .as_deref()
            .and_then(|thread_id| self.sidebar_entry(thread_id))
            .map(|entry| entry.thread.project_id.clone())
            .or_else(|| match self.selected_row() {
                Some(crate::sidebar::SidebarRow::Thread(row)) => self
                    .sidebar_entry(&row.id)
                    .map(|entry| entry.thread.project_id.clone()),
                _ => None,
            })
            .or_else(|| project_choices(self).first().map(|(id, _)| id.clone()))
    }

    fn open_form(&mut self, editing: Option<Automation>) -> Vec<Effect> {
        let default_project = self.default_project();
        let Some(view) = self.automations.as_mut() else {
            return Vec::new();
        };
        let form = match editing {
            Some(automation) => AutomationForm::from_automation(&automation),
            None => match default_project {
                Some(project_id) => AutomationForm::create(project_id),
                None => {
                    self.toast("No project to create an automation in", ToastLevel::Error);
                    return Vec::new();
                }
            },
        };
        let mut effects = vec![Effect::Refresh(bb_store::ResourceKey::Hosts)];
        if !matches!(view.providers, Load::Loaded(_) | Load::Loading) {
            view.providers = Load::Loading;
            effects.push(Effect::Automations(AutomationRequest::Providers));
        }
        if form.is_create() {
            view.project_defaults
                .insert(form.project_id.clone(), Load::Loading);
            effects.push(Effect::Automations(AutomationRequest::ProjectDefaults {
                project_id: form.project_id.clone(),
            }));
        }
        let provider_id = form.provider_id.clone();
        view.form = Some(form);
        effects.extend(self.ensure_models(&provider_id));
        effects
    }

    fn ensure_models(&mut self, provider_id: &str) -> Vec<Effect> {
        let Some(view) = self.automations.as_mut() else {
            return Vec::new();
        };
        if provider_id.is_empty() {
            return Vec::new();
        }
        if let Some(options) = view.models.get(provider_id).and_then(Load::loaded).cloned() {
            if let Some(form) = view.form.as_mut()
                && form.provider_id == provider_id
            {
                reconcile_with_models(form, &options);
            }
            return Vec::new();
        }
        if matches!(view.models.get(provider_id), Some(Load::Loading)) {
            return Vec::new();
        }
        view.models.insert(provider_id.to_owned(), Load::Loading);
        vec![Effect::Automations(AutomationRequest::Models {
            provider_id: provider_id.to_owned(),
        })]
    }

    fn apply_project_defaults(&mut self) -> Vec<Effect> {
        let Some(view) = self.automations.as_mut() else {
            return Vec::new();
        };
        let Some(form) = view.form.as_mut() else {
            return Vec::new();
        };
        if !form.is_create() || form.provider_touched {
            return Vec::new();
        }
        let defaults = match view.project_defaults.get(&form.project_id) {
            Some(Load::Loaded(defaults)) => defaults.clone(),
            Some(Load::Failed(_)) => None,
            _ => return Vec::new(),
        };
        match defaults {
            Some(defaults) => {
                form.provider_id = defaults.provider_id;
                form.model = defaults.model;
                if defaults.reasoning_level.is_known() {
                    form.reasoning = defaults.reasoning_level.as_str().to_owned();
                }
                if defaults.permission_mode.is_known() {
                    form.permission = defaults.permission_mode.as_str().to_owned();
                }
            }
            None if form.provider_id.is_empty() => {
                let Some(provider) = view.providers.loaded().and_then(|providers| {
                    providers
                        .iter()
                        .find(|entry| entry.enabled && entry.available && entry.plugin_enabled)
                }) else {
                    return Vec::new();
                };
                form.provider_id = provider.id.clone();
            }
            None => {}
        }
        let provider_id = form.provider_id.clone();
        self.ensure_models(&provider_id)
    }

    pub fn close_automations(&mut self) {
        self.automations = None;
    }

    pub fn automations_signal(&mut self, payload: &JsonValue) -> Vec<Effect> {
        let Some(view) = self.automations.as_mut() else {
            return Vec::new();
        };
        let Some(project_id) = payload.get("projectId").and_then(JsonValue::as_str) else {
            return Vec::new();
        };
        let kind = payload.get("kind").and_then(JsonValue::as_str);
        if !matches!(
            kind,
            Some("automations-changed" | "automation-runs-changed")
        ) {
            return Vec::new();
        }
        let mut effects = view.fetch_overview();
        if let Some(detail) = view.detail.as_mut()
            && detail.route.project_id == project_id
        {
            effects.extend(detail.fetch());
            if kind == Some("automation-runs-changed") {
                effects.extend(detail.fetch_runs());
            }
        }
        effects
    }

    pub fn automations_refused(&mut self, mutation: &AutomationMutation) {
        if let Some(view) = self.automations.as_mut() {
            view.pending.remove(&mutation.key());
            if let Some(form) = view.form.as_mut() {
                form.saving = false;
            }
        }
        self.toast(
            format!("Read-only mode: cannot {}", mutation.failure_verb()),
            ToastLevel::Error,
        );
    }

    fn open_automation_detail(&mut self, route: Route) -> Vec<Effect> {
        let Some(view) = self.automations.as_mut() else {
            return Vec::new();
        };
        let cached = view.find(&route).cloned();
        let mut detail = DetailState::new(route, cached);
        let mut effects = detail.fetch();
        effects.extend(detail.fetch_runs());
        view.detail = Some(detail);
        view.filter_input = None;
        effects
    }

    fn mutate(&mut self, mutation: AutomationMutation) -> Vec<Effect> {
        let Some(view) = self.automations.as_mut() else {
            return Vec::new();
        };
        let key = mutation.key();
        if view.pending.contains(&key) {
            return Vec::new();
        }
        view.pending.insert(key);
        vec![Effect::Automations(AutomationRequest::Mutate(mutation))]
    }

    fn toggle_automation(&mut self, route: Route) -> Vec<Effect> {
        let now_ms = self.now_ms;
        let Some(automation) = self
            .automations
            .as_ref()
            .and_then(|view| view.find(&route))
            .cloned()
        else {
            return Vec::new();
        };
        if !automation.allows_toggle(now_ms) {
            self.toast(
                format!(
                    "{} cannot be paused or resumed ({})",
                    automation.name,
                    automation.status_label(now_ms)
                ),
                ToastLevel::Info,
            );
            return Vec::new();
        }
        self.mutate(if automation.enabled {
            AutomationMutation::Pause(route)
        } else {
            AutomationMutation::Resume(route)
        })
    }

    fn run_automation(&mut self, route: Route) -> Vec<Effect> {
        let Some(automation) = self
            .automations
            .as_ref()
            .and_then(|view| view.find(&route))
            .cloned()
        else {
            return Vec::new();
        };
        if automation.is_unreadable() || automation.needs_prompt() {
            self.toast(
                format!("{} cannot run until it is fixed", automation.name),
                ToastLevel::Error,
            );
            return Vec::new();
        }
        self.mutate(AutomationMutation::Run(route))
    }

    fn edit_automation(&mut self, route: Route) -> Vec<Effect> {
        let Some(automation) = self
            .automations
            .as_ref()
            .and_then(|view| view.find(&route))
            .cloned()
        else {
            return Vec::new();
        };
        if automation.is_unreadable() {
            self.toast(
                "The stored configuration cannot be read; delete it instead",
                ToastLevel::Error,
            );
            return Vec::new();
        }
        self.open_form(Some(automation))
    }

    fn request_delete(&mut self, route: Route) {
        let Some(view) = self.automations.as_mut() else {
            return;
        };
        let name = view.find(&route).map_or_else(
            || route.automation_id.clone(),
            |automation| automation.name.clone(),
        );
        view.confirm = Some(DeleteConfirm { route, name });
    }

    pub fn automations_loaded(&mut self, loaded: AutomationLoaded) -> Vec<Effect> {
        let Some(view) = self.automations.as_mut() else {
            return Vec::new();
        };
        match loaded {
            AutomationLoaded::Overview(result) => {
                view.overview_inflight = false;
                if result.is_ok() || view.overview.loaded().is_none() {
                    view.overview = Load::from_result(result);
                }
                if std::mem::take(&mut view.overview_stale) {
                    return view.fetch_overview();
                }
                Vec::new()
            }
            AutomationLoaded::Detail { route, result } => {
                let Some(detail) = view.detail.as_mut().filter(|detail| detail.route == route)
                else {
                    return Vec::new();
                };
                detail.inflight = false;
                if result.is_ok() || detail.automation.loaded().is_none() {
                    detail.automation = Load::from_result(result.map(|automation| *automation));
                }
                if std::mem::take(&mut detail.stale) {
                    return detail.fetch();
                }
                Vec::new()
            }
            AutomationLoaded::Runs {
                route,
                cursor,
                result,
            } => {
                let Some(detail) = view.detail.as_mut().filter(|detail| detail.route == route)
                else {
                    return Vec::new();
                };
                match cursor {
                    None => {
                        detail.runs_inflight = false;
                        match result {
                            Ok(page) => {
                                detail.runs = Load::Loaded(page.runs);
                                detail.next_cursor = page.next_cursor;
                                detail.loading_more = false;
                            }
                            Err(error) => {
                                if detail.runs.loaded().is_none() {
                                    detail.runs = Load::Failed(error);
                                }
                            }
                        }
                        if std::mem::take(&mut detail.runs_stale) {
                            return detail.fetch_runs();
                        }
                    }
                    Some(_) => {
                        detail.loading_more = false;
                        match result {
                            Ok(page) => {
                                if let Load::Loaded(runs) = &mut detail.runs {
                                    runs.extend(page.runs);
                                }
                                detail.next_cursor = page.next_cursor;
                            }
                            Err(error) => {
                                self.toast(
                                    format!("Failed to load more runs: {error}"),
                                    ToastLevel::Error,
                                );
                            }
                        }
                    }
                }
                Vec::new()
            }
            AutomationLoaded::Providers(result) => {
                view.providers = Load::from_result(result);
                self.apply_project_defaults()
            }
            AutomationLoaded::Models {
                provider_id,
                result,
            } => {
                view.models
                    .insert(provider_id.clone(), Load::from_result(result));
                if let Some(Load::Loaded(options)) = view.models.get(&provider_id)
                    && let Some(form) = view.form.as_mut()
                    && form.provider_id == provider_id
                {
                    let options = options.clone();
                    reconcile_with_models(form, &options);
                }
                Vec::new()
            }
            AutomationLoaded::ProjectDefaults { project_id, result } => {
                view.project_defaults
                    .insert(project_id, Load::from_result(result));
                self.apply_project_defaults()
            }
            AutomationLoaded::Mutated { mutation, result } => {
                self.automation_mutated(mutation, result)
            }
        }
    }

    fn automation_mutated(
        &mut self,
        mutation: AutomationMutation,
        result: Result<JsonValue, String>,
    ) -> Vec<Effect> {
        let Some(view) = self.automations.as_mut() else {
            return Vec::new();
        };
        view.pending.remove(&mutation.key());
        let value = match result {
            Ok(value) => value,
            Err(error) => {
                if let Some(form) = view.form.as_mut()
                    && matches!(
                        mutation,
                        AutomationMutation::Create { .. } | AutomationMutation::Update { .. }
                    )
                {
                    form.saving = false;
                    form.error = Some(error.clone());
                }
                self.toast(
                    format!("Failed to {}: {error}", mutation.failure_verb()),
                    ToastLevel::Error,
                );
                return Vec::new();
            }
        };
        let mut effects = view.fetch_overview();
        let parsed = serde_json::from_value::<Automation>(value).ok();
        match &mutation {
            AutomationMutation::Create { .. } => {
                view.form = None;
                if let Some(automation) = parsed {
                    let route = automation.route();
                    let mut detail = DetailState::new(route, Some(automation));
                    effects.extend(detail.fetch_runs());
                    view.detail = Some(detail);
                }
            }
            AutomationMutation::Update { route, .. } => {
                view.form = None;
                if let Some(detail) = view.detail.as_mut().filter(|detail| &detail.route == route) {
                    if let Some(automation) = parsed {
                        detail.automation = Load::Loaded(automation);
                    }
                    effects.extend(detail.fetch());
                }
            }
            AutomationMutation::Delete(route) => {
                if view
                    .detail
                    .as_ref()
                    .is_some_and(|detail| &detail.route == route)
                {
                    view.detail = None;
                }
                if let Load::Loaded(entries) = &mut view.overview {
                    entries.retain(|entry| &entry.automation.route() != route);
                }
            }
            AutomationMutation::Pause(route) | AutomationMutation::Resume(route) => {
                if let Some(automation) = parsed {
                    if let Load::Loaded(entries) = &mut view.overview {
                        for entry in entries.iter_mut() {
                            if &entry.automation.route() == route {
                                entry.automation = automation.clone();
                            }
                        }
                    }
                    if let Some(detail) =
                        view.detail.as_mut().filter(|detail| &detail.route == route)
                    {
                        effects.extend(detail.fetch());
                    }
                }
            }
            AutomationMutation::Run(route) => {
                if let Some(detail) = view.detail.as_mut().filter(|detail| &detail.route == route) {
                    effects.extend(detail.fetch_runs());
                    effects.extend(detail.fetch());
                }
            }
        }
        self.toast(mutation.success_message(), ToastLevel::Info);
        effects
    }

    fn submit_form(&mut self) -> Vec<Effect> {
        let host = worktree_host(self);
        let now_ms = self.now_ms;
        let Some(view) = self.automations.as_mut() else {
            return Vec::new();
        };
        let Some(form) = view.form.as_mut() else {
            return Vec::new();
        };
        if form.saving {
            return Vec::new();
        }
        let built = build_payload(form, host, now_ms);
        match built {
            Err(error) => {
                form.error = Some(error);
                Vec::new()
            }
            Ok(None) => {
                view.form = None;
                self.toast("No changes", ToastLevel::Info);
                Vec::new()
            }
            Ok(Some(mutation)) => {
                form.error = None;
                form.saving = true;
                self.mutate(mutation)
            }
        }
    }
}

fn environment_json(choice: &EnvChoice, host: Option<String>) -> Result<JsonValue, String> {
    Ok(match choice {
        EnvChoice::ProjectDefault => json!({"type": "project-default"}),
        EnvChoice::NewWorktree => {
            let host = host.ok_or("No machine is available for a new worktree")?;
            json!({
                "type": "host",
                "hostId": host,
                "workspace": {"type": "managed-worktree", "baseBranch": {"kind": "default"}},
            })
        }
        EnvChoice::Reuse { environment_id, .. } => {
            json!({"type": "reuse", "environmentId": environment_id})
        }
        EnvChoice::Keep { value, .. } => value.clone(),
    })
}

pub fn build_payload(
    form: &AutomationForm,
    host: Option<String>,
    now_ms: i64,
) -> Result<Option<AutomationMutation>, String> {
    let name = form.name.trim();
    if name.is_empty() {
        return Err("Name is required".into());
    }
    if name.chars().count() > NAME_MAX_LENGTH {
        return Err(format!("Name must be at most {NAME_MAX_LENGTH} characters"));
    }
    let trigger = match form.schedule {
        ScheduleKind::Recurring => {
            validate_cron(form.cron.trim())?;
            if form.timezone.trim().is_empty() {
                return Err("Timezone is required".into());
            }
            json!({
                "triggerType": "schedule",
                "cron": form.cron.trim(),
                "timezone": form.timezone.trim(),
            })
        }
        ScheduleKind::Once => {
            let run_at = parse_run_at(&form.run_at, now_ms)?;
            json!({"triggerType": "once", "runAt": run_at})
        }
    };
    if form.agent {
        if form.prompt.trim().is_empty() {
            return Err("Prompt is required".into());
        }
        if form.provider_id.is_empty() {
            return Err("Pick a provider".into());
        }
        if form.model.is_empty() {
            return Err("Pick a model".into());
        }
        if form.permission.is_empty() {
            return Err("Pick a permission mode".into());
        }
    }
    let Some(original) = &form.editing else {
        let environment = environment_json(&form.environment, host)?;
        return Ok(Some(AutomationMutation::Create {
            input: json!({
                "projectId": form.project_id,
                "name": name,
                "enabled": !form.start_paused,
                "trigger": trigger,
                "execution": {
                    "mode": "agent",
                    "prompt": form.prompt,
                    "providerId": form.provider_id,
                    "model": form.model,
                    "reasoningLevel": form.reasoning,
                    "permissionMode": form.permission,
                    "environment": environment,
                },
                "origin": "human",
            }),
        }));
    };
    let route = original.route();
    let mut input = route_input(&route);
    let object = input.as_object_mut().ok_or("invalid input")?;
    if name != original.name {
        object.insert("name".into(), json!(name));
    }
    let original_trigger = match &original.trigger {
        Some(Trigger::Schedule { cron, timezone }) => {
            Some(json!({"triggerType": "schedule", "cron": cron, "timezone": timezone}))
        }
        Some(Trigger::Once { run_at }) => Some(json!({"triggerType": "once", "runAt": run_at})),
        None => None,
    };
    let trigger_unchanged = match (&original.trigger, form.schedule) {
        (Some(Trigger::Once { run_at }), ScheduleKind::Once) => {
            format_run_at_input(*run_at) == form.run_at.trim()
        }
        _ => original_trigger.as_ref() == Some(&trigger),
    };
    if !trigger_unchanged {
        object.insert("trigger".into(), trigger);
    }
    if let Some(Execution::Agent {
        prompt,
        provider_id,
        model,
        reasoning_level,
        permission_mode,
        ..
    }) = &original.execution
    {
        let mut agent = serde_json::Map::new();
        if &form.prompt != prompt {
            agent.insert("prompt".into(), json!(form.prompt));
        }
        if &form.provider_id != provider_id {
            agent.insert("providerId".into(), json!(form.provider_id));
        }
        if &form.model != model {
            agent.insert("model".into(), json!(form.model));
        }
        let original_reasoning = if reasoning_level.is_empty() {
            "medium"
        } else {
            reasoning_level.as_str()
        };
        if form.reasoning != original_reasoning {
            agent.insert("reasoningLevel".into(), json!(form.reasoning));
        }
        if &form.permission != permission_mode {
            agent.insert("permissionMode".into(), json!(form.permission));
        }
        if !matches!(form.environment, EnvChoice::Keep { .. }) {
            agent.insert(
                "target".into(),
                json!({
                    "type": "environment",
                    "environment": environment_json(&form.environment, host)?,
                }),
            );
        }
        if !agent.is_empty() {
            object.insert("agent".into(), JsonValue::Object(agent));
        }
    }
    if object.len() == 2 {
        return Ok(None);
    }
    Ok(Some(AutomationMutation::Update { route, input }))
}

fn is_ctrl(key: &KeyEvent, ch: char) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char(ch)
}

pub fn handle_automations_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    let Some(view) = state.automations.as_mut() else {
        return Vec::new();
    };
    if let Some(confirm) = view.confirm.take() {
        return match key.code {
            KeyCode::Char('y') | KeyCode::Enter => {
                state.mutate(AutomationMutation::Delete(confirm.route))
            }
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q') => Vec::new(),
            _ => {
                view.confirm = Some(confirm);
                Vec::new()
            }
        };
    }
    if view.form.is_some() {
        return handle_form_key(state, key);
    }
    if let Some(mut input) = view.filter_input.take() {
        match input.handle_key(key) {
            InputOutcome::Submit => view.filter = input.text().to_owned(),
            InputOutcome::Cancel => view.filter.clear(),
            _ => view.filter_input = Some(input),
        }
        view.cursor = 0;
        return Vec::new();
    }
    if is_ctrl(&key, 'k') {
        return crate::app::run_command(state, Command::PaletteOpen);
    }
    if is_ctrl(&key, 'c') {
        state.close_automations();
        return Vec::new();
    }
    if is_ctrl(&key, 'r') || key.code == KeyCode::Char('R') {
        return reload(state);
    }
    let rows = automation_rows(state);
    let Some(view) = state.automations.as_mut() else {
        return Vec::new();
    };
    let in_detail = view.detail.is_some();
    let raw_cursor = match &view.detail {
        Some(detail) => detail.cursor,
        None => view.cursor,
    };
    let cursor = clamp_cursor(&rows, raw_cursor).unwrap_or(0);
    let target = rows.get(cursor).and_then(|row| row.target.clone());
    let current_route = match &view.detail {
        Some(detail) => Some(detail.route.clone()),
        None => match &target {
            Some(RowTarget::Automation(route)) => Some(route.clone()),
            _ => None,
        },
    };
    let set_cursor = |view: &mut AutomationsView, next: usize| match view.detail.as_mut() {
        Some(detail) => detail.cursor = next,
        None => view.cursor = next,
    };
    match key.code {
        KeyCode::Down | KeyCode::Char('j') => set_cursor(view, step_cursor(&rows, cursor, 1)),
        KeyCode::Up | KeyCode::Char('k') => set_cursor(view, step_cursor(&rows, cursor, -1)),
        KeyCode::PageDown => set_cursor(view, step_cursor(&rows, cursor, PAGE)),
        KeyCode::PageUp => set_cursor(view, step_cursor(&rows, cursor, -PAGE)),
        KeyCode::Char('g') | KeyCode::Home => set_cursor(view, step_cursor(&rows, 0, 0)),
        KeyCode::Char('G') | KeyCode::End => {
            set_cursor(view, step_cursor(&rows, rows.len().saturating_sub(1), 0));
        }
        KeyCode::Esc | KeyCode::Backspace | KeyCode::Char('h') | KeyCode::Left if in_detail => {
            view.detail = None;
        }
        KeyCode::Esc if !view.filter.is_empty() => view.filter.clear(),
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('A') => state.close_automations(),
        KeyCode::Char('/') if !in_detail => {
            view.filter_input = Some(LineInput::new(view.filter.clone()));
        }
        KeyCode::Char('n') => return state.open_form(None),
        KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => match target {
            Some(RowTarget::Automation(route)) => return state.open_automation_detail(route),
            Some(RowTarget::Run(index)) => return open_run_thread(state, index),
            Some(RowTarget::LoadMore) => return load_more_runs(state),
            Some(RowTarget::Thread(thread_id)) => return open_thread(state, thread_id),
            None => {}
        },
        KeyCode::Char('m') if in_detail => return load_more_runs(state),
        KeyCode::Char(' ') | KeyCode::Char('p') => {
            if let Some(route) = current_route {
                return state.toggle_automation(route);
            }
        }
        KeyCode::Char('r') => {
            if let Some(route) = current_route {
                return state.run_automation(route);
            }
        }
        KeyCode::Char('e') => {
            if let Some(route) = current_route {
                return state.edit_automation(route);
            }
        }
        KeyCode::Char('d') | KeyCode::Char('D') | KeyCode::Delete => {
            if let Some(route) = current_route {
                state.request_delete(route);
            }
        }
        KeyCode::Char('?') => return crate::app::run_command(state, Command::Help),
        _ => {}
    }
    Vec::new()
}

fn reload(state: &mut AppState) -> Vec<Effect> {
    let Some(view) = state.automations.as_mut() else {
        return Vec::new();
    };
    let mut effects = view.fetch_overview();
    if let Some(detail) = view.detail.as_mut() {
        effects.extend(detail.fetch());
        effects.extend(detail.fetch_runs());
    }
    state.toast("Reloading", ToastLevel::Info);
    effects
}

fn open_thread(state: &mut AppState, thread_id: String) -> Vec<Effect> {
    state.close_automations();
    let effects = state.open(thread_id);
    state.focus = Focus::Main;
    effects
}

fn open_run_thread(state: &mut AppState, index: usize) -> Vec<Effect> {
    let thread = state
        .automations
        .as_ref()
        .and_then(|view| view.detail.as_ref())
        .and_then(|detail| detail.runs.loaded())
        .and_then(|runs| runs.get(index))
        .and_then(|run| run.openable_thread().map(str::to_owned));
    match thread {
        Some(thread_id) => open_thread(state, thread_id),
        None => {
            state.toast("This run has no thread to open", ToastLevel::Info);
            Vec::new()
        }
    }
}

fn load_more_runs(state: &mut AppState) -> Vec<Effect> {
    let Some(detail) = state
        .automations
        .as_mut()
        .and_then(|view| view.detail.as_mut())
    else {
        return Vec::new();
    };
    if detail.loading_more {
        return Vec::new();
    }
    let Some(cursor) = detail.next_cursor.clone() else {
        return Vec::new();
    };
    detail.loading_more = true;
    vec![Effect::Automations(AutomationRequest::Runs {
        route: detail.route.clone(),
        cursor: Some(cursor),
    })]
}

fn handle_form_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    let Some(form) = state
        .automations
        .as_mut()
        .and_then(|view| view.form.as_mut())
    else {
        return Vec::new();
    };
    if let Some(mut edit) = form.edit.take() {
        return match edit.input.handle_key(key) {
            InputOutcome::Cancel => Vec::new(),
            InputOutcome::Submit => match commit_edit(form, &edit, state.now_ms) {
                Ok(()) => Vec::new(),
                Err(error) => {
                    edit.error = Some(error);
                    form.edit = Some(edit);
                    Vec::new()
                }
            },
            _ => {
                edit.error = None;
                form.edit = Some(edit);
                Vec::new()
            }
        };
    }
    if let Some(mut choice) = form.choice.take() {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {}
            KeyCode::Enter | KeyCode::Char(' ') => return choose(state, choice),
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                choice.selected = (choice.selected + 1).min(choice.options.len().saturating_sub(1));
                form.choice = Some(choice);
            }
            KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => {
                choice.selected = choice.selected.saturating_sub(1);
                form.choice = Some(choice);
            }
            KeyCode::Char(ch) if ch.is_ascii_digit() && ch != '0' => {
                let index = usize::from(ch as u8 - b'1');
                if index < choice.options.len() {
                    choice.selected = index;
                    return choose(state, choice);
                }
                form.choice = Some(choice);
            }
            _ => form.choice = Some(choice),
        }
        return Vec::new();
    }
    if is_ctrl(&key, 's') {
        return state.submit_form();
    }
    let count = form.fields().len();
    match key.code {
        KeyCode::Esc => {
            if let Some(view) = state.automations.as_mut() {
                view.form = None;
            }
        }
        KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
            form.cursor = (form.cursor + 1).min(count - 1);
        }
        KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => {
            form.cursor = form.cursor.saturating_sub(1);
        }
        KeyCode::Char('g') | KeyCode::Home => form.cursor = 0,
        KeyCode::Char('G') | KeyCode::End => form.cursor = count - 1,
        KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Char('e') => {
            return activate_field(state);
        }
        _ => {}
    }
    Vec::new()
}

fn edit_hint(field: FormField) -> Option<String> {
    Some(
        match field {
            FormField::Cron => "minute hour day-of-month month day-of-week, e.g. 0 9 * * 1-5",
            FormField::Timezone => "IANA timezone, e.g. Europe/Berlin or UTC",
            FormField::RunAt => "YYYY-MM-DD HH:MM (local), ISO time, or a delay like 30m, 2h, 1d",
            FormField::Prompt => "\\n inserts a line break",
            _ => return None,
        }
        .to_owned(),
    )
}

fn commit_edit(form: &mut AutomationForm, edit: &FormEdit, now_ms: i64) -> Result<(), String> {
    let text = edit.input.text();
    match edit.field {
        FormField::Name => {
            let name = text.trim();
            if name.is_empty() {
                return Err("Name is required".into());
            }
            if name.chars().count() > NAME_MAX_LENGTH {
                return Err(format!("At most {NAME_MAX_LENGTH} characters"));
            }
            form.name = name.to_owned();
        }
        FormField::Cron => {
            validate_cron(text.trim())?;
            form.cron = text.trim().to_owned();
        }
        FormField::Timezone => {
            if text.trim().is_empty() {
                return Err("Timezone is required".into());
            }
            form.timezone = text.trim().to_owned();
        }
        FormField::RunAt => {
            let at = parse_run_at(text, now_ms)?;
            form.run_at = format_run_at_input(at);
        }
        FormField::Prompt => {
            let prompt = unescape_multiline(text);
            if prompt.trim().is_empty() {
                return Err("Prompt is required".into());
            }
            form.prompt = prompt;
        }
        _ => {}
    }
    form.error = None;
    Ok(())
}

fn activate_field(state: &mut AppState) -> Vec<Effect> {
    let choices = field_choices(state);
    let Some(view) = state.automations.as_mut() else {
        return Vec::new();
    };
    let Some(form) = view.form.as_mut() else {
        return Vec::new();
    };
    if form.saving {
        return Vec::new();
    }
    let field = form.current_field();
    if field.is_text() {
        let initial = match field {
            FormField::Name => form.name.clone(),
            FormField::Cron => form.cron.clone(),
            FormField::Timezone => form.timezone.clone(),
            FormField::RunAt => form.run_at.clone(),
            FormField::Prompt => escape_multiline(&form.prompt),
            _ => String::new(),
        };
        form.edit = Some(FormEdit {
            field,
            input: LineInput::new(initial),
            error: None,
            hint: edit_hint(field),
        });
        return Vec::new();
    }
    match field {
        FormField::Paused => {
            form.start_paused = !form.start_paused;
            Vec::new()
        }
        FormField::Save => state.submit_form(),
        FormField::Project if !form.is_create() => {
            state.toast(
                "An automation cannot move to another project",
                ToastLevel::Info,
            );
            Vec::new()
        }
        _ => {
            let Some((title, options)) = choices else {
                return Vec::new();
            };
            if options.is_empty() {
                let message = match field {
                    FormField::Provider if !matches!(view.providers, Load::Loaded(_)) => {
                        "Loading providers…"
                    }
                    FormField::Model => "Loading models…",
                    _ => "Nothing to choose from",
                };
                state.toast(message, ToastLevel::Info);
                return Vec::new();
            }
            let current = current_choice_value(form, field);
            let selected = options
                .iter()
                .position(|(value, _)| *value == current)
                .unwrap_or(0);
            form.choice = Some(FormChoice {
                field,
                title,
                options,
                selected,
            });
            Vec::new()
        }
    }
}

fn current_choice_value(form: &AutomationForm, field: FormField) -> String {
    match field {
        FormField::Project => form.project_id.clone(),
        FormField::Schedule => match form.schedule {
            ScheduleKind::Recurring => "recurring".into(),
            ScheduleKind::Once => "once".into(),
        },
        FormField::Provider => form.provider_id.clone(),
        FormField::Model => form.model.clone(),
        FormField::Reasoning => form.reasoning.clone(),
        FormField::Permission => form.permission.clone(),
        FormField::Environment => form.environment.value(),
        _ => String::new(),
    }
}

type ChoiceList = (String, Vec<(String, String)>);

fn field_choices(state: &AppState) -> Option<ChoiceList> {
    let view = state.automations.as_ref()?;
    let form = view.form.as_ref()?;
    let field = form.current_field();
    Some(match field {
        FormField::Project => ("Project".into(), project_choices(state)),
        FormField::Schedule => (
            "Schedule".into(),
            vec![
                ("recurring".into(), "Recurring (cron)".into()),
                ("once".into(), "One time".into()),
            ],
        ),
        FormField::Provider => (
            "Provider".into(),
            view.providers
                .loaded()
                .map(|providers| {
                    providers
                        .iter()
                        .filter(|entry| entry.enabled && entry.available && entry.plugin_enabled)
                        .map(|entry| (entry.id.clone(), entry.display_name.clone()))
                        .collect()
                })
                .unwrap_or_default(),
        ),
        FormField::Model => (
            "Model".into(),
            view.model_options(&form.provider_id)
                .map(|options| {
                    options
                        .models
                        .iter()
                        .map(|model| {
                            (
                                model.model.clone(),
                                format!("{} ({})", model.display_name, model.model),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
        ),
        FormField::Reasoning => {
            let model = view.model_options(&form.provider_id).and_then(|options| {
                options
                    .models
                    .iter()
                    .find(|model| model.model == form.model)
            });
            (
                "Reasoning".into(),
                reasoning_choices(model)
                    .into_iter()
                    .map(|level| (level.clone(), level))
                    .collect(),
            )
        }
        FormField::Permission => (
            "Permission mode".into(),
            permission_choices(view.model_options(&form.provider_id), &form.provider_id)
                .into_iter()
                .map(|mode| (mode.clone(), mode))
                .collect(),
        ),
        FormField::Environment => {
            let mut options = Vec::new();
            if let EnvChoice::Keep { label, .. } = &form.environment {
                options.push(("keep".into(), format!("Keep: {label}")));
            } else if let Some(original) = form.editing.as_ref()
                && let Some(Execution::Agent {
                    environment,
                    target_thread_id,
                    ..
                }) = &original.execution
            {
                options.push((
                    "keep".into(),
                    format!(
                        "Keep: {}",
                        environment_label(environment, target_thread_id.as_deref())
                    ),
                ));
            }
            options.push(("default".into(), "Project default".into()));
            options.push(("worktree".into(), "New worktree each run".into()));
            for (id, label) in project_environments(state, &form.project_id) {
                options.push((format!("reuse:{id}"), format!("Reuse {label}")));
            }
            ("Environment".into(), options)
        }
        _ => return None,
    })
}

fn choose(state: &mut AppState, choice: FormChoice) -> Vec<Effect> {
    let Some((value, label)) = choice.options.get(choice.selected).cloned() else {
        return Vec::new();
    };
    let Some(view) = state.automations.as_mut() else {
        return Vec::new();
    };
    let Some(form) = view.form.as_mut() else {
        return Vec::new();
    };
    match choice.field {
        FormField::Project => {
            if form.project_id != value {
                form.project_id = value.clone();
                if matches!(form.environment, EnvChoice::Reuse { .. }) {
                    form.environment = EnvChoice::ProjectDefault;
                }
                if !form.provider_touched && !view.project_defaults.contains_key(&value) {
                    view.project_defaults.insert(value.clone(), Load::Loading);
                    return vec![Effect::Automations(AutomationRequest::ProjectDefaults {
                        project_id: value,
                    })];
                }
                return state.apply_project_defaults();
            }
        }
        FormField::Schedule => {
            form.schedule = if value == "once" {
                ScheduleKind::Once
            } else {
                ScheduleKind::Recurring
            };
        }
        FormField::Provider => {
            form.provider_touched = true;
            if form.provider_id != value {
                form.provider_id = value.clone();
                form.model.clear();
                form.permission.clear();
                return state.ensure_models(&value);
            }
        }
        FormField::Model => {
            form.model = value;
            if let Some(options) = view
                .models
                .get(&form.provider_id)
                .and_then(Load::loaded)
                .cloned()
            {
                reconcile_with_models(form, &options);
            }
        }
        FormField::Reasoning => form.reasoning = value,
        FormField::Permission => form.permission = value,
        FormField::Environment => {
            form.environment = match value.as_str() {
                "default" => EnvChoice::ProjectDefault,
                "worktree" => EnvChoice::NewWorktree,
                "keep" => match form
                    .editing
                    .as_ref()
                    .and_then(|original| original.execution.as_ref())
                {
                    Some(Execution::Agent {
                        environment,
                        target_thread_id,
                        ..
                    }) => EnvChoice::Keep {
                        value: environment.clone(),
                        target_thread: target_thread_id.clone(),
                        label: environment_label(environment, target_thread_id.as_deref()),
                    },
                    _ => form.environment.clone(),
                },
                other => match other.strip_prefix("reuse:") {
                    Some(id) => EnvChoice::Reuse {
                        environment_id: id.to_owned(),
                        label: label.trim_start_matches("Reuse ").to_owned(),
                    },
                    None => form.environment.clone(),
                },
            };
        }
        _ => {}
    }
    Vec::new()
}

pub fn automations_hints(view: &AutomationsView) -> String {
    if let Some(form) = &view.form {
        if form.edit.is_some() {
            return "RET save field · ESC cancel".into();
        }
        if form.choice.is_some() {
            return "j/k move · RET choose · ESC cancel".into();
        }
        return "j/k field · RET edit · C-s save · ESC discard".into();
    }
    if view.confirm.is_some() {
        return "y/RET delete · ESC cancel".into();
    }
    if view.filter_input.is_some() {
        return "type to filter · RET keep · ESC clear".into();
    }
    if view.detail.is_some() {
        "j/k move · RET open thread · ~ pause/resume · r run now · e edit · dd delete · gm more runs · q back".into()
    } else {
        "j/k move · RET details · o new · ~ pause/resume · r run now · e edit · dd delete · g/ filter · gr reload · q close".into()
    }
}
