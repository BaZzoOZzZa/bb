mod support;

use bb_tui::app::{Action, AppState, Effect, update};
use bb_tui::runtime::run_thread_list_request;
use bb_tui::sidebar::SidebarRow;
use bb_tui::thread_list::ThreadListRequest;
use bb_wire::thread_list::OrganizationMode;
use serde_json::json;
use support::*;

fn ex(state: &mut AppState, text: &str) -> Vec<Effect> {
    keys(state, ":");
    let mut effects = type_chars(state, text);
    effects.extend(keys(state, "RET"));
    effects
}

async fn drive(api: &MockApi, state: &mut AppState, effects: Vec<Effect>) -> Vec<Effect> {
    let mut queue = effects;
    let mut rest = Vec::new();
    while !queue.is_empty() {
        let mut next = Vec::new();
        for effect in queue {
            match effect {
                Effect::ThreadList(request) => {
                    let loaded = run_thread_list_request(api, request).await;
                    next.extend(update(state, Action::ThreadListLoaded(loaded)));
                }
                other => rest.push(other),
            }
        }
        queue = next;
    }
    rest
}

fn group_labels(state: &AppState) -> Vec<String> {
    state
        .rows
        .iter()
        .filter_map(|row| match row {
            SidebarRow::Group(group) => Some(group.label.clone()),
            SidebarRow::Thread(_) => None,
        })
        .collect()
}

fn signal(key: &str, value: serde_json::Value) -> Action {
    Action::PluginSignal {
        plugin_id: "thread-list".into(),
        channel: "preferences".into(),
        payload: json!({ "key": key, "value": value }),
    }
}

fn hosted_sidebar() -> bb_wire::project::SidebarBootstrapResponse {
    let mut sidebar = sample_sidebar();
    for thread in &mut sidebar.projects[0].threads {
        thread.environment_host_id = Some("host_laptop".into());
    }
    sidebar
}

async fn started(api: &MockApi, sidebar: bb_wire::project::SidebarBootstrapResponse) -> AppState {
    let mut state = loaded_state(sidebar);
    state.thread_list.prefs = Default::default();
    let effects = state.thread_list_start();
    state.rebuild_rows();
    assert!(
        state.rows.is_empty(),
        "rows wait for the first preference load"
    );
    drive(api, &mut state, effects).await;
    state
}

#[tokio::test]
async fn preferences_load_from_the_plugin_rpc_and_shape_the_sidebar() {
    let api = MockApi::new(FakeBackend::new(sample_sidebar()));
    api.thread_list
        .preferences
        .lock()
        .unwrap()
        .insert("collapsedProjects".into(), json!(["proj_b"]));
    let state = started(&api, sample_sidebar()).await;
    assert_eq!(
        api.thread_list.calls(),
        vec![("listPreferences".to_owned(), json!(null))]
    );
    assert_eq!(group_labels(&state), ["alpha", "beta"]);
    let lines = render_lines(&state, 100, 12);
    assert!(
        lines.iter().any(|line| line.contains("▸ beta")),
        "{lines:?}"
    );
    assert!(state.selected_thread_id().is_some());
}

#[tokio::test]
async fn set_commands_write_the_shared_preference_and_relayout() {
    let api = MockApi::new(FakeBackend::new(hosted_sidebar()));
    let mut state = started(&api, hosted_sidebar()).await;
    let effects = ex(&mut state, "set organization=machine");
    assert!(
        effects.contains(&Effect::ThreadList(ThreadListRequest::Set {
            key: bb_wire::thread_list::PreferenceKey::OrganizationMode,
            value: json!("machine"),
        }))
    );
    assert_eq!(group_labels(&state), ["host_laptop", "No machine"]);
    drive(&api, &mut state, effects).await;
    let effects = ex(&mut state, "set sort=alpha sortdir=desc");
    drive(&api, &mut state, effects).await;
    let effects = ex(&mut state, "set archived");
    drive(&api, &mut state, effects).await;
    let effects = ex(&mut state, "set organization!");
    drive(&api, &mut state, effects).await;
    assert_eq!(
        api.thread_list.writes(),
        vec![
            json!({"key": "organizationMode", "value": "machine"}),
            json!({"key": "chronologicalSort", "value": "alpha"}),
            json!({"key": "sortDirection", "value": "descending"}),
            json!({"key": "threadLifecycles", "value": ["active", "archived"]}),
            json!({"key": "organizationMode", "value": "project"}),
        ]
    );
    assert_eq!(
        state.thread_list.prefs.organization_mode,
        OrganizationMode::Project
    );
    assert_eq!(
        state.thread_list.prefs.sort_direction,
        bb_wire::thread_list::SortDirection::Descending
    );
    let effects = ex(&mut state, "set sortdir=sideways");
    assert!(effects.is_empty());
    assert!(
        state
            .keys
            .echo
            .as_ref()
            .is_some_and(|echo| echo.error && echo.text.contains("sortdir=sideways"))
    );
}

#[tokio::test]
async fn read_only_mode_refuses_preference_writes_but_folds_locally() {
    let api = MockApi::new(FakeBackend::new(sample_sidebar()));
    let mut state = started(&api, sample_sidebar()).await;
    state.read_only = true;
    let effects = ex(&mut state, "set organization=machine");
    assert!(effects.is_empty());
    assert_eq!(
        state.thread_list.prefs.organization_mode,
        OrganizationMode::Project
    );
    assert!(state.keys.echo.as_ref().is_some_and(|echo| echo.error));
    let effects = keys(&mut state, "SPC t o");
    assert!(effects.is_empty());
    keys(&mut state, "g g");
    let effects = keys(&mut state, "z a");
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::ThreadList(_)))
    );
    assert_eq!(collapsed_projects(&state), ["proj_a"]);
    assert!(api.thread_list.writes().is_empty());
}

#[tokio::test]
async fn folding_a_group_writes_its_collapsed_list() {
    let api = MockApi::new(FakeBackend::new(sample_sidebar()));
    let mut state = started(&api, sample_sidebar()).await;
    keys(&mut state, "g g");
    let effects = keys(&mut state, "z a");
    drive(&api, &mut state, effects).await;
    let effects = keys(&mut state, "z R");
    drive(&api, &mut state, effects).await;
    assert_eq!(
        api.thread_list.writes(),
        vec![
            json!({"key": "collapsedProjects", "value": ["proj_a"]}),
            json!({"key": "collapsedProjects", "value": []}),
        ]
    );
}

#[tokio::test]
async fn realtime_signals_apply_changes_made_by_the_web_or_cli() {
    let api = MockApi::new(FakeBackend::new(hosted_sidebar()));
    let mut state = started(&api, hosted_sidebar()).await;
    update(&mut state, signal("organizationMode", json!("machine")));
    assert_eq!(group_labels(&state), ["host_laptop", "No machine"]);
    update(
        &mut state,
        signal("hiddenGroups", json!(["machine:no-machine"])),
    );
    assert_eq!(group_labels(&state), ["host_laptop", "More"]);
    update(&mut state, signal("organizationMode", json!("bogus")));
    assert_eq!(
        state.thread_list.prefs.organization_mode,
        OrganizationMode::Machine
    );
    update(
        &mut state,
        Action::PluginSignal {
            plugin_id: "thread-list".into(),
            channel: "preferences".into(),
            payload: json!({"key": "organizationMode"}),
        },
    );
    assert_eq!(group_labels(&state), ["host_laptop", "More"]);
    let effects = ex(&mut state, "set organization=project");
    update(&mut state, signal("organizationMode", json!("machine")));
    assert_eq!(
        state.thread_list.prefs.organization_mode,
        OrganizationMode::Project,
        "a signal for a key with a write in flight is ignored"
    );
    drive(&api, &mut state, effects).await;
    update(
        &mut state,
        signal("organizationMode", json!("chronological")),
    );
    assert_eq!(group_labels(&state), ["Threads"]);
}

#[tokio::test]
async fn realtime_signals_reach_the_state_through_the_event_stream() {
    let (tx, rx) = tokio::sync::broadcast::channel(8);
    let mut actions = bb_tui::runtime::realtime_actions(rx);
    tx.send(bb_client::RealtimeEvent::PluginSignal {
        plugin_id: "thread-list".into(),
        channel: "preferences".into(),
        payload: json!({"key": "organizationMode", "value": "machine"}),
    })
    .unwrap();
    let action = futures::StreamExt::next(&mut actions).await.unwrap();
    let api = MockApi::new(FakeBackend::new(hosted_sidebar()));
    let mut state = started(&api, hosted_sidebar()).await;
    update(&mut state, action);
    assert_eq!(group_labels(&state), ["host_laptop", "No machine"]);
}

#[tokio::test]
async fn failed_writes_report_and_reload_the_server_value() {
    let api = MockApi::new(FakeBackend::new(sample_sidebar()));
    let mut state = started(&api, sample_sidebar()).await;
    *api.thread_list.reject.lock().unwrap() = Some("storage unavailable".into());
    let effects = ex(&mut state, "set organization=chronological");
    drive(&api, &mut state, effects).await;
    assert_eq!(
        state.thread_list.prefs.organization_mode,
        OrganizationMode::Project
    );
    assert!(
        state
            .toasts
            .iter()
            .any(|toast| toast.message.contains("storage unavailable"))
    );
}

#[tokio::test]
async fn writes_for_one_key_wait_for_the_previous_reply_and_send_the_latest_value() {
    let api = MockApi::new(FakeBackend::new(sample_sidebar()));
    let mut state = started(&api, sample_sidebar()).await;
    keys(&mut state, "g g");
    let first = keys(&mut state, "z a");
    state.selected = state
        .rows
        .iter()
        .position(|row| matches!(row, SidebarRow::Group(group) if group.label == "beta"))
        .expect("beta group");
    let second = keys(&mut state, "z a");
    assert!(
        !second
            .iter()
            .any(|effect| matches!(effect, Effect::ThreadList(_))),
        "a second write for the same key waits for the first reply"
    );
    assert_eq!(collapsed_projects(&state), ["proj_a", "proj_b"]);
    drive(&api, &mut state, first).await;
    assert_eq!(
        api.thread_list.writes(),
        vec![
            json!({"key": "collapsedProjects", "value": ["proj_a"]}),
            json!({"key": "collapsedProjects", "value": ["proj_a", "proj_b"]}),
        ]
    );
    assert_eq!(collapsed_projects(&state), ["proj_a", "proj_b"]);
    assert!(state.thread_list.pending.is_empty());
}

#[tokio::test]
async fn a_missing_plugin_keeps_its_defaults_and_changes_stay_local() {
    let api = MockApi::new(FakeBackend::new(sample_sidebar()));
    *api.thread_list.unavailable.lock().unwrap() = true;
    let mut state = started(&api, sample_sidebar()).await;
    assert_eq!(
        state.thread_list.prefs.organization_mode,
        OrganizationMode::Chronological
    );
    assert!(!state.rows.is_empty());
    let effects = ex(&mut state, "set organization=project");
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::ThreadList(_)))
    );
    assert_eq!(
        state.thread_list.prefs.organization_mode,
        OrganizationMode::Project
    );
    assert!(
        state
            .keys
            .echo
            .as_ref()
            .is_some_and(|echo| !echo.error && echo.text.contains("not saved"))
    );
    keys(&mut state, "g g");
    let effects = keys(&mut state, "z a");
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::ThreadList(_)))
    );
    assert_eq!(collapsed_projects(&state), ["proj_a"]);
    assert!(api.thread_list.writes().is_empty());
    assert!(state.toasts.is_empty());
}

fn archived(id: &str, title: &str, minutes_ago: i64) -> bb_wire::thread::ThreadListEntry {
    let mut entry = thread(id, "proj_b", title, minutes_ago);
    entry.thread.archived_at = Some(NOW_MS);
    entry
}

#[tokio::test]
async fn archived_lifecycle_pages_archived_threads_with_show_more() {
    let api = MockApi::new(FakeBackend::new(sample_sidebar()));
    *api.settings.archived.lock().unwrap() = (0..55)
        .map(|index| {
            archived(
                &format!("thr_a{index}"),
                &format!("Old {index}"),
                100 + index,
            )
        })
        .collect();
    let mut state = started(&api, sample_sidebar()).await;
    let effects = keys(&mut state, "SPC t a");
    drive(&api, &mut state, effects).await;
    assert_eq!(state.thread_list.archived.entries.len(), 50);
    let more = state
        .rows
        .iter()
        .position(|row| matches!(row, SidebarRow::Group(group) if group.label == "Show more"))
        .expect("show more row");
    assert!(
        state
            .rows
            .iter()
            .any(|row| row.thread().is_some_and(|thread| thread.archived))
    );
    state.selected = more;
    let effects = keys(&mut state, "RET");
    drive(&api, &mut state, effects).await;
    assert_eq!(state.thread_list.archived.entries.len(), 55);
    assert!(
        !state
            .rows
            .iter()
            .any(|row| matches!(row, SidebarRow::Group(group) if group.label == "Show more"))
    );
    let queries = api.settings.thread_queries.lock().unwrap().clone();
    assert_eq!(
        queries
            .iter()
            .map(|query| (query.archived, query.offset, query.limit))
            .collect::<Vec<_>>(),
        [
            (Some(true), Some(0), Some(50)),
            (Some(true), Some(50), Some(50))
        ]
    );

    let mut sidebar = sample_sidebar();
    sidebar.projects[1].threads[0].thread.title = Some("Renamed".into());
    let effects = update(&mut state, Action::Snapshot(snapshot_with(sidebar)));
    assert!(
        effects.contains(&Effect::ThreadList(ThreadListRequest::Archived {
            generation: state.thread_list.archived.generation,
            offset: 0,
            limit: 100,
            append: false,
        }))
    );
    drive(&api, &mut state, effects).await;
    assert_eq!(state.thread_list.archived.entries.len(), 55);

    let effects = ex(&mut state, "set noarchived");
    drive(&api, &mut state, effects).await;
    assert!(state.thread_list.archived.entries.is_empty());
    assert!(
        !state
            .rows
            .iter()
            .any(|row| row.thread().is_some_and(|t| t.archived))
    );
}
