mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use bb_store::{NoRealtime, Store, StoreConfig, StoreSnapshot};
use bb_tui::api::Mutation;
use bb_tui::app::{Action, AppState, ServerInfo};
use bb_tui::runtime::run;
use bb_wire::system::{AppKeybindingOverride, AppShortcut};
use bb_wire::timeline::TurnSummaryDetailsResponse;
use crossterm::event::{KeyCode, KeyEvent};
use futures::stream;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use support::*;
use tokio::sync::mpsc;

async fn wait_for(store: &Store, predicate: impl Fn(&StoreSnapshot) -> bool) {
    let mut rx = store.subscribe();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if predicate(&rx.borrow_and_update()) {
                return;
            }
            rx.changed().await.expect("store alive");
        }
    })
    .await
    .expect("store condition");
}

async fn wait_until(predicate: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("condition");
}

fn key(tx: &mpsc::UnboundedSender<Action>, code: KeyCode) {
    tx.send(Action::Key(KeyEvent::from(code)))
        .expect("send key");
}

fn keys(tx: &mpsc::UnboundedSender<Action>, text: &str) {
    for chord in bb_tui::keymap::seq(text) {
        tx.send(Action::Key(chord.event())).expect("send key");
    }
}

#[tokio::test]
async fn event_loop_applies_mutations_through_api_and_refreshes_sidebar() {
    let backend = FakeBackend::new(sample_sidebar());
    let mut api = MockApi::new(backend.clone());
    Arc::get_mut(&mut api).unwrap().overrides = vec![AppKeybindingOverride {
        command: "thread.archive".into(),
        shortcut: Some(AppShortcut {
            key: "x".into(),
            ..AppShortcut::default()
        }),
        platform: None,
    }];
    let store = Store::spawn(backend.clone(), NoRealtime, None, StoreConfig::default());
    let (tx, mut rx) = mpsc::unbounded_channel::<Action>();
    let events = Box::pin(stream::poll_fn(move |cx| rx.poll_recv(cx)));
    let state = AppState::new(
        ServerInfo {
            url: "http://fake/".into(),
            profile: Some("office".into()),
        },
        Instant::now(),
        NOW_MS,
    );
    let task_store = store.clone();
    let task_api = api.clone();
    let handle = tokio::spawn(async move {
        let mut terminal = Terminal::new(TestBackend::new(100, 14)).unwrap();
        let mut state = state;
        run(
            &mut terminal,
            &mut state,
            task_store,
            task_api,
            Arc::new(support::MockPlatform::default()),
            events,
        )
        .await
        .expect("run");
        (terminal, state)
    });

    wait_for(&store, |snapshot| snapshot.sidebar.data.is_some()).await;
    key(&tx, KeyCode::Char('+'));
    wait_for(&store, |snapshot| {
        snapshot
            .sidebar_thread("thr_wait")
            .is_some_and(|entry| entry.thread.is_pinned())
    })
    .await;

    key(&tx, KeyCode::Char('j'));
    key(&tx, KeyCode::Char('j'));
    key(&tx, KeyCode::Enter);
    wait_for(&store, |snapshot| {
        snapshot.sidebar_thread("thr_unread").is_some_and(|entry| {
            entry.thread.last_read_at == Some(entry.thread.latest_attention_at)
        })
    })
    .await;

    key(&tx, KeyCode::Esc);
    key(&tx, KeyCode::Char('x'));
    wait_for(&store, |snapshot| {
        snapshot
            .sidebar_thread("thr_unread")
            .is_some_and(|entry| entry.thread.is_archived())
    })
    .await;
    let mutations_api = api.clone();
    wait_until(move || mutations_api.recorded().len() == 3).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    keys(&tx, "SPC q q");

    let (terminal, state) = tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("loop exits")
        .expect("join");
    assert_eq!(
        api.recorded(),
        vec![
            Mutation::Pin {
                thread_id: "thr_wait".into()
            },
            Mutation::MarkRead {
                thread_id: "thr_unread".into()
            },
            Mutation::Archive {
                thread_id: "thr_unread".into()
            },
        ]
    );
    assert!(*backend.sidebar_fetches.lock().unwrap() >= 4);
    assert_eq!(state.open_thread.as_deref(), Some("thr_unread"));
    let lines = buffer_lines(&terminal);
    let text = lines.join("\n");
    assert!(lines[1].contains("▾ Pinned"), "{text}");
    assert!(lines[2].contains("Needs approval"), "{text}");
    assert!(
        !lines[..10]
            .iter()
            .any(|line| line.contains("┃ ● Fix flaky")),
        "{text}"
    );
    assert!(
        text.contains("Archived (u to undo): Fix flaky test"),
        "{text}"
    );
    assert!(lines[lines.len() - 2].starts_with(" office "), "{text}");
}

#[tokio::test]
async fn timeline_loads_turn_details_older_rows_and_copies_through_the_store() {
    let backend = FakeBackend::new(sample_sidebar());
    let mut rows = vec![turn("t1", "turn-1", 101, 140)];
    for index in 0..30 {
        rows.push(assistant(
            &format!("a{index}"),
            200 + index,
            &format!("Reply number {index}"),
        ));
    }
    backend
        .timelines
        .lock()
        .unwrap()
        .insert(THREAD.into(), timeline_response(rows, true));
    backend.older_pages.lock().unwrap().insert(
        THREAD.into(),
        timeline_response(vec![user("old", 50, "The very first question")], false),
    );
    backend.turn_details.lock().unwrap().insert(
        "turn-1".into(),
        TurnSummaryDetailsResponse {
            rows: vec![command("t1-c1", 102, "cargo fmt", "")],
            ..TurnSummaryDetailsResponse::default()
        },
    );
    let api = MockApi::new(backend.clone());
    let platform = Arc::new(MockPlatform::default());
    let store = Store::spawn(backend.clone(), NoRealtime, None, StoreConfig::default());
    let (tx, mut rx) = mpsc::unbounded_channel::<Action>();
    let events = Box::pin(stream::poll_fn(move |cx| rx.poll_recv(cx)));
    let mut state = AppState::new(
        ServerInfo {
            url: "http://fake/".into(),
            profile: None,
        },
        Instant::now(),
        NOW_MS,
    );
    state.read_only = true;
    let task_store = store.clone();
    let task_platform = platform.clone();
    let handle = tokio::spawn(async move {
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let mut state = state;
        run(
            &mut terminal,
            &mut state,
            task_store,
            api,
            task_platform,
            events,
        )
        .await
        .expect("run");
        (terminal, state)
    });

    wait_for(&store, |snapshot| snapshot.sidebar.data.is_some()).await;
    keys(&tx, "g o");
    wait_for(&store, |snapshot| {
        snapshot
            .timeline(THREAD)
            .is_some_and(|entry| entry.rows().len() == 31)
    })
    .await;

    keys(&tx, "g g");
    wait_for(&store, |snapshot| {
        snapshot
            .timeline(THREAD)
            .is_some_and(|entry| entry.rows().len() == 32)
    })
    .await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    keys(&tx, "g g j RET");
    wait_for(&store, |snapshot| {
        snapshot
            .turn_details
            .values()
            .any(|entry| entry.loaded && entry.rows.len() == 1)
    })
    .await;
    keys(&tx, "k g y");
    let copied = platform.clone();
    wait_until(move || !copied.copied.lock().unwrap().is_empty()).await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    keys(&tx, "SPC q q");
    let (terminal, state) = tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("loop exits")
        .expect("join");

    assert_eq!(
        platform.copied.lock().unwrap().as_slice(),
        ["The very first question"]
    );
    let text = buffer_lines(&terminal).join("\n");
    assert!(text.contains("❯ The very first question"), "{text}");
    assert!(text.contains("▾ ◦ Worked for"), "{text}");
    assert!(text.contains("$ Ran cargo fmt"), "{text}");
    assert!(text.contains("Copied 23 characters"), "{text}");
    let queries = backend.timeline_queries.lock().unwrap().clone();
    assert!(
        queries
            .iter()
            .any(|query| query.before_anchor_id.as_deref() == Some("anchor"))
    );
    let details = backend.details_queries.lock().unwrap().clone();
    assert_eq!(details.len(), 1);
    assert_eq!(details[0].turn_id, "turn-1");
    assert_eq!(state.open_thread.as_deref(), Some(THREAD));
}

fn chord(
    tx: &mpsc::UnboundedSender<Action>,
    code: KeyCode,
    modifiers: crossterm::event::KeyModifiers,
) {
    tx.send(Action::Key(KeyEvent::new(code, modifiers)))
        .expect("send key");
}

fn type_text(tx: &mpsc::UnboundedSender<Action>, text: &str) {
    for ch in text.chars() {
        key(tx, KeyCode::Char(ch));
    }
}

#[tokio::test]
async fn composer_queues_sends_queued_saves_drafts_and_stops_through_the_loop() {
    use bb_wire::prompt::{
        PermissionMode, PromptInput, ReasoningLevel, ResolvedThreadExecutionOptions,
    };
    use bb_wire::queue::SendQueuedMessageMode;
    use bb_wire::thread::{ThreadDisplayStatus, ThreadStatus};
    use crossterm::event::KeyModifiers;

    let backend = FakeBackend::new(sample_sidebar());
    backend.patch_thread("thr_other", |entry| {
        entry.thread.status = ThreadStatus::Active;
        entry.runtime.display_status = ThreadDisplayStatus::Active;
    });
    let mut api = MockApi::new(backend.clone());
    Arc::get_mut(&mut api).unwrap().defaults = Some(ResolvedThreadExecutionOptions {
        model: "gpt-5".into(),
        reasoning_level: ReasoningLevel::Medium,
        permission_mode: PermissionMode::Auto,
        ..ResolvedThreadExecutionOptions::default()
    });
    let platform = Arc::new(MockPlatform::default());
    let store = Store::spawn(backend.clone(), NoRealtime, None, StoreConfig::default());
    let (tx, mut rx) = mpsc::unbounded_channel::<Action>();
    let events = Box::pin(stream::poll_fn(move |cx| rx.poll_recv(cx)));
    let state = AppState::new(
        ServerInfo {
            url: "http://fake/".into(),
            profile: None,
        },
        Instant::now(),
        NOW_MS,
    );
    let task_store = store.clone();
    let task_api = api.clone();
    let task_platform = platform.clone();
    let handle = tokio::spawn(async move {
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        let mut state = state;
        run(
            &mut terminal,
            &mut state,
            task_store,
            task_api,
            task_platform,
            events,
        )
        .await
        .expect("run");
        (terminal, state)
    });

    wait_for(&store, |snapshot| snapshot.sidebar.data.is_some()).await;
    keys(&tx, "g /");
    type_text(&tx, "Refactor");
    key(&tx, KeyCode::Enter);
    keys(&tx, "g o");
    let execution = api.clone();
    wait_until(move || !execution.execution_queries.lock().unwrap().is_empty()).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    key(&tx, KeyCode::Char('i'));
    type_text(&tx, "queue me");
    chord(&tx, KeyCode::Enter, KeyModifiers::CONTROL);
    wait_for(&store, |snapshot| snapshot.queued("thr_other").len() == 1).await;

    chord(&tx, KeyCode::Char('q'), KeyModifiers::ALT);
    key(&tx, KeyCode::Char('s'));
    wait_for(&store, |snapshot| {
        snapshot
            .queued_messages
            .get("thr_other")
            .and_then(|resource| resource.data.as_deref())
            .is_some_and(Vec::is_empty)
    })
    .await;

    type_text(&tx, "draft text");
    let drafts = platform.clone();
    wait_until(move || {
        drafts
            .drafts
            .lock()
            .unwrap()
            .get("thr_other")
            .is_some_and(|raw| raw.contains("draft text"))
    })
    .await;
    keys(&tx, "ESC d d C-c C-k");
    let recorded = api.clone();
    wait_until(move || {
        recorded
            .recorded()
            .iter()
            .any(|mutation| matches!(mutation, Mutation::Stop { .. }))
    })
    .await;
    keys(&tx, "SPC q q");

    let (_terminal, state) = tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("loop exits")
        .expect("join");
    let recorded = api.recorded();
    let queued = recorded
        .iter()
        .find_map(|mutation| match mutation {
            Mutation::QueueMessage { thread_id, request } => Some((thread_id, request)),
            _ => None,
        })
        .expect("queued");
    assert_eq!(queued.0, "thr_other");
    assert_eq!(queued.1.input, vec![PromptInput::text("queue me")]);
    assert_eq!(queued.1.model.as_deref(), Some("gpt-5"));
    assert!(recorded.contains(&Mutation::SendQueued {
        thread_id: "thr_other".into(),
        queued_message_id: "q1".into(),
        mode: SendQueuedMessageMode::Auto,
    }));
    assert!(recorded.contains(&Mutation::Stop {
        thread_id: "thr_other".into()
    }));
    assert!(state.quit);
    assert!(!platform.drafts.lock().unwrap().contains_key("thr_other"));
}

#[tokio::test]
async fn event_loop_creates_a_thread_from_the_new_thread_form_and_opens_it() {
    use bb_tui::new_thread::CreationConfig;
    use bb_wire::environment::{Host, HostStatus, HostType};
    use bb_wire::project::ProjectSource;
    use bb_wire::system::{EnvironmentProviderRequirements, SystemEnvironmentProvider};

    let mut sidebar = sample_sidebar();
    sidebar.projects[0].sources = vec![ProjectSource {
        id: "src_1".into(),
        project_id: "proj_a".into(),
        source_type: "local_path".into(),
        host_id: "host_1".into(),
        path: "/work/alpha".into(),
        ..ProjectSource::default()
    }];
    let backend = FakeBackend::new(sidebar);
    *backend.hosts.lock().unwrap() = vec![Host {
        id: "host_1".into(),
        name: "laptop".into(),
        host_type: HostType::Persistent,
        status: HostStatus::Connected,
        ..Host::default()
    }];
    let mut api = MockApi::new(backend.clone());
    let mock = Arc::get_mut(&mut api).unwrap();
    mock.environment_providers = vec![SystemEnvironmentProvider {
        id: "project-checkout".into(),
        display_name: "Project checkout".into(),
        plugin_id: "project-checkout".into(),
        accepts_empty_inputs: true,
        inputs: Some(serde_json::json!({"type": "object"})),
        requires: EnvironmentProviderRequirements {
            project_checkout: true,
            ..EnvironmentProviderRequirements::default()
        },
        ..SystemEnvironmentProvider::default()
    }];
    mock.creation_config = CreationConfig {
        primary_host_id: Some("host_1".into()),
        allow_fast_service_tier: true,
    };
    mock.execution_options = bb_wire::system::SystemExecutionOptionsResponse {
        providers: vec![bb_wire::system::ProviderInfo {
            id: "codex".into(),
            display_name: "Codex".into(),
            ..Default::default()
        }],
        models: vec![bb_wire::system::AvailableModel {
            id: "gpt-5".into(),
            model: "gpt-5".into(),
            is_default: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    mock.branches = bb_wire::project::ProjectBranchesResponse {
        checkout: bb_wire::environment::GitCheckoutRef::Branch {
            branch_name: "main".into(),
            head_sha: None,
        },
        branches: vec!["main".into()],
        ..Default::default()
    };
    let store = Store::spawn(backend.clone(), NoRealtime, None, StoreConfig::default());
    let (tx, mut rx) = mpsc::unbounded_channel::<Action>();
    let events = Box::pin(stream::poll_fn(move |cx| rx.poll_recv(cx)));
    let state = AppState::new(
        ServerInfo {
            url: "http://fake/".into(),
            profile: None,
        },
        Instant::now(),
        NOW_MS,
    );
    let task_store = store.clone();
    let task_api = api.clone();
    let handle = tokio::spawn(async move {
        let mut terminal = Terminal::new(TestBackend::new(110, 30)).unwrap();
        let mut state = state;
        run(
            &mut terminal,
            &mut state,
            task_store,
            task_api,
            Arc::new(support::MockPlatform::default()),
            events,
        )
        .await
        .expect("run");
        (terminal, state)
    });

    wait_for(&store, |snapshot| {
        snapshot.sidebar.data.is_some() && snapshot.hosts.data.is_some()
    })
    .await;
    keys(&tx, "SPC b N");
    let ready_api = api.clone();
    wait_until(move || !ready_api.execution_queries.lock().unwrap().is_empty()).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    key(&tx, KeyCode::Char('i'));
    type_text(&tx, "Ship it");
    chord(&tx, KeyCode::Enter, crossterm::event::KeyModifiers::CONTROL);
    let created = api.clone();
    wait_until(move || !created.created.lock().unwrap().is_empty()).await;
    wait_for(&store, |snapshot| {
        snapshot.sidebar_thread("thr_new1").is_some()
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    keys(&tx, "ESC SPC q q");

    let (terminal, state) = tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("loop exits")
        .expect("join");
    let request = api.created.lock().unwrap()[0].clone();
    assert_eq!(request.project_id, "proj_a");
    assert_eq!(request.model.as_deref(), Some("gpt-5"));
    assert_eq!(
        serde_json::to_value(&request.environment).unwrap(),
        serde_json::json!({
            "type": "provider",
            "environmentProviderId": "project-checkout",
            "machine": {"type": "existing", "hostId": "host_1"},
            "inputs": {}
        })
    );
    assert_eq!(state.open_thread.as_deref(), Some("thr_new1"));
    assert!(state.new_thread.is_none());
    let screen = buffer_lines(&terminal).join("\n");
    assert!(screen.contains("Created thread"), "{screen}");
}

#[tokio::test]
async fn palette_debounces_thread_search_and_runs_plugin_commands_through_api() {
    let backend = FakeBackend::new(sample_sidebar());
    let mut api = MockApi::new(backend.clone());
    {
        let api = Arc::get_mut(&mut api).unwrap();
        api.contributions = vec![bb_wire::plugin::PluginCliContribution {
            plugin_id: "keep-awake".into(),
            name: "keep-awake".into(),
            summary: "Configure idle-sleep prevention".into(),
            commands: vec![bb_wire::plugin::PluginCliCommandInfo {
                name: "status".into(),
                summary: "Show status".into(),
                usage: "bb keep-awake status [--json]".into(),
            }],
            renders_help: true,
        }];
        api.cli_result = bb_wire::plugin::PluginCliResult {
            exit_code: 0,
            stdout: "keep-awake: enabled\n".into(),
            ..Default::default()
        };
    }
    let store = Store::spawn(backend.clone(), NoRealtime, None, StoreConfig::default());
    let (tx, mut rx) = mpsc::unbounded_channel::<Action>();
    let events = Box::pin(stream::poll_fn(move |cx| rx.poll_recv(cx)));
    let state = AppState::new(
        ServerInfo {
            url: "http://fake/".into(),
            profile: None,
        },
        Instant::now(),
        NOW_MS,
    );
    let task_store = store.clone();
    let task_api = api.clone();
    let handle = tokio::spawn(async move {
        let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
        let mut state = state;
        run(
            &mut terminal,
            &mut state,
            task_store,
            task_api,
            Arc::new(support::MockPlatform::default()),
            events,
        )
        .await
        .expect("run");
        (terminal, state)
    });
    wait_for(&store, |snapshot| snapshot.sidebar.data.is_some()).await;

    keys(&tx, "SPC /");
    for ch in "flaky".chars() {
        key(&tx, KeyCode::Char(ch));
    }
    let search_api = api.clone();
    wait_until(move || !search_api.search_queries.lock().unwrap().is_empty()).await;
    tokio::time::sleep(Duration::from_millis(250)).await;
    let queries: Vec<String> = api
        .search_queries
        .lock()
        .unwrap()
        .iter()
        .map(|query| query.query.clone())
        .collect();
    assert_eq!(queries, ["flaky"]);
    key(&tx, KeyCode::Esc);

    keys(&tx, "M-x");
    tokio::time::sleep(Duration::from_millis(50)).await;
    for ch in "keep status".chars() {
        key(&tx, KeyCode::Char(ch));
    }
    key(&tx, KeyCode::Enter);
    key(&tx, KeyCode::Enter);
    let cli_api = api.clone();
    wait_until(move || !cli_api.cli_runs.lock().unwrap().is_empty()).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    key(&tx, KeyCode::Char('j'));
    tokio::time::sleep(Duration::from_millis(50)).await;
    tx.send(Action::Resize).unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    keys(&tx, "ESC SPC q q");
    let (_terminal, state) = tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("loop exits")
        .expect("join");
    let runs = api.cli_runs.lock().unwrap().clone();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].0, "keep-awake");
    assert_eq!(runs[0].1.argv, ["status"]);
    assert!(state.result_pane.is_none());
}

#[tokio::test]
async fn cursor_shape_follows_the_evil_state_and_resets_on_exit() {
    use bb_tui::platform::CursorShape;
    let backend = FakeBackend::new(sample_sidebar());
    let api = MockApi::new(backend.clone());
    let store = Store::spawn(backend.clone(), NoRealtime, None, StoreConfig::default());
    let (tx, mut rx) = mpsc::unbounded_channel::<Action>();
    let events = Box::pin(stream::poll_fn(move |cx| rx.poll_recv(cx)));
    let platform = Arc::new(support::MockPlatform::default());
    let state = AppState::new(
        ServerInfo {
            url: "http://fake/".into(),
            profile: None,
        },
        Instant::now(),
        NOW_MS,
    );
    let task_store = store.clone();
    let task_platform = platform.clone();
    let handle = tokio::spawn(async move {
        let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
        let mut state = state;
        run(
            &mut terminal,
            &mut state,
            task_store,
            api,
            task_platform,
            events,
        )
        .await
        .expect("run");
    });
    wait_for(&store, |snapshot| snapshot.sidebar.data.is_some()).await;
    let shapes = platform.clone();
    wait_until(move || !shapes.cursor_shapes.lock().unwrap().is_empty()).await;
    keys(&tx, ":");
    let shapes = platform.clone();
    wait_until(move || shapes.cursor_shapes.lock().unwrap().last() == Some(&CursorShape::Bar))
        .await;
    keys(&tx, "ESC");
    let shapes = platform.clone();
    wait_until(move || shapes.cursor_shapes.lock().unwrap().last() == Some(&CursorShape::Block))
        .await;
    keys(&tx, "SPC q q");
    tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("loop exits")
        .expect("join");
    let shapes = platform.cursor_shapes.lock().unwrap().clone();
    assert_eq!(shapes.first(), Some(&CursorShape::Block));
    assert_eq!(shapes.last(), Some(&CursorShape::Default));
    assert!(
        shapes.windows(2).all(|pair| pair[0] != pair[1]),
        "{shapes:?}"
    );
}
