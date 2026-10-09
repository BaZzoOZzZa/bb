mod support;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use bb_store::StoreSnapshot;
use bb_tui::app::{Action, AppState, Effect, Focus, ToastLevel, update};
use bb_tui::creation::{PERMISSION_CEILING_REASON, PROVIDER_INPUTS_CONTROL_MISSING_REASON};
use bb_tui::new_thread::{CreationConfig, FormField};
use bb_tui::runtime::run_new_thread_request;
use bb_wire::environment::{
    Environment, EnvironmentStatus, GitCheckoutRef, Host, HostStatus, HostType, MachineLifecycle,
    MachinePhase,
};
use bb_wire::project::{ProjectBranchesResponse, ProjectSource, SidebarBootstrapResponse};
use bb_wire::prompt::{PermissionMode, ProjectExecutionDefaults, ReasoningLevel};
use bb_wire::system::{
    AvailableModel, EnvironmentProviderRequirements, ModelReasoningEffort, ProviderAvailability,
    ProviderCapabilities, ProviderInfo, SystemEnvironmentProvider, SystemExecutionOptionsResponse,
};
use bb_wire::thread::ThreadResponse;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use support::*;

fn host(id: &str, name: &str, ceiling: PermissionMode) -> Host {
    Host {
        id: id.into(),
        name: name.into(),
        host_type: HostType::Persistent,
        status: HostStatus::Connected,
        lifecycle: MachineLifecycle {
            phase: MachinePhase::Active,
            ..MachineLifecycle::default()
        },
        max_permission_mode: ceiling,
        ..Host::default()
    }
}

fn source(project: &str, host: &str) -> ProjectSource {
    ProjectSource {
        id: format!("src_{host}"),
        project_id: project.into(),
        source_type: "local_path".into(),
        host_id: host.into(),
        path: format!("/work/{project}"),
        is_default: true,
        ..ProjectSource::default()
    }
}

fn environment_provider(id: &str, name: &str) -> SystemEnvironmentProvider {
    SystemEnvironmentProvider {
        id: id.into(),
        display_name: name.into(),
        plugin_id: id.into(),
        accepts_empty_inputs: true,
        inputs: Some(serde_json::json!({"type": "object"})),
        requires: EnvironmentProviderRequirements {
            project_checkout: true,
            git_checkout: id == "git-worktree",
            ..EnvironmentProviderRequirements::default()
        },
        ..SystemEnvironmentProvider::default()
    }
}

fn registered_providers() -> Vec<SystemEnvironmentProvider> {
    vec![
        environment_provider("project-checkout", "Project checkout"),
        environment_provider("git-worktree", "Git worktree"),
        SystemEnvironmentProvider {
            accepts_empty_inputs: false,
            ..environment_provider("docker", "Docker container")
        },
        SystemEnvironmentProvider {
            machine_provider_id: Some("modal".into()),
            inputs: None,
            requires: EnvironmentProviderRequirements::default(),
            ..environment_provider("modal-sandbox", "Modal sandbox")
        },
        SystemEnvironmentProvider {
            inputs: None,
            requires: EnvironmentProviderRequirements {
                projectless: true,
                ..EnvironmentProviderRequirements::default()
            },
            ..environment_provider("personal-workspace", "Personal workspace")
        },
    ]
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

fn catalog() -> SystemExecutionOptionsResponse {
    use ReasoningLevel as R;
    SystemExecutionOptionsResponse {
        providers: vec![
            ProviderInfo {
                id: "codex".into(),
                display_name: "Codex".into(),
                capabilities: ProviderCapabilities {
                    permission_modes: vec![
                        PermissionMode::AcceptEdits,
                        PermissionMode::Auto,
                        PermissionMode::Full,
                    ],
                    ..ProviderCapabilities::default()
                },
                ..ProviderInfo::default()
            },
            ProviderInfo {
                id: "claude".into(),
                display_name: "Claude".into(),
                ..ProviderInfo::default()
            },
        ],
        permission_ceiling: PermissionMode::Unknown,
        models: vec![
            model("gpt-5", true, &[R::Low, R::Medium, R::High]),
            model("gpt-5-mini", false, &[R::Low]),
        ],
        selected_only_models: Vec::new(),
        model_load_error: None,
    }
}

fn branches(dirty: bool) -> ProjectBranchesResponse {
    ProjectBranchesResponse {
        checkout: GitCheckoutRef::Branch {
            branch_name: "main".into(),
            head_sha: None,
        },
        default_branch: Some("main".into()),
        has_uncommitted_changes: dirty,
        branches: vec!["main".into(), "feature-x".into()],
        remote_branches: vec!["origin/main".into()],
        default_worktree_base_branch: Some("main".into()),
        ..ProjectBranchesResponse::default()
    }
}

fn sidebar() -> SidebarBootstrapResponse {
    let mut sidebar = sample_sidebar();
    let alpha = &mut sidebar.projects[0];
    alpha.sources = vec![source("proj_a", "host_1"), source("proj_a", "host_2")];
    for entry in &mut alpha.threads {
        if entry.thread.id == "thr_old" {
            entry.thread.environment_id = Some("env_wt".into());
            entry.environment_branch_name = Some("feature-x".into());
            entry.environment_host_id = Some("host_1".into());
            entry.thread.section_id = Some("sec_1".into());
            entry.thread.pinned_at = Some(NOW_MS);
        }
    }
    sidebar
}

fn hosts() -> Vec<Host> {
    vec![
        host("host_1", "laptop", PermissionMode::Full),
        host("host_2", "server", PermissionMode::Auto),
    ]
}

struct Fixture {
    api: Arc<MockApi>,
    state: AppState,
}

fn fixture(configure: impl FnOnce(&mut MockApi)) -> Fixture {
    let backend = FakeBackend::new(sidebar());
    let mut api = MockApi::new(backend);
    let mock = Arc::get_mut(&mut api).unwrap();
    mock.environment_providers = registered_providers();
    let mut project_view = registered_providers();
    project_view[1].machine_availability = BTreeMap::from([(
        "host_2".to_owned(),
        Some(ProviderAvailability::Unavailable {
            message: "git is not installed".into(),
        }),
    )]);
    mock.project_environment_providers = HashMap::from([("proj_a".to_owned(), project_view)]);
    mock.creation_config = CreationConfig {
        primary_host_id: Some("host_1".into()),
        allow_fast_service_tier: true,
    };
    mock.execution_options = catalog();
    mock.project_defaults = HashMap::from([(
        "proj_a".to_owned(),
        ProjectExecutionDefaults {
            provider_id: "codex".into(),
            model: "gpt-5".into(),
            service_tier: String::new(),
            reasoning_level: ReasoningLevel::High,
            permission_mode: PermissionMode::Auto,
        },
    )]);
    mock.branches = branches(false);
    configure(mock);
    let mut state = loaded_state(sidebar());
    let mut snapshot = StoreSnapshot::clone(&state.snapshot);
    snapshot.hosts.data = Some(Arc::new(hosts()));
    update(&mut state, Action::Snapshot(Arc::new(snapshot)));
    Fixture { api, state }
}

impl Fixture {
    async fn drive(&mut self, effects: Vec<Effect>) -> Vec<Effect> {
        let mut pending = effects;
        let mut other = Vec::new();
        while !pending.is_empty() {
            for effect in std::mem::take(&mut pending) {
                match effect {
                    Effect::NewThread {
                        generation,
                        request,
                    } => {
                        let loaded = run_new_thread_request(self.api.as_ref(), request).await;
                        pending.extend(update(
                            &mut self.state,
                            Action::NewThreadLoaded { generation, loaded },
                        ));
                    }
                    effect => other.push(effect),
                }
            }
        }
        other
    }

    async fn key(&mut self, event: KeyEvent) -> Vec<Effect> {
        let effects = update(&mut self.state, Action::Key(event));
        self.drive(effects).await
    }

    async fn press(&mut self, code: KeyCode) -> Vec<Effect> {
        self.key(KeyEvent::from(code)).await
    }

    async fn keys(&mut self, text: &str) -> Vec<Effect> {
        let mut effects = Vec::new();
        for chord in bb_tui::keymap::seq(text) {
            effects.extend(self.key(chord.event()).await);
        }
        effects
    }

    async fn submit(&mut self) -> Vec<Effect> {
        self.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL))
            .await
    }

    async fn type_text(&mut self, text: &str) {
        for ch in text.chars() {
            self.press(KeyCode::Char(ch)).await;
        }
    }

    fn screen(&self) -> String {
        render_lines(&self.state, 120, 32).join("\n")
    }

    fn field(&self) -> FormField {
        self.state.new_thread_form().expect("form").field
    }

    fn picker_labels(&self) -> Vec<(String, bool, Option<String>)> {
        let form = self.state.new_thread_form().expect("form");
        let picker = form.picker.as_ref().expect("picker");
        picker
            .visible()
            .into_iter()
            .map(|index| {
                let item = &picker.items[index];
                (item.label.clone(), item.disabled, item.detail.clone())
            })
            .collect()
    }

    async fn go_to(&mut self, field: FormField) {
        for _ in 0..10 {
            if self.field() == field {
                return;
            }
            self.press(KeyCode::Char('j')).await;
        }
        panic!("field {field:?} not reachable");
    }

    fn created_json(&self) -> serde_json::Value {
        let created = self.api.created.lock().unwrap();
        serde_json::to_value(created.last().expect("created request")).unwrap()
    }
}

#[tokio::test]
async fn n_opens_a_form_resolved_from_project_defaults_and_primary_checkout() {
    let mut fx = fixture(|_| {});
    fx.keys("SPC b N").await;
    assert_eq!(fx.state.focus, Focus::NewThread);
    let screen = fx.screen();
    assert!(screen.contains("New thread"), "{screen}");
    assert!(screen.contains("Project      alpha"), "{screen}");
    assert!(
        screen.contains("Environment  laptop · Project checkout"),
        "{screen}"
    );
    assert!(screen.contains("Branch       Current (main)"), "{screen}");
    assert!(screen.contains("Provider     Codex"), "{screen}");
    assert!(screen.contains("Model        GPT-5"), "{screen}");
    assert!(screen.contains("Reasoning    High"), "{screen}");
    assert!(screen.contains("Permissions  Approve for me"), "{screen}");
    assert!(
        screen.contains("Enter a prompt or attach a file."),
        "{screen}"
    );

    let provider_queries = fx.api.provider_queries.lock().unwrap().clone();
    assert!(
        provider_queries
            .iter()
            .any(|query| query.project_id.is_none())
    );
    assert!(
        provider_queries
            .iter()
            .any(|query| query.project_id.as_deref() == Some("proj_a"))
    );
    let catalog = fx.api.execution_queries.lock().unwrap().clone();
    assert_eq!(catalog.last().unwrap().host_id.as_deref(), Some("host_1"));
    assert_eq!(
        catalog.last().unwrap().provider_id.as_deref(),
        Some("codex")
    );
    let branch_queries = fx.api.branch_queries.lock().unwrap().clone();
    assert_eq!(branch_queries.last().unwrap().0, "proj_a");
    assert_eq!(branch_queries.last().unwrap().1.host_id, "host_1");
}

#[tokio::test]
async fn environment_picker_disables_invalid_choices_with_reasons() {
    let mut fx = fixture(|_| {});
    fx.keys("SPC b N").await;
    fx.go_to(FormField::Environment).await;
    fx.press(KeyCode::Enter).await;
    let labels = fx.picker_labels();
    let find = |label: &str| labels.iter().find(|(text, _, _)| text == label).cloned();
    assert!(!find("laptop · Project checkout").unwrap().1);
    assert!(!find("laptop · Git worktree").unwrap().1);
    let docker = find("laptop · Docker container").unwrap();
    assert!(docker.1);
    assert_eq!(
        docker.2.as_deref(),
        Some(PROVIDER_INPUTS_CONTROL_MISSING_REASON)
    );
    assert!(find("server · Git worktree").is_none());
    assert!(!find("server · Project checkout").unwrap().1);
    assert!(!find("Modal sandbox").unwrap().1);
    assert!(find("Personal workspace").is_none());
    assert!(!find("Reuse existing environment…").unwrap().1);
    let screen = fx.screen();
    assert!(screen.contains("Needs its plugin's control"), "{screen}");

    fx.type_text("docker").await;
    fx.press(KeyCode::Enter).await;
    assert!(fx.state.new_thread_form().unwrap().picker.is_some());
    assert!(
        fx.state
            .toasts
            .iter()
            .any(|toast| toast.message == PROVIDER_INPUTS_CONTROL_MISSING_REASON)
    );

    for _ in 0..6 {
        fx.press(KeyCode::Backspace).await;
    }
    fx.type_text("laptop · git").await;
    fx.press(KeyCode::Enter).await;
    assert_eq!(fx.field(), FormField::Branch);
    let screen = fx.screen();
    assert!(
        screen.contains("Environment  laptop · Git worktree"),
        "{screen}"
    );
    assert!(
        screen.contains("Branch       Branch from: main"),
        "{screen}"
    );
}

#[tokio::test]
async fn worktree_thread_is_created_with_the_picked_branch_and_opened() {
    let mut fx = fixture(|_| {});
    fx.keys("SPC b N").await;
    fx.go_to(FormField::Environment).await;
    fx.press(KeyCode::Enter).await;
    fx.type_text("laptop · git").await;
    fx.press(KeyCode::Enter).await;
    fx.press(KeyCode::Enter).await;
    fx.type_text("feature").await;
    fx.press(KeyCode::Enter).await;
    assert_eq!(fx.field(), FormField::Provider);
    fx.press(KeyCode::Char('i')).await;
    fx.type_text("Fix the login bug").await;
    let effects = fx.submit().await;

    let request = fx.created_json();
    assert_eq!(
        request,
        serde_json::json!({
            "projectId": "proj_a",
            "providerId": "codex",
            "origin": "app",
            "input": [{"type": "text", "text": "Fix the login bug", "mentions": []}],
            "model": "gpt-5",
            "reasoningLevel": "high",
            "permissionMode": "auto",
            "executionInputSources": {},
            "environment": {
                "type": "provider",
                "environmentProviderId": "git-worktree",
                "machine": {"type": "existing", "hostId": "host_1"},
                "inputs": {"branch": {"kind": "named", "name": "feature-x"}}
            },
            "sectionId": null,
            "pinned": false
        })
    );
    assert!(fx.state.new_thread_form().is_none());
    assert_eq!(fx.state.open_thread.as_deref(), Some("thr_new1"));
    assert_eq!(fx.state.focus, Focus::Main);
    assert!(effects.contains(&Effect::RefreshSidebar));
    assert!(effects.contains(&Effect::WatchThread(Some("thr_new1".into()))));
    assert!(
        fx.state
            .toasts
            .iter()
            .any(|toast| toast.message == "Created: Created thread")
    );
}

#[tokio::test]
async fn checkout_branch_changes_are_disabled_while_the_checkout_is_dirty() {
    let mut fx = fixture(|api| api.branches = branches(true));
    fx.keys("SPC b N").await;
    fx.go_to(FormField::Branch).await;
    fx.press(KeyCode::Enter).await;
    let labels = fx.picker_labels();
    assert_eq!(labels[0].0, "Current: main");
    assert!(!labels[0].1);
    let checkout = labels
        .iter()
        .find(|(label, _, _)| label == "Checkout: feature-x")
        .unwrap();
    assert!(checkout.1);
    assert_eq!(
        checkout.2.as_deref(),
        Some("Checkout blocked by uncommitted changes")
    );
    assert!(
        labels
            .iter()
            .any(|(label, disabled, _)| label == "New branch from: origin/main" && *disabled)
    );
    let screen = fx.screen();
    assert!(
        screen.contains("Branch       Current (main)  Dirty"),
        "{screen}"
    );
}

#[tokio::test]
async fn clean_checkout_sends_a_new_branch_input() {
    let mut fx = fixture(|_| {});
    fx.keys("SPC b N").await;
    fx.go_to(FormField::Branch).await;
    fx.press(KeyCode::Enter).await;
    fx.type_text("new branch from: main").await;
    fx.press(KeyCode::Enter).await;
    fx.press(KeyCode::Char('i')).await;
    fx.type_text("go").await;
    fx.submit().await;
    assert_eq!(
        fx.created_json()["environment"],
        serde_json::json!({
            "type": "provider",
            "environmentProviderId": "project-checkout",
            "machine": {"type": "existing", "hostId": "host_1"},
            "inputs": {"branch": {"kind": "new", "baseBranch": "main"}}
        })
    );
}

#[tokio::test]
async fn machine_permission_limit_disables_modes_and_explicit_picks_are_marked() {
    let mut fx = fixture(|_| {});
    fx.keys("SPC b N").await;
    fx.go_to(FormField::Environment).await;
    fx.press(KeyCode::Enter).await;
    fx.type_text("server · project").await;
    fx.press(KeyCode::Enter).await;
    let catalog = fx.api.execution_queries.lock().unwrap().clone();
    assert_eq!(catalog.last().unwrap().host_id.as_deref(), Some("host_2"));

    fx.go_to(FormField::Permission).await;
    fx.press(KeyCode::Enter).await;
    let labels = fx.picker_labels();
    let full = labels
        .iter()
        .find(|(label, _, _)| label == "Full Access")
        .unwrap();
    assert!(full.1);
    assert_eq!(full.2.as_deref(), Some(PERMISSION_CEILING_REASON));
    fx.press(KeyCode::Esc).await;

    fx.press(KeyCode::Char('i')).await;
    fx.key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::ALT))
        .await;
    fx.type_text("mini").await;
    fx.press(KeyCode::Enter).await;
    let screen = fx.screen();
    assert!(screen.contains("Model        GPT-5-Mini"), "{screen}");
    assert!(screen.contains("Reasoning    Low"), "{screen}");
    fx.press(KeyCode::Char('i')).await;
    fx.type_text("x").await;
    fx.submit().await;
    let request = fx.created_json();
    assert_eq!(request["model"], "gpt-5-mini");
    assert_eq!(request["reasoningLevel"], "low");
    assert_eq!(
        request["executionInputSources"],
        serde_json::json!({"model": "explicit"})
    );
    assert_eq!(request["environment"]["machine"]["hostId"], "host_2");
}

#[tokio::test]
async fn new_thread_in_same_environment_reuses_it_with_the_thread_placement() {
    let mut fx = fixture(|_| {});
    let mut snapshot = StoreSnapshot::clone(&fx.state.snapshot);
    let entry = fx.state.sidebar_entry("thr_old").unwrap().clone();
    let mut response = ThreadResponse {
        thread: entry.thread.clone(),
        environment: Some(Environment {
            id: "env_wt".into(),
            project_id: "proj_a".into(),
            host_id: "host_1".into(),
            path: Some("/work/wt".into()),
            status: EnvironmentStatus::Ready,
            ..Environment::default()
        }),
        ..ThreadResponse::default()
    };
    response.thread.environment_id = Some("env_wt".into());
    snapshot.threads.insert(
        "thr_old".into(),
        bb_store::Resource {
            data: Some(Arc::new(response)),
            ..bb_store::Resource::default()
        },
    );
    update(&mut fx.state, Action::Snapshot(Arc::new(snapshot)));
    fx.state.open_thread = Some("thr_old".into());
    fx.keys("SPC b c").await;

    let form = fx.state.new_thread_form().expect("form");
    assert!(form.prompt_active);
    let screen = fx.screen();
    assert!(
        screen.contains("Environment  Reuse · feature-x"),
        "{screen}"
    );
    let catalog = fx.api.execution_queries.lock().unwrap().clone();
    assert_eq!(
        catalog.last().unwrap().environment_id.as_deref(),
        Some("env_wt")
    );

    fx.type_text("Follow up work").await;
    fx.submit().await;
    let request = fx.created_json();
    assert_eq!(
        request["environment"],
        serde_json::json!({"type": "reuse", "environmentId": "env_wt"})
    );
    assert_eq!(request["sectionId"], "sec_1");
    assert_eq!(request["pinned"], true);
}

#[tokio::test]
async fn same_environment_shortcut_needs_a_ready_environment() {
    let mut fx = fixture(|_| {});
    fx.keys("SPC b c").await;
    assert!(fx.state.new_thread_form().is_none());
    assert!(
        fx.state
            .toasts
            .iter()
            .any(|toast| toast.message == "Open a thread first")
    );
}

#[tokio::test]
async fn read_only_mode_never_creates_threads() {
    let mut fx = fixture(|_| {});
    fx.state.read_only = true;
    fx.keys("SPC b N").await;
    fx.press(KeyCode::Char('i')).await;
    fx.type_text("hello").await;
    fx.submit().await;
    assert!(fx.api.created.lock().unwrap().is_empty());
    assert!(fx.state.new_thread_form().is_some());
    assert!(fx.state.toasts.iter().any(|toast| {
        toast.level == ToastLevel::Error && toast.message == "Read-only mode: cannot create thread"
    }));
}

#[tokio::test]
async fn failed_creation_keeps_the_prompt_and_shows_the_error() {
    let mut fx = fixture(|api| api.fail = true);
    fx.keys("SPC b N").await;
    fx.press(KeyCode::Char('i')).await;
    fx.type_text("keep me").await;
    fx.submit().await;
    let form = fx.state.new_thread_form().expect("form stays open");
    assert!(!form.submitting);
    assert_eq!(form.composer.text(), "keep me");
    let screen = fx.screen();
    assert!(screen.contains("✗ server said no"), "{screen}");
}

#[tokio::test]
async fn personal_project_uses_the_personal_workspace_on_the_primary_machine() {
    let mut fx = fixture(|_| {});
    fx.keys("SPC b N").await;
    fx.press(KeyCode::Enter).await;
    fx.type_text("personal").await;
    fx.press(KeyCode::Enter).await;
    let screen = fx.screen();
    assert!(screen.contains("Project      Personal"), "{screen}");
    assert!(
        screen.contains("Environment  laptop · Personal workspace"),
        "{screen}"
    );
    assert!(!screen.contains("Branch "), "{screen}");
    fx.press(KeyCode::Char('i')).await;
    fx.type_text("notes").await;
    fx.submit().await;
    let request = fx.created_json();
    assert_eq!(request["projectId"], "proj_personal");
    assert_eq!(
        request["environment"],
        serde_json::json!({
            "type": "provider",
            "environmentProviderId": "personal-workspace",
            "machine": {"type": "existing", "hostId": "host_1"},
            "inputs": null
        })
    );
}

#[tokio::test]
async fn closing_the_form_keeps_the_prompt_for_next_time() {
    let mut fx = fixture(|_| {});
    fx.keys("SPC b N").await;
    fx.press(KeyCode::Char('i')).await;
    fx.type_text("draft text").await;
    fx.press(KeyCode::Esc).await;
    assert!(fx.state.new_thread_form().unwrap().prompt_active);
    fx.press(KeyCode::Esc).await;
    assert!(!fx.state.new_thread_form().unwrap().prompt_active);
    fx.press(KeyCode::Char('q')).await;
    assert!(fx.state.new_thread_form().is_none());
    assert_eq!(fx.state.focus, Focus::Sidebar);
    fx.keys("SPC b N").await;
    assert_eq!(
        fx.state.new_thread_form().unwrap().composer.text(),
        "draft text"
    );
}
