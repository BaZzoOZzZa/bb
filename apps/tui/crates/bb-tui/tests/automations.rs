mod support;

use std::sync::Arc;

use bb_tui::app::{Action, AppState, Effect, update};
use bb_tui::automation_data::Route;
use bb_tui::automations::{AutomationMutation, AutomationRequest};
use bb_wire::prompt::{PermissionMode, ProjectExecutionDefaults, ReasoningLevel};
use bb_wire::system::SystemProviderCatalogEntry;
use crossterm::event::{KeyCode, KeyModifiers};
use serde_json::{Value, json};
use support::*;

fn agent_automation(id: &str, project: &str, name: &str, enabled: bool) -> Value {
    json!({
        "id": id,
        "projectId": project,
        "name": name,
        "enabled": enabled,
        "trigger": {"triggerType": "schedule", "cron": "0 9 * * 1-5", "timezone": "UTC"},
        "execution": {
            "mode": "agent",
            "prompt": "Triage new issues\nand label them",
            "providerId": "codex",
            "model": "gpt-5",
            "reasoningLevel": "high",
            "permissionMode": "auto",
            "environment": {"type": "project-default"},
        },
        "origin": "human",
        "createdByThreadId": null,
        "nextRunAt": if enabled { json!(NOW_MS + 60 * MINUTE) } else { json!(null) },
        "lastRunAt": NOW_MS - 60 * MINUTE,
        "runCount": 3,
        "lastRunStatus": "succeeded",
        "lastRunThreadId": "thr_old",
        "lastError": null,
        "createdAt": NOW_MS - 1000 * MINUTE,
        "updatedAt": NOW_MS - MINUTE,
    })
}

fn script_automation() -> Value {
    json!({
        "id": "auto_deps",
        "projectId": "proj_a",
        "name": "Dependency drift",
        "enabled": true,
        "trigger": {"triggerType": "schedule", "cron": "*/5 * * * *", "timezone": "UTC"},
        "execution": {
            "mode": "script",
            "script": "pnpm outdated",
            "interpreter": "bash",
            "timeoutMs": 60000,
            "workingDirectory": {"type": "project"},
            "resolvedWorkingDirectory": "/work/alpha",
        },
        "origin": "agent",
        "createdByThreadId": null,
        "nextRunAt": NOW_MS + MINUTE,
        "lastRunAt": null,
        "runCount": 0,
        "lastRunStatus": null,
        "lastRunThreadId": null,
        "lastError": null,
        "createdAt": NOW_MS - 10 * MINUTE,
        "updatedAt": NOW_MS - 10 * MINUTE,
    })
}

fn run(id: &str, status: &str, thread: Option<&str>, minutes_ago: i64) -> Value {
    let started = NOW_MS - minutes_ago * MINUTE;
    json!({
        "id": id,
        "automationId": "auto_triage",
        "runMode": "agent",
        "threadId": thread,
        "status": status,
        "trigger": "schedule",
        "skipReason": if status == "skipped" { json!("previous run still active") } else { json!(null) },
        "error": if status == "failed" { json!("provider crashed") } else { json!(null) },
        "output": null,
        "exitCode": null,
        "scheduledFor": started,
        "startedAt": started,
        "finishedAt": started + 12_000,
    })
}

fn triage() -> Route {
    Route {
        project_id: "proj_a".into(),
        automation_id: "auto_triage".into(),
    }
}

fn api() -> Arc<MockApi> {
    let mut api = MockApi::new(FakeBackend::new(sample_sidebar()));
    let mock = Arc::get_mut(&mut api).expect("unique");
    *mock.automations.automations.lock().unwrap() = vec![
        agent_automation("auto_triage", "proj_a", "Nightly triage", true),
        agent_automation("auto_report", "proj_b", "Weekly report", false),
        script_automation(),
        json!({"id": "auto_bad", "projectId": "proj_b", "name": "Broken", "problem": "invalid-stored-data"}),
    ];
    mock.automations.runs.lock().unwrap().insert(
        "auto_triage".into(),
        vec![
            run("run_1", "succeeded", Some("thr_unread"), 60),
            run("run_2", "skipped", None, 120),
            run("run_3", "failed", Some("thr_old"), 180),
        ],
    );
    *mock.settings.catalog.lock().unwrap() = vec![
        SystemProviderCatalogEntry {
            id: "codex".into(),
            display_name: "Codex".into(),
            plugin_enabled: true,
            enabled: true,
            available: true,
            ..SystemProviderCatalogEntry::default()
        },
        SystemProviderCatalogEntry {
            id: "claude-code".into(),
            display_name: "Claude Code".into(),
            plugin_enabled: true,
            enabled: false,
            available: true,
            ..SystemProviderCatalogEntry::default()
        },
    ];
    mock.execution_options = serde_json::from_value(json!({
        "providers": [{
            "id": "codex",
            "pluginId": "provider-codex",
            "displayName": "Codex",
            "capabilities": {"permissionModes": ["accept-edits", "auto", "full"]},
        }],
        "permissionCeiling": "full",
        "models": [
            {
                "id": "gpt-5",
                "model": "gpt-5",
                "displayName": "GPT-5",
                "description": "",
                "supportedReasoningEfforts": [
                    {"reasoningEffort": "low", "description": ""},
                    {"reasoningEffort": "medium", "description": ""},
                    {"reasoningEffort": "high", "description": ""},
                ],
                "defaultReasoningEffort": "medium",
                "isDefault": true,
            },
            {
                "id": "gpt-5-mini",
                "model": "gpt-5-mini",
                "displayName": "GPT-5 mini",
                "description": "",
                "supportedReasoningEfforts": [],
                "defaultReasoningEffort": "low",
                "isDefault": false,
            },
        ],
        "selectedOnlyModels": [],
        "modelLoadError": null,
    }))
    .expect("execution options");
    mock.project_defaults.insert(
        "proj_a".into(),
        ProjectExecutionDefaults {
            provider_id: "codex".into(),
            model: "gpt-5".into(),
            service_tier: "default".into(),
            reasoning_level: ReasoningLevel::High,
            permission_mode: PermissionMode::AcceptEdits,
        },
    );
    api
}

fn screen(state: &AppState) -> String {
    render_lines(state, 140, 44).join("\n")
}

async fn keys(api: &MockApi, state: &mut AppState, codes: &[KeyCode]) -> Vec<Effect> {
    let mut rest = Vec::new();
    for code in codes {
        let effects = press(state, *code);
        rest.extend(run_automations(api, state, effects).await);
    }
    rest
}

async fn ctrl(api: &MockApi, state: &mut AppState, ch: char) -> Vec<Effect> {
    let effects = press_mod(state, KeyCode::Char(ch), KeyModifiers::CONTROL);
    run_automations(api, state, effects).await
}

async fn type_text(api: &MockApi, state: &mut AppState, text: &str) {
    let codes: Vec<KeyCode> = text.chars().map(KeyCode::Char).collect();
    keys(api, state, &codes).await;
}

async fn open_view(api: &MockApi) -> AppState {
    let mut state = loaded_state(sample_sidebar());
    let effects = press(&mut state, KeyCode::Char('A'));
    assert_eq!(
        effects,
        vec![Effect::Automations(AutomationRequest::Overview)]
    );
    run_automations(api, &mut state, effects).await;
    state
}

async fn open_triage(api: &MockApi) -> AppState {
    let mut state = open_view(api).await;
    keys(
        api,
        &mut state,
        &[KeyCode::Char('j'), KeyCode::Char('j'), KeyCode::Enter],
    )
    .await;
    let detail = state.automations.as_ref().unwrap().detail.as_ref().unwrap();
    assert_eq!(detail.route, triage());
    state
}

async fn move_to_field(api: &MockApi, state: &mut AppState, label: &str) {
    keys(api, state, &[KeyCode::Char('g')]).await;
    for _ in 0..20 {
        let form = state.automations.as_ref().unwrap().form.as_ref().unwrap();
        if form.current_field().label() == label {
            return;
        }
        keys(api, state, &[KeyCode::Char('j')]).await;
    }
    panic!("field {label} not found\n{}", screen(state));
}

async fn set_field(api: &MockApi, state: &mut AppState, label: &str, text: &str) {
    move_to_field(api, state, label).await;
    keys(api, state, &[KeyCode::Enter]).await;
    ctrl(api, state, 'e').await;
    ctrl(api, state, 'u').await;
    type_text(api, state, text).await;
    keys(api, state, &[KeyCode::Enter]).await;
}

fn toasts(state: &AppState) -> Vec<String> {
    state
        .toasts
        .iter()
        .map(|toast| toast.message.clone())
        .collect()
}

#[tokio::test]
async fn list_and_detail_render_overview_runs_and_definition() {
    let api = api();
    let mut state = open_view(&api).await;
    let text = screen(&state);
    for expected in [
        "Automations (4)",
        "Nightly triage",
        "alpha · 9AM Mon-Fri · Next ",
        "Weekly report",
        "[paused]",
        "Dependency drift",
        "Every 5 min",
        "[script]",
        "Broken",
        "[invalid]",
    ] {
        assert!(text.contains(expected), "missing {expected}\n{text}");
    }
    let names: Vec<usize> = [
        "Broken",
        "Dependency drift",
        "Nightly triage",
        "Weekly report",
    ]
    .iter()
    .map(|name| text.find(name).unwrap())
    .collect();
    assert!(names.windows(2).all(|pair| pair[0] < pair[1]), "{text}");

    let effects = keys(&api, &mut state, &[KeyCode::Char('j'), KeyCode::Char('j')]).await;
    assert!(effects.is_empty());
    let effects = press(&mut state, KeyCode::Enter);
    assert_eq!(
        effects,
        vec![
            Effect::Automations(AutomationRequest::Detail(triage())),
            Effect::Automations(AutomationRequest::Runs {
                route: triage(),
                cursor: None,
            }),
        ]
    );
    run_automations(&api, &mut state, effects).await;
    let text = screen(&state);
    for expected in [
        "Automations › Nightly triage",
        "Cron",
        "0 9 * * 1-5 (UTC)",
        "Provider",
        "codex",
        "gpt-5",
        "Permission mode",
        "Project default",
        "Triage new issues",
        "and label them",
        "Run history",
        "Succeeded",
        "previous run still active",
        "Load more runs",
        "Write docs (thr_old)",
    ] {
        assert!(text.contains(expected), "missing {expected}\n{text}");
    }
    assert!(!text.contains("provider crashed"), "{text}");

    keys(&api, &mut state, &[KeyCode::Char('m')]).await;
    let text = screen(&state);
    assert!(text.contains("provider crashed"), "{text}");
    assert!(!text.contains("Load more runs"), "{text}");

    press(&mut state, KeyCode::Esc);
    assert!(state.automations.as_ref().unwrap().detail.is_none());
    press(&mut state, KeyCode::Char('q'));
    assert!(state.automations.is_none());
    assert!(!state.quit);
}

#[tokio::test]
async fn pause_resume_and_run_now_call_the_plugin_rpc() {
    let api = api();
    let mut state = open_triage(&api).await;
    let effects = press(&mut state, KeyCode::Char(' '));
    assert_eq!(
        effects,
        vec![Effect::Automations(AutomationRequest::Mutate(
            AutomationMutation::Pause(triage())
        ))]
    );
    assert!(press(&mut state, KeyCode::Char(' ')).is_empty());
    run_automations(&api, &mut state, effects).await;
    assert!(toasts(&state).contains(&"Automation paused".to_owned()));
    assert!(screen(&state).contains("[paused]"), "{}", screen(&state));
    keys(&api, &mut state, &[KeyCode::Char(' ')]).await;
    assert!(toasts(&state).contains(&"Automation resumed".to_owned()));

    keys(&api, &mut state, &[KeyCode::Char('r')]).await;
    assert!(toasts(&state).contains(&"Run started".to_owned()));
    let target = json!({"projectId": "proj_a", "automationId": "auto_triage"});
    assert_eq!(
        api.automations.mutations(),
        vec![
            ("automations_pause".to_owned(), target.clone()),
            ("automations_resume".to_owned(), target.clone()),
            ("automations_run".to_owned(), target),
        ]
    );
    assert!(screen(&state).contains("Running…"), "{}", screen(&state));
}

#[tokio::test]
async fn enter_on_a_run_opens_its_thread() {
    let api = api();
    let mut state = open_triage(&api).await;
    keys(&api, &mut state, &[KeyCode::Char('m'), KeyCode::Char('G')]).await;
    let effects = press(&mut state, KeyCode::Enter);
    assert!(state.automations.is_none());
    assert_eq!(state.open_thread.as_deref(), Some("thr_old"));
    assert!(effects.contains(&Effect::WatchThread(Some("thr_old".into()))));
}

#[tokio::test]
async fn plugin_signals_refresh_the_open_view() {
    let api = api();
    let mut state = open_triage(&api).await;
    let signal = |project: &str, kind: &str| Action::PluginSignal {
        plugin_id: "automations".into(),
        channel: "automations".into(),
        payload: json!({"projectId": project, "kind": kind}),
    };
    let effects = update(&mut state, signal("proj_a", "automation-runs-changed"));
    assert_eq!(
        effects,
        vec![
            Effect::Automations(AutomationRequest::Overview),
            Effect::Automations(AutomationRequest::Detail(triage())),
            Effect::Automations(AutomationRequest::Runs {
                route: triage(),
                cursor: None,
            }),
        ]
    );
    assert!(update(&mut state, signal("proj_a", "automations-changed")).is_empty());
    run_automations(&api, &mut state, effects).await;
    let effects = update(&mut state, signal("proj_b", "automations-changed"));
    assert_eq!(
        effects,
        vec![Effect::Automations(AutomationRequest::Overview)]
    );
    run_automations(&api, &mut state, effects).await;
    assert!(update(&mut state, signal("proj_a", "renamed")).is_empty());
    let garbage = Action::PluginSignal {
        plugin_id: "automations".into(),
        channel: "automations".into(),
        payload: json!("nope"),
    };
    assert!(update(&mut state, garbage).is_empty());
    press(&mut state, KeyCode::Char('q'));
    press(&mut state, KeyCode::Char('q'));
    assert!(update(&mut state, signal("proj_a", "automations-changed")).is_empty());
}

#[tokio::test]
async fn delete_asks_first_and_removes_the_automation() {
    let api = api();
    let mut state = open_view(&api).await;
    keys(&api, &mut state, &[KeyCode::Char('G'), KeyCode::Char('d')]).await;
    assert!(screen(&state).contains("Delete automation?"));
    assert!(screen(&state).contains("Weekly report"));
    keys(&api, &mut state, &[KeyCode::Char('n')]).await;
    assert!(api.automations.mutations().is_empty());
    keys(&api, &mut state, &[KeyCode::Char('d'), KeyCode::Char('y')]).await;
    assert_eq!(
        api.automations.mutations(),
        vec![(
            "automations_delete".to_owned(),
            json!({"projectId": "proj_b", "automationId": "auto_report"})
        )]
    );
    let text = screen(&state);
    assert!(text.contains("Automations (3)"), "{text}");
    assert!(!text.contains("Weekly report"), "{text}");
    assert!(toasts(&state).contains(&"Automation deleted".to_owned()));
}

#[tokio::test]
async fn create_form_validates_and_sends_an_agent_automation() {
    let api = api();
    let mut state = open_view(&api).await;
    let effects = press(&mut state, KeyCode::Char('n'));
    assert!(effects.contains(&Effect::Automations(AutomationRequest::Providers)));
    assert!(
        effects.contains(&Effect::Automations(AutomationRequest::ProjectDefaults {
            project_id: "proj_a".into()
        }))
    );
    run_automations(&api, &mut state, effects).await;
    let text = screen(&state);
    for expected in [
        "New automation",
        "alpha",
        "Recurring (cron)",
        "0 9 * * 1-5  → 9AM Mon-Fri",
        "GPT-5 (gpt-5)",
        "high",
        "accept-edits",
        "Project default",
        "[ Create automation ]",
    ] {
        assert!(text.contains(expected), "missing {expected}\n{text}");
    }

    ctrl(&api, &mut state, 's').await;
    assert!(
        screen(&state).contains("Name is required"),
        "{}",
        screen(&state)
    );
    assert!(api.automations.mutations().is_empty());

    set_field(&api, &mut state, "Name", "Daily digest").await;
    move_to_field(&api, &mut state, "Cron").await;
    keys(&api, &mut state, &[KeyCode::Enter]).await;
    type_text(&api, &mut state, " x").await;
    keys(&api, &mut state, &[KeyCode::Enter]).await;
    assert!(
        screen(&state).contains("exactly 5 fields"),
        "{}",
        screen(&state)
    );
    keys(&api, &mut state, &[KeyCode::Esc]).await;
    set_field(&api, &mut state, "Cron", "30 8 * * *").await;
    set_field(&api, &mut state, "Prompt", "Summarize\\nyesterday").await;
    move_to_field(&api, &mut state, "Environment").await;
    keys(&api, &mut state, &[KeyCode::Enter, KeyCode::Char('2')]).await;
    ctrl(&api, &mut state, 's').await;
    assert!(
        screen(&state).contains("No machine is available for a new worktree"),
        "{}",
        screen(&state)
    );
    keys(&api, &mut state, &[KeyCode::Enter, KeyCode::Char('1')]).await;
    let effects = ctrl(&api, &mut state, 's').await;
    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(
        api.automations.mutations(),
        vec![(
            "automations_create".to_owned(),
            json!({
                "projectId": "proj_a",
                "name": "Daily digest",
                "enabled": true,
                "trigger": {"triggerType": "schedule", "cron": "30 8 * * *", "timezone": bb_tui::automation_data::local_timezone()},
                "execution": {
                    "mode": "agent",
                    "prompt": "Summarize\nyesterday",
                    "providerId": "codex",
                    "model": "gpt-5",
                    "reasoningLevel": "high",
                    "permissionMode": "accept-edits",
                    "environment": {"type": "project-default"},
                },
                "origin": "human",
            })
        )]
    );
    let view = state.automations.as_ref().unwrap();
    assert!(view.form.is_none());
    assert_eq!(
        view.detail
            .as_ref()
            .map(|detail| detail.route.automation_id.as_str()),
        Some("auto_new")
    );
    assert!(screen(&state).contains("Automations › Daily digest"));
    assert!(toasts(&state).contains(&"Automation created".to_owned()));
}

#[tokio::test]
async fn one_shot_automation_sends_a_future_run_time() {
    let api = api();
    let mut state = open_view(&api).await;
    let effects = press(&mut state, KeyCode::Char('n'));
    run_automations(&api, &mut state, effects).await;
    set_field(&api, &mut state, "Name", "Release check").await;
    move_to_field(&api, &mut state, "Schedule").await;
    keys(&api, &mut state, &[KeyCode::Enter, KeyCode::Char('2')]).await;
    assert!(screen(&state).contains("Run at"), "{}", screen(&state));
    assert!(!screen(&state).contains("Timezone"), "{}", screen(&state));
    set_field(&api, &mut state, "Run at", "yesterday").await;
    assert!(
        screen(&state).contains("Use YYYY-MM-DD HH:MM"),
        "{}",
        screen(&state)
    );
    keys(&api, &mut state, &[KeyCode::Esc]).await;
    set_field(&api, &mut state, "Run at", "2h").await;
    set_field(&api, &mut state, "Prompt", "Check the release").await;
    move_to_field(&api, &mut state, "Start paused").await;
    keys(&api, &mut state, &[KeyCode::Char(' ')]).await;
    ctrl(&api, &mut state, 's').await;
    let mutations = api.automations.mutations();
    let input = &mutations[0].1;
    let run_at = input["trigger"]["runAt"].as_i64().unwrap();
    assert_eq!(input["trigger"]["triggerType"], "once");
    assert!(
        run_at > NOW_MS && run_at <= NOW_MS + 120 * MINUTE,
        "{run_at}"
    );
    assert_eq!(input["enabled"], false);
}

#[tokio::test]
async fn edit_sends_only_changed_fields() {
    let api = api();
    let mut state = open_triage(&api).await;
    let effects = press(&mut state, KeyCode::Char('e'));
    assert!(
        effects.contains(&Effect::Automations(AutomationRequest::Models {
            provider_id: "codex".into()
        }))
    );
    run_automations(&api, &mut state, effects).await;
    assert!(screen(&state).contains("Edit Nightly triage"));
    assert!(screen(&state).contains("[ Save changes ]"));
    assert!(!screen(&state).contains("Start paused"));
    ctrl(&api, &mut state, 's').await;
    assert!(toasts(&state).contains(&"No changes".to_owned()));
    assert!(api.automations.mutations().is_empty());

    keys(&api, &mut state, &[KeyCode::Char('e')]).await;
    set_field(&api, &mut state, "Name", "Morning triage").await;
    move_to_field(&api, &mut state, "Model").await;
    keys(&api, &mut state, &[KeyCode::Enter, KeyCode::Char('2')]).await;
    move_to_field(&api, &mut state, "Project").await;
    keys(&api, &mut state, &[KeyCode::Enter]).await;
    assert!(toasts(&state).contains(&"An automation cannot move to another project".to_owned()));
    ctrl(&api, &mut state, 's').await;
    assert_eq!(
        api.automations.mutations(),
        vec![(
            "automations_update".to_owned(),
            json!({
                "projectId": "proj_a",
                "automationId": "auto_triage",
                "name": "Morning triage",
                "agent": {"model": "gpt-5-mini"},
            })
        )]
    );
    let text = screen(&state);
    assert!(text.contains("Automations › Morning triage"), "{text}");
    assert!(text.contains("gpt-5-mini"), "{text}");
}

#[tokio::test]
async fn read_only_mode_refuses_every_change() {
    let api = api();
    let mut state = open_triage(&api).await;
    state.read_only = true;
    for key in [' ', ' ', 'r'] {
        keys(&api, &mut state, &[KeyCode::Char(key)]).await;
    }
    let messages = toasts(&state);
    assert_eq!(
        messages
            .iter()
            .filter(|message| *message == "Read-only mode: cannot pause the automation")
            .count(),
        2
    );
    assert!(messages.contains(&"Read-only mode: cannot run the automation".to_owned()));
    keys(&api, &mut state, &[KeyCode::Char('d'), KeyCode::Char('y')]).await;
    assert!(toasts(&state).contains(&"Read-only mode: cannot delete the automation".to_owned()));
    keys(&api, &mut state, &[KeyCode::Char('e')]).await;
    set_field(&api, &mut state, "Name", "Renamed").await;
    ctrl(&api, &mut state, 's').await;
    let form = state.automations.as_ref().unwrap().form.as_ref().unwrap();
    assert!(!form.saving);
    assert!(api.automations.mutations().is_empty());
}

#[tokio::test]
async fn failures_and_unreadable_automations_are_reported() {
    let api = api();
    *api.automations.reject.lock().unwrap() = Some("plugin is not running".into());
    let mut state = open_triage(&api).await;
    keys(&api, &mut state, &[KeyCode::Char(' ')]).await;
    assert!(
        toasts(&state)
            .contains(&"Failed to pause the automation: plugin is not running".to_owned())
    );
    keys(
        &api,
        &mut state,
        &[KeyCode::Esc, KeyCode::Char('g'), KeyCode::Enter],
    )
    .await;
    let text = screen(&state);
    assert!(text.contains("cannot be read"), "{text}");
    keys(&api, &mut state, &[KeyCode::Char('e'), KeyCode::Char('r')]).await;
    let messages = toasts(&state);
    assert!(
        messages
            .iter()
            .any(|message| message.contains("delete it instead"))
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("cannot run until it is fixed"))
    );
    assert!(state.automations.as_ref().unwrap().form.is_none());
    keys(&api, &mut state, &[KeyCode::Char('d')]).await;
    assert!(screen(&state).contains("Delete automation?"));
}

#[tokio::test]
async fn palette_entries_open_the_view_and_the_form() {
    let api = api();
    let mut state = loaded_state(sample_sidebar());
    let effects = update(
        &mut state,
        Action::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('k'),
            KeyModifiers::CONTROL,
        )),
    );
    let _ = effects;
    type_text(&api, &mut state, "new automation").await;
    let effects = press(&mut state, KeyCode::Enter);
    run_automations(&api, &mut state, effects).await;
    let view = state.automations.as_ref().expect("automations open");
    assert!(view.form.as_ref().is_some_and(|form| form.is_create()));
    assert!(screen(&state).contains("New automation"));
}
