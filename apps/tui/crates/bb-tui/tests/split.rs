mod support;

use std::sync::Arc;
use std::time::Duration;

use bb_store::{NoRealtime, Store, StoreConfig};
use bb_tui::app::{Action, AppState, Effect, Focus, update};
use bb_tui::runtime::EffectRunner;
use crossterm::event::{KeyCode, KeyModifiers};
use support::*;
use tokio::sync::mpsc;

fn select(state: &mut AppState, thread_id: &str) {
    state.selected = state
        .rows
        .iter()
        .position(|row| row.thread().is_some_and(|thread| thread.id == thread_id))
        .expect("thread row");
}

fn split_state() -> (AppState, Vec<Effect>) {
    let mut state = loaded_state(sample_sidebar());
    state.open("thr_old".into());
    state.focus = Focus::Main;
    select(&mut state, "thr_other");
    let effects = press(&mut state, KeyCode::Char('V'));
    (state, effects)
}

fn alt(state: &mut AppState, ch: char) -> Vec<Effect> {
    press_mod(state, KeyCode::Char(ch), KeyModifiers::ALT)
}

#[test]
fn v_opens_the_selected_thread_beside_the_open_one() {
    let (state, effects) = split_state();
    assert_eq!(state.splits, vec!["thr_old", "thr_other"]);
    assert_eq!(state.open_thread.as_deref(), Some("thr_other"));
    assert_eq!(state.focus, Focus::Main);
    assert!(effects.contains(&Effect::WatchThread(Some("thr_other".into()))));
    assert!(effects.contains(&Effect::WatchSplits(vec![
        "thr_old".into(),
        "thr_other".into()
    ])));
    let lines = render_lines(&state, 160, 30);
    let header = lines
        .iter()
        .find(|line| line.contains("1 · Write docs"))
        .unwrap_or_else(|| panic!("{}", lines.join("\n")));
    assert!(header.contains("2 · Refactor parser"), "{header}");
}

#[test]
fn pane_focus_moves_between_splits_and_wraps() {
    let (mut state, _) = split_state();
    let effects = alt(&mut state, 'h');
    assert_eq!(state.open_thread.as_deref(), Some("thr_old"));
    assert!(effects.contains(&Effect::WatchThread(Some("thr_old".into()))));
    assert_eq!(state.splits, vec!["thr_old", "thr_other"]);
    alt(&mut state, 'l');
    assert_eq!(state.open_thread.as_deref(), Some("thr_other"));
    alt(&mut state, 'l');
    assert_eq!(state.open_thread.as_deref(), Some("thr_old"));
    press_mod(&mut state, KeyCode::Left, KeyModifiers::ALT);
    assert_eq!(state.open_thread.as_deref(), Some("thr_other"));
}

#[test]
fn opening_a_thread_replaces_the_focused_pane() {
    let (mut state, _) = split_state();
    alt(&mut state, 'h');
    state.focus = Focus::Sidebar;
    select(&mut state, "thr_unread");
    let effects = press(&mut state, KeyCode::Enter);
    assert_eq!(state.splits, vec!["thr_unread", "thr_other"]);
    assert_eq!(state.open_thread.as_deref(), Some("thr_unread"));
    assert!(effects.contains(&Effect::WatchSplits(vec![
        "thr_unread".into(),
        "thr_other".into()
    ])));
    state.focus = Focus::Sidebar;
    select(&mut state, "thr_other");
    press(&mut state, KeyCode::Enter);
    assert_eq!(state.splits, vec!["thr_unread", "thr_other"]);
    assert_eq!(state.open_thread.as_deref(), Some("thr_other"));
}

#[test]
fn tab_cycles_through_every_pane_and_its_composer() {
    let (mut state, _) = split_state();
    state.focus = Focus::Sidebar;
    let mut stops = Vec::new();
    for _ in 0..5 {
        press(&mut state, KeyCode::Tab);
        stops.push((state.focus, state.open_thread.clone().unwrap()));
    }
    assert_eq!(
        stops,
        vec![
            (Focus::Main, "thr_old".to_owned()),
            (Focus::Composer, "thr_old".to_owned()),
            (Focus::Main, "thr_other".to_owned()),
            (Focus::Composer, "thr_other".to_owned()),
            (Focus::Sidebar, "thr_other".to_owned()),
        ]
    );
}

#[test]
fn closing_panes_returns_to_a_single_thread() {
    let mut state = loaded_state(sample_sidebar());
    state.open("thr_old".into());
    state.focus = Focus::Main;
    for thread in ["thr_other", "thr_unread", "thr_wait"] {
        select(&mut state, thread);
        press(&mut state, KeyCode::Char('V'));
    }
    assert_eq!(state.splits.len(), 4);
    select(&mut state, "thr_old");
    alt(&mut state, 'z');
    assert!(state.split_maximized);
    let lines = render_lines(&state, 160, 30).join("\n");
    assert!(lines.contains("4 · Needs approval · 4 of 4"), "{lines}");
    assert!(!lines.contains("1 · Write docs"), "{lines}");
    alt(&mut state, 'z');

    let effects = press(&mut state, KeyCode::Char('X'));
    assert_eq!(state.splits, vec!["thr_old", "thr_other", "thr_unread"]);
    assert_eq!(state.open_thread.as_deref(), Some("thr_unread"));
    assert!(effects.contains(&Effect::WatchSplits(vec![
        "thr_old".into(),
        "thr_other".into(),
        "thr_unread".into()
    ])));
    press(&mut state, KeyCode::Char('X'));
    let effects = press(&mut state, KeyCode::Char('X'));
    assert!(state.splits.is_empty());
    assert!(effects.contains(&Effect::WatchSplits(Vec::new())));
    assert_eq!(state.open_thread.as_deref(), Some("thr_old"));
    press(&mut state, KeyCode::Char('X'));
    assert_eq!(state.open_thread.as_deref(), Some("thr_old"));
    assert!(
        state
            .toasts
            .iter()
            .any(|toast| toast.message == "Only one pane is open")
    );
}

#[test]
fn a_fifth_pane_is_refused() {
    let mut state = loaded_state(sample_sidebar());
    let mut sidebar = sample_sidebar();
    sidebar.projects[1]
        .threads
        .push(thread("thr_extra", "proj_b", "Extra work", 20));
    update(&mut state, Action::Snapshot(snapshot_with(sidebar)));
    state.open("thr_old".into());
    for thread in ["thr_other", "thr_unread", "thr_wait", "thr_extra"] {
        select(&mut state, thread);
        press(&mut state, KeyCode::Char('V'));
    }
    assert_eq!(state.splits.len(), 4);
    assert!(!state.splits.contains(&"thr_extra".to_owned()));
    assert!(
        state
            .toasts
            .iter()
            .any(|toast| toast.message.starts_with("At most 4 panes"))
    );
}

#[test]
fn background_panes_follow_their_timelines() {
    let (mut state, _) = split_state();
    let mut snapshot = (*state.snapshot).clone();
    snapshot.timelines.insert(
        "thr_old".into(),
        timeline_entry(timeline_response(
            vec![assistant("msg_bg", 5, "background reply")],
            false,
        )),
    );
    snapshot.version += 1;
    update(&mut state, Action::Snapshot(Arc::new(snapshot)));
    assert_eq!(state.open_thread.as_deref(), Some("thr_other"));
    let lines = render_lines(&state, 160, 30).join("\n");
    assert!(lines.contains("background reply"), "{lines}");
}

#[tokio::test]
async fn runner_keeps_every_split_thread_watched() {
    let backend = FakeBackend::new(sample_sidebar());
    let api = MockApi::new(backend.clone());
    let store = Store::spawn(backend.clone(), NoRealtime, None, StoreConfig::default());
    let (tx, _rx) = mpsc::unbounded_channel::<Action>();
    let mut runner = EffectRunner::new(store.clone(), api, Arc::new(MockPlatform::default()), tx);
    runner.run(Effect::WatchThread(Some("thr_other".into())));
    runner.run(Effect::WatchSplits(vec![
        "thr_old".into(),
        "thr_other".into(),
    ]));
    assert_eq!(runner.watched_splits(), vec!["thr_old", "thr_other"]);
    let mut rx = store.subscribe();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            {
                let snapshot = rx.borrow_and_update();
                if snapshot.timelines.contains_key("thr_old")
                    && snapshot.timelines.contains_key("thr_other")
                {
                    break;
                }
            }
            rx.changed().await.expect("store alive");
        }
    })
    .await
    .expect("both timelines fetched");
    runner.run(Effect::WatchSplits(Vec::new()));
    assert!(runner.watched_splits().is_empty());
    assert_eq!(runner.watched_thread(), Some("thr_other"));
}
