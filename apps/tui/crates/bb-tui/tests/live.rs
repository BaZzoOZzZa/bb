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

#[tokio::test]
#[ignore = "needs a running bb server; read-only GET requests only"]
async fn renders_live_pending_interactions_read_only() {
    let found = discover_server(
        &DiscoveryInput::from_env(None),
        discovery::process_is_running,
    )
    .expect("discover");
    let client = BbClient::local(found.endpoint.clone()).expect("client");
    let sidebar = client.sidebar_bootstrap().await.expect("sidebar");
    let mut snapshot = StoreSnapshot::default();
    snapshot.sidebar.data = Some(Arc::new(sidebar.clone()));
    let ids: Vec<String> = snapshot
        .sidebar_threads()
        .map(|entry| entry.thread.id.clone())
        .collect();
    let mut total = 0;
    for id in &ids {
        let interactions = client.thread_interactions(id).await.expect("interactions");
        total += interactions.len();
        snapshot.interactions.insert(
            id.clone(),
            bb_store::Resource {
                data: Some(Arc::new(interactions)),
                ..bb_store::Resource::default()
            },
        );
    }
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
    println!(
        "{} threads, {total} interactions, {} waiting",
        ids.len(),
        state.waiting
    );
    for _ in 0..state.waiting {
        update(
            &mut state,
            Action::Key(crossterm::event::KeyEvent::from(
                crossterm::event::KeyCode::Char('w'),
            )),
        );
        for line in render_lines(&state, 120, 30) {
            println!("{line}");
        }
    }
}

#[tokio::test]
#[ignore = "needs a running bb server; read-only"]
async fn renders_live_new_thread_form_read_only() {
    use bb_tui::app::Effect;
    use bb_tui::new_thread::{FormField, NewThreadRequest};
    use bb_tui::runtime::run_new_thread_request;
    use crossterm::event::{KeyCode, KeyEvent};

    let found = discover_server(
        &DiscoveryInput::from_env(None),
        discovery::process_is_running,
    )
    .expect("discover");
    let client = BbClient::local(found.endpoint.clone()).expect("client");
    let store = Store::spawn(
        Arc::new(client.clone()),
        NoRealtime,
        None,
        StoreConfig::default(),
    );
    let _interest = store.watch_sidebar();
    let mut rx = store.subscribe();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            {
                let snapshot = rx.borrow_and_update();
                if snapshot.sidebar.data.is_some() && snapshot.hosts.data.is_some() {
                    break;
                }
            }
            rx.changed().await.expect("store");
        }
    })
    .await
    .expect("sidebar and hosts loaded");
    let mut state = AppState::new(
        ServerInfo {
            url: found.endpoint.to_string(),
            profile: None,
        },
        Instant::now(),
        now_ms(),
    );
    state.read_only = true;
    update(&mut state, Action::Snapshot(store.snapshot()));

    async fn drive(state: &mut AppState, client: &BbClient, effects: Vec<Effect>) {
        let mut pending = effects;
        while !pending.is_empty() {
            for effect in std::mem::take(&mut pending) {
                if let Effect::NewThread {
                    generation,
                    request,
                } = effect
                {
                    assert!(
                        !matches!(request, NewThreadRequest::Create(_)),
                        "live test must never create threads"
                    );
                    let loaded = run_new_thread_request(client, request).await;
                    pending.extend(update(
                        state,
                        Action::NewThreadLoaded { generation, loaded },
                    ));
                }
            }
        }
    }

    let project_count = state
        .snapshot
        .sidebar
        .data
        .as_ref()
        .map_or(0, |sidebar| sidebar.projects.len() + 1);
    for index in 0..project_count.min(4) {
        let effects = update(&mut state, Action::Key(KeyEvent::from(KeyCode::Char('n'))));
        drive(&mut state, &client, effects).await;
        let effects = update(&mut state, Action::Key(KeyEvent::from(KeyCode::Enter)));
        drive(&mut state, &client, effects).await;
        for _ in 0..index {
            let effects = update(&mut state, Action::Key(KeyEvent::from(KeyCode::Down)));
            drive(&mut state, &client, effects).await;
        }
        let effects = update(&mut state, Action::Key(KeyEvent::from(KeyCode::Enter)));
        drive(&mut state, &client, effects).await;
        let resolved = state.resolved_new_thread().expect("form");
        assert!(resolved.environment_providers.is_some());
        println!("--- project #{index}: {}", resolved.project_label);
        for line in render_lines(&state, 120, 24) {
            println!("{line}");
        }
        let form = state.new_thread_form().expect("form");
        assert_eq!(form.field, FormField::Environment);
        let effects = update(&mut state, Action::Key(KeyEvent::from(KeyCode::Enter)));
        drive(&mut state, &client, effects).await;
        for line in render_lines(&state, 120, 24) {
            println!("{line}");
        }
        let effects = update(&mut state, Action::Key(KeyEvent::from(KeyCode::Esc)));
        drive(&mut state, &client, effects).await;
        let effects = update(&mut state, Action::Key(KeyEvent::from(KeyCode::Esc)));
        drive(&mut state, &client, effects).await;
        assert!(state.new_thread_form().is_none());
    }
}

#[tokio::test]
#[ignore = "needs a running bb server; read-only"]
async fn renders_live_palette_read_only() {
    use bb_tui::app::Effect;
    use bb_tui::palette::{PaletteKind, PaletteRequest};
    use bb_tui::runtime::run_palette_request;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let found = discover_server(
        &DiscoveryInput::from_env(None),
        discovery::process_is_running,
    )
    .expect("discover");
    let client = BbClient::local(found.endpoint.clone()).expect("client");
    let store = Store::spawn(
        Arc::new(client.clone()),
        NoRealtime,
        None,
        StoreConfig::default(),
    );
    let _interest = store.watch_sidebar();
    let mut rx = store.subscribe();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if rx.borrow_and_update().sidebar.data.is_some() {
                break;
            }
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
    state.read_only = true;
    update(&mut state, Action::Snapshot(store.snapshot()));

    async fn drive(state: &mut AppState, client: &BbClient, effects: Vec<Effect>) {
        let mut pending = effects;
        while !pending.is_empty() {
            for effect in std::mem::take(&mut pending) {
                if let Effect::Palette {
                    generation,
                    request,
                } = effect
                {
                    assert!(
                        !matches!(request, PaletteRequest::RunCli { .. }),
                        "live test must never run plugin commands"
                    );
                    let loaded = run_palette_request(client, request).await;
                    pending.extend(update(state, Action::PaletteLoaded { generation, loaded }));
                }
            }
        }
    }
    async fn keys(state: &mut AppState, client: &BbClient, events: Vec<KeyEvent>) {
        for event in events {
            let effects = update(state, Action::Key(event));
            drive(state, client, effects).await;
        }
    }
    fn text(value: &str) -> Vec<KeyEvent> {
        value
            .chars()
            .map(|ch| KeyEvent::from(KeyCode::Char(ch)))
            .collect()
    }
    fn print(state: &AppState, label: &str) {
        if std::env::var_os("BB_TUI_LIVE_PRINT").is_none() {
            return;
        }
        println!("--- {label}");
        for line in render_lines(state, 120, 30) {
            println!("{line}");
        }
    }
    let ctrl = |ch| KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL);
    let esc = KeyEvent::from(KeyCode::Esc);

    keys(&mut state, &client, vec![ctrl('k')]).await;
    let plugin_items = state
        .palette
        .as_ref()
        .expect("palette")
        .items
        .iter()
        .filter(|item| item.group == "Plugin commands")
        .count();
    println!("plugin commands: {plugin_items}");
    print(&state, "commands");
    keys(&mut state, &client, vec![esc]).await;

    let word = state
        .snapshot
        .sidebar_threads()
        .filter_map(|entry| entry.thread.title.clone())
        .flat_map(|title| {
            title
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .find(|word| word.chars().count() >= 4 && word.chars().all(char::is_alphanumeric))
        .unwrap_or_else(|| "test".into());
    keys(
        &mut state,
        &client,
        vec![KeyEvent::from(KeyCode::Char('f'))],
    )
    .await;
    keys(&mut state, &client, text(&word)).await;
    let palette = state.palette.as_ref().expect("threads palette");
    assert_eq!(palette.kind, PaletteKind::Threads);
    assert!(
        palette.threads.error.is_none(),
        "{:?}",
        palette.threads.error
    );
    println!("thread search {word:?}: {} rows", palette.items.len());
    print(&state, "threads");
    keys(&mut state, &client, vec![esc]).await;

    let with_environment = state.rows.iter().position(|row| {
        row.thread().is_some_and(|thread| {
            state
                .sidebar_entry(&thread.id)
                .is_some_and(|entry| entry.thread.environment_id.is_some())
        })
    });
    let Some(index) = with_environment else {
        println!("no thread with an environment; skipping quick open");
        return;
    };
    state.selected = index;
    state.focus = bb_tui::app::Focus::Sidebar;
    keys(&mut state, &client, vec![ctrl('p')]).await;
    keys(&mut state, &client, text("md")).await;
    let palette = state.palette.as_ref().expect("files palette");
    assert!(palette.source.is_some());
    println!(
        "file search: {} rows, error {:?}",
        palette.items.len(),
        palette.paths.error
    );
    print(&state, "files");
    if !palette.items.is_empty() {
        keys(&mut state, &client, vec![KeyEvent::from(KeyCode::Enter)]).await;
        let pane = state.result_pane.as_ref().expect("pane");
        println!("preview {}: {} lines", pane.title, pane.line_count());
        assert!(!matches!(pane.body, bb_tui::palette::PaneBody::Loading));
        print(&state, "preview");
        keys(&mut state, &client, vec![esc]).await;
    }
}

#[tokio::test]
#[ignore = "needs a running bb server; read-only GET requests only"]
async fn renders_live_mentions_read_only() {
    use bb_tui::app::{Effect, Focus};
    use bb_tui::runtime::run_typeahead_request;
    use bb_tui::typeahead::MenuState;
    use crossterm::event::{KeyCode, KeyEvent};

    let found = discover_server(
        &DiscoveryInput::from_env(None),
        discovery::process_is_running,
    )
    .expect("discover");
    let client = BbClient::local(found.endpoint.clone()).expect("client");
    let store = Store::spawn(
        Arc::new(client.clone()),
        NoRealtime,
        None,
        StoreConfig::default(),
    );
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
    state.read_only = true;
    update(&mut state, Action::Snapshot(store.snapshot()));

    async fn drive(state: &mut AppState, client: &BbClient, effects: Vec<Effect>) {
        let mut pending = effects;
        while !pending.is_empty() {
            for effect in std::mem::take(&mut pending) {
                match effect {
                    Effect::Typeahead(request) => {
                        let loaded = run_typeahead_request(client, request).await;
                        pending.extend(update(state, Action::TypeaheadLoaded(loaded)));
                    }
                    Effect::FetchExecutionOptions { thread_id, query } => {
                        let result = client.execution_options(query).await;
                        pending.extend(update(
                            state,
                            Action::ExecutionOptionsLoaded { thread_id, result },
                        ));
                    }
                    Effect::FetchDefaults(thread_id) => {
                        let result = client.default_execution_options(thread_id.clone()).await;
                        pending.extend(update(state, Action::DefaultsLoaded { thread_id, result }));
                    }
                    Effect::Mutate(mutation) => panic!("live test must not mutate: {mutation:?}"),
                    Effect::Upload(request) => panic!("live test must not upload: {request:?}"),
                    Effect::OpenExternal(target) => panic!("live test must not open {target}"),
                    _ => {}
                }
            }
        }
    }
    async fn keys(state: &mut AppState, client: &BbClient, events: Vec<KeyEvent>) {
        for event in events {
            let effects = update(state, Action::Key(event));
            drive(state, client, effects).await;
        }
    }
    fn text(value: &str) -> Vec<KeyEvent> {
        value
            .chars()
            .map(|ch| KeyEvent::from(KeyCode::Char(ch)))
            .collect()
    }
    fn print(state: &AppState, label: &str) {
        if std::env::var_os("BB_TUI_LIVE_PRINT").is_none() {
            return;
        }
        println!("--- {label}");
        for line in render_lines(state, 120, 30) {
            println!("{line}");
        }
    }
    fn menu_summary(state: &AppState) -> String {
        match state.typeahead.as_ref().map(|typeahead| &typeahead.menu) {
            Some(MenuState::Mentions(ordered)) => format!(
                "{} mentions in {:?}",
                ordered.len(),
                ordered
                    .groups
                    .iter()
                    .map(|group| group.label.as_str())
                    .collect::<Vec<_>>()
            ),
            Some(MenuState::Commands(ordered)) => format!(
                "{} commands in {:?}",
                ordered.len(),
                ordered
                    .groups
                    .iter()
                    .map(|group| group.label.as_str())
                    .collect::<Vec<_>>()
            ),
            Some(other) => format!("{other:?}"),
            None => "closed".into(),
        }
    }

    let with_environment = state.rows.iter().position(|row| {
        row.thread().is_some_and(|thread| {
            state.sidebar_entry(&thread.id).is_some_and(|entry| {
                entry.thread.environment_id.is_some() && !entry.thread.is_archived()
            })
        })
    });
    let Some(index) = with_environment else {
        println!("no thread with an environment; skipping");
        return;
    };
    state.selected = index;
    state.focus = Focus::Sidebar;
    keys(&mut state, &client, vec![KeyEvent::from(KeyCode::Enter)]).await;
    keys(
        &mut state,
        &client,
        vec![KeyEvent::from(KeyCode::Char('c'))],
    )
    .await;
    assert_eq!(state.focus, Focus::Composer);
    println!("mention triggers loaded: {:?}", state.mention_providers);

    let word = state
        .snapshot
        .sidebar_threads()
        .filter_map(|entry| entry.thread.title.clone())
        .flat_map(|title| {
            title
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .find(|word| word.chars().count() >= 4 && word.chars().all(char::is_alphanumeric))
        .unwrap_or_else(|| "read".into());
    let query: String = word.chars().take(4).collect();
    keys(&mut state, &client, text(&format!("Привет 😀 @{query}"))).await;
    println!("@{query}: {}", menu_summary(&state));
    print(&state, "mention");
    let has_items = state
        .typeahead
        .as_ref()
        .is_some_and(|typeahead| !typeahead.menu.is_empty());
    if has_items {
        keys(&mut state, &client, vec![KeyEvent::from(KeyCode::Tab)]).await;
        let composer = state.composer().expect("composer");
        let mention = composer.draft().mentions.first().expect("mention chip");
        assert_eq!(mention.start, 10);
        println!(
            "inserted {:?} as {}..{}",
            composer.text(),
            mention.start,
            mention.end
        );
        print(&state, "chip");
    }
    keys(&mut state, &client, text(" /")).await;
    println!("/: {}", menu_summary(&state));
    print(&state, "commands");
    keys(&mut state, &client, vec![KeyEvent::from(KeyCode::Esc)]).await;
    let composer = state.composer().expect("composer");
    let raw = composer.persisted_draft().serialize();
    println!("draft json: {raw:?}");
}

#[tokio::test]
#[ignore = "needs a running bb server; read-only GET requests only"]
async fn renders_live_panel_read_only() {
    use bb_tui::app::Effect;
    use bb_tui::panel::{PanelRequest, PanelTab};
    use bb_tui::runtime::run_panel_request;
    use crossterm::event::{KeyCode, KeyEvent};

    let found = discover_server(
        &DiscoveryInput::from_env(None),
        discovery::process_is_running,
    )
    .expect("discover");
    let client = BbClient::local(found.endpoint.clone()).expect("client");
    let store = Store::spawn(
        Arc::new(client.clone()),
        NoRealtime,
        None,
        StoreConfig::default(),
    );
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
    state.read_only = true;
    update(&mut state, Action::Snapshot(store.snapshot()));

    async fn drive(state: &mut AppState, client: &BbClient, effects: Vec<Effect>) -> usize {
        let mut skipped = 0;
        let mut pending = effects;
        while !pending.is_empty() {
            for effect in std::mem::take(&mut pending) {
                assert!(
                    !matches!(effect, Effect::Mutate(_) | Effect::EditFile { .. }),
                    "live test must not mutate or spawn an editor: {effect:?}"
                );
                if let Effect::Panel(request) = effect {
                    if matches!(request, PanelRequest::DiffPatches { .. }) {
                        skipped += 1;
                        continue;
                    }
                    let loaded = run_panel_request(client, request).await;
                    pending.extend(update(state, Action::PanelLoaded(loaded)));
                }
            }
        }
        skipped
    }
    fn print(state: &AppState, label: &str) {
        if std::env::var_os("BB_TUI_LIVE_PRINT").is_none() {
            return;
        }
        println!("--- {label}");
        for line in render_lines(state, 160, 40) {
            println!("{line}");
        }
    }

    let limit: usize = std::env::var("BB_TUI_LIVE_THREADS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(5);
    let candidates: Vec<String> = state
        .snapshot
        .sidebar_threads()
        .filter(|entry| entry.thread.environment_id.is_some() && !entry.thread.is_archived())
        .map(|entry| entry.thread.id.clone())
        .take(limit)
        .collect();
    for thread_id in candidates {
        let interest = store.watch_thread(&thread_id);
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if rx
                    .borrow_and_update()
                    .thread(&thread_id)
                    .is_some_and(|detail| detail.environment.is_some())
                {
                    break;
                }
                rx.changed().await.expect("store");
            }
        })
        .await
        .expect("thread detail");
        update(&mut state, Action::Snapshot(store.snapshot()));
        let index = state
            .rows
            .iter()
            .position(|row| row.thread().is_some_and(|thread| thread.id == thread_id));
        let Some(index) = index else {
            drop(interest);
            continue;
        };
        state.selected = index;
        state.focus = bb_tui::app::Focus::Sidebar;
        let effects = update(&mut state, Action::Key(KeyEvent::from(KeyCode::Enter)));
        drive(&mut state, &client, effects).await;
        if !state.panel_visible {
            let effects = update(&mut state, Action::Key(KeyEvent::from(KeyCode::Char('i'))));
            drive(&mut state, &client, effects).await;
        }
        state.panel.tab = PanelTab::Info;
        let entries = state.info_entries();
        println!(
            "{thread_id}: status {:?}, info rows {}",
            state.panel.status.loaded().map(|status| match status {
                bb_tui::panel::StatusView::Available(_) => "available".to_owned(),
                bb_tui::panel::StatusView::NotApplicable(message)
                | bb_tui::panel::StatusView::Unavailable(message) => message.clone(),
            }),
            entries.len()
        );
        assert!(!state.panel.status.is_failed(), "{:?}", state.panel.status);
        print(&state, &format!("{thread_id} info"));

        let effects = update(&mut state, Action::Key(KeyEvent::from(KeyCode::Char('D'))));
        let skipped = drive(&mut state, &client, effects).await;
        let started = Instant::now();
        if let Some(document) = state.panel.document() {
            let loaded = document
                .files
                .iter()
                .filter(|file| matches!(file.patch, bb_tui::git_diff::PatchState::Loaded(_)))
                .count();
            println!(
                "  diff {}: {} files, {} rows, {loaded} patches inline, {skipped} patch batches not fetched",
                state.panel.scope.label(),
                document.files.len(),
                document.row_count(),
            );
        } else {
            println!("  diff: {:?}", state.panel.diff.is_failed());
        }
        print(&state, &format!("{thread_id} diff"));
        let _ = render_lines(&state, 160, 40);
        println!("  render {:?}", started.elapsed());
        if state
            .panel
            .document()
            .is_some_and(|document| !document.files.is_empty())
        {
            let effects = update(&mut state, Action::Key(KeyEvent::from(KeyCode::Char('p'))));
            drive(&mut state, &client, effects).await;
            if let Some(preview) = &state.panel.preview {
                println!(
                    "  preview {}: {} lines, failed {}",
                    preview.path,
                    preview.line_count(),
                    preview.body.is_failed()
                );
            }
            print(&state, &format!("{thread_id} file"));
            update(&mut state, Action::Key(KeyEvent::from(KeyCode::Esc)));
        }
        update(&mut state, Action::Key(KeyEvent::from(KeyCode::Char('D'))));
        drop(interest);
    }
}

#[tokio::test]
#[ignore = "needs a running bb server; read-only"]
async fn renders_live_notifications_read_only() {
    use bb_tui::notifications::{NoticeKind, describe_interaction};
    use bb_wire::thread::ThreadStatus;

    let found = discover_server(
        &DiscoveryInput::from_env(None),
        discovery::process_is_running,
    )
    .expect("discover");
    let client = BbClient::local(found.endpoint.clone()).expect("client");
    let store = Store::spawn(
        Arc::new(client.clone()),
        NoRealtime,
        None,
        StoreConfig::default(),
    );
    let _interest = store.watch_sidebar();
    let mut rx = store.subscribe();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            {
                let snapshot = rx.borrow_and_update();
                if snapshot.sidebar.data.is_some() && snapshot.attention.data.is_some() {
                    break;
                }
            }
            rx.changed().await.expect("store");
        }
    })
    .await
    .expect("sidebar and attention loaded");
    let real = store.snapshot();
    println!(
        "attention: {:?}",
        real.attention.data.as_ref().map(|a| a.has_attention)
    );
    let Some(target) = real
        .sidebar_threads()
        .find(|entry| {
            entry.thread.archived_at.is_none()
                && entry.thread.parent_thread_id.is_none()
                && entry.thread.status == ThreadStatus::Idle
        })
        .map(|entry| entry.thread.id.clone())
    else {
        println!("no idle thread to simulate");
        return;
    };
    let mut state = AppState::new(
        ServerInfo {
            url: found.endpoint.to_string(),
            profile: None,
        },
        Instant::now(),
        now_ms(),
    );
    let reshape = |status: ThreadStatus, unread: bool| {
        let mut snapshot = (*real).clone();
        let mut sidebar = (**snapshot.sidebar.data.as_ref().unwrap()).clone();
        for project in sidebar
            .projects
            .iter_mut()
            .chain(std::iter::once(&mut sidebar.personal_project))
        {
            for entry in &mut project.threads {
                if entry.thread.id == target {
                    entry.thread.status = status;
                    if unread {
                        entry.thread.last_read_at = Some(0);
                        entry.thread.latest_attention_at = now_ms();
                    }
                }
            }
        }
        snapshot.sidebar.data = Some(Arc::new(sidebar));
        Arc::new(snapshot)
    };
    update(
        &mut state,
        Action::Snapshot(reshape(ThreadStatus::Active, false)),
    );
    update(
        &mut state,
        Action::Snapshot(reshape(ThreadStatus::Idle, true)),
    );
    let later = state.clock + Duration::from_secs(3);
    update(
        &mut state,
        Action::Tick {
            now: later,
            now_ms: now_ms(),
        },
    );
    let notice = state
        .notifications
        .newest_first()
        .next()
        .expect("simulated completion notified")
        .clone();
    assert_eq!(notice.kind, NoticeKind::TurnFinished);
    assert_eq!(notice.thread_id.as_deref(), Some(target.as_str()));
    let waiting: Vec<String> = real
        .sidebar_threads()
        .filter(|entry| entry.has_pending_interaction)
        .map(|entry| entry.thread.id.clone())
        .take(3)
        .collect();
    for thread_id in waiting {
        let interactions = Api::thread_interactions(&client, thread_id.clone())
            .await
            .expect("interactions");
        for interaction in &interactions {
            println!("{thread_id}: {}", describe_interaction(interaction));
        }
    }
    update(
        &mut state,
        Action::Key(crossterm::event::KeyEvent::from(
            crossterm::event::KeyCode::Char('!'),
        )),
    );
    let screen = render_lines(&state, 120, 20).join("\n");
    assert!(screen.contains("Notifications (1)"), "{screen}");
    if std::env::var_os("BB_TUI_LIVE_PRINT").is_some() {
        println!("{screen}");
    }
}

#[tokio::test]
#[ignore = "needs a running bb server; read-only GET requests only"]
async fn lists_live_thread_terminals_read_only() {
    let found = discover_server(
        &DiscoveryInput::from_env(None),
        discovery::process_is_running,
    )
    .expect("discover");
    let client = BbClient::local(found.endpoint.clone()).expect("client");
    let store = Store::spawn(
        Arc::new(client.clone()),
        NoRealtime,
        None,
        StoreConfig::default(),
    );
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
    state.read_only = true;
    update(&mut state, Action::Snapshot(store.snapshot()));
    let thread_id = store
        .snapshot()
        .sidebar_threads()
        .find(|entry| entry.thread.environment_id.is_some())
        .map(|entry| entry.thread.id.clone())
        .expect("a thread with an environment");
    state.open(thread_id.clone());
    let effects = update(
        &mut state,
        Action::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('T'),
            crossterm::event::KeyModifiers::SHIFT,
        )),
    );
    let scope = effects
        .iter()
        .find_map(|effect| match effect {
            bb_tui::app::Effect::Terminal(bb_tui::terminal::TerminalRequest::List { scope }) => {
                Some(scope.clone())
            }
            _ => None,
        })
        .expect("list request");
    assert_eq!(scope.thread_id, thread_id);
    let result = bb_tui::runtime::list_terminals(&client, &scope).await;
    let sessions = result.clone().expect("terminals listed");
    update(
        &mut state,
        Action::TerminalLoaded(bb_tui::terminal::TerminalLoaded::List { scope, result }),
    );
    let screen = render_lines(&state, 140, 24).join("\n");
    if sessions.iter().any(bb_tui::terminal::is_listed) {
        assert!(screen.contains("Terminal"), "{screen}");
    } else {
        assert!(screen.contains("No terminals for this thread"), "{screen}");
    }
    if std::env::var_os("BB_TUI_LIVE_PRINT").is_some() {
        println!("{screen}");
    }
}

async fn run_live_settings(
    client: &BbClient,
    state: &mut AppState,
    effects: Vec<bb_tui::app::Effect>,
) {
    let mut pending = effects;
    for _ in 0..6 {
        let mut next = Vec::new();
        for effect in pending {
            if let bb_tui::app::Effect::Settings(request) = effect {
                assert!(
                    !matches!(request, bb_tui::settings::SettingsRequest::Mutate(_)),
                    "settings tried to change the server: {request:?}"
                );
                let loaded = bb_tui::runtime::run_settings_request(client, request).await;
                next.extend(update(state, Action::SettingsLoaded(loaded)));
            }
        }
        if next.is_empty() {
            break;
        }
        pending = next;
    }
}

fn live_key(code: crossterm::event::KeyCode) -> Action {
    Action::Key(crossterm::event::KeyEvent::from(code))
}

#[tokio::test]
#[ignore = "needs a running bb server; read-only GET requests only"]
async fn renders_live_settings_read_only() {
    use crossterm::event::KeyCode;
    let found = discover_server(
        &DiscoveryInput::from_env(None),
        discovery::process_is_running,
    )
    .expect("discover");
    let client = BbClient::local(found.endpoint.clone()).expect("client");
    let store = Store::spawn(
        Arc::new(client.clone()),
        NoRealtime,
        None,
        StoreConfig::default(),
    );
    let _interest = store.watch_sidebar();
    let mut rx = store.subscribe();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let snapshot = rx.borrow_and_update().clone();
            if snapshot.sidebar.data.is_some() && snapshot.hosts.data.is_some() {
                break;
            }
            rx.changed().await.expect("store");
        }
    })
    .await
    .expect("sidebar and hosts loaded");
    let mut state = AppState::new(
        ServerInfo {
            url: found.endpoint.to_string(),
            profile: None,
        },
        Instant::now(),
        now_ms(),
    );
    state.read_only = true;
    update(&mut state, Action::Snapshot(store.snapshot()));
    let effects = update(&mut state, live_key(KeyCode::Char(',')));
    run_live_settings(&client, &mut state, effects).await;
    let print = std::env::var_os("BB_TUI_LIVE_PRINT").is_some();
    let mut screens = Vec::new();
    let view = state.settings.as_ref().expect("settings open");
    assert!(view.config.loaded().is_some(), "{:?}", view.config);
    assert!(view.catalog.loaded().is_some(), "{:?}", view.catalog);
    assert!(view.plugins.loaded().is_some(), "{:?}", view.plugins);
    screens.push(render_lines(&state, 140, 40).join("\n"));
    assert!(
        screens[0].contains("Default thread followup behavior"),
        "{}",
        screens[0]
    );

    let refused = update(&mut state, live_key(KeyCode::Char(' ')));
    assert!(refused.is_empty(), "{refused:?}");

    for (digit, label) in [
        ('2', "Providers"),
        ('3', "Keyboard"),
        ('4', "Projects"),
        ('5', "Machines"),
        ('6', "Installed plugins"),
        ('7', "Archived threads"),
    ] {
        update(&mut state, live_key(KeyCode::Esc));
        let effects = update(&mut state, live_key(KeyCode::Char(digit)));
        run_live_settings(&client, &mut state, effects).await;
        update(&mut state, live_key(KeyCode::Enter));
        if digit == '5' || digit == '6' {
            let effects = update(&mut state, live_key(KeyCode::Enter));
            run_live_settings(&client, &mut state, effects).await;
        }
        let screen = render_lines(&state, 140, 40).join("\n");
        assert!(screen.contains(label), "{screen}");
        assert!(!screen.contains("Couldn't load"), "{screen}");
        screens.push(screen);
        if digit == '5' || digit == '6' {
            update(&mut state, live_key(KeyCode::Esc));
        }
    }
    let archived = &state.settings.as_ref().expect("settings").archived;
    assert!(archived.loaded && archived.error.is_none(), "{archived:?}");
    if print {
        for screen in screens {
            println!("{screen}\n");
        }
    }
}
