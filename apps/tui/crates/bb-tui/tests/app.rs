mod support;

use std::sync::Arc;
use std::time::Duration;

use bb_store::{ConnectionState, StoreSnapshot};
use bb_tui::api::Mutation;
use bb_tui::app::{Action, AppState, Effect, Focus, Mode, ToastLevel, update};
use bb_tui::sidebar::{RowKey, SidebarRow};
use bb_wire::system::{AppKeybindingOverride, AppShortcut};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use support::*;

fn press(state: &mut AppState, code: KeyCode) -> Vec<Effect> {
    update(state, Action::Key(KeyEvent::from(code)))
}

fn press_mod(state: &mut AppState, code: KeyCode, modifiers: KeyModifiers) -> Vec<Effect> {
    update(state, Action::Key(KeyEvent::new(code, modifiers)))
}

fn type_text(state: &mut AppState, text: &str) {
    for ch in text.chars() {
        press(state, KeyCode::Char(ch));
    }
}

fn selected_key(state: &AppState) -> Option<RowKey> {
    state.selected_row().map(SidebarRow::key)
}

fn thread_key(id: &str) -> Option<RowKey> {
    Some(RowKey::Thread(id.into()))
}

fn screen(state: &AppState) -> String {
    render_lines(state, 100, 16).join("\n")
}

#[test]
fn sidebar_renders_tree_glyphs_times_and_status_bar() {
    let state = loaded_state(sample_sidebar());
    let lines = render_lines(&state, 100, 12);
    let text = lines.join("\n");
    assert!(lines[1].contains("▾ alpha"), "{text}");
    assert!(lines[1].contains("3┃"), "{text}");
    assert!(lines[2].contains(" ? Needs approval"), "{text}");
    assert!(lines[2].contains("3m"), "{text}");
    assert!(lines[3].contains(" ● Fix flaky test"), "{text}");
    assert!(lines[3].contains("30m"), "{text}");
    assert!(lines[4].contains("Write docs"), "{text}");
    assert!(lines[4].contains("1d"), "{text}");
    assert!(lines[5].contains("▾ beta"), "{text}");
    assert!(lines[6].contains("Refactor parser"), "{text}");
    assert!(lines[7].contains("▾ Personal"), "{text}");
    let status = lines.last().unwrap();
    assert!(status.contains("http://127.0.0.1:4000/"), "{status}");
    assert!(status.contains("● connected"), "{status}");
    assert!(status.contains("⚑ 2"), "{status}");
    assert!(status.contains("? help"), "{status}");
    assert!(text.contains("Select a thread and press Enter"), "{text}");
}

#[test]
fn first_load_selects_first_thread_and_vim_keys_move() {
    let mut state = loaded_state(sample_sidebar());
    assert_eq!(selected_key(&state), thread_key("thr_wait"));
    press(&mut state, KeyCode::Char('j'));
    assert_eq!(selected_key(&state), thread_key("thr_unread"));
    press(&mut state, KeyCode::Char('k'));
    press(&mut state, KeyCode::Char('k'));
    assert_eq!(selected_key(&state), Some(RowKey::Group("proj_a".into())));
    press(&mut state, KeyCode::Char('k'));
    assert_eq!(selected_key(&state), Some(RowKey::Group("proj_a".into())));
    press_mod(&mut state, KeyCode::Char('G'), KeyModifiers::SHIFT);
    assert_eq!(
        selected_key(&state),
        Some(RowKey::Group("proj_personal".into()))
    );
    press(&mut state, KeyCode::Char('g'));
    assert_eq!(state.selected, 0);
}

#[test]
fn enter_opens_thread_marks_unread_read_and_focuses_main() {
    let mut state = loaded_state(sample_sidebar());
    press(&mut state, KeyCode::Char('j'));
    let effects = press(&mut state, KeyCode::Enter);
    assert_eq!(
        effects,
        vec![
            Effect::WatchThread(Some("thr_unread".into())),
            Effect::Mutate(Mutation::MarkRead {
                thread_id: "thr_unread".into()
            }),
        ]
    );
    assert_eq!(state.focus, Focus::Main);
    assert_eq!(state.open_thread.as_deref(), Some("thr_unread"));
    let text = screen(&state);
    assert!(text.contains("┏ Fix flaky test ━"), "{text}");
    assert!(
        text.contains("status: unknown  ● Unread thread succeeded"),
        "{text}"
    );
    assert!(text.contains("project: alpha"), "{text}");
    assert!(text.contains("unread"), "{text}");

    press(&mut state, KeyCode::Char('j'));
    assert_eq!(selected_key(&state), thread_key("thr_unread"));
    press(&mut state, KeyCode::Esc);
    assert_eq!(state.focus, Focus::Sidebar);
    press(&mut state, KeyCode::Char('k'));
    let effects = press(&mut state, KeyCode::Enter);
    assert_eq!(effects, vec![Effect::WatchThread(Some("thr_wait".into()))]);
}

#[test]
fn enter_on_group_and_h_l_collapse_and_expand() {
    let mut state = loaded_state(sample_sidebar());
    press(&mut state, KeyCode::Char('h'));
    assert_eq!(selected_key(&state), Some(RowKey::Group("proj_a".into())));
    press(&mut state, KeyCode::Char('h'));
    assert!(state.collapsed.contains("proj_a"));
    let lines = render_lines(&state, 100, 12);
    assert!(lines[1].contains("▸ alpha"), "{lines:?}");
    assert!(lines[1].contains("●2 3"), "{lines:?}");
    assert!(lines[2].contains("▾ beta"), "{lines:?}");
    press(&mut state, KeyCode::Enter);
    assert!(!state.collapsed.contains("proj_a"));
    press(&mut state, KeyCode::Char(' '));
    assert!(state.collapsed.contains("proj_a"));
    press(&mut state, KeyCode::Char('l'));
    assert!(!state.collapsed.contains("proj_a"));
}

#[test]
fn child_threads_expand_under_parent() {
    let mut sidebar = sample_sidebar();
    let mut child = thread("thr_child", "proj_a", "Subagent", 1);
    child.thread.parent_thread_id = Some("thr_wait".into());
    sidebar.projects[0].threads.push(child);
    let mut state = loaded_state(sidebar);
    let lines = render_lines(&state, 100, 12);
    assert!(lines[2].contains("▸? Needs approval"), "{lines:?}");
    assert!(lines[2].contains("+1 3m"), "{lines:?}");
    press(&mut state, KeyCode::Char('l'));
    let lines = render_lines(&state, 100, 12);
    assert!(lines[2].contains("▾? Needs approval"), "{lines:?}");
    assert!(lines[3].contains("     Subagent"), "{lines:?}");
    press(&mut state, KeyCode::Char('j'));
    assert_eq!(selected_key(&state), thread_key("thr_child"));
    press(&mut state, KeyCode::Char('h'));
    assert_eq!(selected_key(&state), thread_key("thr_wait"));
    press(&mut state, KeyCode::Char('h'));
    assert!(!state.expanded_threads.contains("thr_wait"));
}

#[test]
fn slash_filters_threads_and_esc_clears() {
    let mut state = loaded_state(sample_sidebar());
    press(&mut state, KeyCode::Char('/'));
    assert!(matches!(state.mode, Mode::Filter(_)));
    type_text(&mut state, "parser");
    assert_eq!(state.filter, "parser");
    assert_eq!(selected_key(&state), thread_key("thr_other"));
    let lines = render_lines(&state, 100, 12);
    assert!(lines[1].contains("/parser"), "{lines:?}");
    assert!(lines[2].contains("▾ beta"), "{lines:?}");
    assert!(lines[3].contains("Refactor parser"), "{lines:?}");
    assert!(!lines.join("\n").contains("alpha"), "{lines:?}");
    press(&mut state, KeyCode::Enter);
    assert_eq!(state.mode, Mode::Normal);
    assert_eq!(state.filter, "parser");
    press(&mut state, KeyCode::Char('q'));
    assert!(state.quit);

    let mut state = loaded_state(sample_sidebar());
    press(&mut state, KeyCode::Char('/'));
    type_text(&mut state, "zzzz");
    assert!(screen(&state).contains("No matching threads"));
    press(&mut state, KeyCode::Esc);
    assert_eq!(state.filter, "");
    assert_eq!(state.rows.len(), 7);
    press(&mut state, KeyCode::Char('/'));
    type_text(&mut state, "docs");
    press(&mut state, KeyCode::Enter);
    press(&mut state, KeyCode::Esc);
    assert_eq!(state.filter, "");
    assert_eq!(selected_key(&state), thread_key("thr_old"));
}

#[test]
fn rename_inline_submits_mutation_and_cancel_does_not() {
    let mut state = loaded_state(sample_sidebar());
    press(&mut state, KeyCode::Char('r'));
    let Mode::Rename { thread_id, input } = &state.mode else {
        panic!("rename mode");
    };
    assert_eq!(thread_id, "thr_wait");
    assert_eq!(input.text(), "Needs approval");
    press_mod(&mut state, KeyCode::Char('u'), KeyModifiers::CONTROL);
    type_text(&mut state, "Ship it q");
    let lines = render_lines(&state, 100, 12);
    assert!(lines[2].contains("? Ship it q"), "{lines:?}");
    assert!(!state.quit);
    let effects = press(&mut state, KeyCode::Enter);
    assert_eq!(
        effects,
        vec![Effect::Mutate(Mutation::Rename {
            thread_id: "thr_wait".into(),
            title: "Ship it q".into()
        })]
    );
    assert_eq!(state.mode, Mode::Normal);

    press(&mut state, KeyCode::Char('r'));
    type_text(&mut state, "!!");
    assert!(press(&mut state, KeyCode::Esc).is_empty());
    press(&mut state, KeyCode::Char('r'));
    assert!(press(&mut state, KeyCode::Enter).is_empty());
}

#[test]
fn archive_undo_pin_and_read_toggles() {
    let mut state = loaded_state(sample_sidebar());
    let archive = press(&mut state, KeyCode::Char('a'));
    assert_eq!(
        archive,
        vec![Effect::Mutate(Mutation::Archive {
            thread_id: "thr_wait".into()
        })]
    );
    let undo = press(&mut state, KeyCode::Char('u'));
    assert_eq!(
        undo,
        vec![Effect::Mutate(Mutation::Unarchive {
            thread_id: "thr_wait".into()
        })]
    );
    assert!(press(&mut state, KeyCode::Char('u')).is_empty());
    assert!(state.toasts.iter().any(|t| t.message == "Nothing to undo"));

    assert_eq!(
        press(&mut state, KeyCode::Char('p')),
        vec![Effect::Mutate(Mutation::Pin {
            thread_id: "thr_wait".into()
        })]
    );
    assert_eq!(
        press(&mut state, KeyCode::Char('m')),
        vec![Effect::Mutate(Mutation::MarkUnread {
            thread_id: "thr_wait".into()
        })]
    );
    press(&mut state, KeyCode::Char('j'));
    assert_eq!(
        press(&mut state, KeyCode::Char('m')),
        vec![Effect::Mutate(Mutation::MarkRead {
            thread_id: "thr_unread".into()
        })]
    );

    let mut sidebar = sample_sidebar();
    sidebar.projects[1].threads[0].thread.pinned_at = Some(1);
    let mut state = loaded_state(sidebar);
    assert_eq!(selected_key(&state), thread_key("thr_other"));
    assert!(screen(&state).contains("▾ Pinned"));
    assert_eq!(
        press(&mut state, KeyCode::Char('p')),
        vec![Effect::Mutate(Mutation::Unpin {
            thread_id: "thr_other".into()
        })]
    );
}

#[test]
fn mutation_results_show_toasts_that_expire() {
    let mut state = loaded_state(sample_sidebar());
    let start = state.clock;
    let effects = update(
        &mut state,
        Action::MutationFinished {
            mutation: Mutation::Pin {
                thread_id: "thr_old".into(),
            },
            result: Ok(()),
        },
    );
    assert_eq!(effects, vec![Effect::RefreshSidebar]);
    press(&mut state, KeyCode::Char('a'));
    update(
        &mut state,
        Action::MutationFinished {
            mutation: Mutation::Archive {
                thread_id: "thr_wait".into(),
            },
            result: Err("boom".into()),
        },
    );
    assert_eq!(state.last_archived, None);
    let text = screen(&state);
    assert!(text.contains("Pinned: Write docs"), "{text}");
    assert!(text.contains("Failed to archive: boom"), "{text}");

    update(
        &mut state,
        Action::Tick {
            now: start + Duration::from_secs(5),
            now_ms: NOW_MS,
        },
    );
    assert_eq!(state.toasts.len(), 1);
    assert_eq!(state.toasts[0].level, ToastLevel::Error);
    update(
        &mut state,
        Action::Tick {
            now: start + Duration::from_secs(9),
            now_ms: NOW_MS,
        },
    );
    assert!(state.toasts.is_empty());

    let silent = update(
        &mut state,
        Action::MutationFinished {
            mutation: Mutation::MarkRead {
                thread_id: "thr_old".into(),
            },
            result: Ok(()),
        },
    );
    assert_eq!(silent, vec![Effect::RefreshSidebar]);
    assert!(state.toasts.is_empty());
}

#[test]
fn disconnect_shows_banner_and_status() {
    let mut state = loaded_state(sample_sidebar());
    let mut snapshot = StoreSnapshot::clone(&state.snapshot);
    snapshot.connection = ConnectionState::Disconnected {
        retry_in: Some(Duration::from_secs(4)),
        error: Some("connection refused".into()),
    };
    update(&mut state, Action::Snapshot(Arc::new(snapshot)));
    let lines = render_lines(&state, 100, 12);
    assert_eq!(
        lines[0],
        " Disconnected from server: connection refused · retrying in 4s"
    );
    assert!(lines.last().unwrap().contains("✗ disconnected"));

    let mut snapshot = StoreSnapshot::clone(&state.snapshot);
    snapshot.connection = ConnectionState::Connecting { reconnect: true };
    update(&mut state, Action::Snapshot(Arc::new(snapshot)));
    let lines = render_lines(&state, 100, 12);
    assert!(lines[0].starts_with("┏"), "{lines:?}");
    assert!(lines.last().unwrap().contains("◌ reconnecting"));
}

#[test]
fn sidebar_load_error_and_loading_states() {
    let mut state = loaded_state(sample_sidebar());
    let mut snapshot = StoreSnapshot::default();
    snapshot.sidebar.error = Some(bb_store::FetchError::new("HTTP 500"));
    update(&mut state, Action::Snapshot(Arc::new(snapshot)));
    let lines = render_lines(&state, 80, 8);
    assert_eq!(lines[0], " Failed to load threads: HTTP 500");
    assert!(lines[2].contains("No threads"), "{lines:?}");

    update(
        &mut state,
        Action::Snapshot(Arc::new(StoreSnapshot::default())),
    );
    assert!(screen(&state).contains("Loading…"));
}

#[test]
fn focus_cycles_through_visible_panes() {
    let mut state = loaded_state(sample_sidebar());
    press(&mut state, KeyCode::Tab);
    assert_eq!(state.focus, Focus::Main);
    press(&mut state, KeyCode::Tab);
    assert_eq!(state.focus, Focus::Sidebar);
    press(&mut state, KeyCode::Char('i'));
    press_mod(&mut state, KeyCode::BackTab, KeyModifiers::SHIFT);
    assert_eq!(state.focus, Focus::Panel);
    assert!(screen(&state).contains("┏ Panel ━"));
    press(&mut state, KeyCode::Char('i'));
    assert_eq!(state.focus, Focus::Main);
    press_mod(&mut state, KeyCode::Char('b'), KeyModifiers::CONTROL);
    assert!(!state.sidebar_visible);
    press(&mut state, KeyCode::Tab);
    assert_eq!(state.focus, Focus::Main);
    assert!(!screen(&state).contains("alpha"));
    press(&mut state, KeyCode::Char('/'));
    assert!(state.sidebar_visible);
    assert_eq!(state.focus, Focus::Sidebar);
}

#[test]
fn thread_next_previous_and_jump_open_threads() {
    let mut state = loaded_state(sample_sidebar());
    let effects = press_mod(&mut state, KeyCode::Char('J'), KeyModifiers::SHIFT);
    assert_eq!(effects[0], Effect::WatchThread(Some("thr_unread".into())));
    press_mod(&mut state, KeyCode::Char('J'), KeyModifiers::SHIFT);
    press_mod(&mut state, KeyCode::Char('J'), KeyModifiers::SHIFT);
    assert_eq!(state.open_thread.as_deref(), Some("thr_other"));
    press_mod(&mut state, KeyCode::Char('J'), KeyModifiers::SHIFT);
    assert_eq!(state.open_thread.as_deref(), Some("thr_wait"));
    press_mod(&mut state, KeyCode::Char('K'), KeyModifiers::SHIFT);
    assert_eq!(state.open_thread.as_deref(), Some("thr_other"));
    press_mod(&mut state, KeyCode::Char('3'), KeyModifiers::ALT);
    assert_eq!(state.open_thread.as_deref(), Some("thr_old"));
    assert_eq!(selected_key(&state), thread_key("thr_old"));
    assert!(press_mod(&mut state, KeyCode::Char('9'), KeyModifiers::ALT).is_empty());
}

#[test]
fn help_overlay_lists_keys_and_closes_on_any_key() {
    let mut state = loaded_state(sample_sidebar());
    press(&mut state, KeyCode::Char('?'));
    let text = render_lines(&state, 100, 40).join("\n");
    assert!(text.contains("Keys"), "{text}");
    assert!(text.contains("│ r        Rename thread"), "{text}");
    assert!(text.contains("M-1…M-9"), "{text}");
    assert!(text.contains("j/↓"), "{text}");
    press(&mut state, KeyCode::Char('q'));
    assert!(!state.quit);
    assert_eq!(state.mode, Mode::Normal);
}

#[test]
fn keybinding_overrides_rebind_commands() {
    let mut state = loaded_state(sample_sidebar());
    update(
        &mut state,
        Action::KeymapLoaded(Ok(vec![AppKeybindingOverride {
            command: "thread.rename".into(),
            shortcut: Some(AppShortcut {
                key: "F2".into(),
                ..AppShortcut::default()
            }),
            platform: None,
        }])),
    );
    assert!(press(&mut state, KeyCode::Char('r')).is_empty());
    assert_eq!(state.mode, Mode::Normal);
    press(&mut state, KeyCode::F(2));
    assert!(matches!(state.mode, Mode::Rename { .. }));
    assert!(
        state
            .toasts
            .iter()
            .any(|toast| toast.message == "Applied 1 keybinding override(s)")
    );
}

#[test]
fn snapshot_updates_keep_selection_on_same_thread() {
    let mut state = loaded_state(sample_sidebar());
    press(&mut state, KeyCode::Char('j'));
    assert_eq!(selected_key(&state), thread_key("thr_unread"));
    let mut sidebar = sample_sidebar();
    sidebar.projects[0].threads[0].thread.latest_attention_at = NOW_MS;
    sidebar.projects[0].threads[0].thread.last_read_at = Some(NOW_MS);
    update(&mut state, Action::Snapshot(snapshot_with(sidebar)));
    assert_eq!(selected_key(&state), thread_key("thr_unread"));
    assert_eq!(
        state.rows[1].thread().map(|row| row.id.as_str()),
        Some("thr_old")
    );

    let mut sidebar = sample_sidebar();
    sidebar.projects[0].threads.remove(1);
    update(&mut state, Action::Snapshot(snapshot_with(sidebar)));
    assert_eq!(state.selected, 3);
    assert_eq!(selected_key(&state), Some(RowKey::Group("proj_b".into())));
}

#[test]
fn sidebar_scrolls_to_keep_selection_visible() {
    let mut sidebar = sample_sidebar();
    for n in 0..30 {
        sidebar.projects[1].threads.push(thread(
            &format!("thr_{n:02}"),
            "proj_b",
            &format!("Task {n:02}"),
            100 + n,
        ));
    }
    let mut state = loaded_state(sidebar);
    press_mod(&mut state, KeyCode::Char('G'), KeyModifiers::SHIFT);
    let lines = render_lines(&state, 80, 12);
    assert!(lines[9].contains("Personal"), "{lines:?}");
    press(&mut state, KeyCode::Char('k'));
    let lines = render_lines(&state, 80, 12);
    assert!(lines[8].contains("Task 29"), "{lines:?}");
    press(&mut state, KeyCode::Char('g'));
    let lines = render_lines(&state, 80, 12);
    assert!(lines[1].contains("alpha"), "{lines:?}");
}
