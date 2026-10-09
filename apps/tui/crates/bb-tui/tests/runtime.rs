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
    key(&tx, KeyCode::Char('p'));
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
    key(&tx, KeyCode::Char('q'));

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
    assert!(lines.last().unwrap().starts_with(" office "), "{text}");
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
    key(&tx, KeyCode::Enter);
    wait_for(&store, |snapshot| {
        snapshot
            .timeline(THREAD)
            .is_some_and(|entry| entry.rows().len() == 31)
    })
    .await;

    key(&tx, KeyCode::Char('g'));
    wait_for(&store, |snapshot| {
        snapshot
            .timeline(THREAD)
            .is_some_and(|entry| entry.rows().len() == 32)
    })
    .await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    key(&tx, KeyCode::Char('g'));
    key(&tx, KeyCode::Char('j'));
    key(&tx, KeyCode::Enter);
    wait_for(&store, |snapshot| {
        snapshot
            .turn_details
            .values()
            .any(|entry| entry.loaded && entry.rows.len() == 1)
    })
    .await;
    key(&tx, KeyCode::Char('k'));
    key(&tx, KeyCode::Char('y'));
    let copied = platform.clone();
    wait_until(move || !copied.copied.lock().unwrap().is_empty()).await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    key(&tx, KeyCode::Char('q'));
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
