mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use bb_store::{NoRealtime, Store, StoreConfig};
use bb_tui::app::{Action, AppState, Effect, Focus, Mode, ServerInfo, ToastLevel, update};
use bb_tui::config::{DesktopMethod, NotificationPrefs};
use bb_tui::notifications::{Alert, DesktopNotice, NoticeKind, Notifications};
use bb_tui::runtime::run;
use bb_wire::interaction::{
    ApprovalPayload, ApprovalSubject, InteractionPayload, PendingInteraction,
    PendingInteractionStatus,
};
use bb_wire::project::SidebarBootstrapResponse;
use bb_wire::system::SystemAttentionResponse;
use bb_wire::thread::{ThreadListEntry, ThreadStatus};
use crossterm::event::{KeyCode, KeyEvent};
use futures::stream;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use serde_json::json;
use support::*;
use tokio::sync::mpsc;

const BACKGROUND: &str = "thr_other";
const FOCUSED: &str = "thr_old";

fn running_sidebar() -> SidebarBootstrapResponse {
    let mut sidebar = sample_sidebar();
    patch(&mut sidebar, BACKGROUND, |entry| {
        entry.thread.status = ThreadStatus::Active;
    });
    sidebar
}

fn patch(
    sidebar: &mut SidebarBootstrapResponse,
    thread_id: &str,
    apply: impl Fn(&mut ThreadListEntry),
) {
    for project in sidebar
        .projects
        .iter_mut()
        .chain(std::iter::once(&mut sidebar.personal_project))
    {
        for entry in &mut project.threads {
            if entry.thread.id == thread_id {
                apply(entry);
            }
        }
    }
}

fn finished(sidebar: &SidebarBootstrapResponse, thread_id: &str) -> SidebarBootstrapResponse {
    let mut next = sidebar.clone();
    patch(&mut next, thread_id, |entry| {
        entry.thread.status = ThreadStatus::Idle;
        entry.thread.latest_attention_at = NOW_MS + 1_000;
    });
    next
}

fn state_with(prefs: NotificationPrefs) -> AppState {
    let mut state = loaded_state(running_sidebar());
    state.notifications = Notifications::new(prefs);
    state
}

fn bell_only() -> NotificationPrefs {
    NotificationPrefs::default()
}

fn osc9() -> NotificationPrefs {
    NotificationPrefs {
        desktop: DesktopMethod::Osc9,
        ..NotificationPrefs::default()
    }
}

fn apply_snapshot(state: &mut AppState, sidebar: SidebarBootstrapResponse) -> Vec<Effect> {
    update(state, Action::Snapshot(snapshot_with(sidebar)))
}

fn advance(state: &mut AppState, by: Duration) -> Vec<Effect> {
    let now = state.clock + by;
    let now_ms = state.now_ms + by.as_millis() as i64;
    update(state, Action::Tick { now, now_ms })
}

fn alerts(effects: &[Effect]) -> Vec<Alert> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Alert(alert) => Some(alert.clone()),
            _ => None,
        })
        .collect()
}

fn toasts(state: &AppState) -> Vec<(String, ToastLevel)> {
    state
        .toasts
        .iter()
        .map(|toast| (toast.message.clone(), toast.level))
        .collect()
}

fn command_approval(thread_id: &str, command: &str) -> PendingInteraction {
    PendingInteraction {
        id: "int_1".into(),
        thread_id: thread_id.into(),
        status: PendingInteractionStatus::Pending,
        payload: InteractionPayload::Approval(ApprovalPayload {
            subject: ApprovalSubject::Command {
                item_id: "item".into(),
                command: command.into(),
                cwd: None,
                actions: Vec::new(),
                session_grant: None,
            },
            ..ApprovalPayload::default()
        }),
        ..PendingInteraction::default()
    }
}

#[test]
fn background_turn_completion_notifies_once_after_coalescing() {
    let mut state = state_with(osc9());
    update(&mut state, Action::TerminalFocus(false));
    let sidebar = running_sidebar();
    let effects = apply_snapshot(&mut state, finished(&sidebar, BACKGROUND));
    assert!(alerts(&effects).is_empty());
    assert!(alerts(&advance(&mut state, Duration::from_millis(1_500))).is_empty());
    let effects = advance(&mut state, Duration::from_millis(600));
    assert_eq!(
        alerts(&effects),
        [Alert {
            bell: true,
            desktop: Some(DesktopNotice {
                method: DesktopMethod::Osc9,
                title: "Refactor parser".into(),
                body: "Finished and waiting for you".into(),
            }),
        }]
    );
    assert_eq!(
        toasts(&state),
        [(
            "✓ Refactor parser: Finished and waiting for you".to_owned(),
            ToastLevel::Info
        )]
    );
    assert_eq!(state.notifications.unread(), 1);
    assert!(alerts(&advance(&mut state, Duration::from_secs(5))).is_empty());
    let lines = render_lines(&state, 120, 12);
    let status = lines[lines.len() - 2].clone();
    assert!(status.contains("! 1 new"), "{status}");
}

#[test]
fn focused_thread_is_silent_until_the_terminal_loses_focus() {
    let mut state = state_with(bell_only());
    state.open(BACKGROUND.into());
    let sidebar = running_sidebar();
    apply_snapshot(&mut state, finished(&sidebar, BACKGROUND));
    assert!(alerts(&advance(&mut state, Duration::from_secs(3))).is_empty());
    assert!(state.notifications.is_empty());

    let mut state = state_with(bell_only());
    state.open(BACKGROUND.into());
    update(&mut state, Action::TerminalFocus(false));
    apply_snapshot(&mut state, finished(&sidebar, BACKGROUND));
    assert_eq!(
        alerts(&advance(&mut state, Duration::from_secs(3))).len(),
        1
    );
}

#[test]
fn desktop_notice_is_skipped_while_the_terminal_is_focused() {
    let mut state = state_with(osc9());
    state.open(FOCUSED.into());
    update(&mut state, Action::TerminalFocus(true));
    apply_snapshot(&mut state, finished(&running_sidebar(), BACKGROUND));
    assert_eq!(
        alerts(&advance(&mut state, Duration::from_secs(3))),
        [Alert {
            bell: true,
            desktop: None
        }]
    );
}

#[test]
fn reading_the_thread_elsewhere_before_the_flush_cancels_it() {
    let mut state = state_with(bell_only());
    let sidebar = running_sidebar();
    let done = finished(&sidebar, BACKGROUND);
    apply_snapshot(&mut state, done.clone());
    let mut read = done;
    patch(&mut read, BACKGROUND, |entry| {
        entry.thread.last_read_at = Some(NOW_MS + 5_000);
    });
    apply_snapshot(&mut state, read);
    assert!(alerts(&advance(&mut state, Duration::from_secs(3))).is_empty());
    assert!(state.notifications.is_empty());
}

#[test]
fn needs_input_outranks_completion_and_describes_the_request() {
    let mut state = state_with(bell_only());
    let sidebar = running_sidebar();
    let mut next = finished(&sidebar, BACKGROUND);
    patch(&mut next, BACKGROUND, |entry| {
        entry.has_pending_interaction = true;
    });
    apply_snapshot(&mut state, next);
    let effects = advance(&mut state, Duration::from_secs(3));
    assert_eq!(
        effects,
        [Effect::FetchNotificationInteractions(BACKGROUND.into())]
    );
    let effects = update(
        &mut state,
        Action::NotificationInteractions {
            thread_id: BACKGROUND.into(),
            result: Ok(vec![command_approval(BACKGROUND, "cargo test\n--all")]),
        },
    );
    assert_eq!(alerts(&effects).len(), 1);
    let notice = state.notifications.newest_first().next().unwrap().clone();
    assert_eq!(notice.kind, NoticeKind::NeedsInput);
    assert_eq!(notice.body, "Approve command: cargo test");
    assert_eq!(state.notifications.len(), 1);
    assert_eq!(toasts(&state)[0].1, ToastLevel::Attention);
}

#[test]
fn errors_and_disabled_kinds() {
    let mut state = state_with(NotificationPrefs {
        turn_finished: false,
        ..bell_only()
    });
    let sidebar = running_sidebar();
    apply_snapshot(&mut state, finished(&sidebar, BACKGROUND));
    assert!(alerts(&advance(&mut state, Duration::from_secs(3))).is_empty());

    let mut failed = sidebar.clone();
    patch(&mut failed, BACKGROUND, |entry| {
        entry.thread.status = ThreadStatus::Error;
        entry.thread.latest_attention_at = NOW_MS + 1_000;
    });
    let mut state = state_with(NotificationPrefs {
        toast: false,
        bell: false,
        ..bell_only()
    });
    apply_snapshot(&mut state, failed.clone());
    assert!(alerts(&advance(&mut state, Duration::from_secs(3))).is_empty());
    assert!(state.toasts.is_empty());
    let notice = state.notifications.newest_first().next().unwrap();
    assert_eq!(
        (notice.kind, notice.body.as_str()),
        (NoticeKind::Error, "The thread hit an error")
    );

    let mut state = state_with(NotificationPrefs {
        enabled: false,
        ..bell_only()
    });
    apply_snapshot(&mut state, failed);
    assert!(alerts(&advance(&mut state, Duration::from_secs(3))).is_empty());
    assert!(state.notifications.is_empty());
}

#[test]
fn archived_and_first_load_changes_are_ignored() {
    let mut state = AppState::new(ServerInfo::default(), Instant::now(), NOW_MS);
    apply_snapshot(&mut state, finished(&running_sidebar(), BACKGROUND));
    assert!(advance(&mut state, Duration::from_secs(3)).is_empty());

    let mut state = state_with(bell_only());
    let mut archived = finished(&running_sidebar(), BACKGROUND);
    patch(&mut archived, BACKGROUND, |entry| {
        entry.thread.archived_at = Some(NOW_MS);
    });
    apply_snapshot(&mut state, archived);
    assert!(alerts(&advance(&mut state, Duration::from_secs(3))).is_empty());
}

fn signal(id: &str, thread_id: Option<&str>, channels: &[&str]) -> Action {
    Action::PluginSignal {
        plugin_id: "push-notifications".into(),
        channel: "notification".into(),
        payload: json!({
            "id": id,
            "title": "Thread title",
            "body": "Done",
            "threadId": thread_id,
            "channels": channels,
        }),
    }
}

#[test]
fn plugin_signals_are_deduplicated_against_local_detection() {
    let mut state = state_with(bell_only());
    let effects = update(&mut state, signal("n1", None, &["desktop"]));
    assert_eq!(alerts(&effects).len(), 1);
    assert!(alerts(&update(&mut state, signal("n1", None, &["desktop"]))).is_empty());
    assert!(alerts(&update(&mut state, signal("n2", None, &[]))).is_empty());
    let other = Action::PluginSignal {
        plugin_id: "tasks".into(),
        channel: "notification".into(),
        payload: json!({"id": "n3", "title": "x", "body": "", "threadId": null, "channels": ["web"]}),
    };
    assert!(alerts(&update(&mut state, other)).is_empty());
    let garbage = Action::PluginSignal {
        plugin_id: "push-notifications".into(),
        channel: "notification".into(),
        payload: json!("nope"),
    };
    assert!(alerts(&update(&mut state, garbage)).is_empty());

    apply_snapshot(&mut state, finished(&running_sidebar(), BACKGROUND));
    assert!(
        alerts(&update(
            &mut state,
            signal("n4", Some(BACKGROUND), &["web"])
        ))
        .is_empty()
    );
    assert_eq!(
        alerts(&advance(&mut state, Duration::from_secs(3))).len(),
        1
    );
    assert!(
        alerts(&update(
            &mut state,
            signal("n5", Some(BACKGROUND), &["web"])
        ))
        .is_empty()
    );

    state.open(FOCUSED.into());
    assert!(alerts(&update(&mut state, signal("n6", Some(FOCUSED), &["web"]))).is_empty());
    let effects = update(&mut state, signal("n7", Some("thr_unread"), &["web"]));
    assert_eq!(alerts(&effects).len(), 1);
    let kinds: Vec<NoticeKind> = state
        .notifications
        .newest_first()
        .map(|notice| notice.kind)
        .collect();
    assert_eq!(
        kinds,
        [
            NoticeKind::Plugin,
            NoticeKind::TurnFinished,
            NoticeKind::Plugin
        ]
    );
}

#[test]
fn notification_center_lists_jumps_deletes_and_clears() {
    let mut state = state_with(bell_only());
    update(&mut state, signal("n1", None, &["web"]));
    apply_snapshot(&mut state, finished(&running_sidebar(), BACKGROUND));
    advance(&mut state, Duration::from_secs(3));
    assert_eq!(state.notifications.unread(), 2);

    keys(&mut state, "SPC n n");
    assert_eq!(state.mode, Mode::Notifications { selected: 0 });
    assert_eq!(state.notifications.unread(), 0);
    let screen = render_lines(&state, 110, 16).join("\n");
    assert!(screen.contains("Notifications (2)"), "{screen}");
    assert!(screen.contains("✓"), "{screen}");
    assert!(screen.contains("Refactor parser"), "{screen}");
    assert!(screen.contains("Thread title"), "{screen}");

    press(&mut state, KeyCode::Char('j'));
    assert_eq!(state.mode, Mode::Notifications { selected: 1 });
    press(&mut state, KeyCode::Enter);
    assert_eq!(state.mode, Mode::Notifications { selected: 1 });
    press(&mut state, KeyCode::Char('k'));
    let effects = press(&mut state, KeyCode::Enter);
    assert_eq!(state.mode, Mode::Normal);
    assert_eq!(state.open_thread.as_deref(), Some(BACKGROUND));
    assert_eq!(state.focus, Focus::Main);
    assert!(effects.contains(&Effect::WatchThread(Some(BACKGROUND.into()))));

    keys(&mut state, ": n o t i f RET");
    keys(&mut state, "d d");
    assert_eq!(state.notifications.len(), 1);
    press(&mut state, KeyCode::Char('C'));
    assert!(state.notifications.is_empty());
    let screen = render_lines(&state, 110, 16).join("\n");
    assert!(screen.contains("No notifications yet"), "{screen}");
    press(&mut state, KeyCode::Esc);
    assert_eq!(state.mode, Mode::Normal);
}

#[test]
fn terminal_title_tracks_attention_when_enabled() {
    let mut state = AppState::new(ServerInfo::default(), Instant::now(), NOW_MS);
    state.notifications = Notifications::new(NotificationPrefs {
        terminal_title: true,
        ..NotificationPrefs::default()
    });
    let effects = apply_snapshot(&mut state, sample_sidebar());
    assert!(effects.contains(&Effect::SetTitle("(2) bb".into())));
    let effects = apply_snapshot(&mut state, sample_sidebar());
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::SetTitle(_)))
    );
    let mut quiet = sample_sidebar();
    patch(&mut quiet, "thr_wait", |entry| {
        entry.has_pending_interaction = false
    });
    patch(&mut quiet, "thr_unread", |entry| {
        entry.thread.last_read_at = Some(NOW_MS)
    });
    let mut snapshot = (*snapshot_with(quiet)).clone();
    snapshot.attention.data = Some(SystemAttentionResponse {
        has_attention: true,
    });
    let effects = update(&mut state, Action::Snapshot(Arc::new(snapshot)));
    assert!(effects.contains(&Effect::SetTitle("● bb".into())));

    let state = loaded_state(sample_sidebar());
    assert!(state.desired_title().is_none());
}

#[tokio::test]
async fn event_loop_delivers_plugin_signals_and_fetches_interactions() {
    let mut sidebar = running_sidebar();
    patch(&mut sidebar, BACKGROUND, |entry| {
        entry.has_pending_interaction = false
    });
    let backend = FakeBackend::new(sidebar.clone());
    backend.interactions.lock().unwrap().insert(
        BACKGROUND.into(),
        vec![command_approval(BACKGROUND, "make")],
    );
    let api = MockApi::new(backend.clone());
    let platform = Arc::new(MockPlatform::default());
    let store = Store::spawn(backend.clone(), NoRealtime, None, StoreConfig::default());
    let (tx, mut rx) = mpsc::unbounded_channel::<Action>();
    let events = Box::pin(stream::poll_fn(move |cx| rx.poll_recv(cx)));
    let mut state = AppState::new(ServerInfo::default(), Instant::now(), NOW_MS);
    state.notifications = Notifications::new(bell_only());
    let task_platform = platform.clone();
    let handle = tokio::spawn(async move {
        let mut terminal = Terminal::new(TestBackend::new(100, 14)).unwrap();
        run(&mut terminal, &mut state, store, api, task_platform, events)
            .await
            .unwrap();
        state
    });
    tx.send(signal("sig-1", None, &["desktop"])).unwrap();
    wait_until(|| platform.alerts.lock().unwrap().len() == 1).await;

    wait_until(|| *backend.sidebar_fetches.lock().unwrap() >= 1).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    backend.patch_thread(BACKGROUND, |entry| {
        entry.has_pending_interaction = true;
        entry.thread.latest_attention_at = NOW_MS + 1_000;
    });
    for ch in [' ', 'b', 'r'] {
        tx.send(Action::Key(KeyEvent::from(KeyCode::Char(ch))))
            .unwrap();
    }
    wait_until(|| platform.alerts.lock().unwrap().len() == 2).await;
    for ch in [' ', 'q', 'q'] {
        tx.send(Action::Key(KeyEvent::from(KeyCode::Char(ch))))
            .unwrap();
    }
    let state = handle.await.unwrap();
    let bodies: Vec<String> = state
        .notifications
        .newest_first()
        .map(|notice| notice.body.clone())
        .collect();
    assert_eq!(bodies, ["Approve command: make", "Done"]);
}

async fn wait_until(predicate: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("condition");
}

#[tokio::test]
async fn realtime_stream_forwards_only_plugin_signals() {
    use bb_client::RealtimeEvent;
    use futures::StreamExt;
    let (tx, rx) = tokio::sync::broadcast::channel(8);
    let mut actions = bb_tui::runtime::realtime_actions(rx);
    tx.send(RealtimeEvent::Connected).unwrap();
    tx.send(RealtimeEvent::PluginSignal {
        plugin_id: "push-notifications".into(),
        channel: "notification".into(),
        payload: json!({"id": "x"}),
    })
    .unwrap();
    drop(tx);
    match actions.next().await {
        Some(Action::PluginSignal { plugin_id, .. }) => assert_eq!(plugin_id, "push-notifications"),
        other => panic!("unexpected {other:?}"),
    }
    assert!(actions.next().await.is_none());
}

#[test]
fn skipped_local_notice_also_suppresses_the_plugin_copy() {
    let mut state = state_with(NotificationPrefs {
        turn_finished: false,
        ..bell_only()
    });
    apply_snapshot(&mut state, finished(&running_sidebar(), BACKGROUND));
    assert!(alerts(&advance(&mut state, Duration::from_secs(3))).is_empty());
    assert!(
        alerts(&update(
            &mut state,
            signal("late", Some(BACKGROUND), &["web"])
        ))
        .is_empty()
    );
    assert!(state.notifications.is_empty());
}
