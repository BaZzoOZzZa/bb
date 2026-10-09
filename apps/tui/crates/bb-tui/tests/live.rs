mod support;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bb_client::{BbClient, DiscoveryInput, discover_server, discovery};
use bb_store::{NoRealtime, Store, StoreConfig, StoreSnapshot, TurnDetailsEntry, TurnDetailsKey};
use bb_tui::api::Api;
use bb_tui::app::{Action, AppState, ServerInfo, update};
use bb_tui::time::now_ms;
use bb_tui::timeline::ItemKind;
use bb_wire::routes::threads;
use bb_wire::timeline::{
    ActivityIntent, SystemMessageSubject, ThreadTimelineQuery, ThreadTimelineResponse, TimelineRow,
    TimelineWorkRow, TurnSummaryDetailsResponse,
};
use support::render_lines;

#[tokio::test]
#[ignore = "needs a running bb server; read-only"]
async fn renders_live_sidebar_read_only() {
    let found = discover_server(
        &DiscoveryInput::from_env(None),
        discovery::process_is_running,
    )
    .expect("discover");
    let client = BbClient::local(found.endpoint.clone()).expect("client");
    let overrides = client.keybinding_overrides().await.expect("config");
    let store = Store::spawn(Arc::new(client), NoRealtime, None, StoreConfig::default());
    let _interest = store.watch_sidebar();
    let mut rx = store.subscribe();
    tokio::time::timeout(Duration::from_secs(10), async {
        while rx.borrow_and_update().sidebar.data.is_none() {
            rx.changed().await.expect("store");
        }
    })
    .await
    .expect("sidebar loaded");
    let mut state = AppState::new(
        ServerInfo {
            url: found.endpoint.to_string(),
            profile: None,
        },
        Instant::now(),
        now_ms(),
    );
    update(&mut state, Action::KeymapLoaded(Ok(overrides)));
    update(&mut state, Action::Snapshot(store.snapshot()));
    assert!(!state.rows.is_empty());
    for line in render_lines(&state, 120, 30) {
        println!("{line}");
    }
}

fn collect_drift(
    raw: &serde_json::Value,
    typed: &serde_json::Value,
    path: &str,
    out: &mut BTreeMap<String, usize>,
) {
    match (raw, typed) {
        (serde_json::Value::Object(raw), serde_json::Value::Object(typed)) => {
            for (key, value) in raw {
                let child = format!("{path}.{key}");
                let empty = match value {
                    serde_json::Value::Null => true,
                    serde_json::Value::Array(items) => items.is_empty(),
                    serde_json::Value::Object(fields) => fields.is_empty(),
                    _ => false,
                };
                match typed.get(key) {
                    Some(typed_value) => collect_drift(value, typed_value, &child, out),
                    None if empty || key == "kind" || key == "workKind" => {}
                    None => *out.entry(format!("unmodeled {child}")).or_default() += 1,
                }
            }
        }
        (serde_json::Value::Array(raw), serde_json::Value::Array(typed)) => {
            for (raw, typed) in raw.iter().zip(typed) {
                collect_drift(raw, typed, &format!("{path}[]"), out);
            }
        }
        (serde_json::Value::String(raw), serde_json::Value::String(typed))
            if typed == "unknown" && raw != "unknown" =>
        {
            *out.entry(format!("unknown enum {path}={raw}")).or_default() += 1;
        }
        _ => {}
    }
}

fn count_unknown_rows(rows: &[TimelineRow], out: &mut BTreeMap<String, usize>) {
    for row in rows {
        if row.is_unknown() {
            let value = serde_json::to_value(row).unwrap_or_default();
            let kind = format!(
                "unknown row kind={} workKind={}",
                value["kind"].as_str().unwrap_or("?"),
                value["workKind"].as_str().unwrap_or("-")
            );
            *out.entry(kind).or_default() += 1;
        }
        if let TimelineRow::Work(TimelineWorkRow::Command(command)) = row {
            for intent in &command.activity_intents {
                if matches!(intent, ActivityIntent::Unknown(_)) {
                    *out.entry("unknown activity intent".into()).or_default() += 1;
                }
            }
        }
        if let TimelineRow::Conversation(conversation) = row
            && matches!(
                conversation.system_message_subject,
                Some(SystemMessageSubject::Unknown(_))
            )
        {
            *out.entry("unknown system message subject".into())
                .or_default() += 1;
        }
        if let Some(nested) = row.nested_rows() {
            count_unknown_rows(nested, out);
        }
    }
}

#[tokio::test]
#[ignore = "needs a running bb server; read-only GET requests only"]
async fn renders_live_timelines_read_only() {
    let found = discover_server(
        &DiscoveryInput::from_env(None),
        discovery::process_is_running,
    )
    .expect("discover");
    let client = BbClient::local(found.endpoint.clone()).expect("client");
    let limit: usize = std::env::var("BB_TUI_LIVE_THREADS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(10);
    let print = std::env::var_os("BB_TUI_LIVE_PRINT").is_some();
    let sidebar = client.sidebar_bootstrap().await.expect("sidebar");
    let mut entries: Vec<_> = sidebar
        .projects
        .iter()
        .chain(std::iter::once(&sidebar.personal_project))
        .flat_map(|project| project.threads.iter().cloned())
        .collect();
    entries.sort_by_key(|entry| -entry.thread.updated_at);
    let mut drift = BTreeMap::new();
    let mut total_rows = 0usize;
    let mut total_items = 0usize;
    for entry in entries.iter().take(limit) {
        let thread_id = entry.thread.id.clone();
        let raw: serde_json::Value = client
            .get_with(
                threads::timeline(&thread_id),
                &ThreadTimelineQuery::latest(),
            )
            .await
            .expect("timeline");
        let response: ThreadTimelineResponse =
            serde_json::from_value(raw.clone()).expect("parse timeline");
        collect_drift(
            &raw,
            &serde_json::to_value(&response).unwrap(),
            "timeline",
            &mut drift,
        );
        count_unknown_rows(&response.rows, &mut drift);
        total_rows += response.rows.len();
        let mut snapshot = StoreSnapshot::default();
        snapshot.sidebar.data = Some(Arc::new(sidebar.clone()));
        snapshot.connection = bb_store::ConnectionState::Connected;
        for row in response
            .rows
            .iter()
            .filter(|row| matches!(row, TimelineRow::Turn(turn) if turn.children.is_none()))
        {
            let Some(key) = TurnDetailsKey::for_turn(&thread_id, row) else {
                continue;
            };
            let raw: serde_json::Value = client
                .get_with(
                    threads::timeline_turn_summary_details(&thread_id),
                    &key.query(None),
                )
                .await
                .expect("turn details");
            let details: TurnSummaryDetailsResponse =
                serde_json::from_value(raw.clone()).expect("parse turn details");
            collect_drift(
                &raw,
                &serde_json::to_value(&details).unwrap(),
                "details",
                &mut drift,
            );
            count_unknown_rows(&details.rows, &mut drift);
            total_rows += details.rows.len();
            snapshot.turn_details.insert(
                key,
                Arc::new(TurnDetailsEntry {
                    rows: Arc::new(details.rows),
                    older_cursor: details.older_cursor,
                    loaded: true,
                    ..TurnDetailsEntry::default()
                }),
            );
        }
        snapshot
            .timelines
            .insert(thread_id.clone(), support::timeline_entry(response));
        let mut state = AppState::new(
            ServerInfo {
                url: found.endpoint.to_string(),
                profile: None,
            },
            Instant::now(),
            now_ms(),
        );
        update(&mut state, Action::Snapshot(Arc::new(snapshot)));
        let index = state
            .rows
            .iter()
            .position(|row| row.thread().is_some_and(|thread| thread.id == thread_id))
            .expect("sidebar row");
        state.selected = index;
        let _ignored_effects = update(
            &mut state,
            Action::Key(crossterm::event::KeyEvent::from(
                crossterm::event::KeyCode::Enter,
            )),
        );
        let mut expanded = 0;
        while expanded < 2_000 {
            let next = state.timeline().and_then(|timeline| {
                timeline.items.iter().find_map(|item| match &item.kind {
                    ItemKind::Row(row) if row.expandable && !row.expanded => Some(item.key.clone()),
                    _ => None,
                })
            });
            let Some(key) = next else {
                break;
            };
            state.timelines.get_mut(&thread_id).unwrap().selected = Some(key);
            let _ = update(
                &mut state,
                Action::Key(crossterm::event::KeyEvent::from(
                    crossterm::event::KeyCode::Char('l'),
                )),
            );
            expanded += 1;
        }
        total_items += state.timeline().map_or(0, |timeline| timeline.items.len());
        for width in [60u16, 100, 160] {
            let lines = render_lines(&state, width, 50);
            for line in &lines {
                assert!(unicode_width::UnicodeWidthStr::width(line.as_str()) <= usize::from(width));
            }
            if print && width == 100 {
                for line in lines {
                    println!("{line}");
                }
            }
        }
        let timeline = state.timeline().unwrap();
        let (width, height) = timeline.viewport_size();
        let _ = timeline.layout(width, height);
    }
    println!("rows parsed: {total_rows}, items rendered: {total_items}");
    for (kind, count) in &drift {
        println!("{count:>6}  {kind}");
    }
    assert!(
        !drift.keys().any(|kind| kind.starts_with("unknown row")),
        "unknown timeline rows: {drift:?}"
    );
}

#[tokio::test]
#[ignore = "needs a running bb server; read-only GET requests only"]
async fn renders_live_composer_read_only() {
    use bb_store::Resource;
    use bb_wire::project::PromptHistoryQuery;
    use bb_wire::prompt::{PromptHistoryEntry, ResolvedThreadExecutionOptions};
    use bb_wire::queue::ThreadQueuedMessage;
    use bb_wire::routes::system;
    use bb_wire::system::{SystemExecutionOptionsQuery, SystemExecutionOptionsResponse};
    use crossterm::event::{KeyCode, KeyEvent};

    let found = discover_server(
        &DiscoveryInput::from_env(None),
        discovery::process_is_running,
    )
    .expect("discover");
    let client = BbClient::local(found.endpoint.clone()).expect("client");
    let limit: usize = std::env::var("BB_TUI_LIVE_THREADS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(10);
    let print = std::env::var_os("BB_TUI_LIVE_PRINT").is_some();
    let sidebar = client.sidebar_bootstrap().await.expect("sidebar");
    let mut entries: Vec<_> = sidebar
        .projects
        .iter()
        .chain(std::iter::once(&sidebar.personal_project))
        .flat_map(|project| project.threads.iter().cloned())
        .filter(|entry| !entry.thread.is_archived())
        .collect();
    entries.sort_by_key(|entry| -entry.thread.updated_at);
    let mut drift = BTreeMap::new();
    let mut history_total = 0usize;
    for entry in entries.iter().take(limit) {
        let thread_id = entry.thread.id.clone();
        let query = SystemExecutionOptionsQuery {
            host_id: None,
            environment_id: entry.thread.environment_id.clone(),
            provider_id: Some(entry.thread.provider_id.clone()),
        };
        let raw: serde_json::Value = client
            .get_with(system::execution_options(), &query)
            .await
            .expect("execution options");
        let options: SystemExecutionOptionsResponse =
            serde_json::from_value(raw.clone()).expect("parse execution options");
        collect_drift(
            &raw,
            &serde_json::to_value(&options).unwrap(),
            "executionOptions",
            &mut drift,
        );
        let raw: serde_json::Value = client
            .get(threads::default_execution_options(&thread_id))
            .await
            .expect("defaults");
        let defaults: Option<ResolvedThreadExecutionOptions> =
            serde_json::from_value(raw.clone()).expect("parse defaults");
        if let Some(defaults) = &defaults {
            collect_drift(
                &raw,
                &serde_json::to_value(defaults).unwrap(),
                "defaults",
                &mut drift,
            );
        }
        let raw: serde_json::Value = client
            .get_with(
                threads::prompt_history(&thread_id),
                &PromptHistoryQuery::default(),
            )
            .await
            .expect("history");
        let history: Vec<PromptHistoryEntry> =
            serde_json::from_value(raw.clone()).expect("parse history");
        collect_drift(
            &raw,
            &serde_json::to_value(&history).unwrap(),
            "history",
            &mut drift,
        );
        history_total += history.len();
        let raw: serde_json::Value = client
            .get(threads::queued_messages(&thread_id))
            .await
            .expect("queued");
        let queued: Vec<ThreadQueuedMessage> =
            serde_json::from_value(raw.clone()).expect("parse queued");
        collect_drift(
            &raw,
            &serde_json::to_value(&queued).unwrap(),
            "queued",
            &mut drift,
        );
        let timeline = client
            .thread_timeline(&thread_id, &ThreadTimelineQuery::latest())
            .await
            .expect("timeline");

        let mut snapshot = StoreSnapshot::default();
        snapshot.sidebar.data = Some(Arc::new(sidebar.clone()));
        snapshot.connection = bb_store::ConnectionState::Connected;
        snapshot
            .timelines
            .insert(thread_id.clone(), support::timeline_entry(timeline));
        snapshot.queued_messages.insert(
            thread_id.clone(),
            Resource {
                data: Some(Arc::new(queued)),
                ..Resource::default()
            },
        );
        let mut state = AppState::new(
            ServerInfo {
                url: found.endpoint.to_string(),
                profile: None,
            },
            Instant::now(),
            now_ms(),
        );
        state.read_only = true;
        update(&mut state, Action::Snapshot(Arc::new(snapshot)));
        let index = state
            .rows
            .iter()
            .position(|row| row.thread().is_some_and(|thread| thread.id == thread_id));
        let Some(index) = index else {
            continue;
        };
        state.selected = index;
        update(&mut state, Action::Key(KeyEvent::from(KeyCode::Enter)));
        update(
            &mut state,
            Action::ExecutionOptionsLoaded {
                thread_id: thread_id.clone(),
                result: Ok(options),
            },
        );
        update(
            &mut state,
            Action::DefaultsLoaded {
                thread_id: thread_id.clone(),
                result: Ok(defaults.clone()),
            },
        );
        update(
            &mut state,
            Action::PromptHistoryLoaded {
                thread_id: thread_id.clone(),
                result: Ok(history.clone()),
            },
        );
        let resolved = state.resolved_execution(&thread_id).expect("execution");
        if defaults.is_some() {
            assert!(!resolved.model.is_empty(), "{thread_id}: no model");
        }
        update(&mut state, Action::Key(KeyEvent::from(KeyCode::Char('c'))));
        if !history.is_empty() {
            update(&mut state, Action::Key(KeyEvent::from(KeyCode::Up)));
            assert!(
                !state.composer().expect("composer").text().is_empty(),
                "{thread_id}: history"
            );
        }
        let lines = render_lines(&state, 140, 40);
        assert!(
            lines
                .iter()
                .all(|line| unicode_width::UnicodeWidthStr::width(line.as_str()) <= 140)
        );
        let footer = &lines[lines.len() - 3];
        assert!(
            footer.contains(&resolved.model_label) || resolved.model.is_empty(),
            "{thread_id}: {footer}"
        );
        if print {
            println!(
                "== {thread_id} {}",
                entry.thread.title_or_fallback().unwrap_or("")
            );
            for line in &lines[lines.len().saturating_sub(16)..] {
                println!("{line}");
            }
        }
    }
    println!("history entries: {history_total}");
    for (key, count) in &drift {
        println!("drift {key}: {count}");
    }
}
