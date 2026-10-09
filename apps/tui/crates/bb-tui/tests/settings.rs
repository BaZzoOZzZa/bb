mod support;

use std::sync::Arc;

use bb_tui::app::{Action, AppState, Effect, update};
use bb_tui::settings::{
    ArchivedKind, Detail, SettingsFocus, SettingsMutation, SettingsRequest, SettingsSection,
};
use bb_tui::settings_form::{
    FieldKind, escape_multiline, is_valid_git_branch_name, parse_field_input, unescape_multiline,
    validate_branch_prefix, validate_server_url,
};
use bb_wire::environment::{Host, HostProviderCliStatus, HostStatus, HostType};
use bb_wire::plugin::{InstalledPlugin, PluginRuntimeStatus, PluginSettingsResponse};
use bb_wire::project::ProjectSource;
use bb_wire::prompt::{PermissionMode, ProjectExecutionDefaults, ReasoningLevel};
use bb_wire::system::{
    AvailableModel, SystemConfigResponse, SystemExecutionOptionsResponse,
    SystemProviderCatalogEntry, SystemVersionResponse,
};
use crossterm::event::{KeyCode, KeyModifiers};
use serde_json::json;
use support::*;

fn general() -> serde_json::Value {
    json!({
        "showKeyboardHints": true,
        "steerActiveThreadOnEnter": true,
        "confirmThreadArchive": true,
        "showDiagnosticEvents": false,
        "providerOrder": [],
        "defaultProviderId": null,
        "providerCompletedTurnDisplay": {"codex": "flat"},
        "streamerMode": false,
        "allowFastServiceTier": true,
        "telemetryEnabled": true,
        "managedBranchPrefix": "bb/",
        "machineServerUrl": null,
        "machineGitCredentialsEnabled": true,
        "defaultMachineAccess": null,
        "showUnhandledProviderEvents": false
    })
}

fn catalog_entry(id: &str, name: &str, enabled: bool) -> SystemProviderCatalogEntry {
    SystemProviderCatalogEntry {
        id: id.into(),
        display_name: name.into(),
        plugin_id: format!("provider-{id}"),
        plugin_name: format!("{name} provider"),
        plugin_enabled: true,
        enabled,
        available: true,
        ..SystemProviderCatalogEntry::default()
    }
}

fn host(id: &str, name: &str, connected: bool) -> Host {
    Host {
        id: id.into(),
        name: name.into(),
        host_type: HostType::Persistent,
        status: if connected {
            HostStatus::Connected
        } else {
            HostStatus::Disconnected
        },
        max_permission_mode: PermissionMode::Full,
        last_seen_at: Some(NOW_MS - 5 * MINUTE),
        created_at: NOW_MS - 60 * MINUTE,
        ..Host::default()
    }
}

fn plugin(id: &str, name: &str, enabled: bool, has_settings: bool) -> InstalledPlugin {
    InstalledPlugin {
        id: id.into(),
        name: Some(name.into()),
        version: "1.0.0".into(),
        enabled,
        status: if enabled {
            PluginRuntimeStatus::Running
        } else {
            PluginRuntimeStatus::Disabled
        },
        has_settings,
        ..InstalledPlugin::default()
    }
}

fn plugin_settings() -> PluginSettingsResponse {
    serde_json::from_value(json!({
        "ok": true,
        "schema": {
            "enabled": {"type": "boolean", "label": "Send pings"},
            "mode": {"type": "select", "label": "Mode", "options": ["fast", "slow"]},
            "limit": {"type": "number", "label": "Limit", "description": "Max items"},
            "token": {"type": "string", "label": "API token", "secret": true},
            "notes": {"type": "string", "label": "Notes", "experimental_multiline": true},
            "home": {"type": "project", "label": "Home project"},
            "color": {"type": "color", "label": "Color"}
        },
        "values": {
            "enabled": false,
            "mode": "fast",
            "limit": 3,
            "token": {"set": false},
            "notes": "one\ntwo"
        }
    }))
    .expect("settings")
}

fn api() -> Arc<MockApi> {
    let mut api = MockApi::new(FakeBackend::new(sample_sidebar()));
    let mock = Arc::get_mut(&mut api).expect("unique");
    *mock.settings.config.lock().unwrap() = SystemConfigResponse {
        general_settings: general(),
        server_url: "http://127.0.0.1:4000".into(),
        primary_host_id: Some("host_1".into()),
        data_dir: "/data/bb".into(),
        ..SystemConfigResponse::default()
    };
    mock.settings.version = SystemVersionResponse {
        current_version: "0.45.0".into(),
        latest_version: Some("0.46.0".into()),
        update_available: true,
        upgrade_command: "npx bb-app@latest".into(),
        ..SystemVersionResponse::default()
    };
    *mock.settings.catalog.lock().unwrap() = vec![
        catalog_entry("claude-code", "Claude Code", true),
        catalog_entry("codex", "Codex", false),
    ];
    mock.settings.host_clis.insert(
        "host_1".into(),
        [(
            "claude-code".to_string(),
            HostProviderCliStatus {
                display_name: "Claude Code".into(),
                installed: true,
                current_version: Some("2.1.294".into()),
                latest_version: Some("2.1.295".into()),
                needs_update: true,
                ..HostProviderCliStatus::default()
            },
        )]
        .into_iter()
        .collect(),
    );
    *mock.settings.plugins.lock().unwrap() = vec![
        plugin("push-notifications", "Push notifications", true, true),
        plugin("workflows", "Workflows", false, false),
    ];
    mock.settings
        .plugin_settings
        .lock()
        .unwrap()
        .insert("push-notifications".into(), plugin_settings());
    let mut archived = Vec::new();
    for index in 0..3 {
        let mut entry = thread(
            &format!("thr_arch{index}"),
            "proj_a",
            &format!("Old work {index}"),
            100,
        );
        entry.thread.archived_at = Some(NOW_MS - 2 * 24 * 60 * MINUTE);
        archived.push(entry);
    }
    let mut child = thread("thr_child", "proj_b", "Child task", 90);
    child.thread.archived_at = Some(NOW_MS - MINUTE);
    child.thread.parent_thread_id = Some("thr_other".into());
    archived.push(child);
    *mock.settings.archived.lock().unwrap() = archived;
    mock.settings.child_counts.insert("thr_arch0".into(), 2);
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
    mock.execution_options = SystemExecutionOptionsResponse {
        models: vec![AvailableModel {
            id: "opus".into(),
            model: "claude-opus".into(),
            display_name: "Opus".into(),
            is_default: true,
            ..AvailableModel::default()
        }],
        ..SystemExecutionOptionsResponse::default()
    };
    api
}

fn state_with_hosts() -> AppState {
    let mut sidebar = sample_sidebar();
    sidebar.projects[0].sources = vec![ProjectSource {
        id: "src_1".into(),
        project_id: "proj_a".into(),
        source_type: "local_path".into(),
        host_id: "host_1".into(),
        path: "/work/alpha".into(),
        is_default: true,
        ..ProjectSource::default()
    }];
    sidebar.projects[0].project.git_remote_url = Some("git@github.com:acme/alpha.git".into());
    let mut state = loaded_state(sidebar.clone());
    let mut snapshot = (*snapshot_with(sidebar)).clone();
    snapshot.hosts.data = Some(Arc::new(vec![
        host("host_1", "workstation", true),
        Host {
            last_rejected_protocol_version: Some(200),
            ..host("host_2", "laptop", false)
        },
    ]));
    snapshot.version += 1;
    update(&mut state, Action::Snapshot(Arc::new(snapshot)));
    state
}

async fn open(api: &MockApi, state: &mut AppState, key: char) -> Vec<Effect> {
    let effects = press(state, KeyCode::Char(key));
    run_settings(api, state, effects).await
}

async fn keys(api: &MockApi, state: &mut AppState, codes: &[KeyCode]) -> Vec<Effect> {
    let mut rest = Vec::new();
    for code in codes {
        let effects = press(state, *code);
        rest.extend(run_settings(api, state, effects).await);
    }
    rest
}

async fn select_section(api: &MockApi, state: &mut AppState, section: SettingsSection) {
    let index = SettingsSection::ALL
        .iter()
        .position(|candidate| *candidate == section)
        .unwrap();
    let digit = char::from(b'1' + index as u8);
    keys(
        api,
        state,
        &[KeyCode::Esc, KeyCode::Char(digit), KeyCode::Enter],
    )
    .await;
    let view = state.settings.as_ref().unwrap();
    assert_eq!(view.section, section);
    assert_eq!(view.focus, SettingsFocus::Content);
}

fn screen(state: &AppState) -> String {
    render_lines(state, 140, 44).join("\n")
}

fn move_to(state: &mut AppState, label: &str) {
    press(state, KeyCode::Char('g'));
    for _ in 0..80 {
        let rows = bb_tui::settings::settings_rows(state);
        let cursor =
            bb_tui::settings::clamp_cursor(&rows, state.settings.as_ref().unwrap().cursor());
        if cursor
            .and_then(|index| rows.get(index))
            .is_some_and(|row| row.label == label)
        {
            return;
        }
        press(state, KeyCode::Char('j'));
    }
    panic!("row {label} not found\n{}", screen(state));
}

#[tokio::test]
async fn comma_opens_settings_and_loads_server_data() {
    let api = api();
    let mut state = state_with_hosts();
    let effects = press(&mut state, KeyCode::Char(','));
    let requests: Vec<_> = effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Settings(request) => Some(request.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        requests,
        vec![
            SettingsRequest::Config,
            SettingsRequest::Version,
            SettingsRequest::Catalog,
            SettingsRequest::Plugins,
        ]
    );
    run_settings(&api, &mut state, effects).await;
    let text = screen(&state);
    for expected in [
        "Settings",
        "General",
        "Installed plugins",
        "Archived threads",
        "0.45.0",
        "[update to 0.46.0]",
        "npx bb-app@latest",
        "/data/bb",
        "workstation",
        "Default thread followup behavior",
        "Steer",
        "New branch prefix",
        "bb/",
        "Share anonymous usage data",
    ] {
        assert!(text.contains(expected), "missing {expected}\n{text}");
    }
    assert!(!text.contains("Show Git changes"), "{text}");
    press(&mut state, KeyCode::Char('q'));
    assert!(state.settings.is_none());
    assert!(!state.quit);
}

#[tokio::test]
async fn general_toggle_sends_whole_settings_object_and_applies_reply() {
    let api = api();
    let mut state = state_with_hosts();
    open(&api, &mut state, ',').await;
    move_to(&mut state, "Thread archive confirmation");
    let effects = press(&mut state, KeyCode::Char(' '));
    let Some(Effect::Settings(SettingsRequest::Mutate(SettingsMutation::UpdateGeneral {
        key,
        settings,
    }))) = effects.first()
    else {
        panic!("{effects:?}");
    };
    assert_eq!(*key, "confirmThreadArchive");
    let mut expected = general();
    expected["confirmThreadArchive"] = json!(false);
    assert_eq!(serde_json::Value::Object(settings.clone()), expected);
    assert!(screen(&state).contains("[saving…]"));
    run_settings(&api, &mut state, effects).await;
    let config = state.settings.as_ref().unwrap().config.loaded().unwrap();
    assert_eq!(
        config.general_settings["confirmThreadArchive"],
        json!(false)
    );
    assert!(!screen(&state).contains("[saving…]"));
}

#[tokio::test]
async fn read_only_refuses_settings_changes_without_requests() {
    let api = api();
    let mut state = state_with_hosts();
    state.read_only = true;
    open(&api, &mut state, ',').await;
    move_to(&mut state, "Streamer mode");
    let effects = press(&mut state, KeyCode::Enter);
    assert!(effects.is_empty(), "{effects:?}");
    assert!(api.settings.recorded().is_empty());
    assert!(
        state
            .toasts
            .iter()
            .any(|toast| toast.message == "Read-only mode: cannot change settings")
    );
    assert!(state.settings.as_ref().unwrap().pending.is_empty());
}

#[tokio::test]
async fn branch_prefix_and_server_url_are_validated_before_saving() {
    let api = api();
    let mut state = state_with_hosts();
    open(&api, &mut state, ',').await;
    move_to(&mut state, "New branch prefix");
    press(&mut state, KeyCode::Enter);
    assert!(state.settings.as_ref().unwrap().edit.is_some());
    for _ in 0..3 {
        press(&mut state, KeyCode::Backspace);
    }
    for ch in "bad..name/".chars() {
        press(&mut state, KeyCode::Char(ch));
    }
    let effects = press(&mut state, KeyCode::Enter);
    assert!(effects.is_empty());
    let edit = state.settings.as_ref().unwrap().edit.as_ref().unwrap();
    assert_eq!(
        edit.error.as_deref(),
        Some("Prefix must start a valid git branch name")
    );
    assert!(screen(&state).contains("Prefix must start a valid git branch name"));
    for _ in 0.."bad..name/".len() {
        press(&mut state, KeyCode::Backspace);
    }
    for ch in "team/".chars() {
        press(&mut state, KeyCode::Char(ch));
    }
    let effects = press(&mut state, KeyCode::Enter);
    run_settings(&api, &mut state, effects).await;
    let recorded = api.settings.recorded();
    assert!(matches!(
        recorded.as_slice(),
        [SettingsMutation::UpdateGeneral { key: "managedBranchPrefix", settings }]
            if settings["managedBranchPrefix"] == json!("team/")
    ));

    move_to(&mut state, "Machine server URL");
    press(&mut state, KeyCode::Enter);
    for ch in "ftp://user:pw@host".chars() {
        press(&mut state, KeyCode::Char(ch));
    }
    assert!(press(&mut state, KeyCode::Enter).is_empty());
    assert!(
        state
            .settings
            .as_ref()
            .unwrap()
            .edit
            .as_ref()
            .unwrap()
            .error
            .is_some()
    );
    press(&mut state, KeyCode::Esc);
    assert!(state.settings.as_ref().unwrap().edit.is_none());
    assert_eq!(api.settings.recorded().len(), 1);
}

#[tokio::test]
async fn providers_enable_default_collapse_and_models() {
    let api = api();
    let mut state = state_with_hosts();
    open(&api, &mut state, ',').await;
    select_section(&api, &mut state, SettingsSection::Providers).await;
    let text = screen(&state);
    assert!(text.contains("Claude Code"), "{text}");
    assert!(text.contains("[Default]"), "{text}");
    assert!(text.contains("[Disabled]"), "{text}");
    assert!(text.contains("Allow faster service tiers"), "{text}");

    move_to(&mut state, "Codex");
    keys(&api, &mut state, &[KeyCode::Char(' ')]).await;
    assert!(matches!(
        api.settings.recorded().last(),
        Some(SettingsMutation::SetProviderEnabled { provider_id, enabled: true }) if provider_id == "codex"
    ));
    let catalog = state.settings.as_ref().unwrap().catalog.loaded().unwrap();
    assert!(catalog.iter().all(|entry| entry.enabled));

    move_to(&mut state, "Codex");
    keys(&api, &mut state, &[KeyCode::Char('d')]).await;
    assert!(matches!(
        api.settings.recorded().last(),
        Some(SettingsMutation::UpdateGeneral { key: "defaultProviderId", settings })
            if settings["defaultProviderId"] == json!("codex")
    ));

    move_to(&mut state, "Codex");
    keys(&api, &mut state, &[KeyCode::Char('c')]).await;
    assert!(matches!(
        api.settings.recorded().last(),
        Some(SettingsMutation::UpdateGeneral { key: "providerCompletedTurnDisplay", settings })
            if settings["providerCompletedTurnDisplay"] == json!({"codex": "collapse"})
    ));

    move_to(&mut state, "Claude Code");
    keys(&api, &mut state, &[KeyCode::Enter]).await;
    let queries = api.execution_queries.lock().unwrap().clone();
    assert_eq!(
        queries.last().unwrap().provider_id.as_deref(),
        Some("claude-code")
    );
    let text = screen(&state);
    assert!(text.contains("Opus"), "{text}");
    assert!(text.contains("claude-opus"), "{text}");
}

#[tokio::test]
async fn keyboard_lists_bindings_and_filters() {
    let api = api();
    let mut state = state_with_hosts();
    open(&api, &mut state, ',').await;
    select_section(&api, &mut state, SettingsSection::Keyboard).await;
    let text = screen(&state);
    assert!(text.contains("Command palette"), "{text}");
    assert!(text.contains("[palette.open]"), "{text}");
    press(&mut state, KeyCode::Char('/'));
    for ch in "terminal".chars() {
        press(&mut state, KeyCode::Char(ch));
    }
    press(&mut state, KeyCode::Enter);
    let text = screen(&state);
    assert!(text.contains("Thread terminals"), "{text}");
    assert!(!text.contains("Command palette"), "{text}");
    press(&mut state, KeyCode::Esc);
    assert!(screen(&state).contains("Command palette"));
    press(&mut state, KeyCode::Char('/'));
    for ch in "settings".chars() {
        press(&mut state, KeyCode::Char(ch));
    }
    let text = screen(&state);
    assert!(text.contains("[settings.open]"), "{text}");
    assert!(text.contains("/settings"), "{text}");
}

#[tokio::test]
async fn project_detail_shows_checkouts_defaults_and_renames_and_deletes() {
    let api = api();
    let mut state = state_with_hosts();
    open(&api, &mut state, ',').await;
    select_section(&api, &mut state, SettingsSection::Projects).await;
    let text = screen(&state);
    assert!(
        text.contains("github.com/acme/alpha · 1 of 2 machines · 3 threads"),
        "{text}"
    );
    assert!(text.contains("[needs setup]"), "{text}");

    move_to(&mut state, "alpha");
    keys(&api, &mut state, &[KeyCode::Enter]).await;
    assert_eq!(
        state.settings.as_ref().unwrap().detail,
        Some(Detail::Project("proj_a".into()))
    );
    let text = screen(&state);
    for expected in [
        "Checkouts",
        "/work/alpha",
        "Not set up on this machine",
        "[primary]",
        "gpt-5",
        "Accept Edits",
        "Delete project",
    ] {
        assert!(text.contains(expected), "missing {expected}\n{text}");
    }

    move_to(&mut state, "Name");
    press(&mut state, KeyCode::Char('r'));
    for _ in 0..5 {
        press(&mut state, KeyCode::Backspace);
    }
    for ch in "Alpha app".chars() {
        press(&mut state, KeyCode::Char(ch));
    }
    let rest = keys(&api, &mut state, &[KeyCode::Enter]).await;
    assert!(rest.contains(&Effect::RefreshSidebar));
    assert!(matches!(
        api.settings.recorded().last(),
        Some(SettingsMutation::RenameProject { project_id, name }) if project_id == "proj_a" && name == "Alpha app"
    ));

    move_to(&mut state, "Delete project");
    press(&mut state, KeyCode::Enter);
    assert!(screen(&state).contains("Checkouts stay on disk."));
    assert!(press(&mut state, KeyCode::Esc).is_empty());
    assert!(state.settings.as_ref().unwrap().confirm.is_none());
    press(&mut state, KeyCode::Char('D'));
    keys(&api, &mut state, &[KeyCode::Char('y')]).await;
    assert!(matches!(
        api.settings.recorded().last(),
        Some(SettingsMutation::DeleteProject { project_id }) if project_id == "proj_a"
    ));
    assert_eq!(state.settings.as_ref().unwrap().detail, None);
}

#[tokio::test]
async fn machines_list_detail_cli_versions_permission_and_rename() {
    let api = api();
    let mut state = state_with_hosts();
    open(&api, &mut state, ',').await;
    select_section(&api, &mut state, SettingsSection::Machines).await;
    let text = screen(&state);
    assert!(text.contains("workstation"), "{text}");
    assert!(text.contains("[online]"), "{text}");
    assert!(text.contains("[server]"), "{text}");
    assert!(text.contains("last seen 5m ago"), "{text}");
    assert!(text.contains("[needs update]"), "{text}");

    move_to(&mut state, "workstation");
    keys(&api, &mut state, &[KeyCode::Enter]).await;
    let text = screen(&state);
    assert!(text.contains("2.1.294"), "{text}");
    assert!(text.contains("[update to 2.1.295]"), "{text}");
    assert!(text.contains("alpha"), "{text}");
    assert!(text.contains("Full Access"), "{text}");

    move_to(&mut state, "Permission limit");
    press(&mut state, KeyCode::Enter);
    assert!(screen(&state).contains("Approve for me"));
    let rest = keys(&api, &mut state, &[KeyCode::Char('2')]).await;
    assert!(rest.contains(&Effect::Refresh(bb_store::ResourceKey::Hosts)));
    assert!(matches!(
        api.settings.recorded().last(),
        Some(SettingsMutation::SetHostPermissionCeiling { host_id, mode: PermissionMode::Auto }) if host_id == "host_1"
    ));

    press(&mut state, KeyCode::Char('r'));
    for ch in " 2".chars() {
        press(&mut state, KeyCode::Char(ch));
    }
    keys(&api, &mut state, &[KeyCode::Enter]).await;
    assert!(matches!(
        api.settings.recorded().last(),
        Some(SettingsMutation::RenameHost { name, .. }) if name == "workstation 2"
    ));

    press(&mut state, KeyCode::Esc);
    move_to(&mut state, "laptop");
    let effects = press(&mut state, KeyCode::Enter);
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::Settings(SettingsRequest::HostClis { .. })))
    );
    let text = screen(&state);
    assert!(text.contains("Unavailable while offline"), "{text}");
    assert!(
        text.contains("Needs update · daemon protocol 200"),
        "{text}"
    );
}

#[tokio::test]
async fn archived_threads_filter_unarchive_delete_and_paginate() {
    let api = api();
    let mut state = state_with_hosts();
    open(&api, &mut state, ',').await;
    select_section(&api, &mut state, SettingsSection::Archived).await;
    let query = api.settings.thread_queries.lock().unwrap()[0].clone();
    assert_eq!(query.archived, Some(true));
    assert_eq!(query.limit, Some(50));
    assert_eq!(query.offset, Some(0));
    let text = screen(&state);
    assert!(text.contains("Old work 0"), "{text}");
    assert!(text.contains("[child]"), "{text}");
    assert!(text.contains("archived 2d ago"), "{text}");
    assert!(text.contains("3 threads"), "{text}");

    keys(&api, &mut state, &[KeyCode::Char('t'), KeyCode::Char('t')]).await;
    assert_eq!(
        state.settings.as_ref().unwrap().archived.query.kind,
        ArchivedKind::Child
    );
    let last = api
        .settings
        .thread_queries
        .lock()
        .unwrap()
        .last()
        .cloned()
        .unwrap();
    assert_eq!(last.has_parent, Some(true));
    let text = screen(&state);
    assert!(!text.contains("Old work 0"), "{text}");
    keys(&api, &mut state, &[KeyCode::Char('t')]).await;

    move_to(&mut state, "Old work 1");
    keys(&api, &mut state, &[KeyCode::Char('u')]).await;
    assert!(matches!(
        api.settings.recorded().last(),
        Some(SettingsMutation::UnarchiveThread { thread_id }) if thread_id == "thr_arch1"
    ));
    assert!(!screen(&state).contains("Old work 1"));

    move_to(&mut state, "Old work 0");
    keys(&api, &mut state, &[KeyCode::Char('D')]).await;
    let text = screen(&state);
    assert!(
        text.contains("Its 2 child thread(s) are deleted too."),
        "{text}"
    );
    keys(&api, &mut state, &[KeyCode::Char('y')]).await;
    assert!(matches!(
        api.settings.recorded().last(),
        Some(SettingsMutation::DeleteThread { thread_id, child_threads_confirmed: true }) if thread_id == "thr_arch0"
    ));

    move_to(&mut state, "Old work 2");
    let rest = keys(&api, &mut state, &[KeyCode::Enter]).await;
    assert!(state.settings.is_none());
    assert_eq!(state.open_thread.as_deref(), Some("thr_arch2"));
    assert!(rest.contains(&Effect::WatchThread(Some("thr_arch2".into()))));
}

#[tokio::test]
async fn archived_load_more_requests_next_offset() {
    let api = api();
    {
        let mut archived = api.settings.archived.lock().unwrap();
        for index in 0..60 {
            let mut entry = thread(
                &format!("thr_many{index}"),
                "proj_a",
                &format!("Many {index}"),
                10,
            );
            entry.thread.archived_at = Some(NOW_MS);
            archived.push(entry);
        }
    }
    let mut state = state_with_hosts();
    open(&api, &mut state, ',').await;
    select_section(&api, &mut state, SettingsSection::Archived).await;
    assert!(state.settings.as_ref().unwrap().archived.has_more);
    move_to(&mut state, "Load more");
    keys(&api, &mut state, &[KeyCode::Enter]).await;
    let last = api
        .settings
        .thread_queries
        .lock()
        .unwrap()
        .last()
        .cloned()
        .unwrap();
    assert_eq!(last.offset, Some(50));
    let archived = &state.settings.as_ref().unwrap().archived;
    assert_eq!(archived.threads.len(), 64);
    assert!(!archived.has_more);
}

#[tokio::test]
async fn plugins_toggle_and_generic_settings_form() {
    let api = api();
    let mut state = state_with_hosts();
    open(&api, &mut state, ',').await;
    select_section(&api, &mut state, SettingsSection::Plugins).await;
    let text = screen(&state);
    assert!(text.contains("Push notifications"), "{text}");
    assert!(
        text.contains("push-notifications · v1.0.0 · settings"),
        "{text}"
    );
    assert!(text.contains("[disabled]"), "{text}");

    move_to(&mut state, "Workflows");
    keys(&api, &mut state, &[KeyCode::Char(' ')]).await;
    assert!(matches!(
        api.settings.recorded().last(),
        Some(SettingsMutation::SetPluginEnabled { plugin_id, enabled: true }) if plugin_id == "workflows"
    ));

    move_to(&mut state, "Push notifications");
    keys(&api, &mut state, &[KeyCode::Enter]).await;
    let text = screen(&state);
    for expected in [
        "Send pings",
        "Mode",
        "fast",
        "Limit",
        "Max items",
        "[not set]",
        "[secret]",
        "one … (2 lines)",
        "Select a project…",
        "[web only]",
    ] {
        assert!(text.contains(expected), "missing {expected}\n{text}");
    }
    let keys_in_order: Vec<String> = bb_tui::settings::settings_rows(&state)
        .iter()
        .filter_map(|row| match &row.target {
            Some(bb_tui::settings::RowTarget::PluginField { key, .. }) => Some(key.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        keys_in_order,
        vec![
            "enabled", "mode", "limit", "token", "notes", "home", "color"
        ]
    );

    move_to(&mut state, "Send pings");
    keys(&api, &mut state, &[KeyCode::Char(' ')]).await;
    assert!(matches!(
        api.settings.recorded().last(),
        Some(SettingsMutation::UpdatePluginSetting { key, value, secret: false, .. })
            if key == "enabled" && *value == json!(true)
    ));

    move_to(&mut state, "Mode");
    press(&mut state, KeyCode::Enter);
    keys(&api, &mut state, &[KeyCode::Char('j'), KeyCode::Enter]).await;
    assert!(matches!(
        api.settings.recorded().last(),
        Some(SettingsMutation::UpdatePluginSetting { key, value, .. })
            if key == "mode" && *value == json!("slow")
    ));

    move_to(&mut state, "Limit");
    press(&mut state, KeyCode::Enter);
    press(&mut state, KeyCode::Backspace);
    press(&mut state, KeyCode::Char('x'));
    assert!(press(&mut state, KeyCode::Enter).is_empty());
    assert!(screen(&state).contains("Enter a finite number"));
    press(&mut state, KeyCode::Backspace);
    for ch in "2.5".chars() {
        press(&mut state, KeyCode::Char(ch));
    }
    keys(&api, &mut state, &[KeyCode::Enter]).await;
    assert!(matches!(
        api.settings.recorded().last(),
        Some(SettingsMutation::UpdatePluginSetting { key, value, .. })
            if key == "limit" && *value == json!(2.5)
    ));

    let before = api.settings.recorded().len();
    move_to(&mut state, "API token");
    press(&mut state, KeyCode::Enter);
    assert!(press(&mut state, KeyCode::Enter).is_empty());
    assert_eq!(api.settings.recorded().len(), before);
    press(&mut state, KeyCode::Enter);
    for ch in "s3cret-value".chars() {
        press(&mut state, KeyCode::Char(ch));
    }
    let text = screen(&state);
    assert!(!text.contains("s3cret-value"), "{text}");
    assert!(text.contains("••••••••••••"), "{text}");
    let effects = press(&mut state, KeyCode::Enter);
    let debug = format!("{effects:?}");
    assert!(!debug.contains("s3cret-value"), "{debug}");
    assert!(debug.contains("<redacted>"), "{debug}");
    run_settings(&api, &mut state, effects).await;
    assert!(matches!(
        api.settings.recorded().last(),
        Some(SettingsMutation::UpdatePluginSetting { key, secret: true, .. }) if key == "token"
    ));
    assert!(screen(&state).contains("[set]"));

    move_to(&mut state, "Notes");
    press(&mut state, KeyCode::Enter);
    assert_eq!(
        state
            .settings
            .as_ref()
            .unwrap()
            .edit
            .as_ref()
            .unwrap()
            .input
            .text(),
        "one\\ntwo"
    );
    for ch in "\\nthree".chars() {
        press(&mut state, KeyCode::Char(ch));
    }
    keys(&api, &mut state, &[KeyCode::Enter]).await;
    assert!(matches!(
        api.settings.recorded().last(),
        Some(SettingsMutation::UpdatePluginSetting { key, value, .. })
            if key == "notes" && *value == json!("one\ntwo\nthree")
    ));

    move_to(&mut state, "Home project");
    press(&mut state, KeyCode::Enter);
    assert!(screen(&state).contains("beta"));
    keys(&api, &mut state, &[KeyCode::Char('3')]).await;
    assert!(matches!(
        api.settings.recorded().last(),
        Some(SettingsMutation::UpdatePluginSetting { key, value, .. })
            if key == "home" && *value == json!("proj_b")
    ));
}

#[tokio::test]
async fn plugin_setting_rejection_shows_field_error() {
    let api = api();
    let mut state = state_with_hosts();
    open(&api, &mut state, ',').await;
    select_section(&api, &mut state, SettingsSection::Plugins).await;
    move_to(&mut state, "Push notifications");
    keys(&api, &mut state, &[KeyCode::Enter]).await;
    *api.settings.reject.lock().unwrap() = Some("limit must be positive".into());
    move_to(&mut state, "Limit");
    press(&mut state, KeyCode::Enter);
    press(&mut state, KeyCode::Backspace);
    press(&mut state, KeyCode::Char('9'));
    keys(&api, &mut state, &[KeyCode::Enter]).await;
    let text = screen(&state);
    assert!(text.contains("limit must be positive"), "{text}");
    assert!(
        state
            .toasts
            .iter()
            .any(|toast| toast.message.contains("Failed to save the plugin setting"))
    );
}

#[tokio::test]
async fn palette_commands_open_sections_and_other_commands_close_settings() {
    let api = api();
    let mut state = state_with_hosts();
    let effects = bb_tui::app::update(
        &mut state,
        Action::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('k'),
            KeyModifiers::CONTROL,
        )),
    );
    run_settings(&api, &mut state, effects).await;
    for ch in "archived threads".chars() {
        press(&mut state, KeyCode::Char(ch));
    }
    let effects = press(&mut state, KeyCode::Enter);
    run_settings(&api, &mut state, effects).await;
    assert_eq!(
        state.settings.as_ref().map(|view| view.section),
        Some(SettingsSection::Archived)
    );
    press_mod(&mut state, KeyCode::Char('k'), KeyModifiers::CONTROL);
    assert!(state.palette.is_some());
    for ch in "new thread".chars() {
        press(&mut state, KeyCode::Char(ch));
    }
    press(&mut state, KeyCode::Enter);
    assert!(state.settings.is_none());
    assert!(state.new_thread.is_some());
}

#[test]
fn form_validation_helpers() {
    assert!(is_valid_git_branch_name("bb/slug-thr_id"));
    assert!(!is_valid_git_branch_name("bb/.hidden"));
    assert!(!is_valid_git_branch_name("HEAD"));
    assert!(!is_valid_git_branch_name("a b"));
    assert!(validate_branch_prefix("").is_ok());
    assert!(validate_branch_prefix("team/").is_ok());
    assert!(validate_branch_prefix("x/").is_ok());
    assert!(validate_branch_prefix("-x").is_err());
    assert!(validate_branch_prefix("a~").is_err());
    assert!(validate_branch_prefix(&"a".repeat(65)).is_err());
    assert_eq!(validate_server_url(" "), Ok(None));
    assert_eq!(
        validate_server_url("https://bb.example.com:8443/"),
        Ok(Some("https://bb.example.com:8443/".into()))
    );
    assert!(validate_server_url("bb.example.com").is_err());
    assert!(validate_server_url("https://u:p@bb.example.com").is_err());

    for text in ["", "plain", "a\nb", "back\\slash\\n", "tab\tend\n"] {
        assert_eq!(unescape_multiline(&escape_multiline(text)), text);
    }
    assert_eq!(unescape_multiline("trailing\\"), "trailing\\");

    assert_eq!(
        parse_field_input(&FieldKind::Number, " "),
        Ok(Some(json!(null)))
    );
    assert_eq!(
        parse_field_input(&FieldKind::Number, "1e3"),
        Ok(Some(json!(1000.0)))
    );
    assert!(parse_field_input(&FieldKind::Number, "inf").is_err());
    assert!(parse_field_input(&FieldKind::Number, "NaN").is_err());
    assert_eq!(parse_field_input(&FieldKind::Secret, ""), Ok(None));
    assert!(parse_field_input(&FieldKind::Select(vec!["a".into()]), "b").is_err());
}
